//! The names Symfony's service container takes as strings in PHP: service
//! IDs and container parameters.
//!
//! The configurator functions (`service('app.mailer')`, `param('app.name')`)
//! and the `#[Autowire]` attribute are recognised by their resolved name.
//! A method call names one only on the right receiver: `get()` on a
//! container, `set()` on a configurator.  Those names are far too common to
//! claim on sight, and the receiver's spelling does not settle what it is
//! (`$container` may be anything), so each call is recorded as a
//! [`TypedReceiverSite`] for the type engine to confirm (see
//! [`symfony_receiver_method`]).

use super::*;
use crate::symbol_map::{ReceiverClass, StringKeyKind};
use crate::virtual_members::laravel::helpers::{resolved_string_literal, string_literal_at_range};

const CONFIGURATOR_NS: &str = "Symfony\\Component\\DependencyInjection\\Loader\\Configurator\\";
const AUTOWIRE_ATTRIBUTE: &str = "Symfony\\Component\\DependencyInjection\\Attribute\\Autowire";

/// What a method call names in its positional argument `index`, when its
/// receiver turns out to be the right class: the receiver it needs, and
/// whether the call declares the name rather than reading it.
///
/// Shared by the symbol map and completion, so the two cannot disagree
/// about which calls name what.  Kept allocation-free: it runs on every
/// method call in every file of a Symfony project.
pub(crate) fn symfony_receiver_method(
    member_name: &str,
    index: usize,
) -> Option<(ReceiverClass, bool)> {
    // Every name below starts with one of these, so most method calls are
    // turned away on a single byte.
    if !matches!(
        member_name.as_bytes().first().map(u8::to_ascii_lowercase),
        Some(b'g' | b'h' | b's' | b'a' | b'd')
    ) {
        return None;
    }
    let is = |name: &str| member_name.eq_ignore_ascii_case(name);
    match index {
        0 if is("get") || is("has") => Some((ReceiverClass::ContainerLookup, false)),
        0 if is("getParameter") || is("hasParameter") => {
            Some((ReceiverClass::ParameterHolder, false))
        }
        0 if is("setParameter") => Some((ReceiverClass::ParameterHolder, true)),
        0 if is("set") => Some((ReceiverClass::ConfiguratorSet, true)),
        // `alias('app.mailer', 'app.smtp_mailer')` declares the first ID
        // as another name for the second.
        0 if is("alias") => Some((ReceiverClass::ServicesConfigurator, true)),
        1 if is("alias") => Some((ReceiverClass::ServicesConfigurator, false)),
        0 if is("decorate") => Some((ReceiverClass::ServiceConfigurator, false)),
        _ => None,
    }
}

/// Record the service IDs and parameter names a method call may name, for
/// the type engine to confirm against its receiver.
pub(in crate::symbol_map::extraction) fn record_symfony_receiver_sites(
    member_name: &str,
    argument_list: &ArgumentList<'_>,
    ctx: &mut ExtractionCtx<'_>,
) {
    if !ctx.symfony {
        return;
    }
    for (index, argument) in argument_list.arguments.iter().take(2).enumerate() {
        let Argument::Positional(argument) = argument else {
            break;
        };
        let Some((receiver, is_write)) = symfony_receiver_method(member_name, index) else {
            continue;
        };
        if receiver.is_configurator() && !uses_configurators(ctx) {
            continue;
        }
        let Some((key, start, end)) = resolved_string_literal(argument.value, ctx.content) else {
            continue;
        };
        if key.is_empty() {
            continue;
        }
        ctx.typed_receiver_sites.push(TypedReceiverSite {
            start: start as u32,
            end: end as u32,
            key: key.to_string(),
            is_write,
            is_optional: false,
            receiver,
        });
    }
}

