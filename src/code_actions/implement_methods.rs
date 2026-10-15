//! "Implement missing methods" code action.
//!
//! When the cursor is inside a non-abstract class that extends an abstract
//! class or implements an interface but is missing required method
//! implementations, this module offers a code action to generate stubs
//! for all missing methods.

use std::collections::HashMap;
use std::sync::Arc;

use tower_lsp::lsp_types::*;

use crate::Backend;
use crate::class_lookup::find_class_at_offset;
use crate::signature_render::{detect_class_indent, format_params, format_return_type};
use crate::text_position::offset_to_position;
use crate::types::{ClassInfo, ClassLikeKind, MethodInfo, Visibility};

impl Backend {
    /// Collect "Implement missing methods" code actions for the cursor position.
    ///
    /// When the cursor is inside a concrete class that has unimplemented
    /// abstract or interface methods, this produces one code action that
    /// inserts stubs for all missing methods just before the class's
    /// closing brace.
    pub(crate) fn collect_implement_methods_actions(
        &self,
        uri: &str,
        content: &str,
        params: &CodeActionParams,
        out: &mut Vec<CodeActionOrCommand>,
    ) {
        let cursor_offset = crate::text_position::position_to_offset(content, params.range.start);
        let ctx = self.file_context_at(uri, cursor_offset);

        // Find the class the cursor is inside. `find_class_at_offset`'s
        // lower bound also covers the `class Foo implements Bar`
        // declaration line (and any attributes above it), before the `{`.
        let current_class = match find_class_at_offset(&ctx.classes, cursor_offset) {
            Some(c) => c,
            None => return,
        };

        // Only concrete classes can implement missing methods.
        // Abstract classes, interfaces, traits, and enums are skipped.
        if current_class.kind != ClassLikeKind::Class || current_class.is_abstract {
            return;
        }

        // Resolve the full inheritance hierarchy to collect all abstract
        // and interface methods.
        let class_loader = self.resolved_class_loader(&ctx);

        let missing = collect_missing_methods(current_class, &class_loader);

        if missing.is_empty() {
            return;
        }

        // Determine the use_map so we can shorten FQNs in generated stubs.
        let use_map: HashMap<String, String> = ctx.use_map.clone();
        let file_namespace = ctx.namespace.clone();

        let stub_text =
            build_method_stubs(&missing, &use_map, &file_namespace, content, current_class);

        // Insert position: just before the closing brace of the class.
        // `end_offset` points one byte past the `}`, so `end_offset - 1`
        // is the `}` itself.  We insert before that.
        let insert_offset = (current_class.end_offset - 1) as usize;
        let insert_pos = offset_to_position(content, insert_offset);

        let title = if missing.len() == 1 {
            format!("Implement `{}`", missing[0].name)
        } else {
            format!("Implement {} missing methods", missing.len())
        };

        let edit = TextEdit {
            range: Range {
                start: insert_pos,
                end: insert_pos,
            },
            new_text: stub_text,
        };

        let doc_uri: Url = match uri.parse() {
            Ok(u) => u,
            Err(_) => return,
        };

        out.push(CodeActionOrCommand::CodeAction(CodeAction {
            title,
            kind: Some(CodeActionKind::QUICKFIX),
            diagnostics: None,
            edit: Some(crate::code_actions::single_file_edit(doc_uri, vec![edit])),
            command: None,
            is_preferred: Some(true),
            disabled: None,
            data: None,
        }));
    }
}

