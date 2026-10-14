//! Small helpers for walking the mago-syntax AST: seeing through
//! parentheses and negation, reading array literals, and visiting class
//! members and `if` branches.

/// Unwrap parenthesized expressions to their inner expression.
pub(crate) fn unwrap_parens<'a>(
    expr: &'a mago_syntax::cst::Expression<'a>,
) -> &'a mago_syntax::cst::Expression<'a> {
    match expr {
        mago_syntax::cst::Expression::Parenthesized(p) => unwrap_parens(p.expression),
        other => other,
    }
}

/// Unwrap parentheses and any number of `!` prefixes from a condition,
/// returning the expression underneath and whether an odd number of `!`s
/// wrapped it.
pub(crate) fn unwrap_negation<'a>(
    expr: &'a mago_syntax::cst::Expression<'a>,
) -> (&'a mago_syntax::cst::Expression<'a>, bool) {
    use mago_syntax::cst::Expression;
    let mut negated = false;
    let mut inner = expr;
    loop {
        match inner {
            Expression::Parenthesized(p) => inner = p.expression,
            Expression::UnaryPrefix(prefix) if prefix.operator.is_not() => {
                negated = !negated;
                inner = prefix.operand;
            }
            _ => return (inner, negated),
        }
    }
}

/// The elements of an array literal, in either spelling (`[…]` and the
/// legacy `array(…)`).
///
/// `None` for anything that is not an array literal.
pub(crate) fn array_literal_elements<'a, 'ast>(
    expr: &'a mago_syntax::cst::Expression<'ast>,
) -> Option<
    &'a mago_syntax::cst::sequence::TokenSeparatedSequence<
        'ast,
        mago_syntax::cst::array::ArrayElement<'ast>,
    >,
> {
    use mago_syntax::cst::Expression;
    match expr {
        Expression::Array(array) => Some(&array.elements),
        Expression::LegacyArray(array) => Some(&array.elements),
        _ => None,
    }
}

/// The value expression of an array element, or `None` for a hole in a
/// list (`[$a, , $c]`).
pub(crate) fn array_element_value<'b>(
    elem: &'b mago_syntax::cst::array::ArrayElement<'b>,
) -> Option<&'b mago_syntax::cst::Expression<'b>> {
    use mago_syntax::cst::array::ArrayElement;
    match elem {
        ArrayElement::KeyValue(kv) => Some(kv.value),
        ArrayElement::Value(v) => Some(v.value),
        ArrayElement::Variadic(v) => Some(v.value),
        ArrayElement::Missing(_) => None,
    }
}

/// The key expression of an array element written as `key => value`.
pub(crate) fn array_element_key<'b>(
    elem: &'b mago_syntax::cst::array::ArrayElement<'b>,
) -> Option<&'b mago_syntax::cst::Expression<'b>> {
    use mago_syntax::cst::array::ArrayElement;
    match elem {
        mago_syntax::cst::array::ArrayElement::KeyValue(kv) => Some(kv.key),
        ArrayElement::Value(_) | ArrayElement::Variadic(_) | ArrayElement::Missing(_) => None,
    }
}

/// The member list of a class-like node, in any of the four spellings.
///
/// `None` for an interface (whose methods have no body) and for every
/// other node kind, which a walker treats as "nothing to descend into
/// here".
pub(crate) fn class_like_members<'ast>(
    node: mago_syntax::cst::node::Node<'ast, 'ast>,
) -> Option<&'ast [mago_syntax::cst::class_like::member::ClassLikeMember<'ast>]> {
    use mago_syntax::cst::node::Node;
    match node {
        Node::Class(class) => Some(class.members.as_slice()),
        Node::AnonymousClass(class) => Some(class.members.as_slice()),
        Node::Trait(trait_) => Some(trait_.members.as_slice()),
        Node::Enum(enum_) => Some(enum_.members.as_slice()),
        _ => None,
    }
}

/// Call `visit` for each method of `members` that has a body, handing it
/// the method and the block.
pub(crate) fn for_each_concrete_method<'ast>(
    members: &'ast [mago_syntax::cst::class_like::member::ClassLikeMember<'ast>],
    mut visit: impl FnMut(
        &'ast mago_syntax::cst::class_like::method::Method<'ast>,
        &'ast mago_syntax::cst::Block<'ast>,
    ),
) {
    use mago_syntax::cst::class_like::member::ClassLikeMember;
    use mago_syntax::cst::class_like::method::MethodBody;
    for member in members {
        if let ClassLikeMember::Method(method) = member
            && let MethodBody::Concrete(body) = &method.body
        {
            visit(method, body);
        }
    }
}

/// Call `visit` with each statement list an `if` can run: the body, the
/// body of every `elseif`, and the `else`.
///
/// Both spellings of an `if` (braced and `:`-delimited) are covered, since
/// `IfBody` already unifies them.
pub(crate) fn for_each_if_branch(
    if_stmt: &mago_syntax::cst::If<'_>,
    mut visit: impl FnMut(&[mago_syntax::cst::Statement<'_>]),
) {
    let _ = try_for_each_if_branch(if_stmt, |statements| {
        visit(statements);
        std::ops::ControlFlow::<()>::Continue(())
    });
}

/// [`for_each_if_branch`] for a walker that stops early: the first branch
/// `visit` breaks on ends the walk and its value comes back.
pub(crate) fn try_for_each_if_branch<B>(
    if_stmt: &mago_syntax::cst::If<'_>,
    mut visit: impl FnMut(&[mago_syntax::cst::Statement<'_>]) -> std::ops::ControlFlow<B>,
) -> std::ops::ControlFlow<B> {
    visit(if_stmt.body.statements())?;
    for statements in if_stmt.body.else_if_statements() {
        visit(statements)?;
    }
    if let Some(statements) = if_stmt.body.else_statements() {
        visit(statements)?;
    }
    std::ops::ControlFlow::Continue(())
}