/// Whether the file names anything from Symfony's configurator namespace,
/// which a receiver has to be a class of for `set()`, `alias()`, or
/// `decorate()` to declare or name a service.  Those method names are common
/// enough elsewhere (`$session->set('cart', …)`) that recording every call
/// for the type engine would make every such file one the declared names
/// are looked for in.  A file without resolved names is not judged.
fn uses_configurators(ctx: &mut ExtractionCtx<'_>) -> bool {
    let resolved_names = ctx.resolved_names;
    *ctx.uses_symfony_configurators.get_or_insert_with(|| {
        resolved_names.is_none_or(|names| {
            names.iter().any(|(_, name, _)| {
                name.get(..CONFIGURATOR_NS.len())
                    .is_some_and(|prefix| prefix.eq_ignore_ascii_case(CONFIGURATOR_NS))
            })
        })
    })
}

/// The kind of name a PHP configurator function takes in its first
/// argument: `service('app.mailer')`, `service_closure('app.mailer')`, and
/// `param('app.sender')`.
pub(crate) fn configurator_function_kind(resolved: &str) -> Option<StringKeyKind> {
    let prefix = resolved.get(..CONFIGURATOR_NS.len())?;
    if !prefix.eq_ignore_ascii_case(CONFIGURATOR_NS) {
        return None;
    }
    let name = &resolved[CONFIGURATOR_NS.len()..];
    if name.eq_ignore_ascii_case("service") || name.eq_ignore_ascii_case("service_closure") {
        Some(StringKeyKind::ServiceId)
    } else if name.eq_ignore_ascii_case("param") {
        Some(StringKeyKind::ContainerParameter)
    } else {
        None
    }
}

/// Emit the span a configurator function call names, when the call
/// resolves to one.
pub(in crate::symbol_map::extraction) fn emit_symfony_function_call_span(
    name_offset: u32,
    argument_list: &ArgumentList<'_>,
    ctx: &mut ExtractionCtx<'_>,
) {
    if !ctx.symfony {
        return;
    }
    let Some(kind) = ctx
        .resolved_name_at(name_offset)
        .and_then(configurator_function_kind)
    else {
        return;
    };
    if let Some(Argument::Positional(argument)) = argument_list.arguments.iter().next() {
        push_laravel_string_span(
            kind,
            false,
            false,
            argument.value,
            ctx.content,
            &mut ctx.spans,
        );
    }
}

/// Whether `class_name`, as resolved, is Symfony's `#[Autowire]`.
pub(crate) fn is_autowire_attribute(class_name: &str) -> bool {
    class_name
        .trim_start_matches('\\')
        .eq_ignore_ascii_case(AUTOWIRE_ATTRIBUTE)
}

/// Emit the spans an `#[Autowire]` attribute names: `service:` a service
/// ID, `param:` a parameter, and its value an `@id` service reference or
/// the `%name%` placeholders it is built from.
pub(in crate::symbol_map::extraction) fn emit_autowire_attribute_spans(
    argument_list: &PartialArgumentList<'_>,
    ctx: &mut ExtractionCtx<'_>,
) {
    if !ctx.symfony {
        return;
    }
    for (index, argument) in argument_list.arguments.iter().enumerate() {
        let (parameter, value) = match argument {
            PartialArgument::Named(named) => (bytes_to_str(named.name.value), named.value),
            PartialArgument::Positional(positional) if index == 0 => ("value", positional.value),
            _ => continue,
        };
        match parameter {
            "service" => push_laravel_string_span(
                StringKeyKind::ServiceId,
                false,
                false,
                value,
                ctx.content,
                &mut ctx.spans,
            ),
            "param" => push_laravel_string_span(
                StringKeyKind::ContainerParameter,
                false,
                false,
                value,
                ctx.content,
                &mut ctx.spans,
            ),
            "value" => emit_autowire_value_spans(value, ctx.content, &mut ctx.spans),
            _ => {}
        }
    }
}

