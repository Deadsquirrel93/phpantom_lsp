//! Static method call return-type resolution: `ClassName::method()`,
//! `self::`/`static::`/`parent::` calls, and `$var::method()` on a
//! class-string or an object.

use std::sync::Arc;

use mago_syntax::cst::*;

use crate::Backend;
use crate::atom::bytes_to_str;
use crate::php_type::{PhpType, TypeKind};
use crate::types::{ClassInfo, ResolvedType};

use crate::type_engine::call_resolution::MethodReturnCtx;
use crate::type_engine::resolver::VarResolutionCtx;
use crate::type_engine::variable::raw_type_inference::extract_arg_texts_from_ast;
use crate::type_engine::variable::resolution::build_var_resolver_from_ctx;

use super::laravel_calls::{try_resolve_config_method_type, try_resolve_trans_method_type};
use super::method_calls::resolve_member_call_on_receiver;
use super::owner_method_call::resolve_owner_method_call;
use super::{resolve_rhs_expression, resolve_var_types};

/// Resolve a static method call: `ClassName::method()`, `self::method()`,
/// `static::method()`.
pub(super) fn resolve_rhs_static_call(
    static_call: &StaticMethodCall<'_>,
    ctx: &VarResolutionCtx<'_>,
) -> Vec<ResolvedType> {
    // `Cls::{$expr}()` / `Cls::$name()` — see `runtime_named_member_type`.
    let ClassLikeMemberSelector::Identifier(ident) = &static_call.method else {
        return super::runtime_named_member_type();
    };
    let method_name = bytes_to_str(ident.value);

    let class_name = match static_call.class {
        Expression::Self_(_)
        | Expression::Static(_)
        | Expression::Parent(_)
        | Expression::Identifier(_) => crate::class_lookup::class_expression_name(
            static_call.class,
            ctx.current_class,
            ctx.all_classes,
            ctx.source_class_loader,
            ctx.source_imports,
        ),
        // ── `$var::method()` where `$var` holds a class-string ──
        Expression::Variable(Variable::Direct(dv)) => {
            let var_name = bytes_to_str(dv.name);
            let targets =
                crate::type_engine::variable::class_string_resolution::resolve_class_string_targets(
                    var_name,
                    ctx.cursor_offset,
                    ctx,
                );
            // When there are multiple possible class targets (union class-string),
            // resolve the method return type through each and union the results.
            if targets.len() > 1 {
                return static_call_on_class_strings(&targets, method_name, static_call, ctx);
            }
            if let Some(first) = targets.first() {
                Some(first.fqn().to_string())
            } else {
                // Fallback: resolve the variable's type and extract the
                // inner type from `class-string<T>`.  This handles
                // parameters typed as `@param class-string<Foo> $var`
                // where there is no `$var = Foo::class` assignment.
                let resolved = resolve_var_types(var_name, ctx, ctx.cursor_offset);
                let class_string = class_string_class_name(&resolved);
                if class_string.is_none() && !resolved.is_empty() {
                    return static_call_on_object(static_call, resolved, ctx);
                }
                class_string
            }
        }
        // `$this->prop::method()`, `getFoo()::method()`: the class is the
        // one a class-string names, or else the class of the object.
        _ => {
            let resolved = resolve_rhs_expression(static_call.class, ctx);
            let class_string = resolved.iter().find_map(|rt| match rt.type_string.kind() {
                TypeKind::ClassString(Some(inner)) => inner.base_name().map(|s| s.to_string()),
                _ => None,
            });
            if class_string.is_none() && !resolved.is_empty() {
                return static_call_on_object(static_call, resolved, ctx);
            }
            class_string
        }
    };
    let Some(cls_name) = class_name else {
        return vec![];
    };
    let Some(owner) = (ctx.class_loader)(&cls_name)
        .map(Arc::unwrap_or_clone)
        .or_else(|| {
            crate::class_lookup::find_class_by_fqn(ctx.all_classes, &cls_name)
                .map(|c| ClassInfo::clone(c))
        })
    else {
        return vec![];
    };
    static_call_on_class(&owner, method_name, static_call, ctx)
}

