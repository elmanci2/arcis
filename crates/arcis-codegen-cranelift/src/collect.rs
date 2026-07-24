//! AST pre-passes shared across the lowering.
//!
//! [`collect_reassigned`] walks every `program.stmts` and flags which `let`
//! bindings are later mutated via assignment (`x = ...`), indexed assignment
//! (`arr[i] = ...`), field assignment (`obj.x = ...`), or mutating method
//! calls (`arr.push(...)`, `arr.pop()`, `arr.unshift(...)`).  The Cranelift
//! backend uses this set to mark variables mutable in the SSA lowering so
//! that `use_var` can insert phi nodes correctly across block merges.

use std::collections::{HashMap, HashSet};

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
        Stmt::AssignIndex { object, index, value, .. } => {
            // `arr[i] = val` mutates `arr`.
            out.insert(object.clone());
            visit_expr(index, out);
            visit_expr(value, out);
        }
        Stmt::AssignMember { object, property: _, value, .. } => {
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
        Expr::Call { callee, args, .. } => {
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
            for x in elements {
                match x {
                    arcis_ast::ArrayElement::Item(e) | arcis_ast::ArrayElement::Spread(e) => visit_expr(e, out),
                }
            }
        }
        Expr::ObjectLiteral { fields } => {
            for f in fields {
                match f {
                    arcis_ast::ObjectField::KV(_, v) | arcis_ast::ObjectField::Spread(v) => visit_expr(v, out),
                }
            }
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

/// Collect every `enum` declaration in the module: enum name -> (variant
/// name -> resolved numeric value). Values follow TS numeric-enum rules —
/// an explicit `= N` sets the value, otherwise it's the previous variant's
/// value + 1 (or 0 for the first variant). Enums have no runtime
/// representation of their own in Cranelift: `Color.Red` just resolves to
/// an `f64const` at the value looked up here (see `expr.rs`'s `Expr::Member`
/// handling).
pub(crate) fn collect_enums(program: &arcis_ast::Program) -> HashMap<String, HashMap<String, f64>> {
    let mut out = HashMap::new();
    for stmt in &program.stmts {
        collect_enum_decl(stmt, &mut out);
    }
    out
}

fn collect_enum_decl(stmt: &Stmt, out: &mut HashMap<String, HashMap<String, f64>>) {
    match stmt {
        Stmt::Enum { name, variants, .. } => {
            let mut table = HashMap::new();
            let mut next = 0i64;
            for (variant, explicit) in variants {
                let value = explicit.unwrap_or(next);
                table.insert(variant.clone(), value as f64);
                next = value + 1;
            }
            out.insert(name.clone(), table);
        }
        Stmt::ExportDecl(inner) => collect_enum_decl(inner, out),
        _ => {}
    }
}
