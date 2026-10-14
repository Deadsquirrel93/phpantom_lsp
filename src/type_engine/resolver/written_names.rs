//! How a class name written in a file reads where a resolution stands.

use std::sync::Arc;

use crate::atom::atom;
use crate::php_type::PhpType;
use crate::types::ClassInfo;

use super::SourceImports;

/// What a class name written in a file is read against: the class the code
/// is in, the namespace and `use` imports in force, the file's own classes,
/// and the loader for names as written.
///
/// Every resolution context carries these and hands out a copy through
/// its `written_names()`, so the rule lives here once.
///
/// The narrowing extractors resolve the class an `instanceof`, `is_a()` or
/// `::class` comparison names as they read it, through
/// [`Self::class_type`], so everything that carries a check onward (the
/// narrowing itself, the exclusions a branch records, a boolean standing
/// for the check) holds the class rather than its spelling, and never sends
/// a resolved name through the imports a second time.
#[derive(Clone, Copy)]
pub(in crate::type_engine) struct WrittenNames<'a> {
    current_class: Option<&'a ClassInfo>,
    /// The namespace a relative name is read in: the current class's, or
    /// outside a class the one in force at the cursor.
    namespace: Option<&'a str>,
    imports: SourceImports<'a>,
    all_classes: &'a [Arc<ClassInfo>],
    content: &'a str,
    cursor_offset: u32,
    source_class_loader: &'a dyn Fn(&str) -> Option<Arc<ClassInfo>>,
}

impl<'a> WrittenNames<'a> {
    /// Names written at `cursor_offset` of `content`, inside
    /// `current_class` when there is one.
    pub(in crate::type_engine) fn new(
        current_class: Option<&'a ClassInfo>,
        all_classes: &'a [Arc<ClassInfo>],
        content: &'a str,
        cursor_offset: u32,
        imports: SourceImports<'a>,
        source_class_loader: &'a dyn Fn(&str) -> Option<Arc<ClassInfo>>,
    ) -> Self {
        let namespace = match current_class {
            Some(cls) => cls.file_namespace.as_deref(),
            None => crate::parser::namespace_at_offset(content, cursor_offset as usize),
        };
        Self {
            current_class,
            namespace,
            imports,
            all_classes,
            content,
            cursor_offset,
            source_class_loader,
        }
    }

    /// The class the code is in, if any.
    pub(in crate::type_engine) fn current_class(&self) -> Option<&'a ClassInfo> {
        self.current_class
    }

    /// Names left as written, for a caller with no file to read them in.
    #[cfg(test)]
    pub(in crate::type_engine) fn verbatim() -> WrittenNames<'static> {
        fn no_class(_: &str) -> Option<Arc<ClassInfo>> {
            None
        }
        WrittenNames {
            current_class: None,
            namespace: None,
            imports: None,
            all_classes: &[],
            content: "",
            cursor_offset: 0,
            source_class_loader: &no_class,
        }
    }

    /// Qualify the class names of a type written in this file (a docblock
    /// read inside a body); see
    /// [`resolve_source_php_type_names`](crate::names::resolve_source_php_type_names).
    pub(in crate::type_engine) fn resolve_type(&self, ty: &PhpType) -> PhpType {
        crate::names::resolve_source_php_type_names(
            ty,
            self.namespace,
            self.imports,
            self.content,
            crate::names::written_name_region(self.content, self.current_class, self.cursor_offset),
            self.all_classes,
            self.source_class_loader,
        )
    }

    /// Qualify the class names of a native type hint written in this file;
    /// see [`resolve_code_php_type_names`](crate::names::resolve_code_php_type_names).
    pub(in crate::type_engine) fn resolve_hint(&self, ty: &PhpType) -> PhpType {
        crate::names::resolve_code_php_type_names(
            ty,
            self.namespace,
            self.imports,
            self.all_classes,
            self.source_class_loader,
        )
    }

    /// Resolve a class name written in this file to its FQN; see
    /// [`resolve_code_class_name`](crate::names::resolve_code_class_name).
    /// `self`, `static` and `parent` name the class the code is in.
    pub(in crate::type_engine) fn resolve_class(&self, name: &str) -> String {
        if let Some(fqn) = crate::class_lookup::resolve_class_keyword(name, self.current_class) {
            return fqn;
        }
        crate::names::resolve_code_class_name(
            name,
            self.namespace,
            self.imports,
            self.all_classes,
            self.source_class_loader,
        )
    }

    /// The class a narrowing check names, or `name` itself when it is
    /// `self`/`static`/`parent` (which the enclosing class answers).
    ///
    /// A class named in code (`instanceof Foo`, `Foo::class`) never falls
    /// back to a global class: an unimported `Foo` inside `namespace App`
    /// is `App\Foo` even when only a global `Foo` exists.
    pub(in crate::type_engine) fn class_type(&self, name: &str) -> PhpType {
        if crate::class_lookup::is_class_keyword(name) {
            return PhpType::named(atom(name));
        }
        PhpType::named(atom(&self.resolve_class(name)))
    }
}
