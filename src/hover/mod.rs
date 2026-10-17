//! Hover support (`textDocument/hover`).
//!
//! This module resolves the symbol under the cursor and returns a
//! human-readable description including type information, method
//! signatures, and docblock descriptions.
//!
//! The implementation reuses the same symbol-map lookup that powers
//! go-to-definition, and the same type-resolution pipeline that
//! powers completion.
//!
//! [`Backend::handle_hover`] dispatches by symbol kind into the sibling
//! submodules: [`member`] (methods / properties / constants),
//! [`variable`], [`class`], [`see_refs`], [`templates`], and
//! [`constants`].  The [`formatting`] submodule holds the shared
//! Markdown builders.

mod class;
pub(crate) mod constants;
mod formatting;
mod laravel;
mod laravel_trans;
mod member;
mod see_refs;
mod templates;
mod variable;

use std::sync::Arc;

use tower_lsp::lsp_types::*;

use crate::Backend;
use crate::class_lookup::find_class_at_offset;
use crate::definition::member::MemberKind;
use crate::symbol_map::{SelfStaticParentKind, SymbolKind, SymbolSpan, VarDefKind};
use crate::types::*;

use formatting::*;
use laravel::hover_command_own_param;
use member::HoverMemberHit;

// Re-export `pub(crate)` items so external callers keep using `crate::hover::`.
pub(crate) use formatting::{
    extract_description_from_info, extract_docblock_description, extract_var_description_from_info,
    hover_for_function, shorten_php_type,
};
pub(crate) use member::{MemberKindForOrigin, find_declaring_class};

impl Backend {
    /// Return a Markdown provenance line for a class FQN, or `None` for
    /// project-local classes.
    pub(crate) fn provenance_line_for_class(&self, fqn: &str) -> Option<String> {
        let class_uri = self.symbols.fqn_uri_index.read().get(fqn).cloned()?;
        let (origin, pkg_name) = self.package_info_for_uri(&class_uri);
        format_provenance_line(origin, pkg_name.as_deref())
    }

    /// Return a Markdown provenance line for a function by name.
    ///
    /// By the time hover asks for provenance the function has already
    /// been resolved, so `global_functions` holds its defining URI
    /// (including `phpantom-stub-fn://` for built-ins). Falls back to
    /// the autoload index for functions discovered by the byte-level
    /// scan but not yet parsed.
    pub(crate) fn provenance_line_for_function(&self, func_name: &str) -> Option<String> {
        let uri = self
            .symbols
            .global_functions
            .read()
            .get(func_name)
            .map(|(uri, _)| uri.clone());
        if let Some(uri) = uri {
            let (origin, pkg_name) = self.package_info_for_uri(&uri);
            return format_provenance_line(origin, pkg_name.as_deref());
        }
        let path = self
            .symbols
            .autoload_function_index
            .read()
            .get(func_name)
            .cloned()?;
        let (origin, pkg_name) = self.package_info_for_path(&path);
        format_provenance_line(origin, pkg_name.as_deref())
    }

    /// Handle a `textDocument/hover` request.
    ///
    /// Returns `Some(Hover)` when the symbol under the cursor can be
    /// resolved to a meaningful description, or `None` when resolution
    /// fails or the cursor is not on a navigable symbol.
    pub fn handle_hover(&self, uri: &str, content: &str, position: Position) -> Option<Hover> {
        let _resolver_guard = crate::type_engine::call_resolution::activate_type_engine_caches();
        let offset = crate::text_position::position_to_offset(content, position);

        if let Some(symbol) = self.lookup_symbol_map(uri, offset)
            && let Some(Some(mut hover)) =
                crate::util::catch_panic_unwind_safe("hover", uri, Some(position), || {
                    self.hover_from_symbol(&symbol, uri, content, offset)
                })
        {
            hover.range = Some(symbol_span_to_range(content, &symbol));
            return Some(hover);
        }

        // Retry one byte earlier for end-of-token edge cases.
        if offset > 0
            && let Some(symbol) = self.lookup_symbol_map(uri, offset - 1)
            && let Some(Some(mut hover)) =
                crate::util::catch_panic_unwind_safe("hover", uri, Some(position), || {
                    self.hover_from_symbol(&symbol, uri, content, offset - 1)
                })
        {
            hover.range = Some(symbol_span_to_range(content, &symbol));
            return Some(hover);
        }

        // ── model-property<Model> string hover ─────────────────
        // When the cursor is inside a string argument whose parameter
        // is typed as model-property<Model>, show the property's type
        // and source info (same as hovering over $model->property).
        if let Some(hover) = self.try_model_property_hover(uri, content, position) {
            return Some(hover);
        }

        // ── dropped trailing statement ──────────────────────────
        // An incomplete construct at the end of a file (`$obj->` or
        // `Foo::` with nothing typed after it yet) is a parse error mago's
        // recovery drops entirely rather than keeping as an error node, so
        // the symbol map has no span there at all — not just a gap between
        // spans, but nothing past the last one it could build. Recover the
        // bare `$variable` or class name touching the cursor from the raw
        // text and feed it into the same variable/class hover paths a
        // symbol-map hit would have used.
        if let Some(hover) = self.hover_dropped_tail_identifier(uri, content, offset) {
            return Some(hover);
        }

        None
    }

