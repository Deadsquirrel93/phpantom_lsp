//! What a call does with its arguments: the nth positional argument, an
//! argument's raw type, the native type a call on an assignment's
//! right-hand side returns, and the type a by-reference parameter leaves
//! in the variable passed to it.

use super::*;

use std::sync::Arc;

use mago_span::HasSpan;

use crate::atom::{atom, bytes_to_str};
use crate::docblock;
use crate::php_type::PhpType;
use crate::type_engine::call_resolution::OutParamCallee;
use crate::type_engine::resolver::VarResolutionCtx;
use crate::types::{ParameterInfo, ResolvedType};

/// Extract the "native" return-type string from the RHS of an assignment
/// expression, without resolving it to `ClassInfo`.
///
/// This is used by [`try_inline_var_override`] to feed
/// [`docblock::resolve_effective_type`] with the same kind of parsed
/// `PhpType` that `@return` override checking uses.
///
/// Returns `None` when the native type cannot be determined (the
/// caller should treat this as "unknown", which lets the docblock type
/// win unconditionally).
pub(crate) fn extract_native_type_from_rhs<'b>(
    rhs: &'b Expression<'b>,
    ctx: &VarResolutionCtx<'_>,
) -> Option<PhpType> {
    match rhs {
        // `new ClassName(…)` → the class name.
        Expression::Instantiation(inst) => match inst.class {
            Expression::Identifier(ident) => {
                let fqn = ctx
                    .written_names()
                    .resolve_class(bytes_to_str(ident.value()));
                Some(PhpType::named(atom(&fqn)))
            }
            Expression::Self_(_) => Some(PhpType::named(ctx.current_class.fqn())),
            Expression::Static(_) => Some(PhpType::named(ctx.current_class.fqn())),
            _ => None,
        },
        // Function / method calls → look up the return type.
        Expression::Call(call) => match call {
            Call::Function(func_call) => {
                let func_name = match func_call.function {
                    Expression::Identifier(ident) => Some(bytes_to_str(ident.value()).to_string()),
                    _ => None,
                };
                let func_name_offset = func_call.function.span().start.offset;
                func_name.and_then(|name| {
                    ctx.function_loader()
                        .and_then(|fl| fl(&name, func_name_offset))
                        .and_then(|fi| fi.return_type.clone())
                })
            }
            Call::Method(method_call) => {
                if let Expression::Variable(Variable::Direct(dv)) = method_call.object
                    && dv.name == b"$this"
                    && let ClassLikeMemberSelector::Identifier(ident) = &method_call.method
                {
                    let method_name = bytes_to_str(ident.value).to_string();
                    let current = ctx.current_class.fqn();
                    crate::class_lookup::find_class_by_fqn(ctx.all_classes, &current).and_then(
                        |cls| {
                            cls.get_method(&method_name)
                                .and_then(|m| m.return_type.clone())
                        },
                    )
                } else {
                    None
                }
            }
            Call::StaticMethod(static_call) => {
                let class_name = match static_call.class {
                    Expression::Self_(_) | Expression::Static(_) => {
                        Some(ctx.current_class.fqn().to_string())
                    }
                    Expression::Identifier(ident) => Some(
                        ctx.written_names()
                            .resolve_class(bytes_to_str(ident.value())),
                    ),
                    _ => None,
                };
                if let Some(cls_name) = class_name
                    && let ClassLikeMemberSelector::Identifier(ident) = &static_call.method
                {
                    let method_name = bytes_to_str(ident.value).to_string();
                    let owner = crate::class_lookup::find_class_by_fqn(ctx.all_classes, &cls_name)
                        .cloned()
                        .or_else(|| (ctx.class_loader)(&cls_name));
                    owner.and_then(|o| {
                        o.get_method(&method_name)
                            .and_then(|m| m.return_type.clone())
                    })
                } else {
                    None
                }
            }
            _ => None,
        },
        // First-class callable syntax, closure literals, and arrow
        // functions always produce a Closure. When we can infer the
        // body return type, preserve it in a typed `Closure(): T`.
        Expression::PartialApplication(_)
        | Expression::Closure(_)
        | Expression::ArrowFunction(_) => Some(
            super::super::rhs_resolution::infer_closure_literal_type(rhs, ctx),
        ),
        _ => None,
    }
}

// ── Array function type preservation helpers ─────────────────────────

