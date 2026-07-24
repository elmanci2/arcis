//! General type-mismatch checker.
//!
//! Complements [`crate::nullsafety`] (which only enforces "an optional
//! value must be resolved before use") with the more basic guarantee a
//! statically-typed language is expected to give: **once a binding's type
//! is established — by an explicit annotation, or by inference from its
//! initializer — assigning, returning, or storing a value of an
//! incompatible type is a compile error.** `let numero = 4; numero = "";`
//! is rejected here, on both backends, identically — the type doesn't
//! silently become mutable just because you didn't write it down.
//!
//! Without this pass, an incompatible assignment either produces a
//! confusing `rustc` error several layers removed from the Arcis source
//! (Rust backend) or **crashes the Cranelift backend outright** with a raw
//! `cranelift-frontend` panic ("declared type of variable var0 doesn't
//! match type of value v1") — not a diagnostic at all. This pass turns
//! both into one clean, Arcis-level error before codegen ever runs.
//!
//! ## What counts as a mismatch
//!
//! Deliberately conservative: only `number` / `string` / `boolean` /
//! `bigint` (aliased to `number`) and array-of-those are compared
//! structurally. Objects, interfaces, functions, unions, intersections,
//! and any type this pass can't fully resolve are treated as compatible
//! with anything — false negatives (a real mismatch slipping through) are
//! an acceptable trade-off against false positives (rejecting valid code
//! this pass doesn't fully understand yet). `any` and `null`/`undefined`
//! always pass (the latter is [`crate::nullsafety`]'s job to police).
//!
//! ## Sinks checked
//!
//! - a `let`/`const` with an explicit type annotation, against its
//!   initializer,
//! - `x = value;` (plain reassignment), against `x`'s established type,
//! - `arr[i] = value;`, against the array's element type,
//! - `obj.field = value;`, against the field's resolved type,
//! - `return value;`, against the enclosing function's return type
//!   (concrete post-inference either way — annotated, or derived from an
//!   earlier `return`, so a *later* `return` of a different type is
//!   caught too).
//!
//! Function-call arguments are already checked at codegen time by both
//! backends (`arcis-codegen-cranelift`'s `emit_user_call`, and `rustc`
//! itself for the Rust backend) — not duplicated here.

use std::collections::HashMap;

use arcis_ast::{ExportDefault, Expr, Function, LiteralValue, Program, Stmt, Type};

use crate::infer::{self, TypeEnv};

/// One type-mismatch violation.
#[derive(Debug, Clone)]
pub struct TypeMismatchIssue {
    pub message: String,
    /// Best-effort position — see [`crate::nullsafety::NullSafetyIssue`]'s
    /// doc comment for why this is often `(0, 0)` (many `Expr` nodes carry
    /// no span in this AST).
    pub line: usize,
    pub col: usize,
}

fn issue(message: String, line: usize, col: usize) -> TypeMismatchIssue {
    TypeMismatchIssue { message, line, col }
}

type Scope = HashMap<String, Type>;

fn is_void(t: &Type) -> bool {
    matches!(t, Type::Primitive(p) if p == "void")
}

fn is_any(t: &Type) -> bool {
    matches!(t, Type::Primitive(p) if p == "any")
}

/// The primitive "kind" of a type, if it's a plain primitive or a literal
/// type (a literal always narrows to exactly one primitive kind). `None`
/// for anything structurally more complex (object, function, union,
/// unresolved named type, ...) — see the module doc's "deliberately
/// conservative" note.
fn primitive_kind(t: &Type) -> Option<String> {
    match t {
        Type::Primitive(p) if p == "bigint" => Some("number".to_string()),
        Type::Primitive(p) => Some(p.clone()),
        Type::Literal(LiteralValue::String(_)) => Some("string".to_string()),
        Type::Literal(LiteralValue::Number(_)) => Some("number".to_string()),
        Type::Literal(LiteralValue::Bool(_)) => Some("boolean".to_string()),
        _ => None,
    }
}

/// `true` if a value of type `actual` may be stored where `expected` is
/// required. See the module doc for exactly how conservative this is.
fn types_compatible(expected: &Type, actual: &Type) -> bool {
    let expected = expected.unwrap_optional();
    let actual = actual.unwrap_optional();
    if is_any(expected) || is_any(actual) {
        return true;
    }
    // A bare `null`/`undefined` into a non-optional slot is
    // `nullsafety`'s guarantee to police, not this pass's.
    if matches!(actual, Type::Null | Type::Undefined) {
        return true;
    }
    match (expected, actual) {
        (Type::Array(e), Type::Array(a)) => types_compatible(e, a),
        (Type::Array(_), _) | (_, Type::Array(_)) => false,
        _ => match (primitive_kind(expected), primitive_kind(actual)) {
            (Some(e), Some(a)) => e == a,
            _ => true,
        },
    }
}

/// Best-effort expression type, deferring to [`infer::expr_type`]. `None`
/// (unknown) is treated as compatible with anything — never block valid
/// code just because this pass couldn't pin the type down.
fn check_sink(
    e: &Expr,
    expected: &Type,
    what: &str,
    scope: &Scope,
    env: &TypeEnv,
    out: &mut Vec<TypeMismatchIssue>,
    line: usize,
    col: usize,
) {
    let Some(actual) = infer::expr_type(e, scope, env) else {
        return;
    };
    if !types_compatible(expected, &actual) {
        out.push(issue(
            format!(
                "type mismatch: {what} expects `{}`, found `{}`",
                type_label(expected),
                type_label(&actual),
            ),
            line,
            col,
        ));
    }
}