    /// Fallback for [`handle_hover`] when the symbol map has nothing at or
    /// after `offset`: read the `$variable` or bare identifier touching the
    /// cursor directly from `content` and hover it through the normal
    /// variable/class paths.
    ///
    /// Only consulted when nothing in the map extends this far, so it never
    /// second-guesses successfully parsed code, and a cursor inside a
    /// string literal or comment is left alone.
    fn hover_dropped_tail_identifier(
        &self,
        uri: &str,
        content: &str,
        offset: u32,
    ) -> Option<Hover> {
        let last_span_end = {
            let maps = self.symbol_maps.read();
            let map = maps.get(uri)?;
            if !map.spans.iter().all(|s| s.end <= offset) {
                return None;
            }
            map.spans.iter().map(|s| s.end).max().unwrap_or(0)
        };

        let (is_variable, name, token_start) = identifier_token_at(content, offset as usize)?;
        // Text past the last span is not necessarily code: a string literal
        // or comment there is left alone, as one before it is.
        if inside_string_or_comment(
            content.as_bytes(),
            last_span_end as usize,
            token_start as usize,
        ) {
            return None;
        }

        let ctx = self.file_context_at(uri, offset);
        let current_class = find_class_at_offset(&ctx.classes, offset);

        let mut hover = if is_variable {
            self.hover_variable(&name, uri, content, offset, current_class, &ctx)?
        } else {
            let class_loader = self.class_loader(&ctx);
            self.hover_class_reference(&name, uri, content, &class_loader, offset)?
        };
        let token_end = token_start + name.len() as u32 + u32::from(is_variable);
        hover.range = Some(Range {
            start: crate::text_position::offset_to_position(content, token_start as usize),
            end: crate::text_position::offset_to_position(content, token_end as usize),
        });
        Some(hover)
    }

