//! Pre-passes over the AST.
//!
//! Three walks happen before emission:
//!
//! 1. [`collect_reassigned`] — every name that is the LHS of an `=` (or
//!    a method like `pop`) so that we can emit `let mut` only for those.
//! 2. [`collect_types`] — a `name → declared type` map (used to disambiguate
//!    `.length` on strings vs arrays).
//! 3. [`collect_all_object_types`] / [`collect_object_type_names`] — gather
//!    inline object types so we can declare one struct per shape and emit
//!    `use crate::__Obj...;` from non-root modules.

use std::collections::{HashMap, HashSet};

use arcis_ast::{ExportDefault, Expr, Program, Stmt, Type};

use arcis_linker::Module;

// ── Reassigned variables ──────────────────────────────────────────────────

/// Walk the program and return the set of names that are reassigned at least
/// once (in `main` or inside any function).
pub(crate) fn collect_reassigned(program: &Program) -> HashSet<String> {
    let mut set = HashSet::new();
    for stmt in &program.stmts {
        collect_in_stmt(stmt, &mut set);
    }
    set
}

fn collect_in_stmt(stmt: &Stmt, set: &mut HashSet<String>) {
    match stmt {
        Stmt::Let { .. } | Stmt::Const { .. } => {}
        Stmt::Assign { name, .. } | Stmt::AssignIndex { object: name, .. } => {
            set.insert(name.clone());
        }
        Stmt::AssignMember { object, .. } => {
            // Mark the root identifier (descending through Index/Member) as
            // reassigned. For `arr[i].x = v` we also mark `arr`.
            mark_ident_root_mutated(object, set);
        }
        Stmt::Function(f) => {
            for s in &f.body {
                collect_in_stmt(s, set);
            }
        }
        Stmt::Return(_) => {}
        Stmt::Break | Stmt::Continue => {}
        Stmt::If { then_branch, else_branch, .. } => {
            for s in then_branch {
                collect_in_stmt(s, set);
            }
            if let Some(eb) = else_branch {
                for s in eb {
                    collect_in_stmt(s, set);
                }
            }
        }
        Stmt::While { body, .. } => {
            for s in body {
                collect_in_stmt(s, set);
            }
        }
        Stmt::For { init, update, body, .. } => {
            if let Some(init) = init {
                collect_in_stmt(init, set);
            }
            if let Some(update) = update {
                collect_in_stmt(update, set);
            }
            for s in body {
                collect_in_stmt(s, set);
            }
        }
        Stmt::ForOf { iterable, body, .. } => {
            collect_mutation_in_expr(iterable, set);
            for s in body {
                collect_in_stmt(s, set);
            }
        }
        Stmt::Import { .. } | Stmt::FromImport { .. } | Stmt::ExportSpec(_) => {}
        Stmt::ExportDecl(inner) => collect_in_stmt(inner, set),
        Stmt::ExportDefault(ed) => match ed {
            ExportDefault::Function(f) => {
                for s in &f.body {
                    collect_in_stmt(s, set);
                }
            }
            ExportDefault::Expr(e) => collect_mutation_in_expr(e, set),
        },
        Stmt::Expr(expr) => collect_mutation_in_expr(expr, set),
    }
}

/// Detect calls to mutating methods (`pop`, `unshift`, `push`) on an
/// identifier and flag the receiver as `let mut`. Pure methods like
/// `find`, `filter`, `map`, `reduce` do NOT mutate and are not flagged.
fn collect_mutation_in_expr(expr: &Expr, set: &mut HashSet<String>) {
    if let Expr::Call { callee, args } = expr {
        if let Expr::Member { object, property } = callee.as_ref() {
            if matches!(property.as_str(), "pop" | "unshift" | "push") {
                if let Expr::Ident(name) = object.as_ref() {
                    set.insert(name.clone());
                }
            }
        }
        for a in args {
            collect_mutation_in_expr(a, set);
        }
    }
}

/// Descend through Index/Member until we find the outermost identifier of
/// an assignment expression and mark it as reassigned.
fn mark_ident_root_mutated(expr: &Expr, set: &mut HashSet<String>) {
    match expr {
        Expr::Ident(name) => {
            set.insert(name.clone());
        }
        Expr::Index { object, .. } | Expr::Member { object, .. } => {
            mark_ident_root_mutated(object, set);
        }
        _ => {}
    }
}

// ── Type map ──────────────────────────────────────────────────────────────

/// Build a name → declared type map. Used for `.length` to distinguish
/// between `string` and an array when the object is an identifier.
pub(crate) fn collect_types(program: &Program) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for stmt in &program.stmts {
        collect_types_stmt(stmt, &mut map);
    }
    map
}

