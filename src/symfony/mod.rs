//! Symfony's service container and router: the service IDs, parameters,
//! and route names a project declares, and where.
//!
//! Every declaration and every use is a
//! [`StringKey`](crate::symbol_map::SymbolKind::StringKey) span in some
//! file's symbol map: YAML and XML configuration and Twig templates through
//! the resource symbol maps (see [`resources`] and [`routes`]), and PHP
//! through symbol-map extraction.
//! A declaration is a span with `is_write` set, or a configurator call
//! whose receiver only the type engine can confirm (a
//! [`TypedReceiverSite`](crate::symbol_map::TypedReceiverSite) with
//! `is_write` set).  The few files holding one are tracked here, so the
//! declared names are read from those maps rather than from a second index.

pub(crate) mod resources;
pub(crate) mod routes;

use std::collections::HashSet;
use std::sync::atomic::Ordering;

use parking_lot::RwLock;
use tower_lsp::lsp_types::{Location, Range};

use crate::Backend;
use crate::symbol_map::{StringKeyKind, SymbolKind, SymbolMap};
use crate::text_position::LineIndex;

/// The files whose symbol maps declare a Symfony service ID, container
/// parameter, or route.
#[derive(Default)]
pub(crate) struct SymfonyState {
    declaring_uris: RwLock<HashSet<String>>,
}

/// Whether `map` declares a service ID, a container parameter, or a route,
/// outright or through a configurator call still to be confirmed.
fn declares_symfony_keys(map: &SymbolMap) -> bool {
    map.spans.iter().any(|span| {
        matches!(
            &span.kind,
            SymbolKind::StringKey {
                kind,
                is_write: true,
                ..
            } if kind.is_symfony()
        )
    }) || map
        .typed_receiver_sites
        .iter()
        .any(|site| site.is_write && site.receiver.is_symfony())
}

impl Backend {
    /// Whether the project builds a Symfony service container; see
    /// [`Backend::is_symfony`](crate::Backend::is_symfony).
    pub(crate) fn is_symfony_project(&self) -> bool {
        self.is_symfony.load(Ordering::Relaxed)
    }

    /// Record whether the project builds a Symfony service container, once
    /// the workspace's `composer.json` files have been read.
    pub(crate) fn set_is_symfony(&self, is_symfony: bool) {
        self.is_symfony.store(is_symfony, Ordering::Relaxed);
    }

    /// Whether the framework a string key of `kind` belongs to is the one
    /// this project uses.
    pub(crate) fn string_key_kind_enabled(&self, kind: StringKeyKind) -> bool {
        if kind.is_symfony() {
            self.is_symfony_project()
        } else {
            self.resolved_class_cache.read().laravel().is_enabled()
        }
    }

    /// Keep the set of declaring files in step with a published symbol map.
    ///
    /// Dependencies are never part of the project's own configuration, so a
    /// vendor file that happens to be parsed is left out.
    pub(crate) fn note_symfony_declarations(&self, uri: &str, map: &SymbolMap) {
        if !self.is_symfony_project() {
            return;
        }
        let declares = declares_symfony_keys(map)
            && !self
                .workspace
                .vendor_uri_prefixes
                .lock()
                .iter()
                .any(|prefix| uri.starts_with(prefix.as_str()));
        // Only take the write lock when the answer changes.
        if self.symfony.declaring_uris.read().contains(uri) == declares {
            return;
        }
        let mut uris = self.symfony.declaring_uris.write();
        if declares {
            uris.insert(uri.to_string());
        } else {
            uris.remove(uri);
        }
    }

    /// Forget a file that no longer exists.
    pub(crate) fn forget_symfony_declarations(&self, uri: &str) {
        if self.symfony.declaring_uris.read().contains(uri) {
            self.symfony.declaring_uris.write().remove(uri);
        }
    }

    /// Visit every declaration of a `kind` key: the file, the span's byte
    /// range, and the key.
    fn visit_symfony_declarations(
        &self,
        kind: StringKeyKind,
        mut visit: impl FnMut(&str, u32, u32, &str),
    ) {
        let mut uris: Vec<String> = self.symfony.declaring_uris.read().iter().cloned().collect();
        uris.sort_unstable();
        // The configurator calls are confirmed in their own files, whose
        // offsets have nothing to do with the file a diagnostic pass may be
        // checking on this thread, so neither of its caches may answer for
        // them.
        let _isolated = (
            crate::type_engine::variable::forward_walk::suspend_diagnostic_scope(),
            crate::type_engine::resolver::with_isolated_chain_cache(),
        );
        for uri in uris {
            let Some(map) = self.symbol_map_for(&uri) else {
                continue;
            };
            let typed = self.typed_receiver_spans_for(&uri, &map);
            for span in map.spans.iter().chain(typed.iter()) {
                if let SymbolKind::StringKey {
                    kind: span_kind,
                    key,
                    is_write: true,
                    ..
                } = &span.kind
                    && *span_kind == kind
                {
                    visit(&uri, span.start, span.end, key);
                }
            }
        }
    }

    /// Every service ID, container parameter, or route name the project
    /// declares, sorted.
    pub(crate) fn symfony_declared_names(&self, kind: StringKeyKind) -> Vec<String> {
        let mut names = Vec::new();
        self.visit_symfony_declarations(kind, |_, _, _, key| names.push(key.to_string()));
        names.sort_unstable();
        names.dedup();
        names
    }

    /// Where the project declares the service ID, container parameter, or
    /// route name `key`.
    ///
    /// A service declared under its own class name is that class, and its
    /// ID is spelled as one: one with no declaration of its own (the usual
    /// autowired service) resolves to the class.
    pub(crate) fn symfony_declaration_locations(
        &self,
        kind: StringKeyKind,
        key: &str,
    ) -> Vec<Location> {
        let mut found: Vec<(String, u32, u32)> = Vec::new();
        self.visit_symfony_declarations(kind, |uri, start, end, declared| {
            if declared == key {
                found.push((uri.to_string(), start, end));
            }
        });

        // Visited file by file, so each file's text is read once.
        let mut locations = Vec::with_capacity(found.len());
        for group in found.chunk_by(|a, b| a.0 == b.0) {
            let uri = &group[0].0;
            let (Some(parsed), Some(content)) =
                (crate::util::parse_url(uri), self.get_file_content_arc(uri))
            else {
                continue;
            };
            let lines = LineIndex::new(&content);
            for (_, start, end) in group {
                locations.push(Location {
                    uri: parsed.clone(),
                    range: Range {
                        start: lines.position(*start as usize),
                        end: lines.position(*end as usize),
                    },
                });
            }
        }

        if locations.is_empty()
            && kind == StringKeyKind::ServiceId
            && key.contains('\\')
            && let Some(location) = self.class_declaration_location(key)
        {
            locations.push(location);
        }
        locations
    }
}

#[cfg(test)]
mod tests;
