//! PHPDoc virtual member provider.
//!
//! Extracts `@method`, `@property` / `@property-read` / `@property-write`,
//! and `@mixin` tags from the class-level docblock and presents them as
//! virtual members.  This is the second-highest-priority virtual member
//! provider: framework providers (e.g. Laravel) take precedence, but
//! PHPDoc-sourced members beat all other virtual member sources.
//!
//! Within this provider, `@method` and `@property` tags take precedence
//! over `@mixin` members: if a class declares both `@property int $id`
//! and `@mixin SomeClass` where `SomeClass` also has an `$id` property,
//! the `@property` tag wins.

mod mixins;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::atom::{Atom, AtomSet};
use crate::inheritance;
use crate::inheritance::{ClassRef, bind_template_args};
use crate::php_type::PhpType;
use crate::types::{ClassInfo, MAX_INHERITANCE_DEPTH, MethodInfo, PropertyInfo, PropertySource};

use super::{VirtualMemberProvider, VirtualMembers};
use mixins::{
    MixinCollector, MixinDedup, MixinSubs, build_mixin_substitution_map, collect_mixin_members,
    collect_trait_mixin_members, uses_trait_with_mixins,
};

pub use mixins::resolve_template_param_mixins;
pub(super) use mixins::uses_forwards_calls;

/// Build a virtual [`PropertyInfo`] from a parsed `@property` tag,
/// substituting template parameters in the declared type when `subs`
/// carries bindings for the docblock's owner.
fn doc_property(
    name: Atom,
    type_hint: Option<&PhpType>,
    subs: &HashMap<String, PhpType>,
) -> PropertyInfo {
    let type_hint = type_hint.map(|t| {
        if subs.is_empty() {
            t.clone()
        } else {
            t.substitute(subs)
        }
    });
    PropertyInfo::virtual_property_owned(name, type_hint)
}

/// Like [`doc_property`], but marked [`PropertySource::DocblockTag`].
///
/// Used for `@property` tags read from the class itself, its traits,
/// parents, and interfaces — declarations the user wrote *about this
/// class*, which override schema/convention-inferred types during the
/// virtual-member merge.  `@mixin`-borrowed tags stay unmarked: they
/// describe the mixin class, and the provider treats them as the
/// lowest-precedence source.
fn doc_tag_property(
    name: Atom,
    type_hint: Option<&PhpType>,
    subs: &HashMap<String, PhpType>,
) -> PropertyInfo {
    PropertyInfo {
        source: Some(PropertySource::DocblockTag),
        ..doc_property(name, type_hint, subs)
    }
}

/// Apply template substitutions to a parsed `@method` tag.
///
/// The tags are parsed once per class and shared behind an `Arc`, so the
/// common case (no substitution for this consumer) is a refcount bump
/// rather than a deep clone.
fn substituted_doc_method(
    method: &Arc<MethodInfo>,
    subs: &HashMap<String, PhpType>,
) -> Arc<MethodInfo> {
    if subs.is_empty() {
        return Arc::clone(method);
    }
    let mut cloned = (**method).clone();
    inheritance::apply_substitution_to_method(&mut cloned, subs);
    Arc::new(cloned)
}

/// Virtual member provider for `@method`, `@property`, and `@mixin` docblock tags.
///
/// When a class declares `@method` or `@property` tags in its class-level
/// docblock, those tags describe magic members accessible via `__call`,
/// `__get`, and `__set`.  When a class declares `@mixin ClassName`, all
/// public members of `ClassName` (and its inheritance chain) become
/// available via magic methods.
///
/// Resolution order within this provider:
/// 1. `@method` and `@property` tags (highest precedence)
/// 2. `@mixin` class members (lower precedence, never overwrite tags)
///
/// Mixins are inherited: if `User extends Model` and `Model` has
/// `@mixin Builder`, then `User` also gains Builder's public members.
/// The provider walks the parent chain to collect mixin declarations
/// from ancestors.
///
/// Mixin classes can themselves declare `@mixin`, so the provider
/// recurses up to [`MAX_MIXIN_DEPTH`] levels.
pub struct PHPDocProvider;