/// `ClassName::method()` once the class the call names is known.
fn static_call_on_class(
    owner: &ClassInfo,
    method_name: &str,
    static_call: &StaticMethodCall<'_>,
    ctx: &VarResolutionCtx<'_>,
) -> Vec<ResolvedType> {
    let concrete_owner = crate::type_engine::call_resolution::facade_concrete_owner(
        owner,
        method_name,
        ctx.class_loader,
        ctx.resolved_class_cache,
        ctx.backend,
    );
    let owner = concrete_owner.as_ref().unwrap_or(owner);
    // `parent::get()` (or `Foo::get()` written out) from inside a
    // class that `@extends Foo<Dog>` reads the parent's `T` as the
    // `Dog` the child bound it to, just as `$this->get()` does.
    let bound_owner = (!owner.template_params.is_empty())
        .then(|| crate::inheritance::extends_type_args(ctx.current_class, owner))
        .flatten()
        .map(|args| {
            crate::virtual_members::resolve_class_fully_with_type_args(
                owner,
                ctx.class_loader,
                ctx.resolved_class_cache,
                &args,
            )
        });
    let owner = bound_owner.as_deref().unwrap_or(owner);

    if let Some(result) =
        try_resolve_config_method_type(&owner.fqn(), method_name, &static_call.argument_list, ctx)
    {
        return result;
    }

    if let Some(result) =
        try_resolve_trans_method_type(&owner.fqn(), method_name, &static_call.argument_list, ctx)
    {
        return result;
    }

    let arg_texts = extract_arg_texts_from_ast(&static_call.argument_list, ctx.content);
    let arg_refs: Vec<&str> = arg_texts.iter().map(|s| s.as_str()).collect();
    let rctx = ctx.as_resolution_ctx();
    let template_subs = Backend::build_method_template_subs(owner, method_name, &arg_refs, &rctx);
    let owner_key = owner.fqn();
    // `self::`, `static::`, and `parent::` all forward late static binding, so
    // `@return static` on the target stays bound to the class the call is made
    // *from* — including `parent::`, which reads the annotation off the parent
    // but still resolves `static` to the current class.
    let forwards_lsb = matches!(
        static_call.class,
        Expression::Self_(_) | Expression::Static(_) | Expression::Parent(_)
    );
    // An explicit `A::` on a non-static method is PHP's pre-8
    // instance-forwarding form, which keeps `$this` (and with it late
    // static binding) bound, so only a `static` method written out
    // fixes the class. That forwarding needs a `$this` to forward
    // from, so it does not apply outside any class (global-scope
    // code has no enclosing class to bind `static` open over).
    let target_is_static = method_is_static(owner, method_name, ctx);
    let lsb_class = (!ctx.current_class.name.is_empty() && (forwards_lsb || !target_is_static))
        .then(|| ctx.current_class.fqn());
    let self_replace = |ty: &PhpType| ty.replace_self_bound(&owner_key, lsb_class.as_deref());

    let mut results = resolve_owner_method_call(
        owner,
        method_name,
        &static_call.argument_list,
        ctx,
        true,
        &template_subs,
        &self_replace,
    );
    // `Model::factory(…)`, `UserFactory::times(3)` and
    // `UserFactory::new()` open a factory chain, and what they
    // were opened with is what the `create()` at the far end of
    // it builds.  `factory($count)` only settles that once its
    // argument is resolved, which is why the type is fetched
    // lazily rather than for every static call in the file.
    let first_arg_type = || {
        let arg = static_call.argument_list.arguments.first()?;
        let resolved = resolve_rhs_expression(arg.value(), ctx);
        (!resolved.is_empty()).then(|| ResolvedType::types_joined(&resolved))
    };
    crate::virtual_members::laravel::tag_static_factory_call(
        &mut results,
        method_name,
        arg_refs.first().copied(),
        &first_arg_type,
        &rctx,
    );
    results
}

