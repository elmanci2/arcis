//! `break` / `continue` context check.
//!
//! Both keywords are only legal inside the body of a loop (`while`, `for`,
//! or `for-of`). This module walks every statement in a scope, tracking
//! how deeply we are nested inside loops, and emits a
//! [`BreakOutsideLoop`](super::ValidationIssue::BreakOutsideLoop) issue
//! whenever a `break` or `continue` appears at depth zero.
//!
//! Function declarations reset the depth counter: a `break` inside a
//! nested function is unrelated to any enclosing loop.

use arcis_ast::Stmt;

use super::ValidationIssue;

/// Walk `stmt` and emit an issue for every `break` / `continue` that is not
/// nested inside a loop. `depth` is the current loop-nesting level.
pub(crate) fn check_loop_context(stmt: &Stmt, depth: usize, issues: &mut Vec<ValidationIssue>) {
    let new_depth = match stmt {
        Stmt::While { .. } | Stmt::For { .. } | Stmt::ForOf { .. } => depth + 1,
        _ => depth,
    };
    match stmt {
        Stmt::Break => {
            if depth == 0 {
                issues.push(ValidationIssue::BreakOutsideLoop {
                    keyword: "break",
                    line: 0,
                    col: 0,
                });
            }
        }
        Stmt::Continue => {
            if depth == 0 {
                issues.push(ValidationIssue::BreakOutsideLoop {
                    keyword: "continue",
                    line: 0,
                    col: 0,
                });
            }
        }
        Stmt::While { body, .. } | Stmt::For { body, .. } | Stmt::ForOf { body, .. } => {
            for s in body {
                check_loop_context(s, new_depth, issues);
            }
        }
        Stmt::If { then_branch, else_branch, .. } => {
            for s in then_branch {
                check_loop_context(s, new_depth, issues);
            }
            if let Some(eb) = else_branch {
                for s in eb {
                    check_loop_context(s, new_depth, issues);
                }
            }
        }
        Stmt::Function(f) => {
            for s in &f.body {
                // A new function = a fresh loop scope.
                check_loop_context(s, 0, issues);
            }
        }
        _ => {}
    }
}