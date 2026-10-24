//! The string keys a symbol map records for framework lookups, and the
//! spans that only count while some workspace symbol is absent.

use crate::atom::Atom;

use super::{LaravelConfigResource, SymbolKind, SymbolMap, SymbolSpan};

/// Identifies the category of a [`SymbolKind::StringKey`] span.
///
/// One flat list for every framework: each variant names what its key
/// resolves against, so a framework's kinds sit beside the others rather
/// than behind a per-framework wrapper. The variants below are all Laravel's.
///
/// Adding a new string-key navigation feature only requires adding a variant
/// here and updating the extraction and dispatch paths — the exhaustive
/// match arms in `highlight`, `hover`, `rename`, `semantic_tokens`, and
/// `type_definition` do not need to change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum StringKeyKind {
    /// A `config('dot.key')` or `Config::get('dot.key')` call.
    Config,
    /// A short name whose declaration is a direct child of a known config
    /// subtree, such as a storage disk or database connection.
    ConfigResource(LaravelConfigResource),
    /// A `view('name')` or `View::make('name')` call.
    View,
    /// A `route('name')` call.
    Route,
    /// A `__('key')`, `trans('key')`, or `Lang::get('key')` call.
    Trans,
    /// An Artisan command name, e.g. `Artisan::call('app:sync')`,
    /// `Schedule::command('app:sync')`, or `$this->call('app:sync')`
    /// inside a console command.
    Command,
    /// An Eloquent morph alias: the short name a `Relation::morphMap()`
    /// registration substitutes for a model FQCN in a `*_type` column.
    /// Emitted for the registration's own keys, for
    /// `Relation::getMorphedModel('post')`, and for the `$types` argument of
    /// the `whereHasMorph()` family.
    MorphAlias,
    /// A Blade section name: the string `@yield('content')` renders and
    /// `@section('content')` fills. Emitted for the marker calls the Blade
    /// preprocessor lowers those directives to.
    Section,
    /// A Blade stack name, the `@stack`/`@push` counterpart of
    /// [`Self::Section`].
    Stack,
    /// An authorization ability name: the string a `Gate::allows()`,
    /// `$user->can()`, `$this->authorize()`, `can:` middleware parameter, or
    /// Blade `@can` directive checks.  Resolves against `Gate::define()`
    /// registrations and the methods of the relevant model's policy.
    GateAbility,
    /// A service-container binding key: the string `app('payments')`,
    /// `resolve('payments')`, `App::make('payments')`, or
    /// `$this->app->make('payments')` asks the container for, and the one a
    /// service provider registers it under.
    ContainerBinding,
    /// An environment variable name: the string `env('APP_NAME')` or
    /// `Env::get('APP_NAME')` reads, resolving to the line that declares it
    /// in the project's `.env`.
    Env,
}

impl StringKeyKind {
    /// Whether this key resolves through Laravel's config index.
    pub(crate) const fn is_config_backed(self) -> bool {
        matches!(self, Self::Config | Self::ConfigResource(_))
    }
}

/// A workspace symbol whose presence suppresses an otherwise-valid framework
/// runtime fallback.
///
/// PHP first looks for a namespace-local function before falling back to a
/// global helper, and Laravel's root facade aliases only exist when no real
/// global class has claimed the same name. These dependencies are stored on
/// the handful of ambiguous string spans so edits to another file can toggle
/// the spans without reparsing either file or adding work to request paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum StringKeyDependency {
    Function(Atom),
    Class(Atom),
}

impl StringKeyDependency {
    /// The namespace-local `auth` function that may prevent PHP from reaching
    /// Laravel's global helper.
    pub(crate) fn namespaced_auth(name: &str) -> Option<Self> {
        let (namespace, short) = name.rsplit_once('\\')?;
        (!namespace.is_empty() && short.eq_ignore_ascii_case("auth"))
            .then(|| Self::Function(crate::atom::ascii_lowercase_atom(name)))
    }

    /// A global class name Laravel may otherwise provide as a runtime facade
    /// alias. Fully-qualified facade classes are unambiguous and return none.
    pub(crate) fn root_facade(name: &str) -> Option<Self> {
        let name = name.trim_start_matches('\\');
        if name.contains('\\') {
            return None;
        }
        let is_alias = crate::symbol_map::laravel_resources::RESOURCE_FACADES
            .iter()
            .any(|alias| name.eq_ignore_ascii_case(alias))
            || match name.len() {
                3 => name.eq_ignore_ascii_case("App"),
                4 => {
                    name.eq_ignore_ascii_case("Gate")
                        || name.eq_ignore_ascii_case("Lang")
                        || name.eq_ignore_ascii_case("View")
                }
                5 => name.eq_ignore_ascii_case("Route"),
                6 => name.eq_ignore_ascii_case("Config"),
                7 => name.eq_ignore_ascii_case("Artisan"),
                8 => name.eq_ignore_ascii_case("Response") || name.eq_ignore_ascii_case("Schedule"),
                _ => false,
            };
        is_alias.then(|| Self::Class(crate::atom::ascii_lowercase_atom(name)))
    }
}