impl VirtualMemberProvider for PHPDocProvider {
    /// Returns `true` if the class carries `@method` / `@property` tags
    /// or declares `@mixin` tags (directly or via traits and ancestors).
    ///
    /// This is a cheap pre-check: the tags were parsed once when each
    /// class was extracted, so this only inspects already-structured data.
    fn applies_to(
        &self,
        class: &ClassInfo,
        class_loader: &dyn Fn(&str) -> Option<Arc<ClassInfo>>,
    ) -> bool {
        // Declares @method/@property tags itself.
        if class.doc_members.is_some() {
            return true;
        }

        // Has direct @mixin declarations.
        if !class.mixins.is_empty() {
            return true;
        }

        // Has used traits that carry @method/@property tags.
        for trait_name in &class.used_traits {
            if let Some(trait_info) = class_loader(trait_name)
                && trait_info.doc_members.is_some()
            {
                return true;
            }
        }

        if uses_trait_with_mixins(class, class_loader) {
            return true;
        }

        // Walk the parent chain to check for ancestor mixins or tags.
        let mut current_parent = class.parent_class;
        let mut depth = 0u32;
        while let Some(ref parent_name) = current_parent {
            depth += 1;
            if depth > MAX_INHERITANCE_DEPTH {
                break;
            }
            let parent = if let Some(p) = class_loader(parent_name) {
                p
            } else {
                break;
            };
            if !parent.mixins.is_empty()
                || parent.doc_members.is_some()
                || uses_trait_with_mixins(&parent, class_loader)
            {
                return true;
            }
            // `provide` also reads tags off the traits an ancestor uses.
            for trait_name in &parent.used_traits {
                if let Some(trait_info) = class_loader(trait_name)
                    && trait_info.doc_members.is_some()
                {
                    return true;
                }
            }
            current_parent = parent.parent_class;
        }

        // Walk implemented interfaces (and the interfaces they extend),
        // which `provide` also collects tags from.
        let mut queue: Vec<Atom> = class.interfaces.to_vec();
        let mut visited: AtomSet = AtomSet::default();
        while let Some(iface_name) = queue.pop() {
            if !visited.insert(iface_name) {
                continue;
            }
            let Some(iface) = class_loader(&iface_name) else {
                continue;
            };
            if iface.doc_members.is_some() {
                return true;
            }
            queue.extend(iface.interfaces.iter().copied());
        }

        false
    }

