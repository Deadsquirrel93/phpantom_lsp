//! The types unary prefix operators produce: casts, `!`, `~`, and the
//! numeric step of `++`/`--`.

use mago_syntax::cst::{Expression, unary};

use crate::atom::atom;
use crate::php_type::{PhpType, ShapeEntry, TypeKind, keyword_lowercase};
use crate::types::ResolvedType;

use super::scalar_fold;

/// The type a unary prefix expression produces, for the operators whose
/// result is determined by the operator plus (for `(object)` and `~`) the
/// operand type.
///
/// A cast or `~` of an operand that holds one known scalar folds to the
/// value PHP computes (`(int) '1'` is `1`, `~1` is `-2`). `resolve_operand`
/// is always called for `(object)`, `(array)` and `~`, and for the other casts only
/// when `operand` is cheap to resolve (a literal, variable, or constant), so
/// a cast of a call pays nothing for it.
///
/// Returns `None` for operators the caller must resolve itself: `-`/`+`
/// need the full expression resolver to keep signed numeric literals
/// exact, and `@`/`&`/`++`/`--` take the type of their operand.
pub(crate) fn unary_prefix_result_type(
    operator: &unary::UnaryPrefixOperator<'_>,
    operand: &Expression<'_>,
    resolve_operand: impl FnOnce() -> Vec<ResolvedType>,
) -> Option<PhpType> {
    use unary::UnaryPrefixOperator;

    let base = match operator {
        UnaryPrefixOperator::ObjectCast(..) => return Some(object_cast_type(resolve_operand())),
        UnaryPrefixOperator::BitwiseNot(_) => {
            let resolved = resolve_operand();
            if let Some(scalar_fold::Scalar::Int(value)) = scalar_fold::single_scalar(&resolved) {
                return Some(PhpType::literal_int((!value).to_string()));
            }
            return Some(bitwise_not_type(&resolved));
        }
        UnaryPrefixOperator::IntCast(..) | UnaryPrefixOperator::IntegerCast(..) => PhpType::int(),
        UnaryPrefixOperator::StringCast(..) | UnaryPrefixOperator::BinaryCast(..) => {
            PhpType::string()
        }
        UnaryPrefixOperator::FloatCast(..)
        | UnaryPrefixOperator::DoubleCast(..)
        | UnaryPrefixOperator::RealCast(..) => PhpType::float(),
        UnaryPrefixOperator::BoolCast(..) | UnaryPrefixOperator::BooleanCast(..) => PhpType::bool(),
        UnaryPrefixOperator::ArrayCast(..) => return Some(array_cast_type(resolve_operand())),
        UnaryPrefixOperator::UnsetCast(..) => return Some(PhpType::named(atom("null"))),
        UnaryPrefixOperator::Not(_) => return Some(PhpType::bool()),
        _ => return None,
    };
    if scalar_fold::is_cheap_scalar_operand(operand)
        && let Some(folded) = scalar_fold::single_scalar(&resolve_operand())
            .and_then(|scalar| scalar_fold::fold_cast(operator, &scalar))
    {
        return Some(folded);
    }
    Some(base)
}

/// `~` yields a string for string operands and an int otherwise.
fn bitwise_not_type(operand: &[ResolvedType]) -> PhpType {
    let is_string = !operand.is_empty()
        && operand
            .iter()
            .all(|rt| rt.type_string.is_subtype_of(&PhpType::string()));
    if is_string {
        PhpType::string()
    } else {
        PhpType::int()
    }
}

/// The type `++`/`--` produces for a numeric operand.
///
/// The step widens a literal (`5` becomes `int`, not `6`) but stays inside
/// the operand's own domain. Non-numeric operands are left unresolved: PHP
/// increments strings alphanumerically, leaves bools and arrays untouched,
/// and treats null asymmetrically (`++null` is `1`, `--null` is `null`).
pub(super) fn stepped_numeric_type(operand: &[ResolvedType]) -> Option<PhpType> {
    if operand.is_empty() {
        return None;
    }
    let joined = ResolvedType::types_joined(operand);
    let mut stepped = Vec::new();
    for member in joined.union_members() {
        if member.is_int_subtype() {
            push_unique_type(&mut stepped, PhpType::int());
        } else if member.is_float_subtype() {
            push_unique_type(&mut stepped, PhpType::float());
        } else {
            return None;
        }
    }
    match stepped.len() {
        0 => None,
        1 => stepped.into_iter().next(),
        _ => Some(PhpType::union(stepped)),
    }
}

