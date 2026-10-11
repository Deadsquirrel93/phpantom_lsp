//! The test case a Pest test closure is bound to.
//!
//! Pest runs every `it()`/`test()`/`beforeEach()`/`afterEach()` closure
//! bound to an instance of a PHPUnit test case: `PHPUnit\Framework\TestCase`
//! unless `tests/Pest.php` binds another class to the test file's
//! directory with `pest()->extends(…)->in(…)` (or the older
//! `uses(…)->in(…)`).  Pest's own functions cannot know which, so their
//! `@param-closure-this` names the base class, or `Pest\PendingCalls\TestCall`
//! in newer releases, which is what the call returns rather than what the
//! closure is bound to.
//!
//! [`Backend::file_context`] looks up the classes bound to the file, and
//! the function loader rewrites the tag on Pest's functions to the bound
//! test case (see [`Backend::bind_pest_closure_this`]), so every consumer
//! that reads the tag sees the class the closure actually runs as.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use mago_allocator::LocalArena;
use mago_database::file::FileId;
use mago_names::resolver::NameResolver;
use mago_span::HasSpan;
use mago_syntax::cst::*;
use mago_syntax::parser::parse_file_content;
use tower_lsp::lsp_types::Url;

use crate::Backend;
use crate::atom::{atom, bytes_to_str, literal_bytes_to_str};
use crate::names::OwnedResolvedNames;
use crate::types::{ClassLikeKind, FunctionInfo};

/// The test case Pest binds a closure to when nothing else is configured.
const PHPUNIT_TEST_CASE: &str = "PHPUnit\\Framework\\TestCase";

/// What newer Pest releases declare as the closure's `$this`.
const PEST_TEST_CALL: &str = "Pest\\PendingCalls\\TestCall";

/// The Pest functions whose closure runs bound to the test case.
const PEST_TEST_FUNCTIONS: [&str; 4] = ["it", "test", "beforeEach", "afterEach"];

/// The `->in(…)` bindings declared in a project's `tests/Pest.php`.
#[derive(Default)]
pub(crate) struct PestConfig {
    /// URI of the tests directory, with a trailing `/`.  Only files under
    /// it can be bound, which lets every other file skip the path checks.
    tests_dir_uri: String,
    bindings: Vec<Binding>,
}

/// One `pest()->extends(…)->in(…)` chain.
struct Binding {
    /// The classes and traits, as FQNs, in the order they were written.
    uses: Vec<String>,
    targets: Vec<Target>,
}

/// A path passed to `->in(…)`.  Pest expands it with `glob()` and binds
/// the files it matches and everything under the directories it matches.
enum Target {
    Path(PathBuf),
    Glob(globset::GlobMatcher),
}

impl Target {
    fn new(path: PathBuf) -> Option<Self> {
        let text = path.to_str()?;
        if !text.contains(['*', '?', '[', '{']) {
            return Some(Target::Path(path));
        }
        let glob = globset::GlobBuilder::new(text)
            .literal_separator(true)
            .build()
            .ok()?;
        Some(Target::Glob(glob.compile_matcher()))
    }

    fn contains(&self, file: &Path) -> bool {
        match self {
            Target::Path(target) => file.starts_with(target),
            Target::Glob(glob) => file.ancestors().any(|dir| glob.is_match(dir)),
        }
    }
}

impl PestConfig {
    /// Read `tests/Pest.php` under `root`.  A project without one gets an
    /// empty config, which binds nothing.
    fn load(backend: &Backend, root: &Path) -> Self {
        let tests_dir = root.join("tests");
        let config_uri = crate::util::path_to_uri(&tests_dir.join("Pest.php"));
        let Some(content) = backend.get_file_content(&config_uri) else {
            return Self::default();
        };
        let mut tests_dir_uri = crate::util::path_to_uri(&tests_dir);
        tests_dir_uri.push('/');
        Self {
            tests_dir_uri,
            bindings: parse_bindings(&content, &tests_dir),
        }
    }

