//! AST pre-passes shared across the lowering.
//!
//! Today only [`collect_reassigned`] matters: it walks every `program.stmts`
//! and flags which `let` bindings are later mutated. The Cranelift backend,
//! like the Rust backend, uses this set to decide whether to mark a variable
//! `mut` (i.e. it can be re-`def_var`'d at runtime — Cranelift's `Variable`
//! is SSA so re-definition is valid regardless, but knowing which names get
//! a fresh SSA edge helps the branch quality and avoids surprises).

use std::collections::HashSet;

use arcis_ast::{Expr, Stmt};

/// Return the set of `let` / `const` names that are ever reassigned
/// (`x = ...`, `arr[i] = ...`, `obj.field = ...`) after their declaration.
pub(crate) fn collect_reassigned(program: &arcis_ast::Program) -> HashSet<String> {
    let mut out = HashSet::new();
    for stmt in &program.stmts {
        visit_stmt(stmt, &mut out);
    }
    out
}

fn visit_stmt(stmt: &Stmt, out: &mut HashSet<String>) {
    match stmt {
        Stmt::Let { .. } | Stmt::Const { .. } | Stmt::Function(_) | Stmt::Break | Stmt::Continue => {}
        Stmt::Assign { name, .. } => {
            out.insert(name.clone());
        }
        Stmt::If {
            then_branch,
            else_branch,
            ..
        } => {
            for s in then_branch {
                visit_stmt(s, out);
            }
            if let Some(else_stmts) = else_branch {
                for s in else_stmts {
                    visit_stmt(s, out);
                }
            }
        }
        Stmt::While { body, .. } | Stmt::For { body, .. } | Stmt::ForOf { body, .. } => {
            for s in body {
                visit_stmt(s, out);
            }
        }
        Stmt::ExportDecl(inner) => visit_stmt(inner, out),
        Stmt::Return(Some(e)) => visit_expr(e, out),
        Stmt::Expr(e) => visit_expr(e, out),
        _ => {}
    }
}

fn visit_expr(e: &Expr, out: &mut HashSet<String>) {
    match e {
        Expr::Call { callee, args } => {
            visit_expr(callee, out);
            for a in args {
                visit_expr(a, out);
            }
        }
        Expr::Binary { left, right, .. } => {
            visit_expr(left, out);
            visit_expr(right, out);
        }
        Expr::Unary { operand, .. } => visit_expr(operand, out),
        Expr::Member { object, .. } => visit_expr(object, out),
        Expr::Index { object, index } => {
            visit_expr(object, out);
            visit_expr(index, out);
        }
        Expr::ArrayLiteral { elements } => {
            for x in elements {
                visit_expr(x, out);
            }
        }
        Expr::ObjectLiteral { fields } => {
            for (_, v) in fields {
                visit_expr(v, out);
            }
        }
        _ => {}
    }
}
