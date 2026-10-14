//! Qualifying the class names a parsed declaration spells, against its
//! file's imports and namespace while those are still in scope.
//!
//! `extends`, `implements`, trait `use`, mixins, member and function
//! types, `@property`/`@method` tags, template bounds and constant
//! initializers all come out as resolved names, so a consumer in another
//! file never needs the declaring file's use-map.

use std::collections::HashMap;
use std::sync::Arc;

use crate::Backend;
use crate::atom::{Atom, atom};
use crate::names::{lookup_import, resolve_to_fqn};
use crate::php_type::PhpType;
use crate::type_engine::types::docblock_narrowing;
use crate::types::{
    ClassInfo, DocTypeSlot, DocblockMembers, FunctionInfo, MethodInfo, NamespaceSpan, TypeAliasDef,
    UnverifiedDocType,
};

impl Backend {
    /// Qualify the class names in each standalone function's types against
    /// the `namespace` block that declared it, the way
    /// [`resolve_parent_class_names`](Self::resolve_parent_class_names)
    /// qualifies a method's, so cross-file consumers can resolve them
    /// without the declaring file's use-map.
    ///
    /// `blocks` and the file-wide `use_map`, `constant_imports` and
    /// `namespace` are read as in
    /// [`resolve_parent_class_names_by_block`](Self::resolve_parent_class_names_by_block).
    pub(super) fn qualify_function_names(
        functions: &mut [FunctionInfo],
        blocks: &[NamespaceSpan],
        use_map: &HashMap<String, String>,
        constant_imports: &HashMap<String, String>,
        namespace: &Option<String>,
    ) {
        let multi_block = blocks.len() > 1;
        for func in functions {
            let skip_names: Vec<String> =
                func.template_params.iter().map(|a| a.to_string()).collect();
            // Use the function's own namespace (not the file-level one)
            // so that multi-namespace files resolve return types
            // against the correct namespace block.  A function in a
            // file's global `namespace { }` block has none, and must
            // not borrow the namespace of another block.
            let func_ns = if multi_block {
                func.namespace.clone()
            } else {
                func.namespace.clone().or_else(|| namespace.clone())
            };
            let (func_use_map, func_constant_imports) = match blocks
                .iter()
                .filter(|_| multi_block)
                .find(|span| func.name_offset >= span.start && func.name_offset <= span.end)
            {
                Some(span) => (&span.use_map, &span.symbol_imports.constants),
                None => (use_map, constant_imports),
            };
            let resolver = Self::build_type_resolver(
                func_use_map,
                func_constant_imports,
                &func_ns,
                &skip_names,
            );
            // The types the docblock settled with a class the native
            // hint does not name, for loading the function to check.
            let not_a_class = |name: &str| skip_names.iter().any(|t| t == name);
            let mut unverified: Vec<UnverifiedDocType> = Vec::new();

            // Asked before qualifying, which sets every class name
            // apart from the hint as written.
            let return_from_docblock = func.return_type != func.native_return_type;
            if let Some(ref ret) = func.return_type {
                let resolved = ret.resolve_names(&resolver);
                if resolved != *ret {
                    func.return_type = Some(resolved);
                }
            }
            if let Some(ref ret) = func.native_return_type {
                let resolved = ret.resolve_names(&resolver);
                if resolved != *ret {
                    func.native_return_type = Some(resolved);
                }
            }
            if return_from_docblock
                && let (Some(settled), Some(native)) = (&func.return_type, &func.native_return_type)
                && docblock_narrowing::names_class_beyond_native(settled, native, &not_a_class)
            {
                unverified.push(UnverifiedDocType {
                    member: func.name,
                    slot: DocTypeSlot::Return,
                    native: native.clone(),
                });
            }
            if let Some(ref cond) = func.conditional_return {
                let resolved = cond.resolve_names(&resolver);
                if resolved != *cond {
                    func.conditional_return = Some(resolved);
                }
            }
            let func_name = func.name;
            for (position, param) in func.parameters.make_mut().iter_mut().enumerate() {
                crate::parser::qualify_param_default(param, func_use_map, &func_ns);
                let from_docblock = param.type_hint != param.native_type_hint;
                if let Some(ref hint) = param.type_hint {
                    let resolved = hint.resolve_names(&resolver);
                    if resolved != *hint {
                        param.type_hint = Some(resolved);
                    }
                }
                // The native hint is kept as written.
                if from_docblock
                    && let (Some(settled), Some(native)) =
                        (&param.type_hint, &param.native_type_hint)
                    && docblock_narrowing::hint_names_classes(native)
                {
                    let native = native.resolve_names(&resolver);
                    if docblock_narrowing::names_class_beyond_native(settled, &native, &not_a_class)
                    {
                        unverified.push(UnverifiedDocType {
                            member: func_name,
                            slot: DocTypeSlot::Param(position as u32),
                            native: if param.defaults_to_null() {
                                native.or_null()
                            } else {
                                native
                            },
                        });
                    }
                }
                // `@param-closure-this` names a class the same way
                // the rest of the docblock does, so it is resolved
                // against the declaring file's imports here.  The
                // call site, which is where the closure's `$this` is
                // read, has no access to this file's use-map.
                if let Some(ref this_type) = param.closure_this_type {
                    let resolved = this_type.resolve_names(&resolver);
                    if resolved != *this_type {
                        param.closure_this_type = Some(resolved);
                    }
                }
                if let Some(ref out_type) = param.param_out_type {
                    let resolved = out_type.resolve_names(&resolver);
                    if resolved != *out_type {
                        param.param_out_type = Some(resolved);
                    }
                }
            }
            // Resolve exception class names in @throws tags.
            for throw in &mut func.throws {
                let resolved = throw.resolve_names(&resolver);
                if resolved != *throw {
                    *throw = resolved;
                }
            }
            // `@template T of Bound` bounds, their defaults and
            // `@phpstan-assert` types are matched against fully
            // qualified types.
            let defaults = func.template_param_defaults.iter_mut().map(|(_, d)| d);
            let asserted = func
                .type_assertions
                .iter_mut()
                .map(|a| &mut a.asserted_type);
            for ty in func
                .template_param_bounds
                .values_mut()
                .chain(defaults)
                .chain(asserted)
            {
                let resolved = ty.resolve_names(&resolver);
                if resolved != *ty {
                    *ty = resolved;
                }
            }
            func.unverified_doc_types = unverified.into_boxed_slice();
        }
    }

