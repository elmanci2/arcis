//! Null-safety checker.
//!
//! The core guarantee this pass enforces: **a value typed `T?` can never
//! reach a place that expects a guaranteed `T` without being resolved
//! first** — either with a solid (non-optional) fallback (`x ?? fallback`),
//! a proven-present guard (`if (x != null) { ... }`, narrowed away by
//! [`crate::narrow`] before this pass runs), or an explicit assertion
//! (`x!`, the user's deliberate "trust me"). If none of those apply, this
//! is a **compile error**, not a warning — the same class of guarantee
//! Rust's `Option<T>` gives, surfaced at the Arcis source level so it
//! applies identically to both backends.
//!
//! What counts as a "place that expects a guaranteed `T`" (a *sink*):
//! - the initializer of a `let`/`const` with an explicit non-optional type,
//! - a `return` inside a function with a non-optional (non-`void`) return
//!   type,
//! - an argument passed to a parameter of a known function,
//! - the right-hand side of `??` itself (the fallback must be solid — no
//!   "two optionals", per the language's design goal),
//! - operands of arithmetic/comparison operators (other than the
//!   `== null` / `!= null` idiom),
//! - the receiver of `.field` / `[index]`,
//! - the single argument to `print`/`str`.
//!
//! Run AFTER [`crate::infer::infer_program`] (every relevant type must be
//! concrete) and AFTER [`crate::narrow::narrow_program`] (so proven-safe
//! uses inside a guard are already rewritten to a non-optional shadow and
//! never get flagged).
//!
//! This pass never MODIFIES the AST — it only reports. Deliberately
//! conservative: when a value's type can't be determined, it is treated as
//! safe (no false positives over false negatives — an unhandled optional
//! will still fail loudly at the sink where it's finally consumed, or at
//! worst at codegen).

use std::collections::HashMap;

use arcis_ast::{ArrayElement, BinOp, Expr, ExportDefault, Function, ObjectField, Program, Stmt, Type};

use crate::infer::{self, TypeEnv};

/// One null-safety violation.
#[derive(Debug, Clone)]
pub struct NullSafetyIssue {
    pub message: String,
    /// Best-effort position — many Arcis `Expr` nodes carry no span, so
    /// this anchors to the nearest enclosing declaration that does
    /// (`let`/`const`/`function`). `(0, 0)` when nothing better is known.
    pub line: usize,
    pub col: usize,
}

fn issue(message: String, line: usize, col: usize) -> NullSafetyIssue {
    NullSafetyIssue { message, line, col }
}

type Scope = HashMap<String, Type>;

fn is_void(t: &Type) -> bool {
    matches!(t, Type::Primitive(p) if p == "void")
}

fn is_null_literal(e: &Expr) -> bool {
    matches!(e, Expr::Null | Expr::Undefined)
}

/// `true` if `e`'s value is guaranteed present — either because its
/// (best-effort inferred) type isn't optional, or because it's one of the
/// two forms that neutralize optionality by construction (`x ?? fallback`,
/// `x!`). Unknown types (inference gave up) are treated as safe — see the
/// module doc's "conservative" note.
fn is_definite(e: &Expr, scope: &Scope, env: &TypeEnv) -> bool {
    match e {
        Expr::NonNullAssertion(_) => true,
        Expr::Binary { op: BinOp::NullishCoalesce, .. } => true,
        _ => match infer::expr_type(e, scope, env) {
            Some(t) => !t.is_optional(),
            None => true,
        },
    }
}

/// Check that `e` is an acceptable value for a sink whose declared type is
/// `expected`. Pushes an issue if `expected` is non-optional and `e` isn't
/// guaranteed present.
fn check_sink(
    e: &Expr,
    expected: &Type,
    what: &str,
    scope: &Scope,
    env: &TypeEnv,
    out: &mut Vec<NullSafetyIssue>,
    line: usize,
    col: usize,
) {
    if expected.is_optional() {
        return;
    }
    if is_null_literal(e) {
        out.push(issue(
            format!(
                "`null` {what}, but the target type isn't optional — make it `T?`, or provide `?? fallback`."
            ),
            line,
            col,
        ));
        return;
    }
    if !is_definite(e, scope, env) {
        out.push(issue(
            format!(
                "a possibly-missing value is {what} without being resolved — add `?? fallback`, guard it first with `if (x != null) {{ ... }}`, or assert it with `!` if you're certain it's present."
            ),
            line,
            col,
        ));
    }
}