    /// Collect `@method`, `@property`, and `@mixin` members for the class.
    ///
    /// The `@method` / `@property` tags were parsed into
    /// [`ClassInfo::doc_members`](crate::types::ClassInfo::doc_members)
    /// when each class was extracted, so this only reads structured data
    /// and applies the per-consumer generic substitutions.  It then
    /// collects public members from `@mixin` classes.  Within the
    /// provider, `@method` / `@property` tags take precedence over
    /// `@mixin` members.
    fn provide(
        &self,
        class: &ClassInfo,
        class_loader: &dyn Fn(&str) -> Option<Arc<ClassInfo>>,
        cache: Option<&super::ResolvedClassCache>,
    ) -> VirtualMembers {
        let mut methods = Vec::new();
        let mut properties = Vec::new();
        let constants = Vec::new();

        // Dedup sets for O(1) membership checks.  Seeded from the
        // base-resolved class members (real + inherited) and updated
        // as virtual members are collected.
        //
        // `seen_props` is NOT seeded from existing class properties.
        // Phase 1 (`@property` tags) always emits its properties so
        // that `merge_virtual_members` can compare type specificity
        // and keep the most specific type (e.g. `array<string>` from
        // `@property` beats bare `array` from `$casts`).  After
        // phase 1 emits, names are added to `seen_props` to prevent
        // lower-priority sources (trait tags, parent tags, `@mixin`
        // members) from overriding them.
        let mut seen_methods: AtomSet = class.methods.iter().map(|m| m.name).collect();
        let mut seen_props: AtomSet = AtomSet::default();
        let seen_consts: AtomSet = class.constants.iter().map(|c| c.name).collect();

        // No generic substitution applies to the class's own tags.
        let no_subs: HashMap<String, PhpType> = HashMap::new();

        // ── Phase 1: @method and @property tags (higher precedence) ─────

        if let Some(doc) = class.doc_members.as_deref() {
            for m in &doc.methods {
                seen_methods.insert(m.name);
                methods.push(Arc::clone(m));
            }

            for (name, type_hint) in &doc.properties {
                seen_props.insert(*name);
                properties.push(doc_tag_property(*name, type_hint.as_ref(), &no_subs));
            }
        }

        // ── Phase 1b: @method and @property tags from used traits ───────
        //
        // When a class uses a trait that declares `@method` or `@property`
        // tags in its docblock, those virtual members should propagate to
        // the consuming class.  Real trait methods are already merged by
        // `merge_traits_into`, but virtual members from docblock tags are
        // not — they only exist as text in the trait's `class_docblock`.
        for trait_name in &class.used_traits {
            let trait_info = if let Some(t) = class_loader(trait_name) {
                t
            } else {
                continue;
            };

            if let Some(doc) = trait_info.doc_members.as_deref() {
                // The trait's tags are written against its own template
                // parameters, which the class's `@use Trait<…>` binds.
                let trait_subs = build_trait_substitution_map(class, &trait_info, &no_subs);
                for m in &doc.methods {
                    if seen_methods.insert(m.name) {
                        methods.push(substituted_doc_method(m, &trait_subs));
                    }
                }

                for (name, type_hint) in &doc.properties {
                    if seen_props.insert(*name) {
                        properties.push(doc_tag_property(*name, type_hint.as_ref(), &trait_subs));
                    }
                }
            }
        }

        // ── Phase 1c: @method and @property tags from parent classes ────
        //
        // When a parent class declares `@method` or `@property` tags in
        // its docblock, those virtual members should be visible on child
        // classes.  Real inherited methods are already merged by
        // `resolve_class_with_inheritance`, but virtual members from
        // docblock tags are not — they only exist as text in the parent's
        // `class_docblock`.  Walk the parent chain and collect them.
        //
        // Template substitutions from `@extends` annotations are applied
        // so that `@method T get()` on a parent with `@template T` is
        // resolved to the concrete type when the child declares
        // `@extends Parent<ConcreteType>`.
        {
            let mut current: ClassRef<'_> = ClassRef::Borrowed(class);
            let mut active_subs: HashMap<String, PhpType> = HashMap::new();
            let mut depth = 0u32;

            while let Some(ref parent_name) = current.parent_class {
                depth += 1;
                if depth > MAX_INHERITANCE_DEPTH {
                    break;
                }
                let parent = if let Some(p) = class_loader(parent_name) {
                    p
                } else {
                    break;
                };

                // Build a substitution map for this parent level from
                // the child's `@extends` generics.
                let level_subs = build_mixin_substitution_map(
                    &current,
                    &parent,
                    &active_subs,
                    &class.template_param_bounds,
                );

                if let Some(doc) = parent.doc_members.as_deref() {
                    for m in &doc.methods {
                        if seen_methods.insert(m.name) {
                            methods.push(substituted_doc_method(m, &level_subs));
                        }
                    }

                    for (name, type_hint) in &doc.properties {
                        if seen_props.insert(*name) {
                            properties.push(doc_tag_property(
                                *name,
                                type_hint.as_ref(),
                                &level_subs,
                            ));
                        }
                    }
                }

                // Tags on a trait the *parent* uses are inherited too: the
                // parent merges the trait's members, so a child sees them
                // just like the parent's own tags.  Laravel's collections
                // depend on this — the higher-order proxy properties are
                // declared on the `EnumeratesValues` trait that
                // `Support\Collection` uses, and every collection subclass
                // must see them.
                for trait_name in &parent.used_traits {
                    let Some(trait_info) = class_loader(trait_name) else {
                        continue;
                    };
                    let Some(doc) = trait_info.doc_members.as_deref() else {
                        continue;
                    };
                    let trait_subs =
                        build_trait_substitution_map(&parent, &trait_info, &level_subs);
                    for m in &doc.methods {
                        if seen_methods.insert(m.name) {
                            methods.push(substituted_doc_method(m, &trait_subs));
                        }
                    }
                    for (name, type_hint) in &doc.properties {
                        if seen_props.insert(*name) {
                            properties.push(doc_property(*name, type_hint.as_ref(), &trait_subs));
                        }
                    }
                }

                active_subs = level_subs;
                current = ClassRef::Owned(parent);
            }
        }

        // ── Phase 1d: @method and @property tags from implemented interfaces ─
        //
        // When a class implements an interface that declares `@method` or
        // `@property` tags, those virtual members should be visible on the
        // implementing class.  Template substitutions from `@implements`
        // annotations are applied so that `@method E get()` on an interface
        // with `@template E` is resolved to the concrete type when the class
        // declares `@implements I<ConcreteType>`.
        //
        // We also walk each interface's parent interfaces (via `interfaces`
        // field, which stores `extends` for interfaces).
        {
            let mut iface_queue: Vec<(Atom, HashMap<String, PhpType>)> = Vec::new();

            // Seed with the class's own interfaces, building substitution
            // maps from `@implements` generics.
            for iface_name in &class.interfaces {
                if let Some(iface) = class_loader(iface_name) {
                    let subs = build_interface_substitution_map(class, &iface);
                    iface_queue.push((*iface_name, subs));
                }
            }

            let mut visited: HashSet<Atom> = HashSet::new();
            while let Some((iface_name, subs)) = iface_queue.pop() {
                if !visited.insert(iface_name) {
                    continue;
                }
                let iface = if let Some(i) = class_loader(&iface_name) {
                    i
                } else {
                    continue;
                };

                if let Some(doc) = iface.doc_members.as_deref() {
                    for m in &doc.methods {
                        if seen_methods.insert(m.name) {
                            methods.push(substituted_doc_method(m, &subs));
                        }
                    }

                    for (name, type_hint) in &doc.properties {
                        if seen_props.insert(*name) {
                            properties.push(doc_tag_property(*name, type_hint.as_ref(), &subs));
                        }
                    }
                }

                // Walk parent interfaces (interface extends).
                for parent_iface_name in &iface.interfaces {
                    if let Some(parent_iface) = class_loader(parent_iface_name) {
                        let parent_subs =
                            build_interface_extends_substitution_map(&iface, &parent_iface, &subs);
                        iface_queue.push((*parent_iface_name, parent_subs));
                    }
                }
            }
        }

        // ── Phase 2: @mixin members (lower precedence) ─────────────────

        let mixin_dedup = MixinDedup {
            methods: seen_methods,
            properties: seen_props,
            constants: seen_consts,
        };

        let mut collector = MixinCollector {
            methods,
            properties,
            constants,
            dedup: mixin_dedup,
        };

        // Collect from the class's own mixins.
        //
        // No template substitutions are available at this stage because
        // the concrete generic arguments for the class itself are applied
        // later by `apply_generic_args`.  Template-param mixin names
        // (e.g. `@mixin TWraps`) on the own class are resolved during
        // the ancestor walk when a child class provides concrete types
        // via `@extends`.
        let decorated_forward = uses_forwards_calls(class, class_loader);
        collect_mixin_members(
            &class.mixins,
            &class.mixin_generics,
            class_loader,
            &mut collector,
            &MixinSubs {
                subs: &HashMap::new(),
                bounds: &class.template_param_bounds,
                decorated_forward,
            },
            0,
            cache,
        );
        collect_trait_mixin_members(
            class,
            &HashMap::new(),
            class_loader,
            &mut collector,
            decorated_forward,
            cache,
        );

        // Collect from ancestor mixins.
        //
        // As we walk the parent chain we accumulate a substitution map
        // (template-param → concrete-type) so that mixin generic
        // arguments that reference a parent's template params are
        // resolved to concrete types.  For example, when
        // `BelongsTo extends Relation<Product>` and `Relation` has
        // `@mixin Builder<TRelatedModel>`, the walk builds
        // `{TRelatedModel → Product}` from the child's `@extends`
        // generics and applies it to the mixin's generic args, turning
        // `Builder<TRelatedModel>` into `Builder<Product>`.
        let mut current_ancestor: ClassRef<'_> = ClassRef::Borrowed(class);
        let mut active_subs: HashMap<String, PhpType> = HashMap::new();
        let mut depth = 0u32;
        while let Some(ref parent_name) = current_ancestor.parent_class {
            depth += 1;
            if depth > MAX_INHERITANCE_DEPTH {
                break;
            }
            let parent = if let Some(p) = class_loader(parent_name) {
                p
            } else {
                break;
            };

            // Build the substitution map for this parent level.
            let level_subs = build_mixin_substitution_map(
                &current_ancestor,
                &parent,
                &active_subs,
                &class.template_param_bounds,
            );

            if !parent.mixins.is_empty() {
                // Apply the accumulated substitution map to the
                // parent's mixin generic arguments so that template
                // param names are replaced with concrete types.
                let resolved_mixin_generics: Vec<(Atom, Vec<PhpType>)> = if level_subs.is_empty() {
                    parent.mixin_generics.clone()
                } else {
                    parent
                        .mixin_generics
                        .iter()
                        .map(|(name, args)| {
                            let resolved_args: Vec<PhpType> =
                                args.iter().map(|arg| arg.substitute(&level_subs)).collect();
                            (*name, resolved_args)
                        })
                        .collect()
                };

                collect_mixin_members(
                    &parent.mixins,
                    &resolved_mixin_generics,
                    class_loader,
                    &mut collector,
                    &MixinSubs {
                        subs: &level_subs,
                        bounds: &parent.template_param_bounds,
                        decorated_forward,
                    },
                    0,
                    cache,
                );
            }
            collect_trait_mixin_members(
                &parent,
                &level_subs,
                class_loader,
                &mut collector,
                decorated_forward,
                cache,
            );
            active_subs = level_subs;
            current_ancestor = ClassRef::Owned(parent);
        }

        VirtualMembers {
            methods: collector.methods,
            properties: collector.properties,
            constants: collector.constants,
        }
    }
}