    /// [`resolve_parent_class_names`](Self::resolve_parent_class_names)
    /// for every class of a file, each against the `namespace` block that
    /// declared it.
    ///
    /// `classes` pairs each class with its namespace and the index of its
    /// block in `blocks` (`None` outside any block).  PHP scopes both the
    /// namespace and the `use` imports to a block, so in a file with several
    /// blocks each block's classes are resolved with that block's own
    /// imports.  A file with at most one block resolves everything against
    /// the file-wide `use_map` and `namespace` in one pass.
    pub(crate) fn resolve_parent_class_names_by_block(
        classes: &mut [(ClassInfo, Option<String>, Option<usize>)],
        blocks: &[NamespaceSpan],
        use_map: &HashMap<String, String>,
        constant_imports: &HashMap<String, String>,
        namespace: &Option<String>,
    ) {
        let multi_block = blocks.len() > 1;
        let mut groups: Vec<Option<usize>> = Vec::new();
        for (_, _, block) in classes.iter() {
            let key = if multi_block { *block } else { None };
            if !groups.contains(&key) {
                groups.push(key);
            }
        }
        for group in groups {
            let indices: Vec<usize> = (0..classes.len())
                .filter(|&i| !multi_block || classes[i].2 == group)
                .collect();
            let mut members: Vec<ClassInfo> = indices
                .iter()
                .map(|&i| std::mem::take(&mut classes[i].0))
                .collect();
            match group.and_then(|b| blocks.get(b)) {
                Some(block) => Self::resolve_parent_class_names(
                    &mut members,
                    &block.use_map,
                    &block.symbol_imports.constants,
                    &block.namespace,
                ),
                None if multi_block => {
                    Self::resolve_parent_class_names(&mut members, use_map, constant_imports, &None)
                }
                None => Self::resolve_parent_class_names(
                    &mut members,
                    use_map,
                    constant_imports,
                    namespace,
                ),
            }
            for (cls, &i) in members.into_iter().zip(&indices) {
                classes[i].0 = cls;
            }
        }
    }