/// Recursively check every expression NESTED inside `e` for its own
/// sink violations (`??` fallback, member/index receiver, arithmetic
/// operands, call arguments) — independent of whatever outer sink `e`
/// itself is feeding.
fn check_expr(e: &Expr, scope: &Scope, env: &TypeEnv, out: &mut Vec<NullSafetyIssue>, line: usize, col: usize) {
    match e {
        Expr::Binary { op: BinOp::NullishCoalesce, left, right } => {
            check_expr(left, scope, env, out, line, col);
            check_expr(right, scope, env, out, line, col);
            if is_null_literal(right) || !is_definite(right, scope, env) {
                out.push(issue(
                    "the fallback on the right of `??` must be a guaranteed (non-optional) value — a possibly-missing fallback defeats the guarantee `??` is supposed to give. Chain another `?? realDefault`, or use a plain literal/definite value.".to_string(),
                    line,
                    col,
                ));
            }
        }
        Expr::Binary { op, left, right } => {
            check_expr(left, scope, env, out, line, col);
            check_expr(right, scope, env, out, line, col);
            // `x == null` / `x != null` is the sanctioned way to test
            // presence — exempt it from the "operands must be definite"
            // rule below (that's precisely what it's checking).
            let is_null_test = matches!(op, BinOp::EqEq | BinOp::NotEq)
                && (is_null_literal(left) || is_null_literal(right));
            if !is_null_test {
                if !is_null_literal(left) && !is_definite(left, scope, env) {
                    out.push(issue(
                        "the left-hand side of this operator is a possibly-missing value — resolve it with `?? fallback` or a null check first.".to_string(),
                        line,
                        col,
                    ));
                }
                if !is_null_literal(right) && !is_definite(right, scope, env) {
                    out.push(issue(
                        "the right-hand side of this operator is a possibly-missing value — resolve it with `?? fallback` or a null check first.".to_string(),
                        line,
                        col,
                    ));
                }
            }
        }
        Expr::Unary { operand, .. } => check_expr(operand, scope, env, out, line, col),
        Expr::Member { object, property } => {
            check_expr(object, scope, env, out, line, col);
            if !is_definite(object, scope, env) {
                out.push(issue(
                    format!(
                        "`.{property}` is accessed on a possibly-missing value — resolve it with `?? fallback`, a null check (`if (x != null) {{ ... }}`), or `!` first."
                    ),
                    line,
                    col,
                ));
            }
        }
        Expr::Index { object, index } => {
            check_expr(object, scope, env, out, line, col);
            check_expr(index, scope, env, out, line, col);
            if !is_definite(object, scope, env) {
                out.push(issue(
                    "an index is taken on a possibly-missing value — resolve it with `?? fallback`, a null check, or `!` first.".to_string(),
                    line,
                    col,
                ));
            }
        }
        Expr::Call { callee, args } => {
            check_expr(callee, scope, env, out, line, col);
            for a in args {
                check_expr(a, scope, env, out, line, col);
            }
            if let Expr::Ident(name) = callee.as_ref() {
                // `print`/`str` consume their argument as a real value —
                // treat it as a sink too (matches every other sink; an
                // unresolved optional shouldn't silently print "missing").
                if matches!(name.as_str(), "print" | "str") {
                    if let Some(a) = args.first() {
                        if is_null_literal(a) || !is_definite(a, scope, env) {
                            out.push(issue(
                                format!(
                                    "`{name}(...)` is given a possibly-missing value — resolve it with `?? fallback`, a null check, or `!` first."
                                ),
                                line,
                                col,
                            ));
                        }
                    }
                } else if let Some(params) = env.function_param_types(name) {
                    for (i, (a, pty)) in args.iter().zip(params.iter()).enumerate() {
                        check_sink(
                            a,
                            pty,
                            &format!("passed as argument {} to `{}`", i + 1, name),
                            scope,
                            env,
                            out,
                            line,
                            col,
                        );
                    }
                }
            }
        }
        Expr::ArrayLiteral { elements } => {
            for el in elements {
                match el {
                    ArrayElement::Item(x) | ArrayElement::Spread(x) => {
                        check_expr(x, scope, env, out, line, col)
                    }
                }
            }
        }
        Expr::ObjectLiteral { fields } => {
            for f in fields {
                match f {
                    ObjectField::KV(_, v) | ObjectField::Spread(v) => {
                        check_expr(v, scope, env, out, line, col)
                    }
                }
            }
        }
        Expr::NonNullAssertion(inner) | Expr::AsConst(inner) => {
            check_expr(inner, scope, env, out, line, col)
        }
        Expr::AsAssertion { expr, .. } => check_expr(expr, scope, env, out, line, col),
        Expr::TypeOf(inner) => check_expr(inner, scope, env, out, line, col),
        Expr::Arrow { params, body, .. } => {
            let mut inner = scope.clone();
            for p in params {
                inner.insert(p.name.clone(), p.ty.clone());
            }
            match body {
                arcis_ast::ArrowBody::Expr(e) => check_expr(e, &inner, env, out, line, col),
                arcis_ast::ArrowBody::Block(stmts) => {
                    for s in stmts {
                        check_stmt(s, &mut inner, env, out, None);
                    }
                }
            }
        }
        _ => {}
    }
}

