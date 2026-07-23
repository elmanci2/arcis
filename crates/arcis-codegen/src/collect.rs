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
        Stmt::Let { .. }
        | Stmt::Const { .. }
        | Stmt::TypeAlias { .. }
        | Stmt::Interface { .. }
        | Stmt::Enum { .. } => {}
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
        Stmt::Switch { cases, .. } => {
            for case in cases {
                for s in &case.body {
                    collect_in_stmt(s, set);
                }
            }
        }
        Stmt::Try { body, catch_body, .. } => {
            for s in body {
                collect_in_stmt(s, set);
            }
            for s in catch_body {
                collect_in_stmt(s, set);
            }
        }
        Stmt::Throw(_) => {}
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
        if let Type::Array(inner) = t {
            format!("{}[]", inner.primitive_name())
        } else {
            t.primitive_name().to_string()
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
        Stmt::Switch { cases, .. } => {
            for case in cases {
                for s in &case.body {
                    collect_types_stmt(s, map);
                }
            }
        }
        Stmt::Try { body, catch_body, .. } => {
            for s in body {
                collect_types_stmt(s, map);
            }
            for s in catch_body {
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
    fn note(t: &Type, seen: &mut HashSet<String>) {
        if let Some(name) = t.struct_name() {
            seen.insert(name.to_string());
        }
    }
    match stmt {
        Stmt::Let { ty: Some(t), .. } | Stmt::Const { ty: Some(t), .. } => {
            note(t, seen);
        }
        Stmt::ExportDecl(inner) => collect_object_type_names(inner, seen),
        Stmt::ExportDefault(ed) => match ed {
            ExportDefault::Function(f) => {
                for p in &f.params {
                    note(&p.ty, seen);
                }
                for s in &f.body {
                    collect_object_type_names(s, seen);
                }
            }
            ExportDefault::Expr(_) => {}
        },
        Stmt::Function(f) => {
            for p in &f.params {
                note(&p.ty, seen);
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

/// If `t` is an object type (or array of one), and its name hasn't been seen
/// yet, register it in `seen` and push the *innermost* [`Type::Object`] onto
/// `out` (the struct emitter only ever needs the object shape, not the
/// enclosing array wrapper).
fn note_object_type(t: &Type, seen: &mut HashSet<String>, out: &mut Vec<Type>) {
    fn innermost_object(t: &Type) -> Option<&Type> {
        match t {
            Type::Object { .. } => Some(t),
            Type::Array(inner) => innermost_object(inner),
            _ => None,
        }
    }
    if let Some(obj) = innermost_object(t) {
        if let Some(name) = obj.struct_name() {
            if seen.insert(name.to_string()) {
                out.push(obj.clone());
            }
        }
    }
}

fn collect_object_types_stmt(stmt: &Stmt, seen: &mut HashSet<String>, out: &mut Vec<Type>) {
    match stmt {
        Stmt::Let { ty: Some(t), .. } | Stmt::Const { ty: Some(t), .. } => {
            note_object_type(t, seen, out);
        }
        Stmt::Function(f) => {
            for p in &f.params {
                note_object_type(&p.ty, seen, out);
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
                note_object_type(&p.ty, seen, out);
            }
            for s in &f.body {
                collect_object_types_stmt(s, seen, out);
            }
        }
        Stmt::ExportDefault(ExportDefault::Expr(_)) | Stmt::Import { .. } | Stmt::FromImport { .. } | Stmt::ExportSpec(_) => {}
        _ => {}
    }
}

// ── Interfaces ───────────────────────────────────────────────────────────

type InterfaceField = (String, Box<Type>, bool);
type RawInterface = (Vec<String>, Vec<InterfaceField>);

/// Collect every `interface` declaration across all modules (including ones
/// wrapped in `export interface ...`), resolve `extends` chains by merging
/// base fields (own fields win on name collision), and return one
/// [`Type::Object`] per interface — named after the interface itself
/// (unlike inline object types, which get a hash-based name).
pub(crate) fn collect_interfaces(modules: &[Module]) -> Vec<Type> {
    let mut raw: HashMap<String, RawInterface> = HashMap::new();
    for m in modules {
        for stmt in &m.program.stmts {
            collect_interface_decl(stmt, &mut raw);
        }
    }
    let names: Vec<String> = raw.keys().cloned().collect();
    let mut resolved: HashMap<String, Vec<InterfaceField>> = HashMap::new();
    for name in &names {
        resolve_interface_fields(name, &raw, &mut resolved, &mut HashSet::new());
    }
    resolved
        .into_iter()
        .map(|(name, fields)| Type::Object { name, fields })
        .collect()
}

fn collect_interface_decl(stmt: &Stmt, raw: &mut HashMap<String, RawInterface>) {
    match stmt {
        Stmt::Interface { name, extends, fields, .. } => {
            raw.insert(name.clone(), (extends.clone(), fields.clone()));
        }
        Stmt::ExportDecl(inner) => collect_interface_decl(inner, raw),
        _ => {}
    }
}

/// Depth-first resolve: merge every base interface's fields (recursively)
/// before appending `name`'s own fields, so own fields override same-named
/// base fields. `visiting` guards against `extends` cycles.
fn resolve_interface_fields(
    name: &str,
    raw: &HashMap<String, RawInterface>,
    resolved: &mut HashMap<String, Vec<InterfaceField>>,
    visiting: &mut HashSet<String>,
) -> Vec<InterfaceField> {
    if let Some(fields) = resolved.get(name) {
        return fields.clone();
    }
    let Some((extends, own_fields)) = raw.get(name) else {
        return Vec::new();
    };
    if !visiting.insert(name.to_string()) {
        // Cycle: treat as having no inherited fields to break recursion.
        return own_fields.clone();
    }
    let mut merged: Vec<InterfaceField> = Vec::new();
    for base in extends {
        for f in resolve_interface_fields(base, raw, resolved, visiting) {
            merged.retain(|(fname, _, _)| fname != &f.0);
            merged.push(f);
        }
    }
    for f in own_fields {
        merged.retain(|(fname, _, _)| fname != &f.0);
        merged.push(f.clone());
    }
    visiting.remove(name);
    resolved.insert(name.to_string(), merged.clone());
    merged
}

// ── Type aliases ─────────────────────────────────────────────────────────

/// Replace every `Type::Named(name)` in `program` with its underlying type,
/// transitively (an alias may refer to another alias). `name` may come from
/// either a local `type X = ...;` declaration or `extra` (the interface
/// name → `Type::Object` map built by [`collect_interfaces`]) — both are
/// resolved the same way, so object-literal emission only ever has to
/// handle `Type::Object`, never a bare interface/alias name. Local `type`
/// declarations take priority on a name collision. Names that resolve to
/// neither (e.g. an external Rust type pulled in via
/// `import ... from "crate:<name>"`) are left untouched, passed through to
/// `rustc` as-is.
pub(crate) fn resolve_type_aliases(program: &mut Program, extra: &HashMap<String, Type>) {
    let mut raw: HashMap<String, Type> = extra.clone();
    for stmt in &program.stmts {
        collect_alias_decl(stmt, &mut raw);
    }
    if raw.is_empty() {
        return;
    }
    let mut resolved: HashMap<String, Type> = HashMap::new();
    for stmt in &mut program.stmts {
        rewrite_stmt_types(stmt, &raw, &mut resolved);
    }
}

/// Collect every top-level `type X = ...;` declaration (including
/// `export type ...`) across ALL modules, so aliases resolve across module
/// boundaries — a `type DiscountCode = ...` declared in `models.tsr` must
/// work in `pricing.tsr` too. Only top-level declarations are global;
/// function-local aliases stay local to their module's own resolution pass.
pub(crate) fn collect_global_aliases(modules: &[Module]) -> HashMap<String, Type> {
    let mut raw = HashMap::new();
    for m in modules {
        for stmt in &m.program.stmts {
            match stmt {
                Stmt::TypeAlias { name, ty, .. } => {
                    raw.insert(name.clone(), ty.clone());
                }
                Stmt::ExportDecl(inner) => {
                    if let Stmt::TypeAlias { name, ty, .. } = inner.as_ref() {
                        raw.insert(name.clone(), ty.clone());
                    }
                }
                _ => {}
            }
        }
    }
    raw
}

fn collect_alias_decl(stmt: &Stmt, raw: &mut HashMap<String, Type>) {
    match stmt {
        Stmt::TypeAlias { name, ty, .. } => {
            raw.insert(name.clone(), ty.clone());
        }
        Stmt::ExportDecl(inner) => collect_alias_decl(inner, raw),
        Stmt::Function(f) => {
            for s in &f.body {
                collect_alias_decl(s, raw);
            }
        }
        Stmt::ExportDefault(ExportDefault::Function(f)) => {
            for s in &f.body {
                collect_alias_decl(s, raw);
            }
        }
        Stmt::If { then_branch, else_branch, .. } => {
            for s in then_branch {
                collect_alias_decl(s, raw);
            }
            if let Some(eb) = else_branch {
                for s in eb {
                    collect_alias_decl(s, raw);
                }
            }
        }
        Stmt::While { body, .. } | Stmt::For { body, .. } | Stmt::ForOf { body, .. } => {
            for s in body {
                collect_alias_decl(s, raw);
            }
        }
        _ => {}
    }
}

fn resolve_alias(
    name: &str,
    raw: &HashMap<String, Type>,
    resolved: &mut HashMap<String, Type>,
    visiting: &mut HashSet<String>,
) -> Option<Type> {
    if let Some(t) = resolved.get(name) {
        return Some(t.clone());
    }
    let ty = raw.get(name)?.clone();
    if !visiting.insert(name.to_string()) {
        // Cycle: stop substituting further and return the raw (partially
        // resolved) type rather than recursing forever.
        return Some(ty);
    }
    let substituted = substitute_named(&ty, raw, resolved, visiting);
    visiting.remove(name);
    resolved.insert(name.to_string(), substituted.clone());
    Some(substituted)
}

fn substitute_named(
    ty: &Type,
    raw: &HashMap<String, Type>,
    resolved: &mut HashMap<String, Type>,
    visiting: &mut HashSet<String>,
) -> Type {
    match ty {
        Type::Named(n) => resolve_alias(n, raw, resolved, visiting).unwrap_or_else(|| ty.clone()),
        Type::Array(inner) => Type::Array(Box::new(substitute_named(inner, raw, resolved, visiting))),
        Type::Object { name, fields } => Type::Object {
            name: name.clone(),
            fields: fields
                .iter()
                .map(|(k, t, opt)| (k.clone(), Box::new(substitute_named(t, raw, resolved, visiting)), *opt))
                .collect(),
        },
        Type::Union(members) => {
            Type::Union(members.iter().map(|m| substitute_named(m, raw, resolved, visiting)).collect())
        }
        Type::Intersection(members) => {
            Type::Intersection(members.iter().map(|m| substitute_named(m, raw, resolved, visiting)).collect())
        }
        Type::Function { params, return_type } => Type::Function {
            params: params.iter().map(|p| substitute_named(p, raw, resolved, visiting)).collect(),
            return_type: Box::new(substitute_named(return_type, raw, resolved, visiting)),
        },
        _ => ty.clone(),
    }
}

fn rewrite_stmt_types(stmt: &mut Stmt, raw: &HashMap<String, Type>, resolved: &mut HashMap<String, Type>) {
    let mut visiting = HashSet::new();
    match stmt {
        Stmt::Let { ty, .. } | Stmt::Const { ty, .. } => {
            if let Some(t) = ty {
                *t = substitute_named(t, raw, resolved, &mut visiting);
            }
        }
        Stmt::ForOf { ty, body, .. } => {
            if let Some(t) = ty {
                *t = substitute_named(t, raw, resolved, &mut visiting);
            }
            for s in body {
                rewrite_stmt_types(s, raw, resolved);
            }
        }
        Stmt::Function(f) => {
            for p in &mut f.params {
                p.ty = substitute_named(&p.ty, raw, resolved, &mut visiting);
            }
            f.return_type = substitute_named(&f.return_type, raw, resolved, &mut visiting);
            for s in &mut f.body {
                rewrite_stmt_types(s, raw, resolved);
            }
        }
        Stmt::ExportDefault(ExportDefault::Function(f)) => {
            for p in &mut f.params {
                p.ty = substitute_named(&p.ty, raw, resolved, &mut visiting);
            }
            f.return_type = substitute_named(&f.return_type, raw, resolved, &mut visiting);
            for s in &mut f.body {
                rewrite_stmt_types(s, raw, resolved);
            }
        }
        Stmt::If { then_branch, else_branch, .. } => {
            for s in then_branch {
                rewrite_stmt_types(s, raw, resolved);
            }
            if let Some(eb) = else_branch {
                for s in eb {
                    rewrite_stmt_types(s, raw, resolved);
                }
            }
        }
        Stmt::While { body, .. } => {
            for s in body {
                rewrite_stmt_types(s, raw, resolved);
            }
        }
        Stmt::For { init, body, .. } => {
            if let Some(init_stmt) = init {
                rewrite_stmt_types(init_stmt, raw, resolved);
            }
            for s in body {
                rewrite_stmt_types(s, raw, resolved);
            }
        }
        Stmt::ExportDecl(inner) => rewrite_stmt_types(inner, raw, resolved),
        Stmt::Interface { fields, .. } => {
            for (_, t, _) in fields {
                **t = substitute_named(t, raw, resolved, &mut visiting);
            }
        }
        _ => {}
    }
}

// ── Enums ────────────────────────────────────────────────────────────────

/// Collect every `enum` declaration across all modules (including ones
/// wrapped in `export enum ...`), in source order. Enum names are assumed
/// unique across the whole program (a collision is a user error the
/// generated `rustc` output will report as a duplicate `enum` item).
pub(crate) fn collect_enums(modules: &[Module]) -> Vec<(String, Vec<(String, Option<i64>)>)> {
    let mut out = Vec::new();
    for m in modules {
        for stmt in &m.program.stmts {
            collect_enum_decl(stmt, &mut out);
        }
    }
    out
}

fn collect_enum_decl(stmt: &Stmt, out: &mut Vec<(String, Vec<(String, Option<i64>)>)>) {
    match stmt {
        Stmt::Enum { name, variants, .. } => out.push((name.clone(), variants.clone())),
        Stmt::ExportDecl(inner) => collect_enum_decl(inner, out),
        _ => {}
    }
}