/// A string-key span whose meaning depends on a workspace symbol being
/// absent. The payload moves between this side table and [`SymbolMap::spans`]
/// as it becomes dormant/active, so an active candidate does not duplicate
/// its key string in memory.
#[derive(Debug, Clone)]
pub(crate) struct ConditionalStringKeySpan {
    pub dependency: StringKeyDependency,
    start: u32,
    end: u32,
    dormant_span: Option<SymbolSpan>,
}

impl ConditionalStringKeySpan {
    pub(crate) fn new(dependency: StringKeyDependency, span: SymbolSpan) -> Self {
        Self {
            dependency,
            start: span.start,
            end: span.end,
            dormant_span: Some(span),
        }
    }

    fn is_inserted(&self) -> bool {
        self.dormant_span.is_none()
    }

    /// Heap bytes owned by a dormant payload for the optional memory audit.
    #[cfg(feature = "mem-audit")]
    pub(crate) fn audit_heap(&self) -> usize {
        self.dormant_span
            .as_ref()
            .map_or(0, |span| span.kind.audit_heap())
    }
}

fn is_candidate_span(span: &SymbolSpan, candidate: &ConditionalStringKeySpan) -> bool {
    span.start == candidate.start
        && span.end == candidate.end
        && matches!(&span.kind, SymbolKind::StringKey { .. })
}

impl SymbolMap {
    /// Whether this map has a dormant span gated by one of `affected`.
    pub(crate) fn has_conditional_string_key_dependency(
        &self,
        affected: Option<&std::collections::HashSet<StringKeyDependency>>,
    ) -> bool {
        self.conditional_string_key_spans.iter().any(|candidate| {
            affected.is_none_or(|dependencies| dependencies.contains(&candidate.dependency))
        })
    }

    /// Whether reconciling the selected dependencies would add or remove at
    /// least one span. This lets the publication path avoid cloning an
    /// already-correct map merely to discover that it is unchanged.
    pub(crate) fn conditional_string_key_spans_need_refresh(
        &self,
        affected: Option<&std::collections::HashSet<StringKeyDependency>>,
        mut dependency_exists: impl FnMut(StringKeyDependency) -> bool,
    ) -> bool {
        self.conditional_string_key_spans.iter().any(|candidate| {
            affected.is_none_or(|dependencies| dependencies.contains(&candidate.dependency))
                && candidate.is_inserted() == dependency_exists(candidate.dependency)
        })
    }

    /// Reconcile dormant string-key spans against the current workspace symbol
    /// indexes. When `affected` is supplied, candidates with any other
    /// dependency are skipped, keeping cross-file edits proportional to the
    /// uncommon ambiguous call sites rather than to every span in a file.
    pub(crate) fn refresh_conditional_string_key_spans(
        &mut self,
        affected: Option<&std::collections::HashSet<StringKeyDependency>>,
        mut dependency_exists: impl FnMut(StringKeyDependency) -> bool,
    ) -> bool {
        let mut changed = false;
        let mut added = false;
        for candidate in &mut self.conditional_string_key_spans {
            if affected.is_some_and(|set| !set.contains(&candidate.dependency)) {
                continue;
            }
            let should_insert = !dependency_exists(candidate.dependency);
            if should_insert == candidate.is_inserted() {
                continue;
            }

            if should_insert {
                self.spans.push(
                    candidate
                        .dormant_span
                        .take()
                        .expect("a dormant candidate must own its span"),
                );
                added = true;
            } else {
                let existing = self
                    .spans
                    .iter()
                    .position(|span| is_candidate_span(span, candidate))
                    .expect("an active candidate must be present in the symbol map");
                candidate.dormant_span = Some(self.spans.remove(existing));
            }
            changed = true;
        }
        if added {
            self.spans.sort_by_key(|span| span.start);
        }
        if changed {
            self.member_access_indices.clear();
            for (index, span) in self.spans.iter().enumerate() {
                if let SymbolKind::MemberAccess { member_name, .. } = &span.kind {
                    self.member_access_indices
                        .entry(*member_name)
                        .or_default()
                        .push(index);
                }
            }
        }
        changed
    }
}
