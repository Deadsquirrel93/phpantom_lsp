//! The names static calls on Laravel's facades and its helper functions
//! take in their first argument: `Config::get('app.name')`,
//! `Lang::get('messages.saved')`, `Artisan::call('app:sync')`,
//! `route('home')`, `env('APP_KEY')` and the rest.
//!
//! The symbol map emits its spans through the per-family predicates here,
//! and completion classifies the call it is typed into through
//! [`facade_static_call_first_arg_kind`] and
//! [`helper_function_first_arg_kind`], so the two cannot disagree about
//! which calls name what.

use super::*;
use crate::blade::markers;
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

/// The marker functions the Blade preprocessor lowers the `@include` and
/// `@extends` families, `@component`/`@componentFirst` and `@each` to.
const BLADE_VIEW_MARKERS: [&str; 3] = [markers::VIEW, markers::COMPONENT, markers::EACH];

/// The kind of name a Laravel helper function takes in its first argument,
/// by the function's name as written (without a leading `\`).
///
/// A Blade directive that names a template or an ability is lowered to a
/// marker call, so the marker names what the directive does.
pub(crate) fn helper_function_first_arg_kind(name: &str) -> Option<LaravelStringKind> {
    let is_one_of = |names: &[&str]| names.iter().any(|n| name.eq_ignore_ascii_case(n));
    if name.eq_ignore_ascii_case("view") || is_one_of(&BLADE_VIEW_MARKERS) {
        Some(LaravelStringKind::View)
    } else if is_one_of(&["route", "to_route"]) {
        Some(LaravelStringKind::Route)
    } else if is_one_of(&["__", "trans", "trans_choice"]) {
        Some(LaravelStringKind::Trans)
    } else if name.eq_ignore_ascii_case("config") {
        Some(LaravelStringKind::Config)
    } else if name.eq_ignore_ascii_case("env") {
        Some(LaravelStringKind::Env)
    } else if is_one_of(&["app", "resolve"]) {
        Some(LaravelStringKind::ContainerBinding)
    } else if name.eq_ignore_ascii_case(markers::CAN) {
        Some(LaravelStringKind::GateAbility)
    } else {
        None
    }
}

/// Whether a helper takes a list of names in its first argument as well as
/// a single one: `@includeFirst`, `@componentFirst` and `@extendsFirst`
/// lower to their directive's marker with an array of candidate templates,
/// and `@canany` to the `@can` marker with an array of abilities.
pub(crate) fn helper_function_accepts_name_list(name: &str) -> bool {
    [markers::VIEW, markers::COMPONENT, markers::CAN]
        .iter()
        .any(|n| name.eq_ignore_ascii_case(n))
}