    /// Resolve `parent_class` short names in a list of `ClassInfo` to
    /// fully-qualified names using the file's `use_map` and `namespace`.
    ///
    /// Rules (matching PHP name resolution):
    ///   1. Already fully-qualified (`\Foo\Bar`) → strip leading `\`
    ///   2. Qualified (`Foo\Bar`) → if first segment is in use_map, expand it;
    ///      otherwise prepend current namespace
    ///   3. Unqualified (`Bar`) → check use_map; otherwise prepend namespace
    ///   4. No namespace and not in use_map → keep as-is
    ///
    /// `constant_imports` is the file's `use const` table: a constant may
    /// stand in a docblock type (`$flag is PREG_SPLIT_NO_EMPTY`,
    /// `key-of<TABLE>`), so the type resolver reads it after the class
    /// imports.
    pub fn resolve_parent_class_names(
        classes: &mut [ClassInfo],
        use_map: &HashMap<String, String>,
        constant_imports: &HashMap<String, String>,
        namespace: &Option<String>,
    ) {
        // Collect type alias names from ALL classes in the file up-front.
        // A type alias defined on one class can be referenced from methods
        // in a different class in the same file, so we must skip all of
        // them to avoid mangling alias names into FQN form.
        let all_alias_names: Vec<Atom> = classes
            .iter()
            .flat_map(|c| c.type_aliases.keys().copied())
            .collect();

        for class in classes.iter_mut() {
            if let Some(ref parent) = class.parent_class {
                let resolved = resolve_to_fqn(parent, use_map, namespace);
                class.parent_class = Some(atom(&resolved));
            }
            class.used_traits = class
                .used_traits
                .iter()
                .map(|t| atom(&resolve_to_fqn(t, use_map, namespace)))
                .collect();

            class.interfaces = class
                .interfaces
                .iter()
                .map(|i| atom(&resolve_to_fqn(i, use_map, namespace)))
                .collect();

            // Resolve the `@phpstan-require-extends` base class to its
            // fully-qualified name so it is loadable cross-file.
            if let Some(ref required) = class.require_extends {
                class.require_extends = Some(atom(&resolve_to_fqn(required, use_map, namespace)));
            }

            class.require_implements = class
                .require_implements
                .iter()
                .map(|i| atom(&resolve_to_fqn(i, use_map, namespace)))
                .collect();

            // Resolve trait names in `insteadof` precedence adaptations
            for prec in &mut class.trait_precedences {
                prec.trait_name = atom(&resolve_to_fqn(&prec.trait_name, use_map, namespace));
                prec.insteadof = prec
                    .insteadof
                    .iter()
                    .map(|t| atom(&resolve_to_fqn(t, use_map, namespace)))
                    .collect();
            }

            // Resolve trait names in `as` alias adaptations
            for alias in &mut class.trait_aliases {
                if let Some(ref t) = alias.trait_name {
                    alias.trait_name = Some(atom(&resolve_to_fqn(t, use_map, namespace)));
                }
            }

            // Resolve mixin names to fully-qualified names.
            // Skip names that match a template parameter — these are
            // not class names but placeholders that will be substituted
            // with concrete types when the generic class is instantiated
            // (e.g. `@template TWraps` + `@mixin TWraps`).
            class.mixins = class
                .mixins
                .iter()
                .map(|m| {
                    if class.template_params.contains(m) {
                        *m
                    } else {
                        atom(&resolve_to_fqn(m, use_map, namespace))
                    }
                })
                .collect();

            if let Some(laravel) = class.laravel.as_deref_mut() {
                let resolver = |name: &str| -> String { resolve_to_fqn(name, use_map, namespace) };

                // Resolve custom collection class name to FQN.
                if let Some(collection) = laravel.custom_collection.take() {
                    laravel.custom_collection = Some(collection.resolve_names(&resolver));
                }
                if let Some(factory) = laravel.custom_factory.take() {
                    laravel.custom_factory = Some(factory.resolve_names(&resolver));
                }

                // Resolve custom builder class name to FQN.
                if let Some(builder) = laravel.custom_builder.take() {
                    laravel.custom_builder = Some(builder.resolve_names(&resolver));
                }

                // Resolve a Laravel factory's explicitly configured model
                // class to an FQN. Class-constant initializers follow the
                // file's imports and namespace; literal class strings were
                // marked absolute while being extracted and therefore remain
                // unchanged.
                if let Some(model) = laravel.factory_model.take() {
                    laravel.factory_model = Some(model.resolve_names(&resolver));
                }
            }

            // Resolve a facade's `getFacadeAccessor()` class reference to an
            // FQN so the concrete class it forwards to is loadable from any
            // file, not just the one that imported it.
            if let Some(crate::types::FacadeAccessor::Class(written)) =
                class.laravel().and_then(|l| l.facade_accessor)
            {
                let resolved = atom(&resolve_to_fqn(&written, use_map, namespace));
                class.laravel_mut().facade_accessor =
                    Some(crate::types::FacadeAccessor::Class(resolved));
            }
            // Resolve the `#[UsePolicy]` class name to an FQN so the policy is
            // loadable cross-file.
            if let Some(policy) = class.laravel().and_then(|l| l.policy_class.clone()) {
                class.laravel_mut().policy_class =
                    Some(resolve_to_fqn(&policy, use_map, namespace));
            }

            // Resolve custom pivot class names (`->using(X::class)`) to FQNs so
            // that hover shows the fully-qualified pivot class and it is
            // loadable cross-file.
            if class
                .laravel()
                .is_some_and(|l| l.belongs_to_many_pivots.iter().any(|p| p.using.is_some()))
            {
                for pivot in class.laravel_mut().belongs_to_many_pivots.iter_mut() {
                    if let Some(using) = &pivot.using {
                        pivot.using = Some(resolve_to_fqn(using, use_map, namespace));
                    }
                }
            }

            // Resolve cast class names to FQN so that custom cast
            // classes like `DecimalCast` (imported via `use`) are
            // loadable cross-file when `cast_type_to_php_type` calls
            // the class loader.
            if let Some(laravel) = class.laravel.as_deref_mut() {
                let resolve = |name: &str| resolve_to_fqn(name, use_map, namespace);
                let sources = laravel.cast_sources.as_deref_mut().into_iter();
                for (_, cast_type) in laravel
                    .casts_definitions
                    .iter_mut()
                    .chain(sources.flat_map(|s| s.property.iter_mut().chain(s.method.iter_mut())))
                {
                    // Built-in strings (`boolean`, `datetime:Y-m-d`) are left
                    // alone. Class names, including the enum or collection a
                    // framework cast's `of()` / `using()` names, are resolved
                    // against this file's imports. A name that already
                    // contains `\` is not passed through `resolve_to_fqn`,
                    // which would prepend the file's namespace onto it.
                    *cast_type =
                        crate::virtual_members::laravel::qualify_cast_type(cast_type, resolve);
                }
            }

            // Resolve type arguments in @extends, @implements, and @use
            // generics so that after generic substitution, return types
            // and property types are fully-qualified and can be resolved
            // across files via PSR-4.
            //
            // Template params of the current class must be skipped so
            // that forwarded params (e.g. `@use BuildsQueries<TModel>`
            // where TModel is a class-level template) remain as bare
            // names and match substitution map keys later.
            //
            // Type alias names are skipped too: they are expanded below,
            // once every class's names have been qualified.
            let tpl_params: Vec<String> = class
                .template_params
                .iter()
                .map(|a| a.to_string())
                .collect();
            let generic_arg_skip: Vec<String> = tpl_params
                .iter()
                .cloned()
                .chain(all_alias_names.iter().map(|a| a.to_string()))
                .collect();
            Self::resolve_generics_type_args(
                &mut class.extends_generics,
                use_map,
                constant_imports,
                namespace,
                &generic_arg_skip,
            );
            Self::resolve_generics_type_args(
                &mut class.implements_generics,
                use_map,
                constant_imports,
                namespace,
                &generic_arg_skip,
            );
            Self::resolve_generics_type_args(
                &mut class.use_generics,
                use_map,
                constant_imports,
                namespace,
                &generic_arg_skip,
            );
            Self::resolve_generics_type_args(
                &mut class.mixin_generics,
                use_map,
                constant_imports,
                namespace,
                &generic_arg_skip,
            );

            // Resolve template parameter bounds (`@template T of Bound`)
            // so that short names like `PDependNode` become FQNs like
            // `PDepend\Source\AST\ASTNode`.  Without this, mixin
            // resolution that falls back to bounds gets unresolvable
            // short names.
            {
                let bound_resolver =
                    Self::build_type_resolver(use_map, constant_imports, namespace, &tpl_params);
                let defaults = class.template_param_defaults.values_mut();
                for bound in class.template_param_bounds.values_mut().chain(defaults) {
                    let resolved = bound.resolve_names(&bound_resolver);
                    if resolved != *bound {
                        *bound = resolved;
                    }
                }
            }

            // Resolve class-like names in method return types and property
            // type hints so that cross-file resolution works correctly.
            // For example, if a method returns `Country` and the file has
            // `use Acme\Core\Enums\Country`, the return type becomes
            // the FQN `Acme\Core\Enums\Country`.
            //
            // Template params and type alias names are excluded to avoid
            // mangling generic types and locally-defined type aliases.
            // We collect alias names from ALL classes in the file because
            // a type alias defined on one class may be referenced from a
            // method in a different class in the same file.
            let template_params = &class.template_params;
            let skip_names: Vec<String> = template_params
                .iter()
                .map(|a| a.to_string())
                .chain(all_alias_names.iter().map(|a| a.to_string()))
                .collect();
            let resolver =
                Self::build_type_resolver(use_map, constant_imports, namespace, &skip_names);

            // Also resolve class-like names inside type alias definitions
            // so that `@phpstan-type ActiveUser User` where `User` is
            // imported via `use App\Models\User` becomes `App\Models\User`.
            for def in class.type_aliases.values_mut() {
                match def {
                    TypeAliasDef::Import { source_class, .. } => {
                        // Imported alias — resolve the source class name.
                        let resolved_class = resolve_to_fqn(source_class, use_map, namespace);
                        if resolved_class != *source_class {
                            *source_class = resolved_class;
                        }
                    }
                    TypeAliasDef::Local(php_type) => {
                        // Local alias — resolve class names within the type.
                        let resolved = php_type.resolve_names(&resolver);
                        *php_type = resolved;
                    }
                }
            }

            // The member types the docblock settled with a class the native
            // hint does not name, for loading the class to check.
            let mut unverified: Vec<UnverifiedDocType> = Vec::new();

            let (self_fqn, parent_fqn) = if class.kind == crate::types::ClassLikeKind::Trait {
                (None, None)
            } else {
                // `file_namespace` is not filled in yet, so `fqn()` would
                // give the short name.
                (
                    Some(crate::util::build_fqn(&class.name, namespace.as_deref())),
                    class.parent_class.map(|p| p.to_string()),
                )
            };

            for method in class.methods.make_mut() {
                let method = Arc::make_mut(method);
                // Build a per-method skip list that includes both class-level
                // and method-level template params so that names like `T` in
                // `@return Collection<T>` are not namespace-resolved.
                //
                // When the method has its own template params, build a
                // per-method resolver that skips them in addition to the
                // class-level skip names.  Otherwise reuse the class-level
                // resolver.
                let method_skip: Vec<String>;
                let method_resolver: &dyn Fn(&str) -> String = if method
                    .template_params()
                    .is_empty()
                {
                    &resolver
                } else {
                    method_skip = skip_names
                        .iter()
                        .cloned()
                        .chain(method.template_params().iter().map(|a| a.to_string()))
                        .collect();
                    // SAFETY: `method_skip` lives until end of this
                    // `for method` iteration, so the closure is valid.
                    &Self::build_type_resolver(use_map, constant_imports, namespace, &method_skip)
                };

                // Asked before qualifying, which sets every class name apart
                // from the native return type kept as written.
                let return_from_docblock = method.return_type != method.native_return_type;
                if let Some(ref ret) = method.return_type {
                    let resolved = ret.resolve_names(method_resolver);
                    if resolved != *ret {
                        method.return_type = Some(resolved);
                    }
                }
                for param in method.parameters.make_mut() {
                    crate::parser::qualify_param_default(param, use_map, namespace);
                    crate::parser::qualify_param_default_keyword(
                        param,
                        self_fqn.as_deref(),
                        parent_fqn.as_deref(),
                    );
                    let native_was_effective = param.native_type_hint == param.type_hint;
                    if let Some(ref hint) = param.type_hint {
                        let resolved = hint.resolve_names(method_resolver);
                        if resolved != *hint {
                            param.type_hint = Some(resolved);
                        }
                    }
                    if native_was_effective {
                        param.native_type_hint = param.type_hint.clone();
                    } else if let Some(ref native) = param.native_type_hint {
                        let resolved = native.resolve_names(method_resolver);
                        if resolved != *native {
                            param.native_type_hint = Some(resolved);
                        }
                    }
                    // `@param-closure-this` names a class the same way the
                    // rest of the docblock does, so it is resolved against
                    // the declaring file's imports here.  The call site,
                    // which is where the closure's `$this` is read, has no
                    // access to this file's use-map.
                    if let Some(ref this_type) = param.closure_this_type {
                        let resolved = this_type.resolve_names(method_resolver);
                        if resolved != *this_type {
                            param.closure_this_type = Some(resolved);
                        }
                    }
                    if let Some(ref out_type) = param.param_out_type {
                        let resolved = out_type.resolve_names(method_resolver);
                        if resolved != *out_type {
                            param.param_out_type = Some(resolved);
                        }
                    }
                }
                Self::record_unverified_method_types(
                    method,
                    &skip_names,
                    method_resolver,
                    return_from_docblock,
                    &mut unverified,
                );
                let Some(extras) = method.extras.as_deref_mut() else {
                    continue;
                };
                let resolve = |ty: &mut PhpType| {
                    let resolved = ty.resolve_names(method_resolver);
                    if resolved != *ty {
                        *ty = resolved;
                    }
                };
                if let Some(cond) = extras.conditional_return.as_mut() {
                    resolve(cond);
                }
                // Resolve exception class names in @throws tags.
                extras.throws.iter_mut().for_each(resolve);
                // `@psalm-if-this-is` and `@psalm-this-out` are matched
                // against the receiver's type, and the method's own
                // `@template T of Bound` bounds are matched against the
                // concrete types filling them.  Both sides of those
                // comparisons have to be spelled the same way, and the
                // receiver arrives fully qualified.
                if let Some(pattern) = extras.if_this_is.as_mut() {
                    resolve(pattern);
                }
                if let Some(self_out) = extras.self_out.as_mut() {
                    resolve(self_out);
                }
                extras.template_param_bounds.values_mut().for_each(resolve);
                extras
                    .template_param_defaults
                    .iter_mut()
                    .for_each(|(_, default)| resolve(default));
                extras
                    .type_assertions
                    .iter_mut()
                    .for_each(|assertion| resolve(&mut assertion.asserted_type));
            }
            let not_a_class = |name: &str| skip_names.iter().any(|skipped| skipped == name);
            for prop in class.properties.make_mut() {
                let Some(hint) = prop.type_hint.as_ref() else {
                    continue;
                };
                let from_docblock = prop.native_type_hint.as_ref() != Some(hint);
                let resolved = hint.resolve_names(&resolver);
                // The native hint is kept as written.
                if from_docblock
                    && let Some(native) = prop.native_type_hint.as_ref()
                    && docblock_narrowing::hint_names_classes(native)
                {
                    let native = native.resolve_names(&resolver);
                    if docblock_narrowing::names_class_beyond_native(
                        &resolved,
                        &native,
                        &not_a_class,
                    ) {
                        unverified.push(UnverifiedDocType {
                            member: prop.name,
                            slot: DocTypeSlot::Property,
                            native,
                        });
                    }
                }
                if resolved != *hint {
                    Arc::make_mut(prop).type_hint = Some(resolved);
                }
            }
            unverified.sort_unstable_by(|a, b| {
                (a.member.as_str(), a.slot).cmp(&(b.member.as_str(), b.slot))
            });
            class.unverified_doc_types = unverified.into_boxed_slice();

            // A constant's initializer stays source text, and the file that
            // reads the constant has none of this file's imports, so the
            // classes it names are qualified here.
            // A typed constant's declared type (`const Foo C = …`) is
            // qualified the same way.
            for i in 0..class.constants.len() {
                let qualified = class.constants[i].value.as_deref().and_then(|value| {
                    super::initializers::qualify_class_names(value, namespace.as_deref(), &|name| {
                        resolve_to_fqn(name, use_map, namespace)
                    })
                });
                let typed = class.constants[i]
                    .type_hint
                    .as_ref()
                    .map(|hint| hint.resolve_names(&resolver))
                    .filter(|resolved| Some(resolved) != class.constants[i].type_hint.as_ref());
                if qualified.is_none() && typed.is_none() {
                    continue;
                }
                let constant = Arc::make_mut(&mut class.constants.make_mut()[i]);
                if qualified.is_some() {
                    constant.value = qualified;
                }
                if typed.is_some() {
                    constant.type_hint = typed;
                }
            }

            // Resolve type names inside `@property` / `@property-read` /
            // `@property-write` and `@method` tags.  Their type strings use
            // short names relative to the declaring file's imports, and a
            // cross-file consumer has no access to this file's use-map.
            if let Some(doc) = class.doc_members.as_ref()
                && let Some(resolved) = Self::resolve_doc_member_types(
                    doc,
                    use_map,
                    constant_imports,
                    namespace,
                    &skip_names,
                    &resolver,
                )
            {
                class.doc_members = Some(resolved);
            }
        }

        // A member signature is read from every file that uses the class,
        // where the alias names it would no longer be in scope.
        if !all_alias_names.is_empty() {
            crate::type_engine::types::aliases::expand_local_type_aliases(classes);
        }
    }

