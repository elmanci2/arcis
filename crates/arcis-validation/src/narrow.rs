//! Control-flow narrowing for optional (`T?`) bindings.
//!
//! TypeScript lets you use an optional value directly once you've proven,
//! syntactically, that it can't be missing — `if (x != null) { x.foo() }`.
//! Arcis gets the same ergonomics via an AST *rewrite*, not a separate
//! flow-sensitive type system: when this pass recognizes one of the two
//! common guard shapes below, it inserts a `let` binding under a **fresh,
//! unique name** at the point where the guard proves the original is
//! present, initialized with `original!` (a real, checked unwrap — see
//! `nullsafety`), and rewrites every reference to the original name inside
//! the narrowed region to the fresh one.
//!
//! A fresh name (not the original one, shadowed) is used deliberately:
//! [`crate::shadowing::resolve_shadowing`] only alpha-renames *nested*
//! shadowing (an inner block re-declaring an outer name) — by design, it
//! leaves a *same-scope* re-declaration untouched so [`crate::duplicate`]
//! can still report it as a genuine mistake. The guard-clause idiom below
//! inserts its narrowed binding as a **sibling** of the original
//! declaration (same flat scope), so reusing the same name would either
//! get flagged as a duplicate or, worse, silently confuse every
//! downstream flat, name-keyed map (codegen's `collect_type_scope` and
//! friends) about which declaration is in effect at which point. Minting
//! a new name sidesteps the whole problem without depending on
//! `resolve_shadowing` running again afterward.
//!
//! Two shapes are recognized (identifier operand only — `obj.field != null`
//! is not narrowed, matching the checker's guidance to bind a field to a
//! local first):
//!
//! - `if (x != null) { ... }` → narrowed binding inserted at the front of
//!   the `then` branch, remaining `then`-branch statements rewritten to
//!   use it (and, symmetrically, `if (x == null) { ... } else { ... }`
//!   narrows inside the `else` branch).
//! - `if (x == null) { return/throw/break/continue; }` (no `else`) — the
//!   guard clause idiom: `x` is narrowed for every statement *after* the
//!   `if`, in the same block, so the narrowed binding and rewrite apply
//!   there.
//!
//! Must run AFTER [`crate::infer::infer_program`] (every relevant binding
//! needs a concrete `Type` to know whether it's optional).

use arcis_ast::{Expr, ExportDefault, Function, Program, Stmt, Type};

/// Rewrite every recognized narrowing guard in `program`, in place.
pub fn narrow_program(program: &mut Program) {
    let mut counter: u32 = 0;
    narrow_block(&mut program.stmts, Scope::new(), &mut counter);
}

fn is_null_lit(e: &Expr) -> bool {
    matches!(e, Expr::Null | Expr::Undefined)
}

/// If `cond` is `IDENT != null` / `IDENT == null` (either operand order),
/// return `(name, is_not_equal)`.
fn null_check_ident(cond: &Expr) -> Option<(String, bool)> {
    let Expr::Binary { op, left, right } = cond else {
        return None;
    };
    let is_ne = match op {
        arcis_ast::BinOp::NotEq => true,
        arcis_ast::BinOp::EqEq => false,
        _ => return None,
    };
    let (ident_side, null_side) = if is_null_lit(right) {
        (left.as_ref(), right.as_ref())
    } else if is_null_lit(left) {
        (right.as_ref(), left.as_ref())
    } else {
        return None;
    };
    let _ = null_side;
    match ident_side {
        Expr::Ident(name) => Some((name.clone(), is_ne)),
        _ => None,
    }
}

fn definitely_exits(stmts: &[Stmt]) -> bool {
    matches!(
        stmts.last(),
        Some(Stmt::Return(_) | Stmt::Throw(_) | Stmt::Break | Stmt::Continue)
    )
}

/// `let fresh: inner = name!;`
fn narrowed_let(fresh: &str, name: &str, inner: Type) -> Stmt {
    Stmt::Let {
        name: fresh.to_string(),
        ty: Some(inner),
        value: Expr::NonNullAssertion(Box::new(Expr::Ident(name.to_string()))),
        line: 0,
        col: 0,
    }
}

fn mint(counter: &mut u32, name: &str) -> String {
    *counter += 1;
    format!("{name}__narrow{counter}")
}

/// Rewrite every bare `Expr::Ident(from)` inside `stmts` to `to`. Used right
/// after inserting a narrowed binding, over exactly the statements that are
/// now within its scope.
fn rename_in_block(stmts: &mut [Stmt], from: &str, to: &str) {
    for s in stmts {
        rename_in_stmt(s, from, to);
    }
}