fn push_unique_type(types: &mut Vec<PhpType>, member: PhpType) {
    if !types.iter().any(|existing| existing == &member) {
        types.push(member);
    }
}

/// The object shape `(object) $expr` produces: an array shape casts
/// key-for-key, a scalar becomes `object{scalar: T}`, and anything else
/// (including an unresolved operand) falls back to `stdClass`. A cast
/// always produces a `stdClass` instance, so a shape result keeps
/// `stdClass`'s class identity via an intersection.
///
/// The cast does not preserve literal precision, so shape values widen.
fn object_cast_type(operand: Vec<ResolvedType>) -> PhpType {
    let inner =
        (!operand.is_empty()).then(|| ResolvedType::types_joined(&operand).widen_scalar_literals());
    let std_class = || PhpType::named(atom("stdClass"));
    match inner.as_ref().map(PhpType::kind) {
        // `(object) []` is a `stdClass` with no properties; an `object{}`
        // shape would say the same thing in a spelling nothing else uses.
        Some(TypeKind::ArrayShape(entries)) if entries.is_empty() => std_class(),
        Some(TypeKind::ArrayShape(entries)) => PhpType::intersection(vec![
            PhpType::object_shape(
                entries
                    .iter()
                    .map(|e| ShapeEntry {
                        key: e.key.clone(),
                        value_type: e.value_type.widen_scalar_literals(),
                        optional: e.optional,
                    })
                    .collect(),
            ),
            std_class(),
        ]),
        Some(_) if inner.as_ref().is_some_and(is_object_cast_scalar_type) => {
            PhpType::intersection(vec![
                PhpType::object_shape(vec![ShapeEntry {
                    key: Some("scalar".to_string()),
                    value_type: inner.as_ref().unwrap().clone(),
                    optional: false,
                }]),
                std_class(),
            ])
        }
        _ => std_class(),
    }
}

/// `(array) $expr`, applied member by member: an array stays as it is,
/// `null` becomes `[]`, and a scalar becomes the one-entry list holding it.
/// An object (its properties) or a value of unknown type is some array.
fn array_cast_type(operand: Vec<ResolvedType>) -> PhpType {
    fn cast_member(member: &PhpType, scalar: &PhpType, out: &mut Vec<PhpType>) {
        match member.kind() {
            TypeKind::Union(members) => {
                for member in members.iter() {
                    cast_member(member, scalar, out);
                }
            }
            TypeKind::Nullable(inner) => {
                cast_member(inner, scalar, out);
                out.push(PhpType::array_shape(Vec::new()));
            }
            _ if member.is_null() => out.push(PhpType::array_shape(Vec::new())),
            // `iterable` may be a `Traversable`, which casts to its
            // properties rather than to the values it yields.
            _ if member.is_array_like() && !member.is_named("iterable") => {
                out.push(member.clone());
            }
            _ if member.is_subtype_of(scalar) => out.push(PhpType::array_shape(vec![ShapeEntry {
                key: None,
                value_type: member.clone(),
                optional: false,
            }])),
            _ => out.push(PhpType::array()),
        }
    }

    if operand.is_empty() {
        return PhpType::array();
    }
    let scalar = PhpType::union(vec![
        PhpType::int(),
        PhpType::float(),
        PhpType::string(),
        PhpType::bool(),
    ]);
    let mut members = Vec::new();
    cast_member(&ResolvedType::types_joined(&operand), &scalar, &mut members);
    // The shape a scalar casts to adds nothing beside an array member that
    // already holds it: `string|list<string>` casts to `list<string>`.
    let keep: Vec<bool> = members
        .iter()
        .map(|member| {
            member.shape_entries().is_none()
                || !members
                    .iter()
                    .any(|other| other.shape_entries().is_none() && member.is_subtype_of(other))
        })
        .collect();
    crate::util::retain_by_mask(&mut members, &keep);
    PhpType::join_runtime_value_types(members)
}

/// Whether `(object) $expr` on this type produces an `object{scalar: T}`
/// wrapper rather than a plain `stdClass`.
fn is_object_cast_scalar_type(ty: &PhpType) -> bool {
    match ty.kind() {
        TypeKind::Named(name) => matches!(
            keyword_lowercase(name).as_str(),
            "int"
                | "integer"
                | "string"
                | "float"
                | "double"
                | "real"
                | "bool"
                | "boolean"
                | "true"
                | "false"
        ),
        TypeKind::Union(members) => {
            !members.is_empty() && members.iter().all(is_object_cast_scalar_type)
        }
        _ => false,
    }
}