    /// Dispatch a symbol-map hit to the appropriate hover path.
    fn hover_from_symbol(
        &self,
        symbol: &SymbolSpan,
        uri: &str,
        content: &str,
        cursor_offset: u32,
    ) -> Option<Hover> {
        let kind = &symbol.kind;
        let ctx = self.file_context_at(uri, cursor_offset);
        let current_class = find_class_at_offset(&ctx.classes, cursor_offset);
        let loaders = self.file_loaders(&ctx);
        let class_loader = loaders.class_loader_at(cursor_offset);
        let resolved_class_loader = loaders.resolved_class_loader();
        let function_loader = loaders.function_loader_at(cursor_offset);

        match kind {
            SymbolKind::Variable { name } | SymbolKind::CompactVariable { name } => {
                // Suppress hover when the cursor is on a variable at its
                // definition site where the type is already visible in
                // the signature (properties, static/global declarations).
                // For parameters, assignments, foreach bindings, and catch
                // bindings, hover is useful to show the resolved type and
                // any docblock descriptions.
                if let Some(def_kind) = self.lookup_var_def_kind_at(uri, name, cursor_offset)
                    && (!matches!(
                        def_kind,
                        VarDefKind::Assignment
                            | VarDefKind::CompoundAssignment
                            | VarDefKind::Parameter
                            | VarDefKind::Foreach
                            | VarDefKind::Catch
                            | VarDefKind::ArrayDestructuring
                            | VarDefKind::ListDestructuring
                    ) || (def_kind == VarDefKind::Parameter
                        && self.is_promoted_property_param(uri, symbol.start)))
                {
                    // A constructor-promoted parameter declares a property,
                    // not a local variable, so it follows the same
                    // "already visible on screen" rule as any other
                    // property declaration.
                    return None;
                }
                self.hover_variable(name, uri, content, cursor_offset, current_class, &ctx)
            }

            SymbolKind::MemberAccess {
                subject_text,
                member_name,
                is_static,
                is_method_call,
                ..
            } => {
                let rctx = loaders.resolution_ctx_at(content, cursor_offset);

                let access_kind = AccessKind::of_member_access(*is_static);

                let source = self.symbol_map_source(uri, content)?;
                let candidates = ResolvedType::into_arced_classes(
                    crate::type_engine::resolver::resolve_target_classes(
                        subject_text.as_str(source),
                        access_kind,
                        &rctx,
                    ),
                );

                // Collect hover results from all union candidates,
                // deduplicating by declaring class so that a member
                // inherited from the same interface/parent is shown
                // only once.
                let mut hover_markdowns: Vec<String> = Vec::new();
                let mut seen_declaring_classes: Vec<String> = Vec::new();

                for target_class in &candidates {
                    // Always use a fully-resolved class so that inherited
                    // docblock types (return types, parameter types,
                    // descriptions) are visible on hover.  The candidate
                    // from `resolve_target_classes` may carry model-specific
                    // scope methods that are not in the FQN-keyed cache, so
                    // fall back to the candidate when the member is not
                    // found on the fully-resolved version.
                    let merged = crate::virtual_members::resolve_class_fully_cached(
                        target_class,
                        &resolved_class_loader,
                        &self.resolved_class_cache,
                    );
                    let find_result =
                        Self::find_member_for_hover(&merged, member_name, *is_method_call);

                    let (mut member_result, mut owner) = if find_result.is_some() {
                        (find_result, merged.clone())
                    } else {
                        // Fall back to the candidate directly — it may
                        // contain model-specific members (e.g. Eloquent
                        // scope methods injected onto Builder<Model>)
                        // that the FQN-keyed cache does not have.
                        let result =
                            Self::find_member_for_hover(target_class, member_name, *is_method_call);
                        (result, target_class.clone())
                    };

                    // ── Enrich with call-site generic substitution ──
                    // The merged (cached) class has raw template param
                    // names (e.g. TModel) because the cache is FQN-keyed
                    // and shared across call sites.  The candidate from
                    // resolve_target_classes carries concrete substitutions
                    // (e.g. TModel→BlogAuthor).  When the merged member's
                    // return type still references a template param, swap
                    // it with the candidate's substituted return type and
                    // use the candidate as the owner.
                    if !merged.template_params.is_empty() {
                        let tpl_strings: Vec<String> = merged
                            .template_params
                            .iter()
                            .map(|a| a.to_string())
                            .collect();
                        match &member_result {
                            Some(HoverMemberHit::Method(method)) => {
                                if let Some(ref ret) = method.return_type
                                    && ret.references_any_template_param(&tpl_strings)
                                    && let Some(HoverMemberHit::Method(subst_method)) =
                                        Self::find_member_for_hover(
                                            target_class,
                                            member_name,
                                            *is_method_call,
                                        )
                                {
                                    member_result = Some(HoverMemberHit::Method(subst_method));
                                    owner = target_class.clone();
                                }
                            }
                            Some(HoverMemberHit::Property(prop)) => {
                                if let Some(ref hint) = prop.type_hint
                                    && hint.references_any_template_param(&tpl_strings)
                                    && let Some(HoverMemberHit::Property(subst_prop)) =
                                        Self::find_member_for_hover(
                                            target_class,
                                            member_name,
                                            *is_method_call,
                                        )
                                {
                                    member_result = Some(HoverMemberHit::Property(subst_prop));
                                    owner = target_class.clone();
                                }
                            }
                            _ => {}
                        }
                    }

                    let hover = match member_result {
                        Some(HoverMemberHit::Method(ref method)) => {
                            let mut method = method.clone();
                            if let Some((_date_class, date_return_type)) =
                                Self::configured_laravel_date_return(
                                    &owner,
                                    member_name,
                                    &resolved_class_loader,
                                )
                            {
                                method.return_type = Some(date_return_type);
                                method.is_inferred_return = true;
                            }
                            let declaring = find_declaring_class(
                                &owner,
                                member_name,
                                &MemberKindForOrigin::Method,
                                &resolved_class_loader,
                            );
                            Some((
                                declaring.fqn().to_string(),
                                self.hover_for_method(
                                    &method,
                                    &declaring,
                                    &resolved_class_loader,
                                    uri,
                                    content,
                                ),
                            ))
                        }
                        Some(HoverMemberHit::Property(ref prop)) => {
                            let declaring = find_declaring_class(
                                &owner,
                                &prop.name,
                                &MemberKindForOrigin::Property,
                                &resolved_class_loader,
                            );
                            Some((
                                declaring.fqn().to_string(),
                                self.hover_for_property(prop, &declaring, &resolved_class_loader),
                            ))
                        }
                        Some(HoverMemberHit::Constant(ref constant)) => {
                            let declaring = find_declaring_class(
                                &owner,
                                &constant.name,
                                &MemberKindForOrigin::Constant,
                                &resolved_class_loader,
                            );
                            Some((
                                declaring.fqn().to_string(),
                                self.hover_for_constant(
                                    constant,
                                    &declaring,
                                    &resolved_class_loader,
                                ),
                            ))
                        }
                        None => None,
                    };

                    if let Some((declaring_name, h)) = hover {
                        // Deduplicate: if we already have a hover from this
                        // declaring class, skip it (e.g. both Lamp and Faucet
                        // implement Switchable::turnOff — show once).
                        if seen_declaring_classes.contains(&declaring_name) {
                            continue;
                        }
                        seen_declaring_classes.push(declaring_name);
                        if let HoverContents::Markup(mc) = h.contents {
                            hover_markdowns.push(mc.value);
                        }
                    }
                }

                if hover_markdowns.is_empty() {
                    None
                } else if hover_markdowns.len() == 1 {
                    Some(make_hover(hover_markdowns.into_iter().next().unwrap()))
                } else {
                    Some(make_hover(hover_markdowns.join("\n\n---\n\n")))
                }
            }

            SymbolKind::ClassReference {
                name,
                is_fqn,
                context,
            } => {
                // A fully qualified reference (`\stdClass`, stored without
                // its backslash) must not go through the file's imports
                // and namespace again.
                let class_loader: &dyn Fn(&str) -> Option<Arc<ClassInfo>> = if *is_fqn {
                    &resolved_class_loader
                } else {
                    &class_loader
                };
                // On `new ClassName`, show the __construct method hover
                // instead of the class hover.
                if *context == crate::symbol_map::ClassRefContext::New
                    && let Some(cls) = class_loader(name)
                {
                    let merged = crate::virtual_members::resolve_class_fully_cached(
                        &cls,
                        &resolved_class_loader,
                        &self.resolved_class_cache,
                    );
                    if let Some(constructor) = merged.get_method_ci("__construct") {
                        return Some(self.hover_for_method(
                            constructor,
                            &merged,
                            &resolved_class_loader,
                            uri,
                            content,
                        ));
                    }
                }

                self.hover_class_reference(name, uri, content, class_loader, cursor_offset)
            }

            SymbolKind::ClassDeclaration { .. }
            | SymbolKind::MemberDeclaration { .. }
            | SymbolKind::NamespaceDeclaration { .. } => {
                // The user is already at the definition site — showing
                // hover here would just repeat what they can already see.
                None
            }

            SymbolKind::FunctionCall {
                name,
                is_definition,
                is_docblock_reference,
            } => {
                // The user is already at the function's own declaration —
                // showing hover here would just repeat the signature and
                // docblock that are already on screen, same as a class or
                // member declaration.
                if *is_definition {
                    return None;
                }

                // An unqualified `@see name()` describes the documented
                // class's own member first, as phpDocumentor reads it.
                if *is_docblock_reference
                    && let Some((class, kind)) =
                        self.docblock_scope_member(&ctx, symbol.start, name)
                    && let Some(hit) =
                        Self::find_member_for_hover(&class, name, kind == MemberKind::Method)
                {
                    return Some(match hit {
                        HoverMemberHit::Method(m) => {
                            self.hover_for_method(&m, &class, &resolved_class_loader, uri, content)
                        }
                        HoverMemberHit::Property(p) => {
                            self.hover_for_property(&p, &class, &resolved_class_loader)
                        }
                        HoverMemberHit::Constant(c) => {
                            self.hover_for_constant(&c, &class, &resolved_class_loader)
                        }
                    });
                }

                self.hover_function_call(name, uri, content, &ctx, &function_loader, symbol.start)
            }

            SymbolKind::SelfStaticParent(ssp_kind) => {
                let is_this = *ssp_kind == SelfStaticParentKind::This;

                let resolved = match ssp_kind {
                    SelfStaticParentKind::Self_ | SelfStaticParentKind::Static => {
                        current_class.cloned()
                    }
                    SelfStaticParentKind::This => self
                        .resolve_closure_this_override(uri, content, cursor_offset)
                        .map(Arc::unwrap_or_clone)
                        .or_else(|| current_class.cloned()),
                    SelfStaticParentKind::Parent => current_class
                        .and_then(|cc| cc.parent_class.as_ref())
                        .and_then(|parent_name| {
                            resolved_class_loader(parent_name).map(Arc::unwrap_or_clone)
                        }),
                };
                if let Some(cls) = resolved {
                    let mut lines = Vec::new();

                    if let Some(desc) = extract_docblock_description(cls.class_docblock.as_deref())
                    {
                        lines.push(desc);
                    }

                    if let Some(ref msg) = cls.deprecation_message {
                        lines.push(format_deprecation_line(msg));
                    }

                    let ns_line = namespace_line(cls.file_namespace.as_deref());
                    if is_this {
                        lines.push(format!(
                            "```php\n<?php\n{}$this = {}\n```",
                            ns_line, cls.name
                        ));
                    } else {
                        let keyword_str = match ssp_kind {
                            SelfStaticParentKind::Self_ => "self",
                            SelfStaticParentKind::Static => "static",
                            SelfStaticParentKind::Parent => "parent",
                            SelfStaticParentKind::This => unreachable!(),
                        };
                        lines.push(format!(
                            "```php\n<?php\n{}{} = {}\n```",
                            ns_line, keyword_str, cls.name
                        ));
                    }

                    Some(make_hover(lines.join("\n\n")))
                } else {
                    None
                }
            }

            SymbolKind::ConstantReference {
                name,
                is_definition,
            } => {
                // The user is already at the `const NAME = ...;` declaration
                // — the value is on the same line, so showing hover here
                // would just repeat it, same as a class or member
                // declaration.
                if *is_definition {
                    return None;
                }

                // The name is resolved against the file the same way the
                // type engine resolves it, so a namespaced constant is
                // found through whichever spelling the reference used.
                let lookup = self.resolve_constant_name_at(
                    name,
                    ctx.resolved_names.as_deref(),
                    symbol.start,
                    &ctx.use_map,
                    &ctx.symbol_imports.constants,
                    &ctx.namespace,
                );

                // `lookup` is `Some(Some(val))` when the constant
                // exists with a known value, `Some(None)` when it
                // exists but the value is unknown, and `None` when
                // the constant was not found at all.
                match lookup {
                    Some(Some(val)) => Some(make_hover(format!(
                        "```php\n<?php\nconst {} = {};\n```",
                        name, val
                    ))),
                    Some(None) => Some(make_hover(format!("```php\n<?php\nconst {};\n```", name))),
                    None => None,
                }
            }

            SymbolKind::LaravelStringKey { kind, key, .. } => {
                if !self.resolved_class_cache.read().is_laravel() {
                    return None;
                }
                self.hover_laravel_string_key(kind, key, uri)
            }

            SymbolKind::CommandOwnParam { name, is_option } => {
                hover_command_own_param(content, cursor_offset as usize, name, *is_option)
            }

            SymbolKind::LaravelMacroString { .. }
            | SymbolKind::Keyword
            | SymbolKind::CastType
            | SymbolKind::Comment => None,
        }
    }