/// Extract the nth positional argument expression (0-based).
pub(in crate::type_engine) fn nth_arg_expr<'b>(
    args: &'b ArgumentList<'b>,
    n: usize,
) -> Option<&'b Expression<'b>> {
    args.arguments.iter().nth(n).map(|arg| match arg {
        Argument::Positional(pos) => pos.value,
        Argument::Named(named) => named.value,
    })
}

/// The types the forward walker knows for `$var_name` at `offset`,
/// through whichever of its two scope channels is live.
///
/// Argument-type resolution reaches for this before its backward
/// `@var`/`@param` text scan, because the two answer different
/// questions: the scan describes the variable at its *annotation*, while
/// the walker's scope describes it at the point of *use*, carrying
/// whatever the guards in between proved.  `array_slice($cached, …)`
/// inside `if ($cached !== null)` has to slice the non-null half, not the
/// `null|list<…>` written above the assignment.
///
/// The two channels are the same information reaching different
/// consumers: `scope_var_resolver` is the in-progress `ScopeState` the
/// walker threads down its own call tree, and the diagnostic scope cache
/// is the per-statement recording of it a diagnostic pass leaves behind
/// for the consumers the walker does not drive directly (the return-type
/// check among them).  Both are O(1)/O(log N) lookups.
pub(in crate::type_engine) fn walker_scope_types(
    var_name: &str,
    offset: u32,
    scope_var_resolver: crate::type_engine::resolver::ScopeVarResolverFn<'_>,
) -> Vec<ResolvedType> {
    let prefixed = if var_name.starts_with('$') {
        var_name.to_string()
    } else {
        format!("${}", var_name)
    };
    if let Some(resolver) = scope_var_resolver {
        let from_scope = resolver(&prefixed);
        if !from_scope.is_empty() {
            return from_scope;
        }
    }
    if super::super::forward_walk::is_diagnostic_scope_active()
        && !super::super::forward_walk::is_building_scopes()
        && let Some(types) = super::super::forward_walk::lookup_diagnostic_scope(&prefixed, offset)
    {
        return types;
    }
    Vec::new()
}

/// Resolve the raw iterable type of an argument expression.
///
/// Handles `$variable` (via the walker's scope, then docblock scanning)
/// and delegates to `resolve_expression_type` for method calls, property
/// access, etc.
pub(in crate::type_engine) fn resolve_arg_raw_type<'b>(
    arg_expr: &'b Expression<'b>,
    ctx: &VarResolutionCtx<'_>,
) -> Option<PhpType> {
    if let Expression::Variable(Variable::Direct(dv)) = arg_expr {
        let var_text = bytes_to_str(dv.name).to_string();
        let offset = arg_expr.span().start.offset as usize;
        let from_docblock =
            docblock::find_iterable_raw_type_in_source(ctx.content, offset, &var_text)
                .map(|t| ctx.written_names().resolve_type(&t));

        // The scope the forward walker recorded for this position, from
        // whichever of its two channels is live.  Both are map reads,
        // cheap enough to consult alongside the annotation.
        let from_walker = walker_scope_types(&var_text, offset as u32, ctx.scope_var_resolver);

        // Neither channel had it, so the only thing left below is a full
        // re-resolution of the variable, and an annotation that already
        // answered is taken as-is.
        if from_walker.is_empty()
            && ctx.scope_var_resolver.is_none()
            && let Some(annotated) = from_docblock
        {
            return Some(annotated);
        }

        // Resolve the variable's type to extract the raw iterable type.
        // This handles cases like `$users = $this->getUsers();
        // array_pop($users)` where `$users` has no `@var` annotation but was
        // assigned from a method returning `list<User>`.
        //
        // Only reached when the walker's scope came up empty, and only
        // without a scope_var_resolver: falling through to
        // resolve_variable_types while the forward walker is active would
        // re-enter it, causing infinite recursion on patterns like
        // `$a['k'] = f($a['k'])`.
        let resolved = if !from_walker.is_empty() || ctx.scope_var_resolver.is_some() {
            from_walker
        } else {
            resolve_variable_types(
                &var_text,
                ctx.current_class,
                ctx.all_classes,
                ctx.content,
                offset as u32,
                ctx.class_loader,
                ctx.source_class_loader,
                ctx.source_imports,
                ctx.backend,
                ctx.loaders,
            )
        };
        let from_scope = (!resolved.is_empty())
            .then(|| crate::types::ResolvedType::types_joined(&resolved))
            // `skip_scalar` stays off: a `list<int>` carries an element type
            // every bit as usable as a `list<User>`, and asking whether the
            // element is non-scalar drops every scalar-element array back to
            // the annotation the walker has since narrowed.
            .filter(|joined| joined.extract_value_type(false).is_some());

        match (from_docblock, from_scope) {
            // The annotation is where the walker's own type came from, so
            // when the two disagree the walker has narrowed it since
            // (`assert($a !== [])` proving `non-empty-array`, or an
            // `is_array()` guard ruling out the `string` arm) and its answer
            // is the one that describes the value at this call.  Anything
            // wider than the annotation is the walker having lost the
            // generics on the way through, and the annotation still stands.
            (Some(annotated), Some(scoped)) if scoped.is_subtype_of(&annotated) => {
                return Some(scoped);
            }
            (Some(annotated), _) => return Some(annotated),
            (None, Some(scoped)) => return Some(scoped),
            (None, None) => {}
        }
    }
    // Fall back to the unified pipeline (method calls, etc.)
    super::super::foreach_resolution::resolve_expression_type(arg_expr, ctx)
}