fn rename_in_stmt(stmt: &mut Stmt, from: &str, to: &str) {
    match stmt {
        Stmt::Let { value, .. } | Stmt::Const { value, .. } => rename_in_expr(value, from, to),
        Stmt::Assign { name, value, .. } => {
            rename_in_expr(value, from, to);
            if name == from {
                *name = to.to_string();
            }
        }
        Stmt::AssignIndex { object, index, value, .. } => {
            rename_in_expr(index, from, to);
            rename_in_expr(value, from, to);
            if object == from {
                *object = to.to_string();
            }
        }
        Stmt::AssignMember { object, value, .. } => {
            rename_in_expr(object, from, to);
            rename_in_expr(value, from, to);
        }
        Stmt::Return(Some(e)) | Stmt::Throw(e) | Stmt::Expr(e) => rename_in_expr(e, from, to),
        Stmt::Return(None) | Stmt::Break | Stmt::Continue => {}
        Stmt::If { condition, then_branch, else_branch } => {
            rename_in_expr(condition, from, to);
            rename_in_block(then_branch, from, to);
            if let Some(eb) = else_branch {
                rename_in_block(eb, from, to);
            }
        }
        Stmt::While { condition, body } => {
            rename_in_expr(condition, from, to);
            rename_in_block(body, from, to);
        }
        Stmt::For { init, condition, update, body } => {
            if let Some(i) = init {
                rename_in_stmt(i, from, to);
            }
            if let Some(c) = condition {
                rename_in_expr(c, from, to);
            }
            if let Some(u) = update {
                rename_in_stmt(u, from, to);
            }
            rename_in_block(body, from, to);
        }
        Stmt::ForOf { name, iterable, body, .. } => {
            rename_in_expr(iterable, from, to);
            // A `for-of` loop variable shares `from`'s name: it shadows it
            // for the loop body, so stop renaming there.
            if name != from {
                rename_in_block(body, from, to);
            }
        }
        Stmt::Switch { discriminant, cases } => {
            rename_in_expr(discriminant, from, to);
            for c in cases {
                for v in &mut c.values {
                    rename_in_expr(v, from, to);
                }
                rename_in_block(&mut c.body, from, to);
            }
        }
        Stmt::Try { body, catch_body, .. } => {
            rename_in_block(body, from, to);
            rename_in_block(catch_body, from, to);
        }
        Stmt::ExportDecl(inner) => rename_in_stmt(inner, from, to),
        // Function bodies are their own independent scope tree (functions
        // don't close over the enclosing block — see `shadowing.rs`'s doc
        // comment for the same rule) — a narrowed outer binding is never
        // visible inside one, so don't descend.
        Stmt::Function(_) | Stmt::ExportDefault(_) => {}
        _ => {}
    }
}

fn rename_in_expr(e: &mut Expr, from: &str, to: &str) {
    match e {
        Expr::Ident(name) => {
            if name == from {
                *name = to.to_string();
            }
        }
        Expr::Call { callee, args, .. } => {
            rename_in_expr(callee, from, to);
            for a in args {
                rename_in_expr(a, from, to);
            }
        }
        Expr::Unary { operand, .. } => rename_in_expr(operand, from, to),
        Expr::Binary { left, right, .. } => {
            rename_in_expr(left, from, to);
            rename_in_expr(right, from, to);
        }
        Expr::Member { object, .. } => rename_in_expr(object, from, to),
        Expr::Index { object, index } => {
            rename_in_expr(object, from, to);
            rename_in_expr(index, from, to);
        }
        Expr::ArrayLiteral { elements } => {
            for el in elements {
                match el {
                    arcis_ast::ArrayElement::Item(x) | arcis_ast::ArrayElement::Spread(x) => {
                        rename_in_expr(x, from, to)
                    }
                }
            }
        }
        Expr::ObjectLiteral { fields } => {
            for f in fields {
                match f {
                    arcis_ast::ObjectField::KV(_, v) | arcis_ast::ObjectField::Spread(v) => {
                        rename_in_expr(v, from, to)
                    }
                }
            }
        }
        Expr::TypeOf(inner) | Expr::AsConst(inner) | Expr::NonNullAssertion(inner) => {
            rename_in_expr(inner, from, to)
        }
        Expr::AsAssertion { expr, .. } => rename_in_expr(expr, from, to),
        Expr::Arrow { params, body, .. } => {
            // Shadowed by a same-named parameter: stop renaming inside.
            if params.iter().any(|p| p.name == from) {
                return;
            }
            match body {
                arcis_ast::ArrowBody::Expr(inner) => rename_in_expr(inner, from, to),
                arcis_ast::ArrowBody::Block(stmts) => rename_in_block(stmts, from, to),
            }
        }
        Expr::Number(_)
        | Expr::String(_)
        | Expr::Bool(_)
        | Expr::Path { .. }
        | Expr::Null
        | Expr::Undefined => {}
    }
}

