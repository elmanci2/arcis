//! Arcis semantic validation.
//!
//! Detects three classes of issues:
//! - Declarations (variables, parameters) that are never used
//! - Duplicate declarations inside the same scope
//! - `break` / `continue` outside of any enclosing loop
//!
//! Analysis is **per scope**: `main` is one scope, each function is another.
//! A declaration is reported as `unused` if within ITS scope there is no
//! `Ident`, `Assign` or `Call` that references it.
//!
//! Current limitation: `if`/`while` blocks are NOT tracked as separate
//! scopes. This means:
//!   - False negatives: a `main` variable can be "rescued" by an
//!     identically-named identifier used inside a function with the same name.
//!   - False positives: a variable declared inside an `if` block may be
//!     flagged as unused because it is not used within that same `if`.
//!
//! For the current subset (no shadowing, no closures) the unused / duplicate
//! analyses are correct in practical cases.
//!
//! ## Layout
//!
//! - [`unused`](self::unused) — `UnusedDecl`, use-set collection, and the
//!   unused-declaration check.
//! - [`duplicate`](self::duplicate) — `DuplicateDecl` and the declaration
//!   collector that emits duplicates as a side effect.
//! - [`loop_ctx`](self::loop_ctx) — `break` / `continue` context check.

use arcis_ast::Program;

mod duplicate;
mod loop_ctx;
mod shadowing;
mod unused;

pub use duplicate::DuplicateDecl;
pub use shadowing::resolve_shadowing;
pub use unused::UnusedDecl;

/// Category of a declaration we may flag as unused or duplicate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclKind {
    Variable,
    Parameter,
}

impl DeclKind {
    pub(crate) fn label(&self) -> &'static str {
        match self {
            DeclKind::Variable => "variable",
            DeclKind::Parameter => "parameter",
        }
    }
}

/// Validation issue: an unused declaration, a duplicate declaration, or a
/// `break` / `continue` outside of any enclosing loop.
#[derive(Debug, Clone)]
pub enum ValidationIssue {
    Unused(UnusedDecl),
    Duplicate(DuplicateDecl),
    BreakOutsideLoop {
        keyword: &'static str,
        line: usize,
        col: usize,
    },
}

/// Validate an entire program. The returned list is empty when there are no
/// issues.
pub fn validate(program: &Program) -> Vec<ValidationIssue> {
    use std::collections::{HashMap, HashSet};

    let mut issues = Vec::new();

    // ── Scope 1: main ────────────────────────────────────────────────────
    let mut main_decl: Vec<UnusedDecl> = Vec::new();
    let mut main_used: HashSet<String> = HashSet::new();
    let mut main_scopes: Vec<HashMap<String, UnusedDecl>> = vec![HashMap::new()];
    for stmt in &program.stmts {
        if matches!(stmt, arcis_ast::Stmt::Function(_)) {
            continue;
        }
        duplicate::collect_decl(stmt, &mut main_decl, &mut main_scopes, &mut issues);
        unused::collect_uses(stmt, &mut main_used);
        loop_ctx::check_loop_context(stmt, 0, &mut issues);
    }
    unused::push_unused(&main_decl, &main_used, &mut issues);

    // ── Scope N: each function (parameters + body) ───────────────────────
    for stmt in &program.stmts {
        if let arcis_ast::Stmt::Function(f) = stmt {
            let mut func_decl: Vec<UnusedDecl> = Vec::new();
            let mut func_used: HashSet<String> = HashSet::new();
            let mut func_scopes: Vec<HashMap<String, UnusedDecl>> = vec![HashMap::new()];
            let func_seen = func_scopes.last_mut().expect("at least one scope");
            for p in &f.params {
                let decl = UnusedDecl {
                    kind: DeclKind::Parameter,
                    name: p.name.clone(),
                    line: p.line,
                    col: p.col,
                };
                if let Some(prev) = func_seen.get(&p.name) {
                    issues.push(ValidationIssue::Duplicate(DuplicateDecl {
                        kind: DeclKind::Parameter,
                        name: p.name.clone(),
                        first_line: prev.line,
                        first_col: prev.col,
                        second_line: p.line,
                        second_col: p.col,
                    }));
                } else {
                    func_seen.insert(p.name.clone(), decl.clone());
                    func_decl.push(decl);
                }
            }
            for s in &f.body {
                duplicate::collect_decl(s, &mut func_decl, &mut func_scopes, &mut issues);
                unused::collect_uses(s, &mut func_used);
                loop_ctx::check_loop_context(s, 0, &mut issues);
            }
            unused::push_unused(&func_decl, &func_used, &mut issues);
        }
    }

    issues
}