fn check_stmt(
    stmt: &Stmt,
    scope: &mut Scope,
    env: &TypeEnv,
    out: &mut Vec<NullSafetyIssue>,
    ret: Option<&Type>,
) {
    match stmt {
        Stmt::Let { name, ty, value, line, col } | Stmt::Const { name, ty, value, line, col } => {
            check_expr(value, scope, env, out, *line, *col);
            if let Some(t) = ty {
                check_sink(value, t, &format!("assigned to `{name}`"), scope, env, out, *line, *col);
                scope.insert(name.clone(), t.clone());
            }
        }
        Stmt::Assign { name, value, line, col } => {
            check_expr(value, scope, env, out, *line, *col);
            if let Some(t) = scope.get(name).cloned() {
                check_sink(value, &t, &format!("assigned to `{name}`"), scope, env, out, *line, *col);
            }
        }
        Stmt::AssignIndex { index, value, line, col, .. } => {
            check_expr(index, scope, env, out, *line, *col);
            check_expr(value, scope, env, out, *line, *col);
        }
        Stmt::AssignMember { object, value, line, col, .. } => {
            check_expr(object, scope, env, out, *line, *col);
            check_expr(value, scope, env, out, *line, *col);
            if !is_definite(object, scope, env) {
                out.push(issue(
                    "a field is assigned on a possibly-missing value — resolve it with `?? fallback`, a null check, or `!` first.".to_string(),
                    *line,
                    *col,
                ));
            }
        }
        Stmt::Function(f) => check_function(f, scope, env, out),
        Stmt::Return(Some(e)) => {
            check_expr(e, scope, env, out, 0, 0);
            if let Some(rt) = ret {
                check_sink(e, rt, "returned", scope, env, out, 0, 0);
            }
        }
        Stmt::Return(None) => {}
        Stmt::If { condition, then_branch, else_branch } => {
            check_expr(condition, scope, env, out, 0, 0);
            let mut then_scope = scope.clone();
            for s in then_branch {
                check_stmt(s, &mut then_scope, env, out, ret);
            }
            if let Some(eb) = else_branch {
                let mut else_scope = scope.clone();
                for s in eb {
                    check_stmt(s, &mut else_scope, env, out, ret);
                }
            }
        }
        Stmt::While { condition, body } => {
            check_expr(condition, scope, env, out, 0, 0);
            let mut inner = scope.clone();
            for s in body {
                check_stmt(s, &mut inner, env, out, ret);
            }
        }
        Stmt::For { init, condition, update, body } => {
            let mut inner = scope.clone();
            if let Some(i) = init {
                check_stmt(i, &mut inner, env, out, ret);
            }
            if let Some(c) = condition {
                check_expr(c, &inner, env, out, 0, 0);
            }
            if let Some(u) = update {
                check_stmt(u, &mut inner, env, out, ret);
            }
            for s in body {
                check_stmt(s, &mut inner, env, out, ret);
            }
        }
        Stmt::ForOf { name, ty, iterable, body } => {
            check_expr(iterable, scope, env, out, 0, 0);
            let mut inner = scope.clone();
            if let Some(t) = ty {
                inner.insert(name.clone(), t.clone());
            }
            for s in body {
                check_stmt(s, &mut inner, env, out, ret);
            }
        }
        Stmt::Switch { discriminant, cases } => {
            check_expr(discriminant, scope, env, out, 0, 0);
            for c in cases {
                let mut inner = scope.clone();
                for v in &c.values {
                    check_expr(v, &inner, env, out, 0, 0);
                }
                for s in &c.body {
                    check_stmt(s, &mut inner, env, out, ret);
                }
            }
        }
        Stmt::Try { body, catch_body, .. } => {
            let mut b = scope.clone();
            for s in body {
                check_stmt(s, &mut b, env, out, ret);
            }
            let mut c = scope.clone();
            for s in catch_body {
                check_stmt(s, &mut c, env, out, ret);
            }
        }
        Stmt::Throw(e) => check_expr(e, scope, env, out, 0, 0),
        Stmt::Expr(e) => check_expr(e, scope, env, out, 0, 0),
        Stmt::ExportDecl(inner) => check_stmt(inner, scope, env, out, ret),
        Stmt::ExportDefault(ExportDefault::Function(f)) => check_function(f, scope, env, out),
        Stmt::ExportDefault(ExportDefault::Expr(e)) => check_expr(e, scope, env, out, 0, 0),
        _ => {}
    }
}

fn check_function(f: &Function, outer_scope: &Scope, env: &TypeEnv, out: &mut Vec<NullSafetyIssue>) {
    let mut scope = outer_scope.clone();
    for p in &f.params {
        scope.insert(p.name.clone(), p.ty.clone());
    }
    let ret = if is_void(&f.return_type) { None } else { Some(&f.return_type) };
    for s in &f.body {
        check_stmt(s, &mut scope, env, out, ret);
    }
}

/// Check every statement of `program` and return every null-safety
/// violation found (empty when the program is sound). `env` should be
/// built from every linked module (same `TypeEnv` used for
/// [`crate::infer::infer_program`]) so cross-module function calls are
/// checked too.
pub fn check_null_safety(program: &Program, env: &TypeEnv) -> Vec<NullSafetyIssue> {
    let mut out = Vec::new();
    let mut scope: Scope = HashMap::new();
    for stmt in &program.stmts {
        check_stmt(stmt, &mut scope, env, &mut out, None);
    }
    out
}
