//! Symfony service ID, container parameter, and route name completion.
//!
//! In PHP the cursor's call is read from the raw buffer, which is mid-edit
//! and usually does not parse, and is judged by the same predicates the
//! symbol map uses (`symfony_receiver_method`, the configurator functions,
//! `#[Autowire]`), so completion and navigation agree on which strings name
//! what.  A method call's receiver is resolved by the type engine, as the
//! symbol map's is.
//!
//! In YAML and XML configuration the line before the cursor says it: an `@`
//! starts a service reference, an unclosed `%` a parameter placeholder, and
//! a handful of XML attributes take a service ID.  In a Twig template, the
//! first argument of `path()` and `url()` is a route name.

use tower_lsp::lsp_types::{CompletionResponse, Position};

use crate::Backend;
use crate::completion::eloquent_string::detect_string_call_context;
use crate::completion::laravel_string_keys::{
    ArgumentSlot, attribute_class_start, callable_before_scalar_argument, string_key_item_kind,
    string_key_response,
};
use crate::completion::source::code_context::CodeContext;
use crate::symbol_map::StringKeyKind;
use crate::symbol_map::extraction::{
    configurator_function_kind, is_autowire_attribute, symfony_receiver_method,
};
use crate::text_position::position_to_offset;
use crate::types::{AccessKind, FileContext};

impl Backend {
    /// Complete the service ID, parameter, or route name a PHP string
    /// literal is being typed as.
    pub(crate) fn try_symfony_string_key_completion(
        &self,
        content: &str,
        position: Position,
        code: &CodeContext<'_>,
        ctx: &FileContext,
    ) -> Option<CompletionResponse> {
        let cursor = position_to_offset(content, position) as usize;
        let (quote, _) = code.open_string?;
        let literal_start = quote + 1;
        let literal = content.get(literal_start..cursor)?;

        let (kind, prefix_start) = self
            .symfony_call_key(content, cursor, code, ctx)
            .map(|kind| (kind, literal_start))
            .or_else(|| autowire_key(content, quote, literal, literal_start, ctx))?;
        self.symfony_key_response(kind, content, prefix_start, cursor, position)
    }

    /// What a string passed straight to a method or function call names:
    /// a container lookup or a configurator call on a receiver the type
    /// engine confirms, or a configurator function.
    fn symfony_call_key(
        &self,
        content: &str,
        cursor: usize,
        code: &CodeContext<'_>,
        ctx: &FileContext,
    ) -> Option<StringKeyKind> {
        // A name is the argument itself, not an element of an array one.
        if !matches!(code.last_code_byte(), Some(b'(' | b',')) {
            return None;
        }
        let call = detect_string_call_context(content, cursor, code)?;
        let Some(subject) = call.subject.as_deref() else {
            if call.arg_index != 0 {
                return None;
            }
            let paren = code.enclosing_paren()?;
            let written = trailing_name(&content[..paren.code_before]);
            let offset = (paren.code_before - written.len()) as u32;
            return configurator_function_kind(&resolve_function_name(ctx, written, offset));
        };
        if call.is_static {
            return None;
        }
        let (receiver, _) = symfony_receiver_method(&call.method_name, call.arg_index)?;

        let loaders = self.file_loaders(ctx);
        let class_loader = loaders.resolved_class_loader();
        let rctx = loaders.resolution_ctx_at(content, cursor as u32);
        crate::type_engine::resolver::resolve_target_classes(subject, AccessKind::Arrow, &rctx)
            .into_iter()
            .filter_map(|resolved| resolved.class_info)
            .find_map(|class| {
                receiver.targets().iter().find_map(|(fqn, kind)| {
                    crate::class_lookup::is_subtype_of(&class, fqn, class_loader).then_some(*kind)
                })
            })
    }

    fn symfony_key_response(
        &self,
        kind: StringKeyKind,
        content: &str,
        prefix_start: usize,
        cursor: usize,
        position: Position,
    ) -> Option<CompletionResponse> {
        let prefix = content.get(prefix_start..cursor)?.as_bytes();
        let names: Vec<String> = self
            .symfony_declared_names(kind)
            .into_iter()
            .filter(|name| {
                name.as_bytes()
                    .get(..prefix.len())
                    .is_some_and(|start| start.eq_ignore_ascii_case(prefix))
            })
            .collect();
        string_key_response(
            names,
            string_key_item_kind(&kind),
            content,
            prefix_start,
            position,
        )
    }

    /// Complete a service reference or parameter placeholder in a YAML or
    /// XML configuration file, or a route name in a Twig template.
    pub(crate) fn try_symfony_resource_completion(
        &self,
        uri: &str,
        content: &str,
        position: Position,
    ) -> Option<CompletionResponse> {
        if !self.is_symfony_project() {
            return None;
        }
        let cursor = position_to_offset(content, position) as usize;
        let (kind, prefix_start) = if crate::symfony::routes::is_twig_uri(uri) {
            let start = crate::symfony::routes::twig_route_name_start(content, cursor)?;
            (StringKeyKind::SymfonyRoute, start)
        } else if crate::symfony::resources::is_container_config_uri(uri) {
            resource_key_at(content, cursor)?
        } else {
            return None;
        };
        self.symfony_key_response(kind, content, prefix_start, cursor, position)
    }
}