    /// The classes and traits bound to the file at `uri`, in the order
    /// Pest applies them.
    fn uses_for(&self, uri: &str) -> Option<Arc<[String]>> {
        if self.bindings.is_empty() || !uri.starts_with(&self.tests_dir_uri) {
            return None;
        }
        let path = Url::parse(uri).ok()?.to_file_path().ok()?;
        let uses: Vec<String> = self
            .bindings
            .iter()
            .filter(|binding| binding.targets.iter().any(|t| t.contains(&path)))
            .flat_map(|binding| binding.uses.iter().cloned())
            .collect();
        (!uses.is_empty()).then(|| uses.into())
    }
}

impl Backend {
    /// The classes and traits `tests/Pest.php` binds to the test file at
    /// `uri`, or `None` when it binds none.
    pub(crate) fn pest_uses_for(&self, uri: &str) -> Option<Arc<[String]>> {
        let cached = self.pest_config.read().clone();
        let config = match cached {
            Some(config) => config,
            None => {
                let root = self.workspace.workspace_root.read().clone()?;
                let config = Arc::new(PestConfig::load(self, &root));
                *self.pest_config.write() = Some(Arc::clone(&config));
                config
            }
        };
        config.uses_for(uri)
    }

    /// Forget the parsed `tests/Pest.php` so the next lookup re-reads it.
    pub(crate) fn invalidate_pest_config(&self) {
        *self.pest_config.write() = None;
    }

    /// Point the `@param-closure-this` of Pest's test functions at the
    /// test case the closure is bound to in this file: the class among
    /// `uses` (what `tests/Pest.php` binds here), or PHPUnit's base test
    /// case when it binds none.
    ///
    /// Only Pest's own declarations are rewritten, recognised by the
    /// global name and the class their tag names, so a project's own
    /// `test()` helper keeps whatever it declares.
    pub(crate) fn bind_pest_closure_this(&self, fi: &mut FunctionInfo, uses: Option<&[String]>) {
        if fi.namespace.is_some()
            || !PEST_TEST_FUNCTIONS
                .iter()
                .any(|name| fi.name.eq_ignore_ascii_case(name))
        {
            return;
        }
        let Some(index) = fi.parameters.iter().position(|param| {
            param
                .closure_this_type
                .as_ref()
                .and_then(|ty| ty.base_name())
                .is_some_and(|name| name == PHPUNIT_TEST_CASE || name == PEST_TEST_CALL)
        }) else {
            return;
        };
        let bound = uses
            .into_iter()
            .flatten()
            .find(|name| {
                self.find_or_load_class(name)
                    .is_some_and(|class| class.kind == ClassLikeKind::Class)
            })
            .map_or(PHPUNIT_TEST_CASE, String::as_str);
        fi.parameters.make_mut()[index].closure_this_type =
            Some(crate::php_type::PhpType::named(atom(bound)));
    }
}

/// The bindings of every `pest()->…->in(…)` and `uses(…)->in(…)` chain
/// written as a statement of `content`.  Relative targets are resolved
/// against `tests_dir`, the directory `Pest.php` lives in.
fn parse_bindings(content: &str, tests_dir: &Path) -> Vec<Binding> {
    let arena = LocalArena::new();
    let program = parse_file_content(&arena, FileId::new(b"Pest.php"), content.as_bytes());
    let resolved = NameResolver::new(&arena).resolve(program);
    let names = OwnedResolvedNames::from_resolved(&resolved);
    let mut bindings = Vec::new();
    collect_bindings(program.statements.iter(), &names, tests_dir, &mut bindings);
    bindings
}

fn collect_bindings<'a>(
    statements: impl Iterator<Item = &'a Statement<'a>>,
    names: &OwnedResolvedNames,
    tests_dir: &Path,
    out: &mut Vec<Binding>,
) {
    for statement in statements {
        match statement {
            Statement::Expression(expr) => {
                if let Some(binding) = binding_from_chain(expr.expression, names, tests_dir) {
                    out.push(binding);
                }
            }
            Statement::Namespace(ns) => {
                collect_bindings(ns.statements().iter(), names, tests_dir, out);
            }
            _ => {}
        }
    }
}