    /// Record each type `method` settled from its docblock that names a class
    /// the native hint does not, for loading the class to check (see
    /// [`docblock_narrowing`]).
    ///
    /// Runs once the method's types are qualified.  The native return type
    /// is kept as written, so a copy of it is qualified with `resolver`, and
    /// only when the docblock settled the return type
    /// (`return_from_docblock`).  `class_scoped` names the class's templates
    /// and the file's type aliases.
    fn record_unverified_method_types(
        method: &MethodInfo,
        class_scoped: &[String],
        resolver: &dyn Fn(&str) -> String,
        return_from_docblock: bool,
        out: &mut Vec<UnverifiedDocType>,
    ) {
        let not_a_class = |name: &str| {
            class_scoped.iter().any(|skipped| skipped == name)
                || method.template_params().iter().any(|t| t == name)
        };
        if return_from_docblock
            && let (Some(settled), Some(native)) = (&method.return_type, &method.native_return_type)
            && docblock_narrowing::hint_names_classes(native)
        {
            let native = native.resolve_names(resolver);
            if docblock_narrowing::names_class_beyond_native(settled, &native, &not_a_class) {
                out.push(UnverifiedDocType {
                    member: method.name,
                    slot: DocTypeSlot::Return,
                    native,
                });
            }
        }
        // A parameter's native hint is qualified along with the rest.
        for (position, param) in method.parameters.iter().enumerate() {
            if let (Some(settled), Some(native)) = (&param.type_hint, &param.native_type_hint)
                && settled != native
                && docblock_narrowing::names_class_beyond_native(settled, native, &not_a_class)
            {
                out.push(UnverifiedDocType {
                    member: method.name,
                    slot: DocTypeSlot::Param(position as u32),
                    native: if param.defaults_to_null() {
                        native.clone().or_null()
                    } else {
                        native.clone()
                    },
                });
            }
        }
    }