/// Collect abstract/interface methods that the given concrete class has
/// not yet implemented.
///
/// Walks the full inheritance chain (parent classes, interfaces, traits)
/// and returns methods that are abstract and not already defined on the
/// class itself or inherited as concrete from parent classes.
pub(crate) fn collect_missing_methods(
    class: &ClassInfo,
    class_loader: &dyn Fn(&str) -> Option<Arc<ClassInfo>>,
) -> Vec<MethodInfo> {
    // Build the full set of concrete method names already available on
    // this class: own methods + concrete methods from used traits +
    // concrete methods inherited from the parent chain (including
    // their traits).  This ensures that if a trait or parent implements
    // an interface method, the child doesn't re-stub it.
    let mut implemented_names: Vec<String> = class
        .methods
        .iter()
        .map(|m| m.name.to_lowercase())
        .collect();

    collect_concrete_trait_methods_atoms(
        &class.used_traits,
        class_loader,
        &mut implemented_names,
        0,
    );
    collect_concrete_parent_methods_atom(
        &class.parent_class,
        class_loader,
        &mut implemented_names,
        0,
    );

    let mut missing: Vec<MethodInfo> = Vec::new();
    let mut seen: Vec<String> = Vec::new();

    // ── Interfaces ──────────────────────────────────────────────────────
    for iface_name in &class.interfaces {
        // Skip implicit BackedEnum/UnitEnum interfaces on enums.
        // PHP provides from(), tryFrom(), and cases() automatically
        // at runtime — they don't need to be declared in the enum body.
        if class.kind == crate::types::ClassLikeKind::Enum {
            let iface_str: &str = iface_name;
            let stripped = iface_str.strip_prefix('\\').unwrap_or(iface_str);
            if stripped == "BackedEnum" || stripped == "UnitEnum" {
                continue;
            }
        }
        collect_from_interface(
            iface_name,
            class_loader,
            &implemented_names,
            &mut missing,
            &mut seen,
            0,
        );
    }

    // ── Parent chain (abstract methods) ─────────────────────────────────
    collect_from_parent_chain_atom(
        &class.parent_class,
        class_loader,
        &implemented_names,
        &mut missing,
        &mut seen,
        0,
    );

    // ── Used traits (abstract methods) ──────────────────────────────────
    // For enums, filter out the implicit BackedEnum/UnitEnum traits
    // whose abstract methods (from(), tryFrom(), cases()) are provided
    // by PHP at runtime.
    let trait_names: Vec<_> = if class.kind == crate::types::ClassLikeKind::Enum {
        class
            .used_traits
            .iter()
            .filter(|t| {
                let stripped = t.strip_prefix('\\').unwrap_or(t);
                stripped != "BackedEnum" && stripped != "UnitEnum"
            })
            .cloned()
            .collect()
    } else {
        class.used_traits.to_vec()
    };
    collect_abstract_from_used_traits(
        &trait_names,
        class_loader,
        &implemented_names,
        &mut missing,
        &mut seen,
        0,
    );

    missing
}

/// Walk the parent chain and collect names of concrete (non-abstract)
/// methods into `implemented`.  This lets us know which interface or
/// abstract methods are already satisfied by a parent class.
///
/// Also collects concrete methods from traits used by each parent,
/// since trait methods are effectively part of the class in PHP.
fn collect_concrete_parent_methods_atom(
    parent_name: &Option<crate::atom::Atom>,
    class_loader: &dyn Fn(&str) -> Option<Arc<ClassInfo>>,
    implemented: &mut Vec<String>,
    depth: usize,
) {
    if depth > crate::types::MAX_INHERITANCE_DEPTH as usize {
        return;
    }

    let parent_name = match parent_name {
        Some(n) => n,
        None => return,
    };

    let parent = match class_loader(parent_name) {
        Some(c) => c,
        None => return,
    };

    for method in &parent.methods {
        if !method.is_abstract {
            let lower = method.name.to_lowercase();
            if !implemented.contains(&lower) {
                implemented.push(lower);
            }
        }
    }

    // Traits used by the parent also provide concrete methods.
    collect_concrete_trait_methods_atoms(&parent.used_traits, class_loader, implemented, depth + 1);

    collect_concrete_parent_methods_atom(
        &parent.parent_class,
        class_loader,
        implemented,
        depth + 1,
    );
}

