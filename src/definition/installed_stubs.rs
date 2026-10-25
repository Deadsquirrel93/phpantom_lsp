//! Go-to-definition on built-in symbols through the project's installed
//! `jetbrains/phpstorm-stubs`.
//!
//! The embedded stubs have no file to jump to.  When Composer installed
//! phpstorm-stubs, its `PhpStormStubsMap.php` names the file that declares
//! each built-in, so the jump lands there instead.  Only navigation reads
//! these files: types still come from the embedded stubs, and a file read
//! here is parsed for the declaration's offset and never cached as a
//! source of declarations.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tower_lsp_server::gen_lsp_types::Location;

use super::member::MemberKind;
use super::{class_keyword_location, point_location};
use crate::Backend;
use crate::ci_map::fold;
use crate::stub_map::parse_section;

/// The Composer package whose files the jump lands in.
const PACKAGE: &str = "jetbrains/phpstorm-stubs";

/// The symbol-to-file index of an installed phpstorm-stubs package.
pub(crate) struct InstalledStubsMap {
    /// The package root, which the map's paths are relative to.
    root: PathBuf,
    /// Class-likes and functions, keyed by folded name, since PHP resolves
    /// them case-insensitively.  Each value indexes `files`.
    classes: HashMap<String, u32>,
    functions: HashMap<String, u32>,
    /// Constants, keyed by name as declared.
    constants: HashMap<String, u32>,
    /// Every file the map names, once each: thousands of symbols share a
    /// few hundred files.
    files: Vec<Box<str>>,
}

impl InstalledStubsMap {
    fn parse(root: PathBuf, content: &str) -> Self {
        let mut files: Vec<Box<str>> = Vec::new();
        let mut file_ids: HashMap<String, u32> = HashMap::new();
        let mut section = |name: &str, fold_keys: bool| -> HashMap<String, u32> {
            parse_section(content, name)
                .into_iter()
                .map(|(symbol, file)| {
                    let id = *file_ids.entry(file).or_insert_with_key(|file| {
                        files.push(file.as_str().into());
                        (files.len() - 1) as u32
                    });
                    let key = if fold_keys {
                        fold(&symbol).into_owned()
                    } else {
                        symbol
                    };
                    (key, id)
                })
                .collect()
        };
        let classes = section("CLASSES", true);
        let functions = section("FUNCTIONS", true);
        let constants = section("CONSTANTS", false);
        Self {
            root,
            classes,
            functions,
            constants,
            files,
        }
    }

    fn file(&self, id: Option<&u32>) -> Option<PathBuf> {
        Some(self.root.join(&*self.files[*id? as usize]))
    }

    fn class_file(&self, fqn: &str) -> Option<PathBuf> {
        self.file(self.classes.get(fold(fqn).as_ref()))
    }

    fn function_file(&self, fqn: &str) -> Option<PathBuf> {
        self.file(self.functions.get(fold(fqn).as_ref()))
    }

    fn constant_file(&self, name: &str) -> Option<PathBuf> {
        self.file(self.constants.get(name))
    }
}

/// `path`'s source and the `file://` URI an editor opens it under.
fn read_stub_file(path: &Path) -> Option<(String, String)> {
    let content = crate::util::read_php_source(path)?;
    Some((crate::util::path_to_uri(path), content))
}

/// Whether `name` (namespace and short name, as declared) spells `fqn`,
/// ignoring ASCII case.
fn spells(namespace: Option<&str>, name: &str, fqn: &str) -> bool {
    match namespace.filter(|ns| !ns.is_empty()) {
        Some(ns) => {
            fqn.len() == ns.len() + 1 + name.len()
                && fqn[..ns.len()].eq_ignore_ascii_case(ns)
                && fqn.as_bytes()[ns.len()] == b'\\'
                && fqn[ns.len() + 1..].eq_ignore_ascii_case(name)
        }
        None => fqn.eq_ignore_ascii_case(name),
    }
}