/// Short, human-readable rendering of a type for error messages — not
/// exhaustive (objects/functions collapse to a generic label), since the
/// mismatches this pass actually raises are always primitive/array kinds
/// (see `types_compatible`).
fn type_label(t: &Type) -> String {
    match t {
        Type::Optional(inner) => format!("{}?", type_label(inner)),
        Type::Array(inner) => format!("{}[]", type_label(inner)),
        Type::Primitive(p) => p.clone(),
        Type::Named(n) => n.clone(),
        Type::Object { name, .. } => name.clone(),
        Type::Null => "null".to_string(),
        Type::Undefined => "undefined".to_string(),
        Type::Literal(LiteralValue::String(s)) => format!("\"{s}\""),
        Type::Literal(LiteralValue::Number(n)) => format!("{n}"),
        Type::Literal(LiteralValue::Bool(b)) => format!("{b}"),
        Type::Union(_) => "union".to_string(),
        Type::Intersection(_) => "intersection".to_string(),
        Type::Function { .. } => "function".to_string(),
    }
}

fn check_stmt(
    stmt: &Stmt,
    scope: &mut Scope,
    env: &TypeEnv,
    out: &mut Vec<TypeMismatchIssue>,
    ret: Option<&Type>,
) {
    match stmt {
        Stmt::Let { name, ty, value, line, col } | Stmt::Const { name, ty, value, line, col } => {
            if let Some(t) = ty {
                check_sink(value, t, &format!("`{name}`"), scope, env, out, *line, *col);
                scope.insert(name.clone(), t.clone());
            } else if let Some(t) = infer::expr_type(value, scope, env) {
                scope.insert(name.clone(), t);
            }
        }
        Stmt::Assign { name, value } => {
            if let Some(t) = scope.get(name).cloned() {
                check_sink(value, &t, &format!("`{name}`"), scope, env, out, 0, 0);
            }
        }
        Stmt::AssignIndex { object, index: _, value } => {
            if let Some(Type::Array(elem)) = scope.get(object).cloned() {
                check_sink(value, &elem, &format!("an element of `{object}`"), scope, env, out, 0, 0);
            }
        }
        Stmt::AssignMember { object, property, value } => {
            let field_expr = Expr::Member { object: object.clone(), property: property.clone() };
            if let Some(field_ty) = infer::expr_type(&field_expr, scope, env) {
                check_sink(value, &field_ty, &format!("field `.{property}`"), scope, env, out, 0, 0);
            }
        }
        Stmt::Function(f) => check_function(f, scope, env, out),
        Stmt::Return(Some(e)) => {
            if let Some(rt) = ret {
                if !is_void(rt) {
                    check_sink(e, rt, "the function's return type", scope, env, out, 0, 0);
                }
            }
        }
        Stmt::Return(None) => {}
        Stmt::If { condition: _, then_branch, else_branch } => {
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
        Stmt::While { condition: _, body } => {
            let mut inner = scope.clone();
            for s in body {
                check_stmt(s, &mut inner, env, out, ret);
            }
        }
        Stmt::For { init, condition: _, update, body } => {
            let mut inner = scope.clone();
            if let Some(i) = init {
                check_stmt(i, &mut inner, env, out, ret);
            }
            if let Some(u) = update {
                check_stmt(u, &mut inner, env, out, ret);
            }
            for s in body {
                check_stmt(s, &mut inner, env, out, ret);
            }
        }
        Stmt::ForOf { name, ty, iterable: _, body } => {
            let mut inner = scope.clone();
            if let Some(t) = ty {
                inner.insert(name.clone(), t.clone());
            }
            for s in body {
                check_stmt(s, &mut inner, env, out, ret);
            }
        }
        Stmt::Switch { discriminant: _, cases } => {
            for c in cases {
                let mut inner = scope.clone();
                for s in &c.body {
                    check_stmt(s, &mut inner, env, out, ret);
                }
            }
        }
        Stmt::Try { body, catch_name, catch_body } => {
            let mut b = scope.clone();
            for s in body {
                check_stmt(s, &mut b, env, out, ret);
            }
            let mut c = scope.clone();
            if let Some(n) = catch_name {
                c.insert(n.clone(), Type::string());
            }
            for s in catch_body {
                check_stmt(s, &mut c, env, out, ret);
            }
        }
        Stmt::ExportDecl(inner) => check_stmt(inner, scope, env, out, ret),
        Stmt::ExportDefault(ExportDefault::Function(f)) => check_function(f, scope, env, out),
        _ => {}
    }
}

fn check_function(f: &Function, outer_scope: &Scope, env: &TypeEnv, out: &mut Vec<TypeMismatchIssue>) {
    let mut scope = outer_scope.clone();
    for p in &f.params {
        scope.insert(p.name.clone(), p.ty.clone());
    }
    let ret = if is_void(&f.return_type) { None } else { Some(&f.return_type) };
    for s in &f.body {
        check_stmt(s, &mut scope, env, out, ret);
    }
}

/// Check every statement of `program` and return every type mismatch
/// found (empty when the program is sound). `env` should be the same
/// [`TypeEnv`] used for [`crate::infer::infer_program`] (built from every
/// linked module), so cross-module function calls/returns are checked
/// with accurate signatures too.
pub fn check_types(program: &Program, env: &TypeEnv) -> Vec<TypeMismatchIssue> {
    let mut out = Vec::new();
    let mut scope: Scope = HashMap::new();
    for stmt in &program.stmts {
        check_stmt(stmt, &mut scope, env, &mut out, None);
    }
    out
}