/// Look up `name`'s currently-known type in a simple, flat, sequentially-
/// built scope (Arcis has no real block scoping — see the module docs on
/// [`crate::infer`] for the same simplification made there).
type Scope = std::collections::HashMap<String, Type>;

fn record_decl(stmt: &Stmt, scope: &mut Scope) {
    match stmt {
        Stmt::Let { name, ty, .. } | Stmt::Const { name, ty, .. } => {
            if let Some(t) = ty {
                scope.insert(name.clone(), t.clone());
            }
        }
        Stmt::ForOf { name, ty, .. } => {
            if let Some(t) = ty {
                scope.insert(name.clone(), t.clone());
            }
        }
        Stmt::ExportDecl(inner) => record_decl(inner, scope),
        _ => {}
    }
}

/// Walk one statement list, rewriting narrowing guards in place. `scope` is
/// pre-seeded with everything already known from enclosing blocks
/// (function parameters, outer `let`s) so narrowing also works on names
/// declared outside this particular block. Recurses into every nested
/// block (if/while/for/switch/try/function bodies).
fn narrow_block(stmts: &mut Vec<Stmt>, mut scope: Scope, counter: &mut u32) {
    let mut i = 0;
    while i < stmts.len() {
        // Recurse into this statement's own nested blocks first, using the
        // scope built from everything declared so far, including in this
        // block up to (not including) statement `i`.
        narrow_stmt_nested(&mut stmts[i], &scope, counter);

        if let Stmt::If { condition, then_branch, else_branch } = &mut stmts[i] {
            if let Some((name, is_ne)) = null_check_ident(condition) {
                if let Some(ty) = scope.get(&name) {
                    if ty.is_optional() {
                        let inner = ty.unwrap_optional().clone();
                        if is_ne {
                            let fresh = mint(counter, &name);
                            then_branch.insert(0, narrowed_let(&fresh, &name, inner));
                            rename_in_block(&mut then_branch[1..], &name, &fresh);
                        } else if let Some(eb) = else_branch {
                            let fresh = mint(counter, &name);
                            eb.insert(0, narrowed_let(&fresh, &name, inner));
                            rename_in_block(&mut eb[1..], &name, &fresh);
                        } else if definitely_exits(then_branch) {
                            // Guard clause: `if (x == null) { return; }` —
                            // narrow for the rest of THIS block, inserted
                            // right after the `if`.
                            let fresh = mint(counter, &name);
                            stmts.insert(i + 1, narrowed_let(&fresh, &name, inner));
                            rename_in_block(&mut stmts[i + 2..], &name, &fresh);
                        }
                    }
                }
            }
        }

        record_decl(&stmts[i], &mut scope);
        i += 1;
    }
}

/// Recurse into every `Vec<Stmt>` nested inside `stmt` (but not `stmt`
/// itself — the caller's `narrow_block` already owns that list).
fn narrow_stmt_nested(stmt: &mut Stmt, outer_scope: &Scope, counter: &mut u32) {
    match stmt {
        Stmt::If { then_branch, else_branch, .. } => {
            narrow_block(then_branch, outer_scope.clone(), counter);
            if let Some(eb) = else_branch {
                narrow_block(eb, outer_scope.clone(), counter);
            }
        }
        Stmt::While { body, .. } | Stmt::For { body, .. } | Stmt::ForOf { body, .. } => {
            narrow_block(body, outer_scope.clone(), counter);
        }
        Stmt::Switch { cases, .. } => {
            for c in cases {
                narrow_block(&mut c.body, outer_scope.clone(), counter);
            }
        }
        Stmt::Try { body, catch_body, .. } => {
            narrow_block(body, outer_scope.clone(), counter);
            narrow_block(catch_body, outer_scope.clone(), counter);
        }
        Stmt::Function(f) => narrow_function(f, counter),
        Stmt::ExportDecl(inner) => narrow_stmt_nested(inner, outer_scope, counter),
        Stmt::ExportDefault(ExportDefault::Function(f)) => narrow_function(f, counter),
        _ => {}
    }
}

fn narrow_function(f: &mut Function, counter: &mut u32) {
    let mut scope: Scope = Scope::new();
    for p in &f.params {
        scope.insert(p.name.clone(), p.ty.clone());
    }
    narrow_block(&mut f.body, scope, counter);
}