/// Walk a list of used traits (and their sub-traits and parent classes)
/// and collect names of concrete (non-abstract) methods into
/// `implemented`.  In PHP, trait methods are effectively part of the
/// class that uses them, so they satisfy interface and abstract-method
/// requirements.
fn collect_concrete_trait_methods_atoms(
    trait_names: &[crate::atom::Atom],
    class_loader: &dyn Fn(&str) -> Option<Arc<ClassInfo>>,
    implemented: &mut Vec<String>,
    depth: usize,
) {
    if depth > crate::types::MAX_INHERITANCE_DEPTH as usize {
        return;
    }

    for trait_name in trait_names {
        let trait_info = match class_loader(trait_name) {
            Some(c) => c,
            None => continue,
        };

        for method in &trait_info.methods {
            if !method.is_abstract {
                let lower = method.name.to_lowercase();
                if !implemented.contains(&lower) {
                    implemented.push(lower);
                }
            }
        }

        // Traits can use other traits — recurse into sub-traits.
        if !trait_info.used_traits.is_empty() {
            collect_concrete_trait_methods_atoms(
                &trait_info.used_traits,
                class_loader,
                implemented,
                depth + 1,
            );
        }

        // Traits can also extend a parent class (rare but valid in
        // the class model — e.g. stubs may model this).
        collect_concrete_parent_methods_atom(
            &trait_info.parent_class,
            class_loader,
            implemented,
            depth + 1,
        );
    }
}

/// Recursively collect unimplemented methods from an interface and its
/// parent interfaces.
fn collect_from_interface(
    iface_name: &str,
    class_loader: &dyn Fn(&str) -> Option<Arc<ClassInfo>>,
    own_methods: &[String],
    missing: &mut Vec<MethodInfo>,
    seen: &mut Vec<String>,
    depth: usize,
) {
    if depth > crate::types::MAX_INHERITANCE_DEPTH as usize {
        return;
    }

    let iface = match class_loader(iface_name) {
        Some(c) if c.kind == ClassLikeKind::Interface => c,
        _ => return,
    };

    for method in &iface.methods {
        let lower = method.name.to_lowercase();
        if own_methods.contains(&lower) || seen.contains(&lower) {
            continue;
        }
        seen.push(lower);
        missing.push((**method).clone());
    }

    // Recurse into parent interfaces.
    for parent_iface in &iface.interfaces {
        collect_from_interface(
            parent_iface,
            class_loader,
            own_methods,
            missing,
            seen,
            depth + 1,
        );
    }

    // Interfaces can also extend other interfaces via parent_class in
    // some parser representations.
    if let Some(parent) = iface.parent_class {
        collect_from_interface(&parent, class_loader, own_methods, missing, seen, depth + 1);
    }
}

/// Walk the parent class chain and collect abstract methods that need
/// implementation.
fn collect_from_parent_chain_atom(
    parent_name: &Option<crate::atom::Atom>,
    class_loader: &dyn Fn(&str) -> Option<Arc<ClassInfo>>,
    own_methods: &[String],
    missing: &mut Vec<MethodInfo>,
    seen: &mut Vec<String>,
    depth: usize,
) {
    if depth > crate::types::MAX_INHERITANCE_DEPTH as usize {
        return;
    }

    let parent_name = match parent_name {
        Some(n) => n,
        None => return,
    };

    let parent = match class_loader(parent_name) {
        Some(c) => c,
        None => return,
    };

    // Collect interfaces from the parent that the child inherits
    // transitively.
    for iface_name in &parent.interfaces {
        collect_from_interface(
            iface_name,
            class_loader,
            own_methods,
            missing,
            seen,
            depth + 1,
        );
    }

    // Collect abstract methods from the parent itself.
    for method in &parent.methods {
        if !method.is_abstract {
            continue;
        }
        let lower = method.name.to_lowercase();
        if own_methods.contains(&lower) || seen.contains(&lower) {
            continue;
        }
        seen.push(lower);
        missing.push((**method).clone());
    }

    collect_from_parent_chain_atom(
        &parent.parent_class,
        class_loader,
        own_methods,
        missing,
        seen,
        depth + 1,
    );
}

/// Walk used traits and collect their abstract methods that need
/// implementation.  Recurses into sub-traits.
fn collect_abstract_from_used_traits(
    trait_names: &[crate::atom::Atom],
    class_loader: &dyn Fn(&str) -> Option<Arc<ClassInfo>>,
    own_methods: &[String],
    missing: &mut Vec<MethodInfo>,
    seen: &mut Vec<String>,
    depth: usize,
) {
    if depth > crate::types::MAX_INHERITANCE_DEPTH as usize {
        return;
    }

    for trait_name in trait_names {
        let trait_info = match class_loader(trait_name) {
            Some(c) => c,
            None => continue,
        };

        for method in &trait_info.methods {
            if !method.is_abstract {
                continue;
            }
            let lower = method.name.to_lowercase();
            if own_methods.contains(&lower) || seen.contains(&lower) {
                continue;
            }
            seen.push(lower);
            missing.push((**method).clone());
        }

        if !trait_info.used_traits.is_empty() {
            collect_abstract_from_used_traits(
                &trait_info.used_traits,
                class_loader,
                own_methods,
                missing,
                seen,
                depth + 1,
            );
        }
    }
}