/// Build a substitution map for a directly implemented interface.
///
/// Maps the interface's template parameters to the concrete types provided
/// in the class's `@implements` generics.
fn build_interface_substitution_map(
    class: &ClassInfo,
    iface: &ClassInfo,
) -> HashMap<String, PhpType> {
    bind_template_args(iface, &class.implements_generics, &HashMap::new()).unwrap_or_default()
}

/// Build a substitution map for an interface's parent interface (interface extends).
///
/// Maps the parent interface's template parameters to concrete types by
/// resolving through the child interface's `@extends` generics and applying
/// the already-accumulated substitutions.
fn build_interface_extends_substitution_map(
    child_iface: &ClassInfo,
    parent_iface: &ClassInfo,
    active_subs: &HashMap<String, PhpType>,
) -> HashMap<String, PhpType> {
    bind_template_args(parent_iface, &child_iface.extends_generics, active_subs)
        .unwrap_or_else(|| active_subs.clone())
}

/// Map a used trait's template parameters onto the consuming class's
/// `@use Trait<…>` arguments, composed with the substitutions already in
/// force at that point in the inheritance walk.
///
/// Falls back to `active_subs` when the trait is not parameterised or the
/// consumer declared no `@use` generics — in that case the trait's tags are
/// written against whatever names the consumer happens to share, which is
/// the same assumption the class's own trait tags are read under.
fn build_trait_substitution_map(
    consumer: &ClassInfo,
    trait_info: &ClassInfo,
    active_subs: &HashMap<String, PhpType>,
) -> HashMap<String, PhpType> {
    bind_template_args(trait_info, &consumer.use_generics, active_subs)
        .unwrap_or_else(|| active_subs.clone())
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "phpdoc_tests.rs"]
mod tests;