/// What an `#[Autowire]` string names, and where the name in it starts:
/// `service:` a service, `param:` a parameter, and its value an `@id`
/// service reference or a `%name%` placeholder.
fn autowire_key(
    content: &str,
    quote: usize,
    literal: &str,
    literal_start: usize,
    ctx: &FileContext,
) -> Option<(StringKeyKind, usize)> {
    let before_quote = content[..quote].trim_end();
    let (callable, slot) = callable_before_scalar_argument(before_quote)?;
    let name_start = callable.len() - trailing_name(callable).len();
    let class_start = attribute_class_start(callable, name_start)?;
    let written = &callable[class_start..];
    if !is_autowire_attribute(&ctx.resolve_name_at(written, class_start as u32)) {
        return None;
    }
    match slot {
        ArgumentSlot::Named("service") => Some((StringKeyKind::ServiceId, literal_start)),
        ArgumentSlot::Named("param") => Some((StringKeyKind::ContainerParameter, literal_start)),
        ArgumentSlot::Named("value") | ArgumentSlot::Positional(0) => {
            if literal.starts_with('@') && !literal.starts_with("@@") {
                return Some((StringKeyKind::ServiceId, literal_start + 1));
            }
            open_placeholder(literal)
                .map(|start| (StringKeyKind::ContainerParameter, literal_start + start))
        }
        _ => None,
    }
}

/// Where the name of a `%placeholder%` the end of `text` is inside starts.
fn open_placeholder(text: &str) -> Option<usize> {
    let percents = text.bytes().filter(|byte| *byte == b'%').count();
    (percents % 2 == 1).then(|| text.rfind('%').map(|index| index + 1))?
}

/// The service reference or parameter placeholder being typed at `cursor`
/// in a YAML or XML line, and where the name in it starts.
fn resource_key_at(content: &str, cursor: usize) -> Option<(StringKeyKind, usize)> {
    let line_start = content[..cursor].rfind('\n').map_or(0, |index| index + 1);
    let line = &content[line_start..cursor];
    let is_name_byte =
        |byte: u8| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-');

    if let Some(start) = open_placeholder(line)
        && line[start..].bytes().all(is_name_byte)
    {
        return Some((StringKeyKind::ContainerParameter, line_start + start));
    }

    let name_start = line
        .bytes()
        .rposition(|byte| !is_name_byte(byte))
        .map_or(0, |index| index + 1);
    let before = &line[..name_start];
    let before = before
        .strip_suffix(['?', '!'])
        .filter(|rest| rest.ends_with('@'))
        .unwrap_or(before);
    if let Some(at) = before.strip_suffix('@') {
        let opens_value = at.is_empty() || at.ends_with([' ', '\'', '"', '[', ',', '{', '\t']);
        return opens_value.then_some((StringKeyKind::ServiceId, line_start + name_start));
    }

    // An XML attribute that takes a service ID.
    let attribute = before.strip_suffix('"')?.strip_suffix('=')?;
    let attribute = trailing_name(attribute.trim_end());
    let takes_service = match attribute {
        "alias" | "decorates" | "parent" | "service" => true,
        "id" => {
            let tag = &line[line.rfind('<')?..];
            tag.starts_with("<argument") && tag.contains("type=\"service\"")
        }
        _ => false,
    };
    takes_service.then_some((StringKeyKind::ServiceId, line_start + name_start))
}

/// The (possibly qualified) name `text` ends with.
fn trailing_name(text: &str) -> &str {
    let start = text
        .bytes()
        .rposition(|byte| !(byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'\\')))
        .map_or(0, |index| index + 1);
    &text[start..]
}

/// The function a call written as `written` at `offset` names, by PHP's
/// rule: a `use function` import, else the current namespace.  A name
/// resolved by the last parse is used as it is.
fn resolve_function_name(ctx: &FileContext, written: &str, offset: u32) -> String {
    if let Some(resolved) = ctx
        .resolved_names
        .as_deref()
        .and_then(|names| names.get(offset))
    {
        return resolved.to_string();
    }
    if let Some(qualified) = written.strip_prefix('\\') {
        return qualified.to_string();
    }
    let imports = &ctx.symbol_imports_at(offset).functions;
    if let Some(imported) = imports
        .iter()
        .find(|(alias, _)| alias.eq_ignore_ascii_case(written))
        .map(|(_, fqn)| fqn)
    {
        return imported.clone();
    }
    crate::util::build_fqn(written, ctx.namespace_at(offset).as_deref())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The key being typed where `|` is, and the name typed so far.
    fn at(content: &str) -> Option<(StringKeyKind, String)> {
        let cursor = content.find('|').unwrap();
        let content = content.replacen('|', "", 1);
        let (kind, start) = resource_key_at(&content, cursor)?;
        Some((kind, content[start..cursor].to_string()))
    }

    #[test]
    fn yaml_references_and_placeholders_are_recognised() {
        assert_eq!(
            at("    arguments: ['@app.ma|']"),
            Some((StringKeyKind::ServiceId, "app.ma".to_string()))
        );
        assert_eq!(
            at("    arguments: ['@?app.|']"),
            Some((StringKeyKind::ServiceId, "app.".to_string()))
        );
        assert_eq!(
            at("    dir: '%kernel.pro|"),
            Some((StringKeyKind::ContainerParameter, "kernel.pro".to_string()))
        );
        assert_eq!(at("    email: 'me@exam|'"), None);
        assert_eq!(at("    done: '%app.x%/ab|'"), None);
    }

    #[test]
    fn xml_attributes_that_take_a_service_are_recognised() {
        assert_eq!(
            at(r#"<service id="b" alias="app.m|"#),
            Some((StringKeyKind::ServiceId, "app.m".to_string()))
        );
        assert_eq!(
            at(r#"<argument type="service" id="app.|"#),
            Some((StringKeyKind::ServiceId, "app.".to_string()))
        );
        assert_eq!(at(r#"<service id="app.|"#), None);
    }
}