/// Read one call chain rooted at `pest()` or `uses()`.  `None` when the
/// chain has another root or binds nothing to any path; a chain without
/// `->in(…)` configures the file it is written in, which for `Pest.php`
/// is no test at all.
fn binding_from_chain(
    expr: &Expression<'_>,
    names: &OwnedResolvedNames,
    tests_dir: &Path,
) -> Option<Binding> {
    let mut uses = Vec::new();
    let mut targets = Vec::new();
    let mut current = expr;
    loop {
        match current {
            Expression::Call(Call::Method(call)) => {
                let ClassLikeMemberSelector::Identifier(method) = &call.method else {
                    return None;
                };
                let method = bytes_to_str(method.value);
                let args = call.argument_list.arguments.iter().map(|arg| arg.value());
                if method.eq_ignore_ascii_case("in") {
                    targets.extend(args.filter_map(|arg| target(arg, tests_dir)));
                } else if ["extends", "extend", "uses", "use"]
                    .iter()
                    .any(|name| method.eq_ignore_ascii_case(name))
                {
                    // Walking outward-in, so a later call's classes go first
                    // and the whole list is reversed at the end.
                    let mut classes: Vec<String> =
                        args.filter_map(|arg| class_name(arg, names)).collect();
                    classes.reverse();
                    uses.extend(classes);
                }
                current = call.object;
            }
            Expression::Call(Call::Function(call)) => {
                let Expression::Identifier(function) = call.function else {
                    return None;
                };
                let function = bytes_to_str(function.value()).trim_start_matches('\\');
                if function.eq_ignore_ascii_case("uses") {
                    let mut classes: Vec<String> = call
                        .argument_list
                        .arguments
                        .iter()
                        .filter_map(|arg| class_name(arg.value(), names))
                        .collect();
                    classes.reverse();
                    uses.extend(classes);
                } else if !function.eq_ignore_ascii_case("pest") {
                    return None;
                }
                break;
            }
            _ => return None,
        }
    }
    if uses.is_empty() || targets.is_empty() {
        return None;
    }
    uses.reverse();
    Some(Binding { uses, targets })
}

/// The FQN a `Foo::class` or `'Foo'` argument names.
fn class_name(expr: &Expression<'_>, names: &OwnedResolvedNames) -> Option<String> {
    if let Expression::Access(Access::ClassConstant(access)) = expr
        && let ClassLikeConstantSelector::Identifier(constant) = &access.constant
        && bytes_to_str(constant.value).eq_ignore_ascii_case("class")
        && let Expression::Identifier(class) = access.class
    {
        let fqn = names
            .get(class.span().start.offset)
            .unwrap_or_else(|| bytes_to_str(class.value()));
        return Some(fqn.trim_start_matches('\\').to_string());
    }
    let name = string_value(expr)?;
    Some(name.trim_start_matches('\\').to_string())
}

/// The path an `->in(…)` argument names: a string relative to the tests
/// directory (or absolute), or `__DIR__ . '/…'`.
fn target(expr: &Expression<'_>, tests_dir: &Path) -> Option<Target> {
    if let Expression::Binary(binary) = expr
        && binary.operator.is_concatenation()
        && matches!(
            binary.lhs,
            Expression::MagicConstant(MagicConstant::Directory(_))
        )
    {
        let rest = string_value(binary.rhs)?;
        return Target::new(tests_dir.join(rest.trim_start_matches('/')));
    }
    let path = string_value(expr)?;
    Target::new(tests_dir.join(path.trim_end_matches('/')))
}

fn string_value<'a>(expr: &Expression<'a>) -> Option<&'a str> {
    match expr {
        Expression::Literal(Literal::String(s)) => literal_bytes_to_str(s.value?),
        _ => None,
    }
}