/// Format validation issues for display to the user.
pub fn format_issues(issues: &[ValidationIssue], source_path: &str) -> String {
    let mut out = String::new();
    for issue in issues {
        match issue {
            ValidationIssue::Unused(u) => {
                out.push_str(&format!(
                    "error: {} `{}` declared but never used\n --> {}:{}:{}\n",
                    u.kind.label(),
                    u.name,
                    source_path,
                    u.line,
                    u.col,
                ));
            }
            ValidationIssue::Duplicate(d) => {
                out.push_str(&format!(
                    "error: {} `{}` already declared at {}:{}:{}\n --> {}:{}:{}\n",
                    d.kind.label(),
                    d.name,
                    source_path,
                    d.first_line,
                    d.first_col,
                    source_path,
                    d.second_line,
                    d.second_col,
                ));
            }
            ValidationIssue::BreakOutsideLoop { keyword, .. } => {
                out.push_str(&format!(
                    "error: `{}` used outside of any loop\n",
                    keyword,
                ));
            }
        }
    }
    if !issues.is_empty() {
        let unused = issues
            .iter()
            .filter(|i| matches!(i, ValidationIssue::Unused(_)))
            .count();
        let dup = issues
            .iter()
            .filter(|i| matches!(i, ValidationIssue::Duplicate(_)))
            .count();
        let outside = issues
            .iter()
            .filter(|i| matches!(i, ValidationIssue::BreakOutsideLoop { .. }))
            .count();
        out.push('\n');
        if unused > 0 {
            out.push_str(&format!("{} declaration(s) unused.\n", unused));
        }
        if dup > 0 {
            out.push_str(&format!("{} duplicate declaration(s).\n", dup));
        }
        if outside > 0 {
            out.push_str(&format!(
                "{} use(s) of break/continue outside of a loop.\n",
                outside
            ));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use arcis_ast::Program;

    #[test]
    fn empty_program_has_no_issues() {
        let prog = Program { stmts: vec![] };
        assert!(validate(&prog).is_empty());
    }

    #[test]
    fn flags_unused_variable() {
        use arcis_ast::{Expr, Stmt};
        let prog = Program {
            stmts: vec![Stmt::Let {
                name: "x".into(),
                ty: None,
                value: Expr::Number(42.0),
                line: 1,
                col: 5,
            }],
        };
        let issues = validate(&prog);
        assert!(matches!(issues.as_slice(), [ValidationIssue::Unused(_)]));
    }

    #[test]
    fn flags_duplicate_declaration() {
        use arcis_ast::{Expr, Stmt};
        // Two `let x = ...` in the same scope → at least one issue must be a
        // Duplicate. (We also get two Unused issues because `x` is never
        // referenced, but that's fine — we just want at least one Duplicate.)
        let prog = Program {
            stmts: vec![
                Stmt::Let {
                    name: "x".into(),
                    ty: None,
                    value: Expr::Number(1.0),
                    line: 1,
                    col: 5,
                },
                Stmt::Let {
                    name: "x".into(),
                    ty: None,
                    value: Expr::Number(2.0),
                    line: 2,
                    col: 5,
                },
            ],
        };
        let issues = validate(&prog);
        assert!(
            issues
                .iter()
                .any(|i| matches!(i, ValidationIssue::Duplicate(_))),
            "expected at least one Duplicate issue, got: {issues:?}"
        );
    }

    #[test]
    fn flags_break_outside_loop() {
        use arcis_ast::Stmt;
        let prog = Program {
            stmts: vec![Stmt::Break],
        };
        let issues = validate(&prog);
        assert!(matches!(
            issues.as_slice(),
            [ValidationIssue::BreakOutsideLoop { .. }]
        ));
    }
}