    /// Resolve short class names in the `@property` and `@method` tags that
    /// were parsed out of a class-level docblock.
    ///
    /// The tag types are written against the declaring file's imports, so
    /// they are qualified here while that use-map is still in scope.  A
    /// consumer in another file only ever sees the resolved form.
    ///
    /// Returns `None` when nothing changed, so the common case keeps
    /// sharing the existing `Arc` rather than rebuilding it.
    fn resolve_doc_member_types(
        doc: &Arc<DocblockMembers>,
        use_map: &HashMap<String, String>,
        constant_imports: &HashMap<String, String>,
        namespace: &Option<String>,
        skip_names: &[String],
        resolver: &dyn Fn(&str) -> String,
    ) -> Option<Arc<DocblockMembers>> {
        let mut changed = false;

        let properties: Vec<(Atom, Option<PhpType>)> = doc
            .properties
            .iter()
            .map(|(name, ty)| {
                let resolved = ty.as_ref().map(|t| {
                    let r = t.resolve_names(resolver);
                    if r != *t {
                        changed = true;
                    }
                    r
                });
                (*name, resolved)
            })
            .collect();

        let methods: Vec<Arc<MethodInfo>> = doc
            .methods
            .iter()
            .map(|m| {
                // `@method foo<T>(T $x): T` declares its own template
                // params, which must not be namespace-qualified.
                let method_skip: Vec<String>;
                let owned_resolver;
                let method_resolver: &dyn Fn(&str) -> String = if m.template_params().is_empty() {
                    resolver
                } else {
                    method_skip = skip_names
                        .iter()
                        .cloned()
                        .chain(m.template_params().iter().map(|a| a.to_string()))
                        .collect();
                    owned_resolver = Self::build_type_resolver(
                        use_map,
                        constant_imports,
                        namespace,
                        &method_skip,
                    );
                    &owned_resolver
                };

                let mut resolved = (**m).clone();
                let mut method_changed = false;

                if let Some(ref ret) = resolved.return_type {
                    let r = ret.resolve_names(method_resolver);
                    if r != *ret {
                        resolved.return_type = Some(r);
                        method_changed = true;
                    }
                }
                for param in resolved.parameters.make_mut() {
                    if let Some(ref hint) = param.type_hint {
                        let r = hint.resolve_names(method_resolver);
                        if r != *hint {
                            param.native_type_hint = Some(r.clone());
                            param.type_hint = Some(r);
                            method_changed = true;
                        }
                    }
                }
                if let Some(extras) = resolved.extras.as_deref_mut() {
                    for bound in extras.template_param_bounds.values_mut() {
                        let r = bound.resolve_names(method_resolver);
                        if r != *bound {
                            *bound = r;
                            method_changed = true;
                        }
                    }
                }

                if method_changed {
                    changed = true;
                    Arc::new(resolved)
                } else {
                    Arc::clone(m)
                }
            })
            .collect();

        changed.then(|| {
            Arc::new(DocblockMembers {
                methods,
                properties,
            })
        })
    }

