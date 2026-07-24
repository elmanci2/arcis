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

use arcis_ast::{ExportDefault, Expr, Function, Program, Stmt, Type};

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
    if let Expr::Call { callee, args, .. } = expr {
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

/// Flat name → full `Type` scope for the MODULE level only (`main`'s own
/// statements for the root module; a non-root module's own top-level
/// statements) — deliberately does NOT descend into any function's body.
/// Unlike [`collect_types`] (which only stores a primitive-name string,
/// for `.length` disambiguation), this keeps the whole `Type` so
/// `arcis_validation::expr_type` can answer "is this expression optional?"
/// genuinely — used by `Stmt::Return`'s auto-`Some(...)` wrapping and
/// `Expr::NonNullAssertion`'s `.unwrap()` decision.
///
/// Each function gets its OWN independent scope via
/// [`collect_function_type_scope`] instead of being folded into this one
/// flat map — matching `arcis_validation::resolve_shadowing`'s "each
/// function is its own scope tree" model (see that module's doc comment).
/// Building one giant module-wide map here would let two DIFFERENT
/// functions' same-named locals collide (last declaration wins for the
/// whole module), silently mistyping an early function's binding once a
/// later function reused its name — a real bug caught by
/// `examples/optionals/optionals.tsr` during development (`findById`'s
/// non-optional loop variable `p` was mistyped as `Product?`` because a
/// later, unrelated function also happened to declare a `let p: Product?`).
pub(crate) fn collect_type_scope(program: &Program) -> HashMap<String, Type> {
    let mut map = HashMap::new();
    for stmt in &program.stmts {
        collect_type_scope_stmt(stmt, &mut map, false);
    }
    map
}

/// Flat name → full `Type` scope for one function's own parameters and
/// body (only). Merge this OVER a clone of the module-level scope (see
/// [`collect_type_scope`]) to get the full scope visible inside that
/// function's body.
pub(crate) fn collect_function_type_scope(f: &Function) -> HashMap<String, Type> {
    let mut map = HashMap::new();
    for p in &f.params {
        map.insert(p.name.clone(), p.ty.clone());
    }
    for s in &f.body {
        collect_type_scope_stmt(s, &mut map, true);
    }
    map
}

/// `descend_into_functions`: `false` when walking at module level (a
/// `Stmt::Function` is registered as opaque — see [`collect_type_scope`]'s
/// doc comment for why); `true` when walking a single function's own body
/// (used by [`collect_function_type_scope`] — a NESTED function
/// declaration inside it still gets its params registered so calls to it
/// can be typed, matching this module's other collectors' existing
/// behavior for nested declarations).
fn collect_type_scope_stmt(stmt: &Stmt, map: &mut HashMap<String, Type>, descend_into_functions: bool) {
    match stmt {
        Stmt::Let { name, ty: Some(t), .. } | Stmt::Const { name, ty: Some(t), .. } => {
            map.insert(name.clone(), t.clone());
        }
        Stmt::ForOf { name, ty, body, .. } => {
            if let Some(t) = ty {
                map.insert(name.clone(), t.clone());
            }
            for s in body {
                collect_type_scope_stmt(s, map, descend_into_functions);
            }
        }
        Stmt::Function(f) => {
            if descend_into_functions {
                for p in &f.params {
                    map.insert(p.name.clone(), p.ty.clone());
                }
                for s in &f.body {
                    collect_type_scope_stmt(s, map, descend_into_functions);
                }
            }
        }
        Stmt::If { then_branch, else_branch, .. } => {
            for s in then_branch {
                collect_type_scope_stmt(s, map, descend_into_functions);
            }
            if let Some(eb) = else_branch {
                for s in eb {
                    collect_type_scope_stmt(s, map, descend_into_functions);
                }
            }
        }
        Stmt::While { body, .. } | Stmt::For { body, .. } => {
            for s in body {
                collect_type_scope_stmt(s, map, descend_into_functions);
            }
        }
        Stmt::Switch { cases, .. } => {
            for c in cases {
                for s in &c.body {
                    collect_type_scope_stmt(s, map, descend_into_functions);
                }
            }
        }
        Stmt::Try { body, catch_body, .. } => {
            for s in body {
                collect_type_scope_stmt(s, map, descend_into_functions);
            }
            for s in catch_body {
                collect_type_scope_stmt(s, map, descend_into_functions);
            }
        }
        Stmt::ExportDecl(inner) => collect_type_scope_stmt(inner, map, descend_into_functions),
        Stmt::ExportDefault(ExportDefault::Function(f)) => {
            if descend_into_functions {
                for p in &f.params {
                    map.insert(p.name.clone(), p.ty.clone());
                }
                for s in &f.body {
                    collect_type_scope_stmt(s, map, descend_into_functions);
                }
            }
        }
        _ => {}
    }
}

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
/// enclosing array wrapper). Recurses into the object's OWN fields too —
/// `{ a: { b: number } }`'s inner `{ b: number }` shape needs its own
/// struct definition emitted just as much as the outer one does (a field
/// whose struct never gets emitted is a `cannot find type` `rustc` error
/// downstream), and a naturally deeply-nested shape (JSON-inferred object
/// trees especially, but also any hand-written nested object literal) can
/// go arbitrarily deep.
fn note_object_type(t: &Type, seen: &mut HashSet<String>, out: &mut Vec<Type>) {
    fn innermost_object(t: &Type) -> Option<&Type> {
        match t {
            Type::Object { .. } => Some(t),
            Type::Array(inner) | Type::Optional(inner) => innermost_object(inner),
            _ => None,
        }
    }
    if let Some(obj) = innermost_object(t) {
        if let Some(name) = obj.struct_name() {
            if seen.insert(name.to_string()) {
                out.push(obj.clone());
                if let Type::Object { fields, .. } = obj {
                    for (_, field_ty, _) in fields {
                        note_object_type(field_ty, seen, out);
                    }
                }
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
/// `(type_params, extends, fields)`. Type params are NOT merged through
/// `extends` — a generic interface extending another generic interface is
/// out of scope for now (see `resolve_interface_fields`'s doc comment).
type RawInterface = (Vec<String>, Vec<String>, Vec<InterfaceField>);

/// Collect every `interface` declaration across all modules (including ones
/// wrapped in `export interface ...`), resolve `extends` chains by merging
/// base fields (own fields win on name collision), and return one
/// [`Type::Object`] per interface — named after the interface itself
/// (unlike inline object types, which get a hash-based name) — alongside a
/// name → own `type_params` map (interfaces collapse to `Type::Object`,
/// which has no room for type params of its own, so callers that need to
/// emit a generic struct header must look them up here).
pub(crate) fn collect_interfaces(modules: &[Module]) -> (Vec<Type>, HashMap<String, Vec<String>>) {
    let mut raw: HashMap<String, RawInterface> = HashMap::new();
    for m in modules {
        for stmt in &m.program.stmts {
            collect_interface_decl(stmt, &mut raw);
        }
    }
    let type_params: HashMap<String, Vec<String>> =
        raw.iter().map(|(name, (tp, _, _))| (name.clone(), tp.clone())).collect();
    let names: Vec<String> = raw.keys().cloned().collect();
    let mut resolved: HashMap<String, Vec<InterfaceField>> = HashMap::new();
    for name in &names {
        resolve_interface_fields(name, &raw, &mut resolved, &mut HashSet::new());
    }
    let types = resolved
        .into_iter()
        .map(|(name, fields)| Type::Object { name, fields })
        .collect();
    (types, type_params)
}

fn collect_interface_decl(stmt: &Stmt, raw: &mut HashMap<String, RawInterface>) {
    match stmt {
        Stmt::Interface { name, type_params, extends, fields, .. } => {
            raw.insert(name.clone(), (type_params.clone(), extends.clone(), fields.clone()));
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
    let Some((_type_params, extends, own_fields)) = raw.get(name) else {
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

/// `name → type_params` for every generic `type X<T> = ...;` alias.
pub(crate) type AliasTypeParams = HashMap<String, Vec<String>>;

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
///
/// Generic aliases split in two: a non-object RHS (`type Wrapper<T> = T[]`)
/// is substituted+inlined like today, using `extra_type_params`/locally
/// collected type params to bind each usage's `Type::Generic` args. An
/// object-shaped RHS (`type Pair<A,B> = { ... }`) is treated like a generic
/// interface — never inlined, left as a `Type::Generic` reference to the
/// one real struct emitted for it elsewhere (see `collect_object_alias_structs`).
pub(crate) fn resolve_type_aliases(
    program: &mut Program,
    extra: &HashMap<String, Type>,
    extra_type_params: &AliasTypeParams,
) {
    let mut raw: HashMap<String, Type> = extra.clone();
    let mut type_params: AliasTypeParams = extra_type_params.clone();
    for stmt in &program.stmts {
        collect_alias_decl(stmt, &mut raw, &mut type_params);
    }
    if raw.is_empty() {
        return;
    }
    let mut resolved: HashMap<String, Type> = HashMap::new();
    for stmt in &mut program.stmts {
        rewrite_stmt_types(stmt, &raw, &mut resolved, &type_params);
    }
}

/// Collect every top-level `type X = ...;` declaration (including
/// `export type ...`) across ALL modules, so aliases resolve across module
/// boundaries — a `type DiscountCode = ...` declared in `models.tsr` must
/// work in `pricing.tsr` too. Only top-level declarations are global;
/// function-local aliases stay local to their module's own resolution pass.
/// For a generic, object-shaped alias (`type Pair<A,B> = {...}`), the
/// returned `Type::Object`'s `name` is rewritten to the alias's own
/// declared name (overriding the parser's hash-based inline-object name) —
/// it needs a stable, referenceable name since it's emitted as a real
/// struct rather than inlined at every usage site.
pub(crate) fn collect_global_aliases(modules: &[Module]) -> (HashMap<String, Type>, AliasTypeParams) {
    let mut raw = HashMap::new();
    let mut type_params = HashMap::new();
    for m in modules {
        for stmt in &m.program.stmts {
            let inner = match stmt {
                Stmt::ExportDecl(inner) => inner.as_ref(),
                other => other,
            };
            if let Stmt::TypeAlias { name, type_params: tp, ty, .. } = inner {
                raw.insert(name.clone(), named_object_alias(name, ty));
                if !tp.is_empty() {
                    type_params.insert(name.clone(), tp.clone());
                }
            }
        }
    }
    (raw, type_params)
}

/// A generic, object-shaped alias's `Type::Object` gets its `name` field
/// rewritten to `alias_name` (see `collect_global_aliases`'s doc comment);
/// every other type is returned unchanged.
fn named_object_alias(alias_name: &str, ty: &Type) -> Type {
    match ty {
        Type::Object { fields, .. } => Type::Object { name: alias_name.to_string(), fields: fields.clone() },
        other => other.clone(),
    }
}

fn collect_alias_decl(stmt: &Stmt, raw: &mut HashMap<String, Type>, type_params: &mut AliasTypeParams) {
    match stmt {
        Stmt::TypeAlias { name, type_params: tp, ty, .. } => {
            raw.insert(name.clone(), named_object_alias(name, ty));
            if !tp.is_empty() {
                type_params.insert(name.clone(), tp.clone());
            }
        }
        Stmt::ExportDecl(inner) => collect_alias_decl(inner, raw, type_params),
        Stmt::Function(f) => {
            for s in &f.body {
                collect_alias_decl(s, raw, type_params);
            }
        }
        Stmt::ExportDefault(ExportDefault::Function(f)) => {
            for s in &f.body {
                collect_alias_decl(s, raw, type_params);
            }
        }
        Stmt::If { then_branch, else_branch, .. } => {
            for s in then_branch {
                collect_alias_decl(s, raw, type_params);
            }
            if let Some(eb) = else_branch {
                for s in eb {
                    collect_alias_decl(s, raw, type_params);
                }
            }
        }
        Stmt::While { body, .. } | Stmt::For { body, .. } | Stmt::ForOf { body, .. } => {
            for s in body {
                collect_alias_decl(s, raw, type_params);
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
    type_params: &AliasTypeParams,
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
    let substituted = substitute_named(&ty, raw, resolved, visiting, type_params);
    visiting.remove(name);
    resolved.insert(name.to_string(), substituted.clone());
    Some(substituted)
}

fn substitute_named(
    ty: &Type,
    raw: &HashMap<String, Type>,
    resolved: &mut HashMap<String, Type>,
    visiting: &mut HashSet<String>,
    type_params: &AliasTypeParams,
) -> Type {
    match ty {
        Type::Named(n) => {
            resolve_alias(n, raw, resolved, visiting, type_params).unwrap_or_else(|| ty.clone())
        }
        Type::Generic { name, args } => {
            let args: Vec<Type> =
                args.iter().map(|a| substitute_named(a, raw, resolved, visiting, type_params)).collect();
            // Object-shaped aliases (and interfaces, which never appear in
            // `raw` at all) are never inlined — they're emitted as one real
            // generic struct and referenced by name everywhere. Only a
            // non-object alias (`type Wrapper<T> = T[]`) gets substituted
            // away here.
            let is_object_shaped = matches!(raw.get(name), Some(Type::Object { .. }));
            if !is_object_shaped {
                if let (Some(params), Some(underlying)) = (type_params.get(name), raw.get(name)) {
                    if params.len() == args.len() {
                        let subst: HashMap<&str, &Type> =
                            params.iter().map(String::as_str).zip(args.iter()).collect();
                        let bound = bind_type_params(underlying, &subst);
                        return substitute_named(&bound, raw, resolved, visiting, type_params);
                    }
                }
            }
            Type::Generic { name: name.clone(), args }
        }
        Type::Array(inner) => Type::Array(Box::new(substitute_named(inner, raw, resolved, visiting, type_params))),
        Type::Object { name, fields } => Type::Object {
            name: name.clone(),
            fields: fields
                .iter()
                .map(|(k, t, opt)| {
                    (k.clone(), Box::new(substitute_named(t, raw, resolved, visiting, type_params)), *opt)
                })
                .collect(),
        },
        Type::Union(members) => Type::Union(
            members.iter().map(|m| substitute_named(m, raw, resolved, visiting, type_params)).collect(),
        ),
        Type::Intersection(members) => Type::Intersection(
            members.iter().map(|m| substitute_named(m, raw, resolved, visiting, type_params)).collect(),
        ),
        Type::Function { params, return_type } => Type::Function {
            params: params.iter().map(|p| substitute_named(p, raw, resolved, visiting, type_params)).collect(),
            return_type: Box::new(substitute_named(return_type, raw, resolved, visiting, type_params)),
        },
        _ => ty.clone(),
    }
}

/// Replace every `Type::Named(p)` in `ty` where `p` is a key of `subst`
/// with its bound concrete type — a single, non-alias-table substitution
/// pass used to instantiate a generic alias's underlying type with a
/// specific usage site's type arguments (`Wrapper<T> = T[]` + `args=[number]`
/// → `number[]`).
fn bind_type_params(ty: &Type, subst: &HashMap<&str, &Type>) -> Type {
    match ty {
        Type::Named(n) => subst.get(n.as_str()).map(|t| (*t).clone()).unwrap_or_else(|| ty.clone()),
        Type::Generic { name, args } => Type::Generic {
            name: name.clone(),
            args: args.iter().map(|a| bind_type_params(a, subst)).collect(),
        },
        Type::Array(inner) => Type::Array(Box::new(bind_type_params(inner, subst))),
        Type::Object { name, fields } => Type::Object {
            name: name.clone(),
            fields: fields.iter().map(|(k, t, opt)| (k.clone(), Box::new(bind_type_params(t, subst)), *opt)).collect(),
        },
        Type::Optional(inner) => Type::Optional(Box::new(bind_type_params(inner, subst))),
        Type::Union(members) => Type::Union(members.iter().map(|m| bind_type_params(m, subst)).collect()),
        Type::Intersection(members) => {
            Type::Intersection(members.iter().map(|m| bind_type_params(m, subst)).collect())
        }
        Type::Function { params, return_type } => Type::Function {
            params: params.iter().map(|p| bind_type_params(p, subst)).collect(),
            return_type: Box::new(bind_type_params(return_type, subst)),
        },
        _ => ty.clone(),
    }
}

fn rewrite_stmt_types(
    stmt: &mut Stmt,
    raw: &HashMap<String, Type>,
    resolved: &mut HashMap<String, Type>,
    type_params: &AliasTypeParams,
) {
    let mut visiting = HashSet::new();
    match stmt {
        Stmt::Let { ty, value, .. } | Stmt::Const { ty, value, .. } => {
            if let Some(t) = ty {
                *t = substitute_named(t, raw, resolved, &mut visiting, type_params);
            }
            rewrite_expr_types(value, raw, resolved, type_params);
        }
        Stmt::Assign { value, .. } => rewrite_expr_types(value, raw, resolved, type_params),
        Stmt::AssignIndex { index, value, .. } => {
            rewrite_expr_types(index, raw, resolved, type_params);
            rewrite_expr_types(value, raw, resolved, type_params);
        }
        Stmt::AssignMember { object, value, .. } => {
            rewrite_expr_types(object, raw, resolved, type_params);
            rewrite_expr_types(value, raw, resolved, type_params);
        }
        Stmt::ForOf { ty, iterable, body, .. } => {
            if let Some(t) = ty {
                *t = substitute_named(t, raw, resolved, &mut visiting, type_params);
            }
            rewrite_expr_types(iterable, raw, resolved, type_params);
            for s in body {
                rewrite_stmt_types(s, raw, resolved, type_params);
            }
        }
        Stmt::Function(f) => {
            for p in &mut f.params {
                p.ty = substitute_named(&p.ty, raw, resolved, &mut visiting, type_params);
            }
            f.return_type = substitute_named(&f.return_type, raw, resolved, &mut visiting, type_params);
            for s in &mut f.body {
                rewrite_stmt_types(s, raw, resolved, type_params);
            }
        }
        Stmt::ExportDefault(ExportDefault::Function(f)) => {
            for p in &mut f.params {
                p.ty = substitute_named(&p.ty, raw, resolved, &mut visiting, type_params);
            }
            f.return_type = substitute_named(&f.return_type, raw, resolved, &mut visiting, type_params);
            for s in &mut f.body {
                rewrite_stmt_types(s, raw, resolved, type_params);
            }
        }
        Stmt::ExportDefault(ExportDefault::Expr(e)) => rewrite_expr_types(e, raw, resolved, type_params),
        Stmt::Return(Some(e)) | Stmt::Throw(e) | Stmt::Expr(e) => {
            rewrite_expr_types(e, raw, resolved, type_params)
        }
        Stmt::Return(None) | Stmt::Break | Stmt::Continue => {}
        Stmt::If { condition, then_branch, else_branch } => {
            rewrite_expr_types(condition, raw, resolved, type_params);
            for s in then_branch {
                rewrite_stmt_types(s, raw, resolved, type_params);
            }
            if let Some(eb) = else_branch {
                for s in eb {
                    rewrite_stmt_types(s, raw, resolved, type_params);
                }
            }
        }
        Stmt::While { condition, body } => {
            rewrite_expr_types(condition, raw, resolved, type_params);
            for s in body {
                rewrite_stmt_types(s, raw, resolved, type_params);
            }
        }
        Stmt::For { init, condition, update, body } => {
            if let Some(init_stmt) = init {
                rewrite_stmt_types(init_stmt, raw, resolved, type_params);
            }
            if let Some(c) = condition {
                rewrite_expr_types(c, raw, resolved, type_params);
            }
            if let Some(update_stmt) = update {
                rewrite_stmt_types(update_stmt, raw, resolved, type_params);
            }
            for s in body {
                rewrite_stmt_types(s, raw, resolved, type_params);
            }
        }
        Stmt::Switch { discriminant, cases } => {
            rewrite_expr_types(discriminant, raw, resolved, type_params);
            for case in cases {
                for v in &mut case.values {
                    rewrite_expr_types(v, raw, resolved, type_params);
                }
                for s in &mut case.body {
                    rewrite_stmt_types(s, raw, resolved, type_params);
                }
            }
        }
        Stmt::Try { body, catch_body, .. } => {
            for s in body {
                rewrite_stmt_types(s, raw, resolved, type_params);
            }
            for s in catch_body {
                rewrite_stmt_types(s, raw, resolved, type_params);
            }
        }
        Stmt::ExportDecl(inner) => rewrite_stmt_types(inner, raw, resolved, type_params),
        Stmt::Interface { fields, .. } => {
            for (_, t, _) in fields {
                **t = substitute_named(t, raw, resolved, &mut visiting, type_params);
            }
        }
        Stmt::Import { .. }
        | Stmt::FromImport { .. }
        | Stmt::ExportSpec(_)
        | Stmt::TypeAlias { .. }
        | Stmt::Enum { .. } => {}
    }
}

/// Rewrite every `Type::Named`/`Type::Generic` reachable from `expr` — today
/// that's just `Expr::Call.type_args` (`json<Person>(...)`, or a
/// user-declared generic function called with explicit turbofish args) and
/// `Expr::AsAssertion.ty`/`Expr::Arrow`'s param/return types, the only
/// places a bare `Type` value lives inside an expression tree. Without this,
/// an explicit type argument stays an unresolved `Type::Named`/`Type::Generic`
/// all the way to codegen, which only knows how to read fields off a real
/// `Type::Object`.
fn rewrite_expr_types(
    expr: &mut Expr,
    raw: &HashMap<String, Type>,
    resolved: &mut HashMap<String, Type>,
    type_params: &AliasTypeParams,
) {
    let mut visiting = HashSet::new();
    match expr {
        Expr::Call { callee, args, type_args } => {
            rewrite_expr_types(callee, raw, resolved, type_params);
            for a in args {
                rewrite_expr_types(a, raw, resolved, type_params);
            }
            for t in type_args {
                *t = substitute_named(t, raw, resolved, &mut visiting, type_params);
            }
        }
        Expr::Unary { operand, .. }
        | Expr::TypeOf(operand)
        | Expr::NonNullAssertion(operand)
        | Expr::AsConst(operand) => rewrite_expr_types(operand, raw, resolved, type_params),
        Expr::Binary { left, right, .. } => {
            rewrite_expr_types(left, raw, resolved, type_params);
            rewrite_expr_types(right, raw, resolved, type_params);
        }
        Expr::Member { object, .. } => rewrite_expr_types(object, raw, resolved, type_params),
        Expr::Index { object, index } => {
            rewrite_expr_types(object, raw, resolved, type_params);
            rewrite_expr_types(index, raw, resolved, type_params);
        }
        Expr::ArrayLiteral { elements } => {
            for el in elements {
                match el {
                    arcis_ast::ArrayElement::Item(e) | arcis_ast::ArrayElement::Spread(e) => {
                        rewrite_expr_types(e, raw, resolved, type_params)
                    }
                }
            }
        }
        Expr::ObjectLiteral { fields } => {
            for f in fields {
                match f {
                    arcis_ast::ObjectField::KV(_, e) | arcis_ast::ObjectField::Spread(e) => {
                        rewrite_expr_types(e, raw, resolved, type_params)
                    }
                }
            }
        }
        Expr::AsAssertion { expr, ty } => {
            *ty = substitute_named(ty, raw, resolved, &mut visiting, type_params);
            rewrite_expr_types(expr, raw, resolved, type_params);
        }
        Expr::Arrow { params, return_type, body } => {
            for p in params {
                p.ty = substitute_named(&p.ty, raw, resolved, &mut visiting, type_params);
            }
            if let Some(rt) = return_type {
                *rt = substitute_named(rt, raw, resolved, &mut visiting, type_params);
            }
            match body {
                arcis_ast::ArrowBody::Expr(e) => rewrite_expr_types(e, raw, resolved, type_params),
                arcis_ast::ArrowBody::Block(stmts) => {
                    for s in stmts {
                        rewrite_stmt_types(s, raw, resolved, type_params);
                    }
                }
            }
        }
        Expr::Number(_)
        | Expr::String(_)
        | Expr::Bool(_)
        | Expr::Ident(_)
        | Expr::Path { .. }
        | Expr::Null
        | Expr::Undefined => {}
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