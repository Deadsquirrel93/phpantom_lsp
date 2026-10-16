//! The names static calls on Laravel's facades take in their first
//! argument: `Config::get('app.name')`, `Lang::get('messages.saved')`,
//! `Env::get('APP_KEY')`, `Artisan::call('app:sync')` and the rest.
//!
//! The symbol map emits its spans through the per-family predicates here,
//! and completion classifies the call it is typed into through
//! [`facade_static_call_first_arg_kind`], so the two cannot disagree about
//! which calls name what.

use super::*;
use crate::symbol_map::LaravelStringKind;

/// Whether a static call's class, as written, is `Illuminate\Support\Env`.
pub(in crate::symbol_map::extraction) fn is_env_class_name(name: &str) -> bool {
    matches_laravel_class(name, "Illuminate\\Support", "Env")
}

/// Whether a static call reads an environment variable the way the `env()`
/// helper it backs does: `Env::get('APP_NAME')`.
pub(in crate::symbol_map::extraction) fn is_env_key_call(
    clean_subject: &str,
    member_name: &str,
) -> bool {
    is_env_class_name(clean_subject)
        && (member_name.eq_ignore_ascii_case("get")
            || member_name.eq_ignore_ascii_case("getOrFail"))
}

/// Whether a static call on the `Lang` facade names a translation key.
pub(in crate::symbol_map::extraction) fn is_lang_key_call(
    clean_subject: &str,
    member_name: &str,
) -> bool {
    matches_laravel_facade(clean_subject, "Lang")
        && ["get", "has", "hasForLocale", "choice"]
            .iter()
            .any(|name| member_name.eq_ignore_ascii_case(name))
}

/// Whether a static call's class, as written, is Eloquent's `Relation`.
pub(in crate::symbol_map::extraction) fn is_relation_class_name(name: &str) -> bool {
    matches_laravel_class(
        name,
        "Illuminate\\Database\\Eloquent\\Relations",
        "Relation",
    )
}

/// Whether a static call looks a morph alias up:
/// `Relation::getMorphedModel('post')`, or
/// `Post::getActualClassNameForMorph('post')` on any model.
pub(in crate::symbol_map::extraction) fn is_morph_alias_lookup_call(
    clean_subject: &str,
    member_name: &str,
) -> bool {
    member_name.eq_ignore_ascii_case("getActualClassNameForMorph")
        || (is_relation_class_name(clean_subject)
            && member_name.eq_ignore_ascii_case("getMorphedModel"))
}

/// Whether a static call names a template in its first argument: a view
/// factory method on the `View` facade, or `Response::view()`.
pub(in crate::symbol_map::extraction) fn is_view_first_arg_call(
    clean_subject: &str,
    member_name: &str,
) -> bool {
    (matches_laravel_facade(clean_subject, "View")
        && view_factory_method_name_index(member_name) == Some(0))
        || (matches_laravel_facade(clean_subject, "Response")
            && member_name.eq_ignore_ascii_case("view"))
}

/// The kind of name a static Laravel call takes in its first argument, by
/// the class as written (without a leading `\`) and the method.
pub(crate) fn facade_static_call_first_arg_kind(
    clean_subject: &str,
    member_name: &str,
) -> Option<LaravelStringKind> {
    if is_config_facade_name(clean_subject) && is_config_repository_method(member_name) {
        Some(LaravelStringKind::Config)
    } else if is_view_first_arg_call(clean_subject, member_name) {
        Some(LaravelStringKind::View)
    } else if is_route_name_facade_call(clean_subject, member_name)
        || (matches_laravel_facade(clean_subject, "Route")
            && is_route_facade_pattern_method(member_name))
    {
        Some(LaravelStringKind::Route)
    } else if is_env_key_call(clean_subject, member_name) {
        Some(LaravelStringKind::Env)
    } else if is_lang_key_call(clean_subject, member_name) {
        Some(LaravelStringKind::Trans)
    } else if is_artisan_command_static_call(clean_subject, member_name) {
        Some(LaravelStringKind::Command)
    } else if is_gate_facade(clean_subject)
        && (member_name.eq_ignore_ascii_case("define") || is_gate_check_method(member_name))
    {
        Some(LaravelStringKind::GateAbility)
    } else if is_morph_alias_lookup_call(clean_subject, member_name) {
        Some(LaravelStringKind::MorphAlias)
    } else {
        None
    }
}
