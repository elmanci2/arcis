//! Duplicate-declaration detection.
//!
//! [`collect_decl`] walks the tree and accumulates declarations into the
//! `out` list, keeping a `seen` map of names already declared in the current
//! scope. If the same name is declared twice in the same scope, the second
//! occurrence produces a [`DuplicateDecl`] issue.
//!
//! The function also populates [`UnusedDecl`](super::UnusedDecl) entries —
//! the unused-detection pass needs that exact list to know what was declared.

use std::collections::HashMap;

use arcis_ast::Stmt;

use super::{DeclKind, UnusedDecl, ValidationIssue};

/// Two declarations with the same name inside the same scope.
#[derive(Debug, Clone)]
pub struct DuplicateDecl {
    pub kind: DeclKind,
    pub name: String,
    pub first_line: usize,
    pub first_col: usize,
    pub second_line: usize,
    pub second_col: usize,
}

/// Walk `stmt` and record every declaration into `out`. For any name that
/// was already declared in the current scope, push a `Duplicate` issue
/// instead of inserting a second entry.
pub(crate) fn collect_decl(
    stmt: &Stmt,
    out: &mut Vec<UnusedDecl>,
    seen: &mut HashMap<String, UnusedDecl>,
    issues: &mut Vec<ValidationIssue>,
) {
    match stmt {
        Stmt::Let { name, line, col, .. } | Stmt::Const { name, line, col, .. } => {
            let decl = UnusedDecl {
                kind: DeclKind::Variable,
                name: name.clone(),
                line: *line,
                col: *col,
            };
            if let Some(prev) = seen.get(name) {
                issues.push(ValidationIssue::Duplicate(DuplicateDecl {
                    kind: DeclKind::Variable,
                    name: name.clone(),
                    first_line: prev.line,
                    first_col: prev.col,
                    second_line: *line,
                    second_col: *col,
                }));
            } else {
                seen.insert(name.clone(), decl.clone());
                out.push(decl);
            }
        }
        Stmt::If { then_branch, else_branch, .. } => {
            // `if`/`else` blocks share the scope with `main` in this simple pass.
            for s in then_branch {
                collect_decl(s, out, seen, issues);
            }
            if let Some(eb) = else_branch {
                for s in eb {
                    collect_decl(s, out, seen, issues);
                }
            }
        }
        Stmt::While { body, .. } | Stmt::For { body, .. } => {
            // The body of `while` / `for` shares the `main` scope here too.
            for s in body {
                collect_decl(s, out, seen, issues);
            }
        }
        // Functions are handled separately in `validate`.
        // Assignments, returns, expressions declare nothing.
        Stmt::Function(_)
        | Stmt::Assign { .. }
        | Stmt::AssignIndex { .. }
        | Stmt::AssignMember { .. }
        | Stmt::Return(_)
        | Stmt::Break
        | Stmt::Continue
        | Stmt::ForOf { .. }
        | Stmt::Import { .. }
        | Stmt::ExportDecl(_)
        | Stmt::ExportSpec(_)
        | Stmt::ExportDefault(_)
        | Stmt::Expr(_) => {}
    }
}