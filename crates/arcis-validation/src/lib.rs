//! Arcis semantic validation.
//!
//! Detects two classes of errors:
//! - Declarations (variables, parameters) that are never used
//! - Duplicate declarations inside the same scope
//! - `break` / `continue` outside of any loop
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
//! In phase 2 this will be split into `unused.rs`, `duplicate.rs`,
//! `loop_ctx.rs`, and `format.rs`.

use arcis_ast::{Expr, Program, Stmt};
use std::collections::{HashMap, HashSet};

/// Category of a declaration we may flag as unused or duplicate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclKind {
    Variable,
    Parameter,
}

impl DeclKind {
    fn label(&self) -> &'static str {
        match self {
            DeclKind::Variable => "variable",
            DeclKind::Parameter => "parameter",
        }
    }
}

#[derive(Debug, Clone)]
pub struct UnusedDecl {
    pub kind: DeclKind,
    pub name: String,
    pub line: usize,
    pub col: usize,
}

#[derive(Debug, Clone)]
pub struct DuplicateDecl {
    pub kind: DeclKind,
    pub name: String,
    pub first_line: usize,
    pub first_col: usize,
    pub second_line: usize,
    pub second_col: usize,
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
    let mut issues = Vec::new();

    // Scope 1: main
    let mut main_decl: Vec<UnusedDecl> = Vec::new();
    let mut main_used: HashSet<String> = HashSet::new();
    let mut main_seen: HashMap<String, UnusedDecl> = HashMap::new();
    for stmt in &program.stmts {
        if matches!(stmt, Stmt::Function(_)) {
            continue;
        }
        collect_decl(stmt, &mut main_decl, &mut main_seen, &mut issues);
        collect_uses(stmt, &mut main_used);
        check_loop_context(stmt, 0, &mut issues);
    }
    push_unused(&main_decl, &main_used, &mut issues);

    // Scope N: each function (parameters + body)
    for stmt in &program.stmts {
        if let Stmt::Function(f) = stmt {
            let mut func_decl: Vec<UnusedDecl> = Vec::new();
            let mut func_used: HashSet<String> = HashSet::new();
            let mut func_seen: HashMap<String, UnusedDecl> = HashMap::new();
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
                collect_decl(s, &mut func_decl, &mut func_seen, &mut issues);
                collect_uses(s, &mut func_used);
                check_loop_context(s, 0, &mut issues);
            }
            push_unused(&func_decl, &func_used, &mut issues);
        }
    }

    issues
}

/// Verify that `break` / `continue` only appear inside a loop. `depth`
/// counts the loops we are currently nested inside.
fn check_loop_context(stmt: &Stmt, depth: usize, issues: &mut Vec<ValidationIssue>) {
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
                check_loop_context(s, 0, issues); // a new function = a fresh loop scope
            }
        }
        _ => {}
    }
}

fn push_unused(
    declared: &[UnusedDecl],
    used: &HashSet<String>,
    issues: &mut Vec<ValidationIssue>,
) {
    for d in declared {
        if !used.contains(&d.name) {
            issues.push(ValidationIssue::Unused(d.clone()));
        }
    }
}

fn collect_decl(
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
        // Functions are handled separately in [`validate`].
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

fn collect_uses(stmt: &Stmt, used: &mut HashSet<String>) {
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
        Stmt::Import { .. } | Stmt::ExportDecl(_) | Stmt::ExportSpec(_) | Stmt::ExportDefault(_) => {}
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
        Expr::Number(_) | Expr::String(_) | Expr::Bool(_) => {}
    }
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
}