fn collect_types_stmt(stmt: &Stmt, map: &mut HashMap<String, String>) {
    fn type_name_with_array(t: &Type) -> String {
        if t.is_array {
            format!("{}[]", t.name)
        } else {
            t.name.clone()
        }
    }
    match stmt {
        Stmt::Let { name, ty, .. } | Stmt::Const { name, ty, .. } => {
            if let Some(t) = ty {
                map.insert(name.clone(), type_name_with_array(t));
            }
        }
        Stmt::Function(f) => {
            for p in &f.params {
                map.insert(p.name.clone(), type_name_with_array(&p.ty));
            }
            for s in &f.body {
                collect_types_stmt(s, map);
            }
        }
        Stmt::If { then_branch, else_branch, .. } => {
            for s in then_branch {
                collect_types_stmt(s, map);
            }
            if let Some(eb) = else_branch {
                for s in eb {
                    collect_types_stmt(s, map);
                }
            }
        }
        Stmt::While { body, .. } | Stmt::For { body, .. } => {
            for s in body {
                collect_types_stmt(s, map);
            }
        }
        Stmt::ExportDecl(inner) => collect_types_stmt(inner, map),
        Stmt::ExportDefault(ExportDefault::Function(f)) => {
            for p in &f.params {
                map.insert(p.name.clone(), type_name_with_array(&p.ty));
            }
            for s in &f.body {
                collect_types_stmt(s, map);
            }
        }
        Stmt::ExportDefault(ExportDefault::Expr(_))
        | Stmt::Import { .. }
        | Stmt::FromImport { .. }
        | Stmt::ExportSpec(_) => {}
        _ => {}
    }
}

// ── Object types ──────────────────────────────────────────────────────────

/// Recursively collect the NAMES of object types referenced in a statement
/// (deduped). Used to emit `use crate::__Obj...;` in non-root modules.
pub(crate) fn collect_object_type_names(stmt: &Stmt, seen: &mut HashSet<String>) {
    match stmt {
        Stmt::Let { ty: Some(t), .. } | Stmt::Const { ty: Some(t), .. } => {
            if !t.fields.is_empty() {
                seen.insert(t.name.clone());
            }
        }
        Stmt::ExportDecl(inner) => collect_object_type_names(inner, seen),
        Stmt::ExportDefault(ed) => match ed {
            ExportDefault::Function(f) => {
                for p in &f.params {
                    if !p.ty.fields.is_empty() {
                        seen.insert(p.ty.name.clone());
                    }
                }
                for s in &f.body {
                    collect_object_type_names(s, seen);
                }
            }
            ExportDefault::Expr(_) => {}
        },
        Stmt::Function(f) => {
            for p in &f.params {
                if !p.ty.fields.is_empty() {
                    seen.insert(p.ty.name.clone());
                }
            }
            for s in &f.body {
                collect_object_type_names(s, seen);
            }
        }
        Stmt::If { then_branch, else_branch, .. } => {
            for s in then_branch {
                collect_object_type_names(s, seen);
            }
            if let Some(eb) = else_branch {
                for s in eb {
                    collect_object_type_names(s, seen);
                }
            }
        }
        Stmt::While { body, .. } | Stmt::For { body, .. } | Stmt::ForOf { body, .. } => {
            for s in body {
                collect_object_type_names(s, seen);
            }
        }
        _ => {}
    }
}

/// Collect all object types from EVERY module (so we can centralise the
/// struct definitions in the root). Dedup by name (the name is already a
/// deterministic hash).
pub(crate) fn collect_all_object_types(modules: &[Module]) -> Vec<Type> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut result: Vec<Type> = Vec::new();
    for m in modules {
        for stmt in &m.program.stmts {
            collect_object_types_stmt(stmt, &mut seen, &mut result);
        }
    }
    result
}

fn collect_object_types_stmt(stmt: &Stmt, seen: &mut HashSet<String>, out: &mut Vec<Type>) {
    match stmt {
        Stmt::Let { ty: Some(t), .. } | Stmt::Const { ty: Some(t), .. } => {
            if !t.fields.is_empty() && seen.insert(t.name.clone()) {
                out.push(t.clone());
            }
        }
        Stmt::Function(f) => {
            for p in &f.params {
                if !p.ty.fields.is_empty() && seen.insert(p.ty.name.clone()) {
                    out.push(p.ty.clone());
                }
            }
            for s in &f.body {
                collect_object_types_stmt(s, seen, out);
            }
        }
        Stmt::If { then_branch, else_branch, .. } => {
            for s in then_branch {
                collect_object_types_stmt(s, seen, out);
            }
            if let Some(eb) = else_branch {
                for s in eb {
                    collect_object_types_stmt(s, seen, out);
                }
            }
        }
        Stmt::While { body, .. } | Stmt::For { body, .. } | Stmt::ForOf { body, .. } => {
            for s in body {
                collect_object_types_stmt(s, seen, out);
            }
        }
        Stmt::ExportDecl(inner) => collect_object_types_stmt(inner, seen, out),
        Stmt::ExportDefault(ExportDefault::Function(f)) => {
            for p in &f.params {
                if !p.ty.fields.is_empty() && seen.insert(p.ty.name.clone()) {
                    out.push(p.ty.clone());
                }
            }
            for s in &f.body {
                collect_object_types_stmt(s, seen, out);
            }
        }
        Stmt::ExportDefault(ExportDefault::Expr(_)) | Stmt::Import { .. } | Stmt::FromImport { .. } | Stmt::ExportSpec(_) => {}
        _ => {}
    }
}