    /// Produce hover information for a function call.
    fn hover_function_call(
        &self,
        name: &str,
        uri: &str,
        content: &str,
        _ctx: &FileContext,
        function_loader: &dyn Fn(&str, u32) -> Option<FunctionInfo>,
        offset: u32,
    ) -> Option<Hover> {
        if let Some(mut func) = function_loader(name, offset) {
            let is_configured_date_helper = matches!(
                name.trim_start_matches('\\').rsplit('\\').next(),
                Some("now" | "today")
            );
            // Only when the configured date class actually resolves do we
            // override the declared return type; the `(inferred)` annotation
            // must reflect that override, not merely the helper's name (a
            // user-defined `now()` in a non-Laravel project keeps its own
            // return type unannotated).
            let mut inferred_date_return = false;
            if is_configured_date_helper
                && let Some(date_class) = self
                    .find_or_load_class(crate::virtual_members::laravel::CONFIGURED_DATE_CLASS_FQN)
            {
                let date_type = crate::php_type::PhpType::named(date_class.fqn());
                func.return_type = Some(date_type);
                inferred_date_return = true;
            }
            let resolved_see = self.resolve_see_refs(&func.see_refs, uri, content);
            let provenance = self.provenance_line_for_function(name);
            Some(hover_for_function(
                &func,
                Some(&resolved_see),
                provenance,
                inferred_date_return,
            ))
        } else {
            None
        }
    }
}