/// Check whether a call expression passes the target variable to a
/// pass-by-reference parameter with a type hint, and if so, push the
/// resolved type into `results`.
///
/// For example, given `function foo(Baz &$bar): void {}` and the call
/// `foo($bar)`, this function detects that `$bar` is passed to a `&`
/// parameter typed as `Baz` and resolves `$bar` to `Baz`.
///
/// Handles standalone function calls, instance method calls, static
/// method calls, and constructor calls.
pub(in crate::type_engine::variable) fn try_apply_pass_by_reference_type(
    expr: &Expression<'_>,
    ctx: &VarResolutionCtx<'_>,
    results: &mut Vec<ResolvedType>,
    conditional: bool,
) {
    let (argument_list, parameters, callee) = match expr {
        Expression::Call(Call::Function(func_call)) => {
            let func_name = match func_call.function {
                Expression::Identifier(ident) => bytes_to_str(ident.value()).to_string(),
                _ => return,
            };
            let func_name_offset = func_call.function.span().start.offset;
            let fl = match ctx.function_loader() {
                Some(fl) => fl,
                None => return,
            };
            let func_info = match fl(&func_name, func_name_offset) {
                Some(fi) => fi,
                None => return,
            };
            // Borrow the argument list and clone the parameters so we
            // can iterate them together.
            let parameters = func_info.parameters.clone();
            (
                &func_call.argument_list,
                parameters,
                OutParamCallee::Function(Box::new(func_info)),
            )
        }
        Expression::Call(Call::Method(method_call)) => {
            match try_resolve_method_params(method_call.object, &method_call.method, ctx) {
                Some((params, callee)) => (&method_call.argument_list, params, callee),
                None => return,
            }
        }
        Expression::Call(Call::NullSafeMethod(method_call)) => {
            match try_resolve_method_params(method_call.object, &method_call.method, ctx) {
                Some((params, callee)) => (&method_call.argument_list, params, callee),
                None => return,
            }
        }
        Expression::Call(Call::StaticMethod(static_call)) => {
            match try_resolve_static_method_params(static_call, ctx) {
                Some((params, arg_list, callee)) => (arg_list, params, callee),
                None => return,
            }
        }
        Expression::Instantiation(inst) => match try_resolve_constructor_params(inst, ctx) {
            Some((params, arg_list, callee)) => (arg_list, params, callee),
            None => return,
        },
        _ => return,
    };

    // Bind arguments to parameters following PHP's rules so that named
    // arguments consult the parameter they actually target, not the one at
    // their ordinal position in the call.
    let bound = crate::call_args::bind_args_to_params(&parameters, argument_list);

    for (param_index, (param, arg_expr)) in parameters.iter().zip(bound.iter()).enumerate() {
        let arg_expr = match arg_expr {
            Some(expr) => *expr,
            None => continue,
        };

        // Check if this argument is our target variable.
        let is_our_var = match arg_expr {
            Expression::Variable(Variable::Direct(dv)) => bytes_to_str(dv.name) == ctx.var_name,
            _ => false,
        };
        if !is_our_var {
            continue;
        }

        // Check if the corresponding parameter is pass-by-reference
        // with a type hint.
        if param.is_reference
            && let Some(out_hint) = crate::type_engine::call_resolution::effective_out_type(
                param,
                param_index,
                &callee,
                ctx.backend,
            )
        {
            // A PHPStan conditional out type (`@param-out ($arg is null ?
            // A&I : A) $arg`) is call-site-agnostic up to this point; only
            // this call's own arguments say which branch it actually takes.
            let var_resolver = build_var_resolver_from_ctx(ctx);
            let out_hint = crate::type_engine::call_resolution::resolve_out_type_for_call(
                out_hint,
                &parameters,
                &callee,
                argument_list,
                ctx.content,
                &ctx.as_resolution_ctx(),
                Some(&var_resolver),
            );
            let resolved = crate::type_engine::type_resolution::type_hint_to_classes_typed(
                &out_hint,
                &ctx.current_class.fqn(),
                ctx.all_classes,
                ctx.class_loader,
            );
            if !resolved.is_empty() {
                if !conditional {
                    results.clear();
                }
                ResolvedType::extend_unique(
                    results,
                    ResolvedType::from_classes_with_hint(resolved, out_hint),
                );
            }
        }
    }
}

