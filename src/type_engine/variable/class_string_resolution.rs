/// Class-string variable resolution.
///
/// Resolves variables that hold a class-string value (e.g. `$cls = User::class`)
/// to the referenced `ClassInfo` instances.  This is used when the access kind
/// is `::` (`$cls::`) so that static members are offered instead of string
/// methods.
///
/// Handles simple assignments, match expressions, ternary / null-coalescing
/// branches, and `self` / `static` / `parent` keywords.
use std::sync::Arc;

use mago_span::HasSpan;
use mago_syntax::cst::*;

use crate::atom::bytes_to_str;
use crate::parser::{for_each_nested_statement_list, with_parsed_program};
use crate::types::ClassInfo;

use crate::type_engine::conditional_resolution::extract_class_string_from_expr;
use crate::type_engine::resolver::VarResolutionCtx;

/// Resolve the class a `::class` literal names, as written (`self`,
/// `static` and `parent` included), and append it to `results`.
fn push_class_string_target(name: &str, ctx: &VarResolutionCtx<'_>, results: &mut Vec<ClassInfo>) {
    let fqn = ctx.written_names().resolve_class(name);
    if let Some(cls) =
        crate::class_lookup::find_or_load_resolved(&fqn, ctx.all_classes, ctx.class_loader)
    {
        ClassInfo::push_unique(results, Arc::unwrap_or_clone(cls));
    }
}

/// Resolve a `$variable` that holds a class-string (e.g. `$cls = User::class`)
/// to the referenced class(es).
///
/// Instead of resolving the variable to its *value type* (`string`),
/// this resolves it to the *referenced class* so that static members
/// are offered — see the module docs for the supported patterns.
///
/// Only the class, file, loaders and backend of `outer` are used: the
/// variable is looked up at `cursor_offset` from a fresh walk, not from
/// whatever scope `outer` carries.
pub(in crate::type_engine) fn resolve_class_string_targets(
    var_name: &str,
    cursor_offset: u32,
    outer: &VarResolutionCtx<'_>,
) -> Vec<ClassInfo> {
    with_parsed_program(
        outer.content,
        "resolve_class_string_targets",
        |program, _content| {
            let ctx = VarResolutionCtx {
                source_imports: outer.source_imports,
                backend: outer.backend,
                resolved_class_cache: crate::virtual_members::active_resolved_class_cache(),
                ..VarResolutionCtx::new(
                    var_name,
                    outer.current_class,
                    outer.all_classes,
                    outer.content,
                    cursor_offset,
                    outer.class_loader,
                    outer.source_class_loader,
                )
            };
            resolve_class_string_in_statements(program.statements.iter(), &ctx)
        },
    )
}

/// Walk statements to find class-string assignments to the target variable.
fn resolve_class_string_in_statements<'b>(
    statements: impl Iterator<Item = &'b Statement<'b>>,
    ctx: &VarResolutionCtx<'_>,
) -> Vec<ClassInfo> {
    let stmts: Vec<&Statement> = statements.collect();

    // Check class bodies first (same pattern as resolve_variable_in_statements).
    for &stmt in &stmts {
        match stmt {
            Statement::Class(class) => {
                let start = class.left_brace.start.offset;
                let end = class.right_brace.end.offset;
                if ctx.cursor_offset >= start && ctx.cursor_offset <= end {
                    return resolve_class_string_in_members(class.members.iter(), ctx);
                }
            }
            Statement::Interface(iface) => {
                let start = iface.left_brace.start.offset;
                let end = iface.right_brace.end.offset;
                if ctx.cursor_offset >= start && ctx.cursor_offset <= end {
                    return resolve_class_string_in_members(iface.members.iter(), ctx);
                }
            }
            Statement::Enum(enum_def) => {
                let start = enum_def.left_brace.start.offset;
                let end = enum_def.right_brace.end.offset;
                if ctx.cursor_offset >= start && ctx.cursor_offset <= end {
                    return resolve_class_string_in_members(enum_def.members.iter(), ctx);
                }
            }
            Statement::Trait(trait_def) => {
                let start = trait_def.left_brace.start.offset;
                let end = trait_def.right_brace.end.offset;
                if ctx.cursor_offset >= start && ctx.cursor_offset <= end {
                    return resolve_class_string_in_members(trait_def.members.iter(), ctx);
                }
            }
            Statement::Namespace(ns) => {
                let results = resolve_class_string_in_statements(ns.statements().iter(), ctx);
                if !results.is_empty() {
                    return results;
                }
            }
            Statement::Function(func) => {
                let body_start = func.body.left_brace.start.offset;
                let body_end = func.body.right_brace.end.offset;
                if ctx.cursor_offset >= body_start && ctx.cursor_offset <= body_end {
                    let mut results = Vec::new();
                    walk_class_string_assignments(func.body.statements.iter(), ctx, &mut results);
                    return results;
                }
            }
            _ => {}
        }
    }

    // Top-level code.
    let mut results = Vec::new();
    walk_class_string_assignments(stmts.into_iter(), ctx, &mut results);
    results
}

/// Resolve class-string assignments inside class-like members.
fn resolve_class_string_in_members<'b>(
    members: impl Iterator<Item = &'b ClassLikeMember<'b>>,
    ctx: &VarResolutionCtx<'_>,
) -> Vec<ClassInfo> {
    for member in members {
        if let ClassLikeMember::Method(method) = member {
            let body = match &method.body {
                MethodBody::Concrete(body) => body,
                _ => continue,
            };
            let start = body.left_brace.start.offset;
            let end = body.right_brace.end.offset;
            if ctx.cursor_offset >= start && ctx.cursor_offset <= end {
                let mut results = Vec::new();
                walk_class_string_assignments(body.statements.iter(), ctx, &mut results);
                return results;
            }
        }
    }
    vec![]
}