    /// Resolve type arguments in a generics list (e.g. `@extends`, `@implements`,
    /// `@use`) to fully-qualified names.
    ///
    /// Each entry is `(ClassName, [TypeArg1, TypeArg2, …])`.  The class name
    /// itself is resolved (e.g. `HasFactory` → `App\Concerns\HasFactory`),
    /// and each type argument that looks like a class name (i.e. not a scalar
    /// like `int`, `string`, etc.) is also resolved.
    ///
    /// `skip_names` contains template parameter names that must NOT be
    /// resolved.  Without this, a forwarded template param like `TModel`
    /// in `@use BuildsQueries<TModel>` would be namespace-qualified to
    /// e.g. `Illuminate\Database\Eloquent\TModel`, preventing it from
    /// matching substitution map keys during generic resolution.
    fn resolve_generics_type_args(
        generics: &mut [(Atom, Vec<PhpType>)],
        use_map: &HashMap<String, String>,
        constant_imports: &HashMap<String, String>,
        namespace: &Option<String>,
        skip_names: &[String],
    ) {
        let resolver = Self::build_type_resolver(use_map, constant_imports, namespace, skip_names);
        for (class_name, type_args) in generics.iter_mut() {
            // Resolve the base class/trait/interface name
            let resolved: String = resolve_to_fqn(class_name, use_map, namespace);
            *class_name = atom(&resolved);

            // Resolve each type argument (now PhpType) via resolve_names
            for arg in type_args.iter_mut() {
                let resolved = arg.resolve_names(&resolver);
                if resolved != *arg {
                    *arg = resolved;
                }
            }
        }
    }

    /// Build a resolver closure that resolves class-like names to FQNs,
    /// skipping template parameters, type aliases, and keyword types.
    ///
    /// The returned closure is suitable for passing to
    /// `PhpType::resolve_names()`.  `is_keyword_type` inside `resolve_names`
    /// already handles scalar and keyword types; this closure additionally
    /// skips names in `skip_names` (template params and type alias names).
    ///
    /// A bare name no class import claims is read as a `use const` alias
    /// when it is spelled exactly like one (constant names are
    /// case-sensitive), since a type may name a constant
    /// (`$flag is PREG_SPLIT_NO_EMPTY`, `key-of<TABLE>`).
    fn build_type_resolver<'a>(
        use_map: &'a HashMap<String, String>,
        constant_imports: &'a HashMap<String, String>,
        namespace: &'a Option<String>,
        skip_names: &'a [String],
    ) -> impl Fn(&str) -> String + 'a {
        move |name: &str| {
            if skip_names.iter().any(|s| s == name) {
                return name.to_string();
            }
            if let Some(fqn) = constant_imports.get(name)
                && lookup_import(use_map, name).is_none()
            {
                return fqn.clone();
            }
            resolve_to_fqn(name, use_map, namespace)
        }
    }
}