/// Resolve parameters for an instance method call.
///
/// Currently only handles `$this->method()` where the current class
/// is known.  General variable receiver resolution is deferred to the
/// forward-walking scope model to avoid re-entrant variable resolution.
fn try_resolve_method_params(
    object: &Expression<'_>,
    method: &ClassLikeMemberSelector<'_>,
    ctx: &VarResolutionCtx<'_>,
) -> Option<(crate::types::SharedVec<ParameterInfo>, OutParamCallee)> {
    let method_name = match method {
        ClassLikeMemberSelector::Identifier(ident) => bytes_to_str(ident.value),
        _ => return None,
    };

    // Only handle `$this->method()` — we know the current class.
    match object {
        Expression::Variable(Variable::Direct(dv)) if dv.name == b"$this" => {}
        _ => return None,
    }

    let method_info = ctx.current_class.get_method(method_name)?;
    // The callee wants the class shared, and the file's own classes and
    // the resolved-class cache already hold it that way; copying the whole
    // `ClassInfo` here would repeat for every local in scope on every call
    // statement (see `forward_walk/by_ref.rs`).
    let current_fqn = ctx.current_class.fqn();
    let owner = ctx
        .all_classes
        .iter()
        .find(|c| c.fqn() == current_fqn)
        .cloned()
        .or_else(|| (ctx.class_loader)(&current_fqn))
        .unwrap_or_else(|| Arc::new(ctx.current_class.clone()));
    Some((
        method_info.parameters.clone(),
        OutParamCallee::Method(owner, atom(method_name)),
    ))
}

/// Resolve parameters for a static method call.
fn try_resolve_static_method_params<'a>(
    static_call: &'a StaticMethodCall<'a>,
    ctx: &VarResolutionCtx<'_>,
) -> Option<(
    crate::types::SharedVec<ParameterInfo>,
    &'a ArgumentList<'a>,
    OutParamCallee,
)> {
    let method_name = match &static_call.method {
        ClassLikeMemberSelector::Identifier(ident) => bytes_to_str(ident.value),
        _ => return None,
    };

    let class_name = crate::class_lookup::class_expression_name(
        static_call.class,
        ctx.current_class,
        ctx.all_classes,
        ctx.source_class_loader,
        ctx.source_imports,
    )?;

    let cls = (ctx.class_loader)(&class_name)?;
    let method_info = cls.get_method(method_name)?;
    Some((
        method_info.parameters.clone(),
        &static_call.argument_list,
        OutParamCallee::Method(cls, atom(method_name)),
    ))
}

/// Resolve parameters for a constructor call (`new Cls(...)`).
fn try_resolve_constructor_params<'a>(
    inst: &'a Instantiation<'a>,
    ctx: &VarResolutionCtx<'_>,
) -> Option<(
    crate::types::SharedVec<ParameterInfo>,
    &'a ArgumentList<'a>,
    OutParamCallee,
)> {
    let class_name = crate::class_lookup::class_expression_name(
        inst.class,
        ctx.current_class,
        ctx.all_classes,
        ctx.source_class_loader,
        ctx.source_imports,
    )?;

    let args = inst.argument_list.as_ref()?;
    let cls = (ctx.class_loader)(&class_name)?;
    let ctor = cls.get_method("__construct")?;
    Some((
        ctor.parameters.clone(),
        args,
        OutParamCallee::Method(cls, atom("__construct")),
    ))
}