/// Walk statements collecting class names from `$var = Foo::class` assignments.
fn walk_class_string_assignments<'b>(
    statements: impl Iterator<Item = &'b Statement<'b>>,
    ctx: &VarResolutionCtx<'_>,
    results: &mut Vec<ClassInfo>,
) {
    // Track variables assigned an array literal of `::class` entries
    // (e.g. `$repos = [Foo::class, Bar::class]`) so that a later
    // `foreach ($repos as $repository)` can resolve `$repository` to the
    // union of element classes.  Assignments precede the foreach in the
    // statement stream, so recording them as we go makes the mapping
    // available by the time the foreach is processed.
    let mut array_class_vars: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    for stmt in statements {
        if stmt.span().start.offset >= ctx.cursor_offset {
            continue;
        }
        // Record `$var = [A::class, B::class]` array-literal assignments.
        if let Statement::Expression(expr_stmt) = stmt
            && let Expression::Assignment(assignment) = expr_stmt.expression
            && assignment.operator.is_assign()
            && let Expression::Variable(Variable::Direct(dv)) = assignment.lhs
        {
            let names = extract_class_string_names_from_array(assignment.rhs);
            if !names.is_empty() {
                array_class_vars.insert(bytes_to_str(dv.name).to_string(), names);
            }
        }
        match stmt {
            Statement::Expression(expr_stmt) => {
                check_class_string_assignment(expr_stmt.expression, ctx, results);
            }
            Statement::Foreach(foreach) => {
                // Check if the foreach value variable matches our target
                // and the iterated expression is an array of ::class literals.
                let value_expr = foreach.target.value();
                let value_name = match value_expr {
                    Expression::Variable(Variable::Direct(dv)) => {
                        Some(bytes_to_str(dv.name).to_string())
                    }
                    _ => None,
                };
                if let Some(name) = value_name
                    && name == ctx.var_name
                {
                    // Extract class names from the iterated expression.
                    // The iterable may be an inline array literal
                    // (`[Page::class, CustomPage::class]`) or a variable
                    // that was assigned one earlier (`$pages = [...]`).
                    let mut class_names = extract_class_string_names_from_array(foreach.expression);
                    if class_names.is_empty()
                        && let Expression::Variable(Variable::Direct(dv)) = foreach.expression
                        && let Some(names) = array_class_vars.get(bytes_to_str(dv.name))
                    {
                        class_names = names.clone();
                    }
                    if !class_names.is_empty() {
                        results.clear();
                        for cn in class_names {
                            push_class_string_target(&cn, ctx, results);
                        }
                    }
                }
            }
            _ => {}
        }
        // Descend into nested control-flow blocks so that an assignment
        // inside a braced foreach/if/while/for/switch/try body (the common
        // case: `foreach (...) { $cls = Foo::class; }`) is still found.
        for_each_nested_statement_list(stmt, |nested| {
            walk_class_string_assignments(nested.iter(), ctx, results);
        });
    }
}

/// Check if an expression is an assignment of a `::class` literal
/// to the target variable, and if so, resolve the class.
fn check_class_string_assignment(
    expr: &Expression<'_>,
    ctx: &VarResolutionCtx<'_>,
    results: &mut Vec<ClassInfo>,
) {
    let Expression::Assignment(assignment) = expr else {
        return;
    };
    if !assignment.operator.is_assign() {
        return;
    }
    let lhs_name = match assignment.lhs {
        Expression::Variable(Variable::Direct(dv)) => bytes_to_str(dv.name).to_string(),
        _ => return,
    };
    if lhs_name != ctx.var_name {
        return;
    }

    let class_names = extract_class_string_names(assignment.rhs);
    // Clear previous results — the last unconditional assignment wins.
    results.clear();
    for name in class_names {
        push_class_string_target(&name, ctx, results);
    }
}

/// Extract class names from `::class` expressions, recursing into
/// match arms, ternary branches, null-coalescing, and parenthesized
/// expressions.
fn extract_class_string_names(expr: &Expression<'_>) -> Vec<String> {
    if let Some(name) = extract_class_string_from_expr(expr) {
        return vec![name];
    }
    match expr {
        Expression::Parenthesized(p) => extract_class_string_names(p.expression),
        Expression::Match(match_expr) => {
            let mut names = Vec::new();
            for arm in match_expr.arms.iter() {
                names.extend(extract_class_string_names(arm.expression()));
            }
            names
        }
        Expression::Conditional(cond) => {
            let mut names = Vec::new();
            let then_expr = cond.then.unwrap_or(cond.condition);
            names.extend(extract_class_string_names(then_expr));
            names.extend(extract_class_string_names(cond.r#else));
            names
        }
        Expression::Binary(binary) if binary.operator.is_null_coalesce() => {
            let mut names = Vec::new();
            names.extend(extract_class_string_names(binary.lhs));
            names.extend(extract_class_string_names(binary.rhs));
            names
        }
        _ => vec![],
    }
}

/// Extract class names from array elements that are `::class` literals.
///
/// Handles `[Page::class, CustomPage::class]` and similar array
/// expressions used as foreach iterables.
fn extract_class_string_names_from_array(expr: &Expression<'_>) -> Vec<String> {
    match expr {
        Expression::Array(array) => {
            let mut names = Vec::new();
            for item in array.elements.iter() {
                if let ArrayElement::Value(val) = item {
                    names.extend(extract_class_string_names(val.value));
                }
            }
            names
        }
        Expression::LegacyArray(array) => {
            let mut names = Vec::new();
            for item in array.elements.iter() {
                if let ArrayElement::Value(val) = item {
                    names.extend(extract_class_string_names(val.value));
                }
            }
            names
        }
        _ => vec![],
    }
}