/// Whether `target` lies inside a string literal or comment, scanning
/// from `from`, a point known to be outside both.
fn inside_string_or_comment(bytes: &[u8], from: usize, target: usize) -> bool {
    let mut pos = from;
    while pos < target {
        let end = match bytes[pos] {
            b'\'' | b'"' => crate::text_scan::skip_string_forward(bytes, pos),
            _ => match crate::text_scan::skip_php_comment(bytes, pos) {
                Some(end) => end,
                None => {
                    pos += 1;
                    continue;
                }
            },
        };
        if end > target {
            return true;
        }
        pos = end;
    }
    false
}

/// Whether `c` can appear inside a PHP identifier.  PHP allows every byte
/// from `0x80` up, which is why this is not just `is_alphanumeric`.
fn is_php_identifier_char(c: char) -> bool {
    c == '_' || c.is_ascii_alphanumeric() || (c as u32) >= 0x80
}

/// Find the `$variable` or bare identifier touching byte offset `offset`
/// in `content`, for cursor positions no AST node covers.
///
/// Returns `(is_variable, name, token_start)`: `name` excludes the `$`
/// sigil, and `token_start` is the byte offset of the `$` for a variable
/// or of the identifier's first byte otherwise. `None` when the offset
/// (or the byte just before it, for end-of-token cursors) does not sit on
/// an identifier character.
fn identifier_token_at(content: &str, offset: usize) -> Option<(bool, String, u32)> {
    let anchor = if content
        .get(offset..)
        .and_then(|s| s.chars().next())
        .is_some_and(is_php_identifier_char)
    {
        offset
    } else {
        let prev = content.get(..offset).and_then(|s| s.chars().next_back())?;
        if !is_php_identifier_char(prev) {
            return None;
        }
        offset - prev.len_utf8()
    };

    let mut start = anchor;
    while start > 0 {
        let Some(prev) = content[..start].chars().next_back() else {
            break;
        };
        if !is_php_identifier_char(prev) {
            break;
        }
        start -= prev.len_utf8();
    }

    let mut end = anchor;
    if let Some(c) = content[anchor..].chars().next()
        && is_php_identifier_char(c)
    {
        end += c.len_utf8();
    }
    while end < content.len() {
        let Some(c) = content[end..].chars().next() else {
            break;
        };
        if !is_php_identifier_char(c) {
            break;
        }
        end += c.len_utf8();
    }

    let is_variable = start > 0 && content.as_bytes()[start - 1] == b'$';
    let name = content[start..end].to_string();
    let token_start = if is_variable { start - 1 } else { start };
    Some((is_variable, name, token_start as u32))
}

#[cfg(test)]
mod tests;
