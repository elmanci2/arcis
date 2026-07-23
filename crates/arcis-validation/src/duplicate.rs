//! Duplicate-declaration detection.
//!
//! [`collect_decl`] walks the tree and accumulates declarations into the
//! `out` list (a flat, whole-function record used by [`super::unused`] to
//! check "declared but never used" — that check doesn't care about scope
//! nesting, only whether a name appears anywhere).
//!
//! Duplicate detection itself, however, IS scope-aware: `scopes` is a stack
//! of per-block declaration maps. Entering an `if`/`while`/`for`/`switch`
//! case/`try` body pushes a fresh scope; only a second declaration within
//! the SAME (innermost) scope is a real duplicate. Two sibling blocks that
//! happen to redeclare the same name (`if (a) { let x = 1; } if (b) { let
//! x = 2; }`) are not conflated — before this scope stack existed, they
//! were, which produced spurious `Duplicate` errors for perfectly valid
//! code (this crate's own tests originally only exercised same-block
//! duplicates, so the bug went unnoticed until `try`/`switch` bodies made
//! sibling-scope collisions common).
//!
//! Note: legitimate *shadowing* (an inner declaration reusing an ancestor
//! scope's name) never reaches this scope stack at all — [`super::shadowing`]
//! alpha-renames it away in an earlier pass, so by the time this runs, two
//! same-named declarations only remain if they're either a genuine
//! same-scope duplicate or two unrelated sibling declarations, both of
//! which this module now handles correctly.

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

/// One lexical scope for duplicate-detection purposes: name -> the
/// declaration that first claimed it in this scope.
type Scope = HashMap<String, UnusedDecl>;

/// Walk `stmt` and record every declaration into `out` (flat, whole-function
/// — see module docs). `scopes` is the current duplicate-detection scope
/// stack; the caller pushes the outermost frame (`main`'s or a function's).
pub(crate) fn collect_decl(
    stmt: &Stmt,
    out: &mut Vec<UnusedDecl>,
    scopes: &mut Vec<Scope>,
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
            let current = scopes.last_mut().expect("at least one scope");
            if let Some(prev) = current.get(name) {
                issues.push(ValidationIssue::Duplicate(DuplicateDecl {
                    kind: DeclKind::Variable,
                    name: name.clone(),
                    first_line: prev.line,
                    first_col: prev.col,
                    second_line: *line,
                    second_col: *col,
                }));
            } else {
                current.insert(name.clone(), decl.clone());
                out.push(decl);
            }
        }
        Stmt::If { then_branch, else_branch, .. } => {
            collect_decl_block(then_branch, out, scopes, issues);
            if let Some(eb) = else_branch {
                collect_decl_block(eb, out, scopes, issues);
            }
        }
        Stmt::While { body, .. } | Stmt::For { body, .. } => {
            collect_decl_block(body, out, scopes, issues);
        }
        Stmt::Switch { cases, .. } => {
            for case in cases {
                collect_decl_block(&case.body, out, scopes, issues);
            }
        }
        Stmt::Try { body, catch_body, .. } => {
            collect_decl_block(body, out, scopes, issues);
            collect_decl_block(catch_body, out, scopes, issues);
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
        | Stmt::FromImport { .. }
        | Stmt::ExportDecl(_)
        | Stmt::ExportSpec(_)
        | Stmt::ExportDefault(_)
        | Stmt::TypeAlias { .. }
        | Stmt::Interface { .. }
        | Stmt::Enum { .. }
        | Stmt::Throw(_)
        | Stmt::Expr(_) => {}
    }
}

/// Push a fresh child scope, walk `stmts` within it, then pop — so sibling
/// blocks never see each other's declarations.
fn collect_decl_block(
    stmts: &[Stmt],
    out: &mut Vec<UnusedDecl>,
    scopes: &mut Vec<Scope>,
    issues: &mut Vec<ValidationIssue>,
) {
    scopes.push(Scope::new());
    for s in stmts {
        collect_decl(s, out, scopes, issues);
    }
    scopes.pop();
}