/// Build the source text for all missing method stubs.
///
/// Each stub includes visibility, static modifier, parameter list with
/// type hints and defaults, return type, and an empty body.
fn build_method_stubs(
    methods: &[MethodInfo],
    use_map: &HashMap<String, String>,
    file_namespace: &Option<String>,
    content: &str,
    class: &ClassInfo,
) -> String {
    let indent = detect_class_indent(content, class);

    let mut result = String::new();

    for method in methods {
        result.push('\n');

        // Visibility — interface methods become public, abstract methods
        // keep their declared visibility (but never private).
        let vis = match method.visibility {
            Visibility::Private => "public",
            Visibility::Protected => "protected",
            Visibility::Public => "public",
        };

        let static_kw = if method.is_static { "static " } else { "" };

        let params = format_params(method, use_map, file_namespace);
        let return_type = format_return_type(method, use_map, file_namespace);

        result.push_str(&indent);
        result.push_str(&format!(
            "{} {}function {}({}){}\n",
            vis, static_kw, method.name, params, return_type,
        ));
        result.push_str(&indent);
        result.push_str("{\n");
        result.push_str(&indent);
        result.push_str("}\n");
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::php_type::PhpType;
    use crate::types::{ParameterInfo, Visibility};

    // ── collect_missing_methods tests ───────────────────────────────────

    #[test]
    fn collects_interface_methods() {
        let interface = ClassInfo {
            kind: ClassLikeKind::Interface,
            name: crate::atom::atom("Renderable"),
            methods: vec![Arc::new(MethodInfo::virtual_method(
                "render",
                Some("string"),
            ))]
            .into(),
            ..Default::default()
        };

        let class = ClassInfo {
            kind: ClassLikeKind::Class,
            name: crate::atom::atom("Page"),
            interfaces: vec![crate::atom::atom("Renderable")],
            methods: Default::default(),
            ..Default::default()
        };

        let loader = |name: &str| -> Option<Arc<ClassInfo>> {
            if name == "Renderable" {
                Some(Arc::new(interface.clone()))
            } else {
                None
            }
        };

        let missing = collect_missing_methods(&class, &loader);
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].name, "render");
    }

    #[test]
    fn skips_already_implemented_methods() {
        let interface = ClassInfo {
            kind: ClassLikeKind::Interface,
            name: crate::atom::atom("Renderable"),
            methods: vec![Arc::new(MethodInfo::virtual_method(
                "render",
                Some("string"),
            ))]
            .into(),
            ..Default::default()
        };

        let class = ClassInfo {
            kind: ClassLikeKind::Class,
            name: crate::atom::atom("Page"),
            interfaces: vec![crate::atom::atom("Renderable")],
            methods: vec![Arc::new(MethodInfo::virtual_method(
                "render",
                Some("string"),
            ))]
            .into(),
            ..Default::default()
        };

        let loader = |name: &str| -> Option<Arc<ClassInfo>> {
            if name == "Renderable" {
                Some(Arc::new(interface.clone()))
            } else {
                None
            }
        };

        let missing = collect_missing_methods(&class, &loader);
        assert!(missing.is_empty());
    }

    #[test]
    fn collects_abstract_parent_methods() {
        let parent = ClassInfo {
            kind: ClassLikeKind::Class,
            name: crate::atom::atom("AbstractBase"),
            is_abstract: true,
            methods: vec![
                Arc::new(MethodInfo {
                    is_abstract: true,
                    ..MethodInfo::virtual_method("doWork", None)
                }),
                // Concrete method — should NOT be in missing list.
                Arc::new(MethodInfo::virtual_method("helper", Some("void"))),
            ]
            .into(),
            ..Default::default()
        };

        let class = ClassInfo {
            kind: ClassLikeKind::Class,
            name: crate::atom::atom("ConcreteChild"),
            parent_class: Some(crate::atom::atom("AbstractBase")),
            methods: Default::default(),
            ..Default::default()
        };

        let loader = |name: &str| -> Option<Arc<ClassInfo>> {
            if name == "AbstractBase" {
                Some(Arc::new(parent.clone()))
            } else {
                None
            }
        };

        let missing = collect_missing_methods(&class, &loader);
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].name, "doWork");
    }

    #[test]
    fn case_insensitive_method_matching() {
        let interface = ClassInfo {
            kind: ClassLikeKind::Interface,
            name: crate::atom::atom("Renderable"),
            methods: vec![Arc::new(MethodInfo::virtual_method(
                "Render",
                Some("string"),
            ))]
            .into(),
            ..Default::default()
        };

        let class = ClassInfo {
            kind: ClassLikeKind::Class,
            name: crate::atom::atom("Page"),
            interfaces: vec![crate::atom::atom("Renderable")],
            methods: vec![Arc::new(MethodInfo::virtual_method(
                "render",
                Some("string"),
            ))]
            .into(),
            ..Default::default()
        };

        let loader = |name: &str| -> Option<Arc<ClassInfo>> {
            if name == "Renderable" {
                Some(Arc::new(interface.clone()))
            } else {
                None
            }
        };

        let missing = collect_missing_methods(&class, &loader);
        assert!(missing.is_empty(), "PHP method names are case-insensitive");
    }

    #[test]
    fn collects_from_parent_interfaces() {
        let parent = ClassInfo {
            kind: ClassLikeKind::Class,
            name: crate::atom::atom("AbstractBase"),
            is_abstract: true,
            interfaces: vec![crate::atom::atom("Serializable")],
            methods: Default::default(),
            ..Default::default()
        };

        let serializable = ClassInfo {
            kind: ClassLikeKind::Interface,
            name: crate::atom::atom("Serializable"),
            methods: vec![
                Arc::new(MethodInfo::virtual_method("serialize", Some("string"))),
                Arc::new(MethodInfo::virtual_method("unserialize", None)),
            ]
            .into(),
            ..Default::default()
        };

        let class = ClassInfo {
            kind: ClassLikeKind::Class,
            name: crate::atom::atom("ConcreteChild"),
            parent_class: Some(crate::atom::atom("AbstractBase")),
            methods: Default::default(),
            ..Default::default()
        };

        let loader = |name: &str| -> Option<Arc<ClassInfo>> {
            match name {
                "AbstractBase" => Some(Arc::new(parent.clone())),
                "Serializable" => Some(Arc::new(serializable.clone())),
                _ => None,
            }
        };

        let missing = collect_missing_methods(&class, &loader);
        assert_eq!(missing.len(), 2);
        let names: Vec<&str> = missing.iter().map(|m| m.name.as_str()).collect();
        assert!(names.contains(&"serialize"));
        assert!(names.contains(&"unserialize"));
    }

    // ── build_method_stubs tests ────────────────────────────────────────

    #[test]
    fn stub_includes_return_type() {
        let methods = vec![MethodInfo {
            native_return_type: Some(PhpType::parse("string")),
            visibility: Visibility::Public,
            ..MethodInfo::virtual_method("render", Some("string"))
        }];

        let content = "<?php\nclass Foo {\n    \n}\n";
        let class = ClassInfo {
            name: crate::atom::atom("Foo"),
            start_offset: content.find('{').unwrap() as u32,
            end_offset: content.rfind('}').unwrap() as u32 + 1,
            ..Default::default()
        };

        let result = build_method_stubs(&methods, &HashMap::new(), &None, content, &class);
        assert!(result.contains("public function render(): string"));
    }

    #[test]
    fn stub_preserves_static_modifier() {
        let methods = vec![MethodInfo {
            is_static: true,
            native_return_type: Some(PhpType::parse("void")),
            visibility: Visibility::Public,
            ..MethodInfo::virtual_method("init", Some("void"))
        }];

        let content = "<?php\nclass Foo {\n    \n}\n";
        let class = ClassInfo {
            name: crate::atom::atom("Foo"),
            start_offset: content.find('{').unwrap() as u32,
            end_offset: content.rfind('}').unwrap() as u32 + 1,
            ..Default::default()
        };

        let result = build_method_stubs(&methods, &HashMap::new(), &None, content, &class);
        assert!(result.contains("public static function init(): void"));
    }

    #[test]
    fn stub_omits_generic_docblock_return_type() {
        // When a method has no native return type but only a @return with
        // generic syntax (array<K,V>), the stub must NOT emit the generic
        // type as a native PHP return type hint because that is a syntax
        // error in PHP.
        let methods = vec![MethodInfo {
            native_return_type: None,
            return_type: Some(PhpType::parse("array<TKey, TValue>")),
            visibility: Visibility::Public,
            ..MethodInfo::virtual_method("toArray", None)
        }];

        let content = "<?php\nclass Foo {\n    \n}\n";
        let class = ClassInfo {
            name: crate::atom::atom("Foo"),
            start_offset: content.find('{').unwrap() as u32,
            end_offset: content.rfind('}').unwrap() as u32 + 1,
            ..Default::default()
        };

        let result = build_method_stubs(&methods, &HashMap::new(), &None, content, &class);
        // No return type hint should be present.
        assert!(
            !result.contains(": array<"),
            "expected no generic return type hint, got: {result}"
        );
        assert!(
            result.contains("public function toArray()"),
            "expected stub without return type, got: {result}"
        );
    }

    #[test]
    fn stub_keeps_protected_visibility() {
        let methods = vec![MethodInfo {
            visibility: Visibility::Protected,
            ..MethodInfo::virtual_method("doWork", None)
        }];

        let content = "<?php\nclass Foo {\n    \n}\n";
        let class = ClassInfo {
            name: crate::atom::atom("Foo"),
            start_offset: content.find('{').unwrap() as u32,
            end_offset: content.rfind('}').unwrap() as u32 + 1,
            ..Default::default()
        };

        let result = build_method_stubs(&methods, &HashMap::new(), &None, content, &class);
        assert!(result.contains("protected function doWork()"));
    }

    #[test]
    fn stub_promotes_private_to_public() {
        let methods = vec![MethodInfo {
            visibility: Visibility::Private,
            ..MethodInfo::virtual_method("doWork", None)
        }];

        let content = "<?php\nclass Foo {\n    \n}\n";
        let class = ClassInfo {
            name: crate::atom::atom("Foo"),
            start_offset: content.find('{').unwrap() as u32,
            end_offset: content.rfind('}').unwrap() as u32 + 1,
            ..Default::default()
        };

        let result = build_method_stubs(&methods, &HashMap::new(), &None, content, &class);
        assert!(result.contains("public function doWork()"));
    }

    #[test]
    fn stub_with_parameters_and_defaults() {
        let methods = vec![MethodInfo {
            parameters: vec![
                ParameterInfo {
                    name: crate::atom::atom("$name"),
                    is_required: true,
                    type_hint: Some(PhpType::parse("string")),
                    native_type_hint: Some(PhpType::parse("string")),
                    description: None,
                    default_value: None,
                    is_variadic: false,
                    is_reference: false,
                    closure_this_type: None,
                    param_out_type: None,
                },
                ParameterInfo {
                    name: crate::atom::atom("$options"),
                    is_required: false,
                    type_hint: Some(PhpType::parse("array")),
                    native_type_hint: Some(PhpType::parse("array")),
                    description: None,
                    default_value: Some("[]".to_string()),
                    is_variadic: false,
                    is_reference: false,
                    closure_this_type: None,
                    param_out_type: None,
                },
            ]
            .into(),
            native_return_type: Some(PhpType::parse("void")),
            visibility: Visibility::Public,
            ..MethodInfo::virtual_method("process", Some("void"))
        }];

        let content = "<?php\nclass Foo {\n    \n}\n";
        let class = ClassInfo {
            name: crate::atom::atom("Foo"),
            start_offset: content.find('{').unwrap() as u32,
            end_offset: content.rfind('}').unwrap() as u32 + 1,
            ..Default::default()
        };

        let result = build_method_stubs(&methods, &HashMap::new(), &None, content, &class);
        assert!(
            result.contains("public function process(string $name, array $options = []): void")
        );
    }
}
