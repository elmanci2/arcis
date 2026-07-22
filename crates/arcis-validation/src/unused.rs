//! Unused-declaration detection.
//!
//! Tracks every identifier that appears as the **target** of a use site —
//! any `Ident`, the LHS of an `Assign`, the receiver of an `AssignIndex`,
//! etc. — and compares the result against the declared set. Anything in
//! the declared set that has no use site is reported as `Unused`.
//!
//! The flow is:
//! 1. [`collect_uses`] walks the tree and fills a `HashSet<String>` of used
//!    names.
//! 2. [`push_unused`] compares the declared list with the used set and
//!    emits an `Unused` issue for every name that was never used.
//!
//! The *declared* list itself is populated by [`crate::duplicate::collect_decl`],
//! which incidentally also catches duplicate declarations as a side effect.

use std::collections::HashSet;

use arcis_ast::{Expr, Stmt};

use super::DeclKind;

/// A declaration that was never used inside its scope.
#[derive(Debug, Clone)]
pub struct UnusedDecl {
    pub kind: DeclKind,
    pub name: String,
    pub line: usize,
    pub col: usize,
}

/// Walk `stmt` and add every identifier that appears in a "use" position to
/// `used`. See the module doc for the definition of "use position".
pub(crate) fn collect_uses(stmt: &Stmt, used: &mut HashSet<String>) {
    match stmt {
        Stmt::Let { value, .. } | Stmt::Const { value, .. } => {
            collect_uses_expr(value, used);
        }
        Stmt::Assign { name, value } => {
            used.insert(name.clone());
            collect_uses_expr(value, used);
        }
        Stmt::AssignIndex { object, index, value } => {
            used.insert(object.clone());
            collect_uses_expr(index, used);
            collect_uses_expr(value, used);
        }
        Stmt::AssignMember { object, value, .. } => {
            collect_uses_expr(object, used);
            collect_uses_expr(value, used);
        }
        Stmt::Function(f) => {
            for s in &f.body {
                collect_uses(s, used);
            }
        }
        Stmt::Return(Some(expr)) => collect_uses_expr(expr, used),
        Stmt::Return(None) => {}
        Stmt::If { condition, then_branch, else_branch } => {
            collect_uses_expr(condition, used);
            for s in then_branch {
                collect_uses(s, used);
            }
            if let Some(eb) = else_branch {
                for s in eb {
                    collect_uses(s, used);
                }
            }
        }
        Stmt::While { condition, body } => {
            collect_uses_expr(condition, used);
            for s in body {
                collect_uses(s, used);
            }
        }
        Stmt::For { init, condition, update, body } => {
            if let Some(init) = init {
                collect_uses(init, used);
            }
            if let Some(cond) = condition {
                collect_uses_expr(cond, used);
            }
            if let Some(upd) = update {
                collect_uses(upd, used);
            }
            for s in body {
                collect_uses(s, used);
            }
        }
        Stmt::ForOf { iterable, body, .. } => {
            collect_uses_expr(iterable, used);
            for s in body {
                collect_uses(s, used);
            }
        }
        Stmt::Break | Stmt::Continue => {}
        Stmt::Import { .. } | Stmt::FromImport { .. } | Stmt::ExportDecl(_) | Stmt::ExportSpec(_) | Stmt::ExportDefault(_) => {}
        Stmt::Expr(expr) => collect_uses_expr(expr, used),
    }
}

fn collect_uses_expr(expr: &Expr, used: &mut HashSet<String>) {
    match expr {
        Expr::Ident(name) => {
            used.insert(name.clone());
        }
        Expr::Call { callee, args } => {
            collect_uses_expr(callee, used);
            for a in args {
                collect_uses_expr(a, used);
            }
        }
        Expr::Binary { left, right, .. } => {
            collect_uses_expr(left, used);
            collect_uses_expr(right, used);
        }
        Expr::Unary { operand, .. } => {
            collect_uses_expr(operand, used);
        }
        Expr::Member { object, .. } => {
            collect_uses_expr(object, used);
        }
        Expr::Index { object, index } => {
            collect_uses_expr(object, used);
            collect_uses_expr(index, used);
        }
        Expr::ArrayLiteral { elements } => {
            for e in elements {
                collect_uses_expr(e, used);
            }
        }
        Expr::ObjectLiteral { fields } => {
            for (_, v) in fields {
                collect_uses_expr(v, used);
            }
        }
        Expr::Path { segments } => {
            // Mark each segment as used: lets us detect unused imports later
            // when the path only references names already pulled in via `crate:`.
            for s in segments {
                used.insert(s.clone());
            }
        }
        Expr::TypeOf(inner) => {
            collect_uses_expr(inner, used);
        }
        Expr::Number(_) | Expr::String(_) | Expr::Bool(_) => {}
    }
}

/// Emit one `Unused` issue for every declared name that does not appear in
/// the `used` set.
pub(crate) fn push_unused(
    declared: &[UnusedDecl],
    used: &HashSet<String>,
    issues: &mut Vec<super::ValidationIssue>,
) {
    for d in declared {
        if !used.contains(&d.name) {
            issues.push(super::ValidationIssue::Unused(d.clone()));
        }
    }
}