/// `$var::method()` where `$var` may hold any of several class-strings:
/// the method resolved on each class, joined.
fn static_call_on_class_strings(
    targets: &[ClassInfo],
    method_name: &str,
    static_call: &StaticMethodCall<'_>,
    ctx: &VarResolutionCtx<'_>,
) -> Vec<ResolvedType> {
    let arg_texts = extract_arg_texts_from_ast(&static_call.argument_list, ctx.content);
    let arg_refs: Vec<&str> = arg_texts.iter().map(|s| s.as_str()).collect();
    let text_args = arg_texts.join(", ");
    let rctx = ctx.as_resolution_ctx();
    let var_resolver = build_var_resolver_from_ctx(ctx);
    let mut union_types: Vec<PhpType> = Vec::new();
    let mut union_classes: Vec<ResolvedType> = Vec::new();
    for target in targets {
        let template_subs =
            Backend::build_method_template_subs(target, method_name, &arg_refs, &rctx);
        // Get the method's return type string.
        let merged = crate::virtual_members::resolve_class_fully_maybe_cached(
            target,
            ctx.class_loader,
            ctx.resolved_class_cache,
        );
        let method_ref = target
            .get_method_ci(method_name)
            .or_else(|| merged.get_method_ci(method_name));
        if let Some(m) = method_ref {
            if let Some(ref ret) = m.return_type {
                let substituted = if !template_subs.is_empty() {
                    ret.substitute(&template_subs)
                } else {
                    ret.clone()
                };
                // Each target is a concrete class-string, so late static
                // binding has nothing left to resolve on this branch.
                union_types.push(substituted.replace_self_bound(&target.fqn(), None));
            }
            continue;
        }
        let mr_ctx = MethodReturnCtx {
            all_classes: ctx.all_classes,
            class_loader: ctx.class_loader,
            backend: ctx.backend,
            template_subs: &template_subs,
            var_resolver: Some(&var_resolver),
            cache: ctx.resolved_class_cache,
            calling_class_name: Some(ctx.current_class.fqn().as_str()),
            written_class: Some(&|name: &str| ctx.written_names().resolve_class(name)),
            is_static: true,
            // Only reached when the target has no such method, so the
            // call resolves through a magic method with no parameters to
            // seed.
            call_args: None,
        };
        let results = Backend::resolve_method_return_types_with_args(
            target,
            method_name,
            &text_args,
            &mr_ctx,
        );
        for r in results {
            union_classes.push(ResolvedType::from_both_arc(PhpType::named(r.fqn()), r));
        }
    }
    if union_types.is_empty() {
        // Either no target yielded a return type, or every one answered
        // through a magic method.
        return union_classes;
    }
    // Build a unified type from all resolved return types, including the
    // class each target that only has a magic method for the call returns.
    union_types.extend(union_classes.into_iter().map(|rt| rt.type_string));
    let combined = PhpType::union(union_types);
    let resolved_classes = crate::type_engine::type_resolution::type_hint_to_classes_typed(
        &combined,
        &ctx.current_class.fqn(),
        ctx.all_classes,
        ctx.class_loader,
    );
    if !resolved_classes.is_empty() {
        return ResolvedType::from_classes_with_hint(resolved_classes, combined);
    }
    vec![ResolvedType::from_type_string(combined)]
}

/// The class a `class-string<Foo>` among `resolved` names, looking through
/// a nullable and a union.
fn class_string_class_name(resolved: &[ResolvedType]) -> Option<String> {
    fn named(ty: &PhpType) -> Option<String> {
        match ty.kind() {
            TypeKind::ClassString(Some(inner)) => inner.base_name().map(|s| s.to_string()),
            TypeKind::Nullable(inner) => match inner.kind() {
                TypeKind::ClassString(Some(cs_inner)) => {
                    cs_inner.base_name().map(|s| s.to_string())
                }
                _ => None,
            },
            _ => None,
        }
    }
    resolved.iter().find_map(|rt| match rt.type_string.kind() {
        TypeKind::Union(members) => members.iter().find_map(named),
        _ => named(&rt.type_string),
    })
}

/// `$obj::method()` where `$obj` holds an object rather than a class-string.
///
/// PHP calls the method on the object's own class, so this resolves exactly
/// like `$obj->method()` would: a union receiver unions the results, an
/// intersection intersects them, and `static` binds to the receiver.
fn static_call_on_object(
    static_call: &StaticMethodCall<'_>,
    resolved: Vec<ResolvedType>,
    ctx: &VarResolutionCtx<'_>,
) -> Vec<ResolvedType> {
    let classes = ResolvedType::into_arced_classes(resolved.clone());
    if classes.is_empty() {
        return vec![];
    }
    resolve_member_call_on_receiver(
        static_call.class,
        &static_call.method,
        &static_call.argument_list,
        Some((classes, resolved)),
        true,
        ctx,
    )
}

/// Whether `method_name` on `class_info` is declared `static`, looking through
/// the inheritance merge so an inherited or `@method static` member counts.
///
/// A method the merge cannot find is treated as static: an unknown target
/// reached through `ClassName::` is a static call as far as anything we can
/// still say about it goes.
fn method_is_static(class_info: &ClassInfo, method_name: &str, ctx: &VarResolutionCtx<'_>) -> bool {
    if let Some(method) = class_info.get_method_ci(method_name) {
        return method.is_static;
    }
    let merged = crate::virtual_members::resolve_class_fully_maybe_cached(
        class_info,
        ctx.class_loader,
        ctx.resolved_class_cache,
    );
    merged
        .get_method_ci(method_name)
        .is_none_or(|method| method.is_static)
}
