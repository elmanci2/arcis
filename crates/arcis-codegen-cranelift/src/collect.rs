//! AST pre-passes shared across the lowering.
//!
//! [`collect_reassigned`] walks every `program.stmts` and flags which `let`
//! bindings are later mutated via assignment (`x = ...`), indexed assignment
//! (`arr[i] = ...`), field assignment (`obj.x = ...`), or mutating method
//! calls (`arr.push(...)`, `arr.pop()`, `arr.unshift(...)`).  The Cranelift
//! backend uses this set to mark variables mutable in the SSA lowering so
//! that `use_var` can insert phi nodes correctly across block merges.

use std::collections::HashSet;

use arcis_ast::{Expr, Stmt};

/// Return the set of `let` / `const` names that are ever reassigned
/// after their declaration.
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
        Stmt::AssignIndex { object, index, value } => {
            // `arr[i] = val` mutates `arr`.
            out.insert(object.clone());
            visit_expr(index, out);
            visit_expr(value, out);
        }
        Stmt::AssignMember { object, property: _, value } => {
            // `obj.field = val` — if object is an ident, mutate it.
            mark_ident_mutated(object, out);
            visit_expr(value, out);
        }
        Stmt::If { then_branch, else_branch, .. } => {
            for s in then_branch { visit_stmt(s, out); }
            if let Some(else_stmts) = else_branch {
                for s in else_stmts { visit_stmt(s, out); }
            }
        }
        Stmt::While { body, .. } | Stmt::For { body, .. } => {
            for s in body { visit_stmt(s, out); }
        }
        Stmt::ForOf { name: _, ty: _, iterable, body } => {
            // The for-of loop variable is SSA-redefined in each iteration;
            // mark the iterable as read but no mutation from `for-of` itself.
            visit_expr(iterable, out);
            for s in body { visit_stmt(s, out); }
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
            // Detect mutating method calls: arr.push(x) / arr.pop() / arr.unshift(x)
            if let Expr::Member { object, property } = callee.as_ref() {
                match property.as_str() {
                    "push" | "pop" | "unshift" => {
                        mark_ident_mutated(object, out);
                    }
                    _ => {}
                }
            }
            visit_expr(callee, out);
            for a in args { visit_expr(a, out); }
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
            for x in elements { visit_expr(x, out); }
        }
        Expr::ObjectLiteral { fields } => {
            for (_, v) in fields { visit_expr(v, out); }
        }
        _ => {}
    }
}

/// If `e` is an `Ident`, mark it as reassigned.
fn mark_ident_mutated(e: &Expr, out: &mut HashSet<String>) {
    if let Expr::Ident(name) = e {
        out.insert(name.clone());
    }
    // For Phase 2 we don't recurse through nested member/index —
    // the outermost object is the one being mutated.
}