impl Backend {
    /// The installed phpstorm-stubs' map, read on first use.
    ///
    /// `None` when the project has not installed the package, or its map
    /// file cannot be read.  The cache is dropped whenever the Composer
    /// package roots are replaced, so a `composer update` or `remove` is
    /// seen on the next jump.
    fn installed_stubs_map(&self) -> Option<Arc<InstalledStubsMap>> {
        if let Some(map) = self.workspace.installed_stubs_map.read().as_ref() {
            return Some(map.clone());
        }
        let root = self
            .workspace
            .vendor_package_origin_roots
            .read()
            .iter()
            .find(|(_, _, name)| name == PACKAGE)
            .map(|(root, _, _)| root.clone())?;
        let content = std::fs::read_to_string(root.join("PhpStormStubsMap.php")).ok()?;
        let map = Arc::new(InstalledStubsMap::parse(root, &content));
        *self.workspace.installed_stubs_map.write() = Some(map.clone());
        Some(map)
    }

    /// The classes declared in the installed stub file that declares the
    /// built-in class `fqn`, with that file's URI and source.
    fn installed_stub_class(&self, fqn: &str) -> Option<(crate::types::ClassInfo, String, String)> {
        let fqn = fqn.trim_start_matches('\\');
        let path = self.installed_stubs_map()?.class_file(fqn)?;
        let (uri, content) = read_stub_file(&path)?;
        let class = Self::parse_php_versioned_with_namespaces(&content, Some(self.php_version()))
            .into_iter()
            .find(|(cls, ns)| spells(ns.as_deref(), &cls.name, fqn))
            .map(|(cls, _)| cls)?;
        Some((class, uri, content))
    }

    /// The declaration of the built-in class `fqn` in the installed
    /// phpstorm-stubs, when the embedded stubs declare it.
    pub(crate) fn builtin_class_location(&self, fqn: &str) -> Option<Location> {
        let fqn = fqn.trim_start_matches('\\');
        if !self.stub_index.read().contains_key(fqn) {
            return None;
        }
        let (class, uri, content) = self.installed_stub_class(fqn)?;
        class_keyword_location(&class, &uri, &content)
    }

    /// The declaration of a member of the built-in class `class_fqn` in the
    /// installed phpstorm-stubs.
    pub(crate) fn builtin_member_location(
        &self,
        class_fqn: &str,
        member_name: &str,
        kind: MemberKind,
    ) -> Option<Location> {
        let (class, uri, content) = self.installed_stub_class(class_fqn)?;
        let position = Self::find_member_position(&content, member_name, kind, &class)?;
        Some(point_location(crate::util::parse_url(&uri)?, position))
    }

    /// The declaration of the built-in function `fqn` in the installed
    /// phpstorm-stubs.
    pub(crate) fn builtin_function_location(&self, fqn: &str) -> Option<Location> {
        let fqn = fqn.trim_start_matches('\\');
        let path = self.installed_stubs_map()?.function_file(fqn)?;
        let (uri, content) = read_stub_file(&path)?;
        // A function declared once per PHP version range is the one the
        // configured version gets.
        let function = self
            .parse_functions_versioned(&content, Some(self.php_version()))
            .into_iter()
            .find(|f| spells(f.namespace.as_deref(), &f.name, fqn))?;
        offset_location(&uri, &content, function.name_offset)
    }

    /// The declaration of the first of `candidates` the embedded stubs
    /// declare as a constant, in the installed phpstorm-stubs.
    pub(crate) fn builtin_constant_location(&self, candidates: &[&str]) -> Option<Location> {
        let name = {
            let stubs = self.stub_constant_index.read();
            candidates.iter().copied().find(|c| stubs.contains_key(c))?
        };
        let path = self.installed_stubs_map()?.constant_file(name)?;
        let (uri, content) = read_stub_file(&path)?;
        let (_, offset, _) = self
            .parse_defines(&content)
            .into_iter()
            .find(|(declared, _, _)| declared == name)?;
        offset_location(&uri, &content, offset)
    }
}

/// A point location at `offset` in `content`, the source of `uri`.
fn offset_location(uri: &str, content: &str, offset: u32) -> Option<Location> {
    if offset == 0 || offset as usize > content.len() {
        return None;
    }
    let position = crate::text_position::offset_to_position(content, offset as usize);
    Some(point_location(crate::util::parse_url(uri)?, position))
}