/// The names an `#[Autowire]` value is written in terms of: `'@app.mailer'`
/// injects a service (`'@@'` escapes a literal `@`), and any other string
/// is resolved for `%name%` parameter placeholders.
fn emit_autowire_value_spans(value: &Expression<'_>, content: &str, spans: &mut Vec<SymbolSpan>) {
    // The raw source: a name has no escapes in it, so its offsets are the
    // same in the source as in the value.
    let Some((raw, start, _)) = string_literal_at_range(value, content) else {
        return;
    };
    let push = |spans: &mut Vec<SymbolSpan>, kind, name: &str, offset: usize| {
        spans.push(SymbolSpan {
            start: start + offset as u32,
            end: start + (offset + name.len()) as u32,
            kind: SymbolKind::StringKey {
                kind,
                key: name.to_string(),
                is_write: false,
                is_optional: false,
            },
        });
    };
    if let Some(id) = raw.strip_prefix('@') {
        if !id.is_empty() && !id.starts_with(['@', '=']) && !id.contains('\\') {
            push(spans, StringKeyKind::ServiceId, id, 1);
        }
        return;
    }
    let mut cursor = 0usize;
    while let Some(open) = raw[cursor..].find('%').map(|n| cursor + n) {
        let Some(close) = raw[open + 1..].find('%').map(|n| open + 1 + n) else {
            break;
        };
        let name = &raw[open + 1..close];
        if !name.is_empty()
            && name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-'))
        {
            push(spans, StringKeyKind::ContainerParameter, name, open + 1);
            cursor = close + 1;
        } else {
            cursor = if name.is_empty() { close + 1 } else { close };
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::names::OwnedResolvedNames;
    use crate::symbol_map::{ReceiverClass, SymbolMap};

    fn map_of(php: &str) -> SymbolMap {
        let arena = mago_allocator::LocalArena::new();
        let file_id = mago_database::file::FileId::new(b"test.php");
        let program = mago_syntax::parser::parse_file_content(&arena, file_id, php.as_bytes());
        let resolver = mago_names::resolver::NameResolver::new(&arena);
        let names = OwnedResolvedNames::from_resolved(&resolver.resolve(program));
        crate::symbol_map::extraction::extract_symbol_map_for_index(program, php, &names, true)
    }

    fn receivers(map: &SymbolMap) -> Vec<(&str, ReceiverClass, bool)> {
        map.typed_receiver_sites
            .iter()
            .map(|site| (site.key.as_str(), site.receiver, site.is_write))
            .collect()
    }

    #[test]
    fn configurator_calls_are_recorded_only_where_configurators_are_named() {
        let session = "<?php\nfunction f($session, $container) {\n    $session->set('app.cart', 1);\n    $container->get('app.mailer');\n}\n";
        assert_eq!(
            receivers(&map_of(session)),
            vec![("app.mailer", ReceiverClass::ContainerLookup, false)]
        );

        let configurator = "<?php\nnamespace Symfony\\Component\\DependencyInjection\\Loader\\Configurator;\nreturn static function (ContainerConfigurator $c): void {\n    $c->services()->set('app.mailer')->decorate('app.base');\n    $c->services()->alias('app.alias', 'app.mailer');\n};\n";
        assert_eq!(
            receivers(&map_of(configurator)),
            vec![
                ("app.mailer", ReceiverClass::ConfiguratorSet, true),
                ("app.base", ReceiverClass::ServiceConfigurator, false),
                ("app.alias", ReceiverClass::ServicesConfigurator, true),
                ("app.mailer", ReceiverClass::ServicesConfigurator, false),
            ]
        );
    }

    #[test]
    fn nothing_is_recorded_outside_a_symfony_project() {
        let php = "<?php\nfunction f($container) { $container->get('app.mailer'); }\n";
        let arena = mago_allocator::LocalArena::new();
        let file_id = mago_database::file::FileId::new(b"test.php");
        let program = mago_syntax::parser::parse_file_content(&arena, file_id, php.as_bytes());
        let resolver = mago_names::resolver::NameResolver::new(&arena);
        let names = OwnedResolvedNames::from_resolved(&resolver.resolve(program));
        let map = crate::symbol_map::extraction::extract_symbol_map_for_index(
            program, php, &names, false,
        );
        assert!(map.typed_receiver_sites.is_empty());
    }
}
