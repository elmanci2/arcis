//! Arcis code generator: translates an AST into Rust source code.
//!
//! Type mapping:
//! ```text
//!   string  → String
//!   number  → f64
//!   boolean → bool
//!   void    → ()
//! ```
//!
//! Concatenation:
//! The `+` operator is translated to `format!("{}{}", a, b)` when AT LEAST ONE
//! operand is a string literal (a simple heuristic). Otherwise it emits plain
//! `a + b` (number + number, bool + bool, ...).
//!
//! Print:
//! `print(expr)` becomes `println!("{}", expr)`. If the expression is itself
//! a `format!` call, the result is nested and works identically.
//!
//! In phase 2 this single file will be split into `context.rs`, `collect.rs`,
//! `types.rs`, `module.rs`, `stmt.rs`, `expr.rs`, `builtin.rs`, `method.rs`,
//! and `function.rs` to mirror the AST node categories.
//!
//! NOTE: error messages and function-level doc-comments in this file are
//! still in Spanish from the original implementation. They will be translated
//! to English in a separate follow-up so this phase stays a pure structural
//! change.

use arcis_ast::{BinOp, ExportDefault, Expr, Function, Program, Stmt, Type, UnaryOp};
use arcis_linker::{resolve_specifier, Module, ModuleTarget};
use std::collections::{HashMap, HashSet};

/// Context passed to the emit functions: reassigned variables, the map of
/// declared types, the declared type of the let/const whose `ObjectLiteral`
/// we are currently emitting, and a flag indicating whether this is the root
/// `main` module (where object-type structs are defined) or a non-root
/// module (where they are referenced as `crate::__ObjNAME`).
struct Ctx<'a> {
    reassigned: &'a HashSet<String>,
    types: &'a HashMap<String, String>,
    /// Declared type of the let/const that contains the `ObjectLiteral`
    /// currently being emitted. `None` for free-floating expressions; in
    /// that case the codegen emits a `todo!()` (rustc will then report).
    current_let_type: Option<&'a Type>,
    /// `true` if we are generating the root (`main`) module.
    is_root: bool,
}

/// Generate the Rust source code for every module of the program.
/// Returns `(id, rust_source)` per module; the first element is the root.
pub fn generate_all(modules: &[Module]) -> Result<Vec<(String, String)>, String> {
    // Object-type structs are centralised in the root module (deduped by name).
    let all_obj_types = collect_all_object_types(modules);

    // Canonical-path → id map, for resolving `use crate::<id>::...`.
    let path_to_id: HashMap<std::path::PathBuf, String> = modules
        .iter()
        .map(|m| (m.path.clone(), m.id.clone()))
        .collect();

    let mut out = Vec::with_capacity(modules.len());
    for (i, m) in modules.iter().enumerate() {
        let is_root = i == 0;
        let src = generate_module(m, is_root, modules, &all_obj_types, &path_to_id)?;
        out.push((m.id.clone(), src));
    }
    Ok(out)
}

fn generate_module(
    m: &Module,
    is_root: bool,
    modules: &[Module],
    all_obj_types: &[Type],
    path_to_id: &HashMap<std::path::PathBuf, String>,
) -> Result<String, String> {
    let mut out = String::new();
    out.push_str("#![allow(unused_parens, non_snake_case, while_true, unused_imports, dead_code)]\n\n");

    let reassigned = collect_reassigned(&m.program);
    let types = collect_types(&m.program);
    let ctx = Ctx {
        reassigned: &reassigned,
        types: &types,
        current_let_type: None,
        is_root,
    };

    // The root declares every sub-module and defines the object-type structs.
    if is_root {
        for other in modules.iter().skip(1) {
            out.push_str(&format!("mod {};\n", other.id));
        }
        out.push('\n');
        for ty in all_obj_types {
            emit_struct_def(&mut out, ty);
        }
    }

    // imports → `use crate::<id>::...`
    emit_imports(&mut out, &m.program, &m.path, modules, path_to_id)?;

    // Object-type references in this module → `use crate::__Obj...;`
    // (necessary in non-root modules because the structs live in the root).
    if !is_root {
        let mut seen: HashSet<String> = HashSet::new();
        for stmt in &m.program.stmts {
            collect_object_type_names(stmt, &mut seen);
        }
        for name in seen {
            out.push_str(&format!("use crate::{};\n", name));
        }
    }

    // `export { a, b as c }` → `pub use self::a; pub use self::b as c;`
    emit_export_specs(&mut out, &m.program);

    // Top-level functions (pub if exported) and default export.
    for stmt in &m.program.stmts {
        emit_top_item(&mut out, stmt, &ctx);
    }

    if is_root {
        // fn main() wrapping the lets/consts and expression statements.
        // Functions were emitted above as items.
        out.push_str("fn main() {\n");
        for stmt in &m.program.stmts {
            emit_stmt(&mut out, stmt, 1, &ctx);
        }
        out.push_str("}\n");
    } else {
        // In a non-root module, top-level lets/consts become `const` items
        // (Rust requires const-evaluable values for items; see the README).
        for stmt in &m.program.stmts {
            emit_module_const(&mut out, stmt, &ctx);
        }
    }

    Ok(out)
}

/// `use crate::<id>::<name> [as <local>];` for each imported binding.
/// For external crates (`crate:<name>`), emit `use <crate>::<name>;`.
fn emit_imports(
    out: &mut String,
    program: &Program,
    importer_path: &std::path::Path,
    modules: &[Module],
    path_to_id: &HashMap<std::path::PathBuf, String>,
) -> Result<(), String> {
    for stmt in &program.stmts {
        if let Stmt::Import { default, named, module: spec } = stmt {
            match resolve_specifier(importer_path, spec)? {
                ModuleTarget::Crate(crate_name) => {
                    // The linker already rejected `default.is_some()` for crates.
                    for n in named {
                        let local = n.alias.clone().unwrap_or_else(|| n.name.clone());
                        if local == n.name {
                            out.push_str(&format!("use {}::{};\n", crate_name, n.name));
                        } else {
                            out.push_str(&format!(
                                "use {}::{} as {};\n",
                                crate_name, n.name, local
                            ));
                        }
                    }
                }
                ModuleTarget::Local(dep_path) => {
                    let dep_id = path_to_id.get(&dep_path).ok_or_else(|| {
                        format!("module `{}` not resolved to an id (internal error)", spec)
                    })?;
                    if let Some(local) = default {
                        let target = modules
                            .iter()
                            .find(|m| m.path == dep_path)
                            .expect("target module loaded");
                        let default_name =
                            target.exports.default.clone().ok_or_else(|| {
                                format!("`{}` has no default export", spec)
                            })?;
                        out.push_str(&format!(
                            "use crate::{}::{} as {};\n",
                            dep_id, default_name, local
                        ));
                    }
                    for n in named {
                        let local = n.alias.clone().unwrap_or_else(|| n.name.clone());
                        if local == n.name {
                            out.push_str(&format!("use crate::{}::{};\n", dep_id, n.name));
                        } else {
                            out.push_str(&format!(
                                "use crate::{}::{} as {};\n",
                                dep_id, n.name, local
                            ));
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

/// `export { a, b as c };` → `pub use self::a;` / `pub use self::b as c;`
fn emit_export_specs(out: &mut String, program: &Program) {
    for stmt in &program.stmts {
        if let Stmt::ExportSpec(items) = stmt {
            for it in items {
                match &it.alias {
                    Some(alias) => out.push_str(&format!("pub use self::{} as {};\n", it.name, alias)),
                    None => out.push_str(&format!("pub use self::{};\n", it.name)),
                }
            }
        }
    }
}

/// Emit top-level functions (free or exported) and the default export as
/// items. Lets / consts are handled separately.
fn emit_top_item(out: &mut String, stmt: &Stmt, ctx: &Ctx) {
    match stmt {
        Stmt::Function(f) => emit_function(out, f, ctx, false),
        Stmt::ExportDecl(inner) => {
            if let Stmt::Function(f) = inner.as_ref() {
                emit_function(out, f, ctx, true);
            }
            // exported lets/consts: in the root they live in `main()`;
            // in non-root they become `const` items (`emit_module_const`).
        }
        Stmt::ExportDefault(ed) => match ed {
            ExportDefault::Function(f) => {
                let mut f = f.clone();
                if f.name.is_empty() {
                    f.name = "__default".into();
                }
                emit_function(out, &f, ctx, true);
            }
            ExportDefault::Expr(e) => emit_default_const(out, e, ctx),
        },
        _ => {}
    }
}

/// Top-level lets / consts in a non-root module become `const` items.
fn emit_module_const(out: &mut String, stmt: &Stmt, ctx: &Ctx) {
    let (name, ty, value, pub_) = match stmt {
        Stmt::Const { name, ty, value, .. } => (name, ty, value, false),
        Stmt::Let { name, ty, value, .. } => (name, ty, value, false),
        Stmt::ExportDecl(inner) => match inner.as_ref() {
            Stmt::Const { name, ty, value, .. } | Stmt::Let { name, ty, value, .. } => {
                (name, ty, value, true)
            }
            _ => return,
        },
        _ => return,
    };
    out.push_str(if pub_ { "pub const " } else { "const " });
    out.push_str(name);
    if let Some(t) = ty {
        out.push_str(": ");
        out.push_str(&ts_type_to_rust(t, ctx.is_root));
    }
    out.push_str(" = ");
    if let (Some(t), Expr::ArrayLiteral { elements }) = (ty, value) {
        if elements.is_empty() && t.is_array {
            out.push_str("Vec::new();\n\n");
            return;
        }
    }
    let nested = Ctx {
        reassigned: ctx.reassigned,
        types: ctx.types,
        current_let_type: ty.as_ref(),
        is_root: ctx.is_root,
    };
    emit_expr(out, value, &nested);
    out.push_str(";\n\n");
}

/// `export default <expr>;` → `pub const __default: T = expr;`
/// Only works if `expr` is const-evaluable (rustc will enforce this).
fn emit_default_const(out: &mut String, e: &Expr, ctx: &Ctx) {
    match infer_expr_rust_type(e) {
        Some(ty) => {
            out.push_str(&format!("pub const __default: {} = ", ty));
        }
        None => out.push_str("pub const __default = "),
    }
    emit_expr(out, e, ctx);
    out.push_str(";\n\n");
}

/// Simple Rust type inference for literals (used by `export default`).
fn infer_expr_rust_type(e: &Expr) -> Option<String> {
    match e {
        Expr::Number(_) => Some("f64".into()),
        Expr::String(_) => Some("String".into()),
        Expr::Bool(_) => Some("bool".into()),
        Expr::ArrayLiteral { elements } => {
            let inner = elements
                .first()
                .and_then(infer_expr_rust_type)
                .unwrap_or_else(|| "f64".into());
            Some(format!("Vec<{}>", inner))
        }
        _ => None,
    }
}

/// Recursively collect the NAMES of object types referenced in a statement
/// (deduped). Used to emit `use crate::__Obj...;` in non-root modules.
fn collect_object_type_names(stmt: &Stmt, seen: &mut HashSet<String>) {
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
fn collect_all_object_types(modules: &[Module]) -> Vec<Type> {
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
        Stmt::ExportDefault(ExportDefault::Expr(_)) | Stmt::Import { .. } | Stmt::ExportSpec(_) => {}
        _ => {}
    }
}

fn emit_struct_def(out: &mut String, ty: &Type) {
    out.push_str("#[derive(Clone)]\n");
    out.push_str("pub struct ");
    out.push_str(&ty.name);
    out.push_str(" {\n");
    for (k, fty) in &ty.fields {
        out.push_str("    pub ");
        out.push_str(k);
        out.push_str(": ");
        out.push_str(&ts_type_to_rust(fty, true));
        out.push_str(",\n");
    }
    out.push_str("}\n\n");
}

/// Walk the program and return the set of names that are reassigned at least
/// once (in `main` or inside any function).
fn collect_reassigned(program: &Program) -> HashSet<String> {
    let mut set = HashSet::new();
    for stmt in &program.stmts {
        collect_in_stmt(stmt, &mut set);
    }
    set
}

/// Build a name → declared type map. Used for `.length` to distinguish
/// between `string` and an array when the object is an identifier.
fn collect_types(program: &Program) -> HashMap<String, String> {
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
        | Stmt::ExportSpec(_) => {}
        _ => {}
    }
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
        Stmt::Import { .. } | Stmt::ExportSpec(_) => {}
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

/// Descend through Index/Member until we find the outermost identifier of an
/// assignment expression and mark it as reassigned.
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

fn indent(out: &mut String, level: usize) {
    for _ in 0..level {
        out.push_str("    ");
    }
}

fn ts_type_to_rust(ty: &Type, is_root: bool) -> String {
    if ty.is_array {
        // Array of T: `Vec<T>` where T is the inner type (with its fields).
        let inner = Type {
            name: ty.name.clone(),
            fields: ty.fields.clone(),
            is_array: false,
        };
        return format!("Vec<{}>", ts_type_to_rust(&inner, is_root));
    }
    // Object types: the struct lives in the root, so from a sub-module we
    // reference it as `crate::__ObjNAME`.
    if !ty.fields.is_empty() {
        return if is_root {
            ty.name.clone()
        } else {
            format!("crate::{}", ty.name)
        };
    }
    match ty.name.as_str() {
        "string" => "String".to_string(),
        "number" => "f64".to_string(),
        "boolean" => "bool".to_string(),
        "void" => "()".to_string(),
        other => other.to_string(),
    }
}

fn rust_string_literal(s: &str) -> String {
    let mut out = String::from("String::from(\"");
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out.push_str("\")");
    out
}

fn emit_function(out: &mut String, f: &Function, ctx: &Ctx, pub_: bool) {
    out.push_str(if pub_ { "pub fn " } else { "fn " });
    out.push_str(&f.name);
    out.push('(');
    for (i, p) in f.params.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(&p.name);
        out.push_str(": ");
        // Arrays are passed by reference so we don't consume the argument
        // (TS semantics: passing an array to a function does not invalidate it).
        if p.ty.is_array {
            out.push('&');
        }
        out.push_str(&ts_type_to_rust(&p.ty, ctx.is_root));
    }
    out.push(')');
    out.push_str(" -> ");
    out.push_str(&ts_type_to_rust(&f.return_type, ctx.is_root));
    out.push_str(" {\n");
    for stmt in &f.body {
        emit_stmt(out, stmt, 1, ctx);
    }
    out.push_str("}\n\n");
}

fn emit_stmt(out: &mut String, stmt: &Stmt, level: usize, ctx: &Ctx) {
    match stmt {
        Stmt::Let { name, ty, value, .. } => {
            // `let` in TS allows reassignment, so we only emit `mut` when
            // this variable is reassigned somewhere in the program.
            indent(out, level);
            if ctx.reassigned.contains(name) {
                out.push_str("let mut ");
            } else {
                out.push_str("let ");
            }
            out.push_str(name);
            if let Some(t) = ty {
                out.push_str(": ");
                out.push_str(&ts_type_to_rust(t, ctx.is_root));
            }
            out.push_str(" = ");
            // Special case: `let x: T[] = []` — `vec![]` does not infer T,
            // so we emit `Vec::new()` and let the declared type guide
            // inference (Rust fills in the generic parameter).
            if let (Some(t), Expr::ArrayLiteral { elements }) = (ty, value) {
                if elements.is_empty() && t.is_array {
                    out.push_str("Vec::new()");
                    out.push_str(";\n");
                    return;
                }
            }
            // Pass the declared type down so an `ObjectLiteral` can find it.
            let nested_ctx = Ctx {
                reassigned: ctx.reassigned,
                types: ctx.types,
                current_let_type: ty.as_ref(),
                is_root: ctx.is_root,
            };
            emit_expr(out, value, &nested_ctx);
            out.push_str(";\n");
        }
        Stmt::Const { name, ty, value, .. } => {
            // Rust `const` requires a compile-time constant value, so it
            // fails for values computed at runtime. We use `let` with the
            // name in UPPER-CASE as a simple approximation (immutable).
            indent(out, level);
            out.push_str("let ");
            out.push_str(name);
            if let Some(t) = ty {
                out.push_str(": ");
                out.push_str(&ts_type_to_rust(t, ctx.is_root));
            }
            out.push_str(" = ");
            // Same special-case as `Let`.
            if let (Some(t), Expr::ArrayLiteral { elements }) = (ty, value) {
                if elements.is_empty() && t.is_array {
                    out.push_str("Vec::new()");
                    out.push_str(";\n");
                    return;
                }
            }
            let nested_ctx = Ctx {
                reassigned: ctx.reassigned,
                types: ctx.types,
                current_let_type: ty.as_ref(),
                is_root: ctx.is_root,
            };
            emit_expr(out, value, &nested_ctx);
            out.push_str(";\n");
        }
        Stmt::Assign { name, value } => {
            indent(out, level);
            out.push_str(name);
            out.push_str(" = ");
            emit_expr(out, value, ctx);
            out.push_str(";\n");
        }
        Stmt::AssignIndex { object, index, value } => {
            indent(out, level);
            out.push_str(object);
            out.push('[');
            emit_expr(out, index, ctx);
            out.push_str(" as usize] = ");
            emit_expr(out, value, ctx);
            out.push_str(";\n");
        }
        Stmt::AssignMember { object, property, value } => {
            indent(out, level);
            emit_expr(out, object, ctx);
            out.push('.');
            out.push_str(property);
            out.push_str(" = ");
            emit_expr(out, value, ctx);
            out.push_str(";\n");
        }
        Stmt::Function(_) => {
            // Functions were emitted before `main`; ignore them here.
        }
        Stmt::Return(expr) => {
            indent(out, level);
            out.push_str("return");
            if let Some(e) = expr {
                out.push(' ');
                emit_expr(out, e, ctx);
            }
            out.push_str(";\n");
        }
        Stmt::If { condition, then_branch, else_branch } => {
            indent(out, level);
            out.push_str("if ");
            emit_expr(out, condition, ctx);
            out.push_str(" {\n");
            for s in then_branch {
                emit_stmt(out, s, level + 1, ctx);
            }
            indent(out, level);
            out.push('}');
            if let Some(eb) = else_branch {
                out.push_str(" else {\n");
                for s in eb {
                    emit_stmt(out, s, level + 1, ctx);
                }
                indent(out, level);
                out.push('}');
            }
            out.push('\n');
        }
        Stmt::While { condition, body } => {
            indent(out, level);
            out.push_str("while ");
            emit_expr(out, condition, ctx);
            out.push_str(" {\n");
            for s in body {
                emit_stmt(out, s, level + 1, ctx);
            }
            indent(out, level);
            out.push_str("}\n");
        }
        Stmt::ForOf { name, ty, iterable, body } => {
            // For an identifier (local variable of type `Vec`) we use
            // `.iter().cloned()` so we don't consume it. For calls, member
            // accesses or index accesses (typical of crate-produced
            // iterators like `server.incoming_requests()`) we use
            // `.into_iter()` because `Iterator` does not have `.iter()`.
            indent(out, level);
            out.push_str("for ");
            out.push_str(name);
            out.push_str(" in ");
            let use_iter = matches!(iterable.as_ref(), Expr::Ident(_));
            emit_expr(out, iterable, ctx);
            if use_iter {
                out.push_str(".iter().cloned()");
            } else {
                out.push_str(".into_iter()");
            }
            out.push_str(" {\n");
            for s in body {
                emit_stmt(out, s, level + 1, ctx);
            }
            indent(out, level);
            out.push_str("}\n");
            let _ = ty; // the declared type of `let x: T of arr` is ignored for now
        }
        Stmt::For { init, condition, update, body } => {
            // Desugared using a `loop` with a flag so that `continue` runs
            // the update before the next iteration:
            //   {
            //     init;
            //     let mut __for_first = true;
            //     loop {
            //         if !__for_first { update; }
            //         __for_first = false;
            //         if !(cond) { break; }
            //         body;
            //     }
            //   }
            indent(out, level);
            out.push_str("{\n");
            if let Some(init) = init {
                emit_stmt(out, init, level + 1, ctx);
            } else {
                indent(out, level + 1);
                out.push_str(";\n");
            }
            indent(out, level + 1);
            out.push_str("let mut __for_first = true;\n");
            indent(out, level + 1);
            out.push_str("loop {\n");
            indent(out, level + 2);
            out.push_str("if !__for_first {\n");
            if let Some(upd) = update {
                emit_stmt(out, upd, level + 3, ctx);
            }
            indent(out, level + 2);
            out.push_str("}\n");
            indent(out, level + 2);
            out.push_str("__for_first = false;\n");
            indent(out, level + 2);
            out.push_str("if !(");
            if let Some(cond) = condition {
                emit_expr(out, cond, ctx);
            } else {
                out.push_str("true");
            }
            out.push_str(") { break; }\n");
            for s in body {
                emit_stmt(out, s, level + 2, ctx);
            }
            indent(out, level + 1);
            out.push_str("}\n");
            indent(out, level);
            out.push_str("}\n");
        }
        Stmt::Break => {
            indent(out, level);
            out.push_str("break;\n");
        }
        Stmt::Continue => {
            indent(out, level);
            out.push_str("continue;\n");
        }
        Stmt::Expr(expr) => {
            indent(out, level);
            emit_expr(out, expr, ctx);
            out.push_str(";\n");
        }
        Stmt::ExportDecl(inner) => {
            // In the root, exported lets/consts live in `main()` just like
            // unexported ones; functions / defaults were emitted as items
            // by `emit_top_item`.
            match inner.as_ref() {
                s @ (Stmt::Let { .. } | Stmt::Const { .. }) => emit_stmt(out, s, level, ctx),
                _ => {}
            }
        }
        Stmt::Import { .. } | Stmt::ExportSpec(_) | Stmt::ExportDefault(_) => {
            // Module structure (mod/use/pub) was emitted outside `emit_stmt`.
        }
    }
}

fn emit_expr(out: &mut String, expr: &Expr, ctx: &Ctx) {
    match expr {
        Expr::Number(n) => {
            // Always emit with a decimal point so Rust treats it as f64.
            if n.fract() == 0.0 {
                out.push_str(&format!("{}.0", *n as i64));
            } else {
                out.push_str(&format!("{}", n));
            }
        }
        Expr::String(s) => {
            out.push_str(&rust_string_literal(s));
        }
        Expr::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Expr::Ident(name) => out.push_str(name),

        Expr::Call { callee, args } => {
            // print(x) -> println!("{}", x)
            if let Expr::Ident(name) = callee.as_ref() {
                if name == "print" {
                    out.push_str("println!(\"{}\", ");
                    if let Some(arg) = args.first() {
                        emit_expr(out, arg, ctx);
                    }
                    out.push(')');
                    return;
                }
                // input() -> read one line from stdin and return it as String.
                // (No arguments; if a prompt is desired, print it first.)
                if name == "input" {
                    out.push_str(
                        "{ let mut __arcis_input = String::new(); \
                         std::io::stdin().read_line(&mut __arcis_input).unwrap(); \
                         __arcis_input.trim_end().to_string() }",
                    );
                    return;
                }
            }
            // Builtin `sys.*` (system calls, no import needed).
            if let Expr::Member { object, property } = callee.as_ref() {
                if let Expr::Ident(module) = object.as_ref() {
                    if module == "sys" {
                        emit_sys_call(out, property, args, ctx);
                        return;
                    }
                }
            }
            // Chained array methods: `arr.find(cb)`, `arr.filter(cb)`, etc.
//
// Rust method detail:
//   Vec<T>::iter() → Iterator<Item = &T>
//   .find(cb)   expects Fn(&&T) → bool     pattern |&&x|  → x: &T
//   .filter(cb) expects Fn(&&T) → bool     pattern |&&x|  → x: &T
//   .map(cb)    expects FnMut(T) → U       pattern |&x|   → x: T
//   .fold(init, cb) expects FnMut(B, T) → B  pattern |acc, &x|  → x: T
// User-defined functions take T by value, so we do NOT dereference when
// passing `x` as an argument.
if let Expr::Member { object, property } = callee.as_ref() {
    match property.as_str() {
        "find" => {
            emit_expr(out, object, ctx);
            out.push_str(".iter().find(|&&x| ");
            if let Some(cb) = args.first() {
                emit_callback_call(out, cb, "x");
            }
            out.push_str(").cloned().unwrap_or_default()");
            return;
        }
        "filter" => {
            emit_expr(out, object, ctx);
            out.push_str(".iter().filter(|&&x| ");
            if let Some(cb) = args.first() {
                emit_callback_call(out, cb, "x");
            }
            out.push_str(").cloned().collect()");
            return;
        }
        "map" => {
            emit_expr(out, object, ctx);
            out.push_str(".iter().map(|&x| ");
            if let Some(cb) = args.first() {
                emit_callback_call(out, cb, "x");
            }
            out.push_str(").collect()");
            return;
        }
        "reduce" => {
            emit_expr(out, object, ctx);
            out.push_str(".iter().fold(");
            if let Some(init) = args.get(1) {
                emit_expr(out, init, ctx);
            }
            out.push_str(", |acc, &x| ");
            if let Some(cb) = args.first() {
                emit_callback_call2(out, cb, "acc", "x");
            }
            out.push(')');
            return;
        }
        "pop" => {
            emit_expr(out, object, ctx);
            out.push_str(".pop().unwrap_or_default()");
            return;
        }
        "unshift" => {
            emit_expr(out, object, ctx);
            out.push_str(".insert(0, ");
            if let Some(v) = args.first() {
                emit_expr(out, v, ctx);
            }
            out.push(')');
            return;
        }
        // ── String methods ──────────────────────────────────────────────
        "toUpperCase" => {
            emit_expr(out, object, ctx);
            out.push_str(".to_uppercase()");
            return;
        }
        "toLowerCase" => {
            emit_expr(out, object, ctx);
            out.push_str(".to_lowercase()");
            return;
        }
        "trim" => {
            emit_expr(out, object, ctx);
            out.push_str(".trim().to_string()");
            return;
        }
        "substring" => {
            // s[a as usize..b as usize].to_string()
            emit_expr(out, object, ctx);
            out.push('[');
            if let Some(a) = args.first() {
                emit_expr(out, a, ctx);
                out.push_str(" as usize");
            }
            out.push_str("..");
            if let Some(b) = args.get(1) {
                emit_expr(out, b, ctx);
                out.push_str(" as usize");
            }
            out.push_str("].to_string()");
            return;
        }
        "indexOf" => {
            // s.find(&sub).map(|i| i as f64).unwrap_or(-1.0)
            emit_expr(out, object, ctx);
            out.push_str(".find(&");
            if let Some(sub) = args.first() {
                emit_expr(out, sub, ctx);
            }
            out.push_str(").map(|i| i as f64).unwrap_or(-1.0)");
            return;
        }
        "includes" => {
            // s.contains(&sub) — `&` because String does not implement
            // Pattern but `&str` does.
            emit_expr(out, object, ctx);
            out.push_str(".contains(&");
            if let Some(sub) = args.first() {
                emit_expr(out, sub, ctx);
            }
            out.push(')');
            return;
        }
        "charAt" => {
            // s.chars().nth(i as usize).unwrap_or_default().to_string()
            emit_expr(out, object, ctx);
            out.push_str(".chars().nth(");
            if let Some(i) = args.first() {
                emit_expr(out, i, ctx);
                out.push_str(" as usize");
            }
            out.push_str(").unwrap_or_default().to_string()");
            return;
        }
        _ => {}
    }
}
            // Named function call: f(args)
            if let Expr::Ident(name) = callee.as_ref() {
                out.push_str(name);
                out.push('(');
                for (i, a) in args.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    if let Expr::Ident(ref n) = a {
                        if ctx.types.get(n).map(|t| t.ends_with("[]")).unwrap_or(false) {
                            out.push('&');
                        }
                    }
                    emit_expr(out, a, ctx);
                }
                out.push(')');
                return;
            }
            // Generic call: (callee)(args)
            emit_expr(out, callee, ctx);
            out.push('(');
            for (i, a) in args.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                emit_expr(out, a, ctx);
            }
            out.push(')');
        }

        Expr::Unary { op, operand } => {
            match op {
                UnaryOp::Not => {
                    out.push('!');
                    emit_expr(out, operand, ctx);
                }
                UnaryOp::Neg => {
                    out.push('-');
                    emit_expr(out, operand, ctx);
                }
            }
        }

        Expr::Member { object, property } => {
            // Builtin `sys.args` → program arguments (Vec<String>).
            if let Expr::Ident(module) = object.as_ref() {
                if module == "sys" && property == "args" {
                    out.push_str("std::env::args().collect::<Vec<String>>()");
                    return;
                }
            }
            // `.length` is special-cased based on the type:
            //   string → `.chars().count() as f64` (Unicode codepoints)
            //   T[]    → `.len() as f64` (element count)
            //   other  → `.len() as f64` (default; rustc reports otherwise)
            //
            // We use the type env when the object is an identifier with a
            // declared type.
            emit_expr(out, object, ctx);
            if property == "length" {
                let is_string = matches!(object.as_ref(),
                    Expr::Ident(name) if ctx.types.get(name).map(|t| t == "string").unwrap_or(false));
                let is_array = matches!(object.as_ref(),
                    Expr::Ident(name) if ctx.types.get(name).map(|t| t.ends_with("[]")).unwrap_or(false));
                if is_string {
                    out.push_str(".chars().count() as f64");
                } else {
                    // array or other type: `.len() as f64`
                    out.push_str(".len() as f64");
                }
                let _ = is_array; // currently we only branch on `is_string`
            } else {
                out.push('.');
                out.push_str(property);
            }
        }

        Expr::Index { object, index } => {
            emit_expr(out, object, ctx);
            out.push('[');
            emit_expr(out, index, ctx);
            out.push_str(" as usize]");
        }

        Expr::ArrayLiteral { elements } => {
            if elements.is_empty() {
                // Without a declared type, default to `Vec<f64>`. For other
                // types, declare the type on the let/const (handled in
                // `Stmt::Let`/`Stmt::Const`).
                out.push_str("Vec::<f64>::new()");
            } else {
                out.push_str("vec![");
                for (i, e) in elements.iter().enumerate() {
                    if i > 0 {
                        out.push_str(", ");
                    }
                    emit_expr(out, e, ctx);
                }
                out.push(']');
            }
        }
        Expr::ObjectLiteral { fields } => {
            // Requires that the context (let/const) has declared the type,
            // because we emit `StructName { field: value, ... }` directly.
            if let Some(ty) = ctx.current_let_type {
                if !ty.fields.is_empty() {
                    out.push_str(&ty.name);
                    out.push_str(" { ");
                    let mut emitted = 0;
                    for (k, v) in fields {
                        if emitted > 0 {
                            out.push_str(", ");
                        }
                        out.push_str(k);
                        out.push_str(": ");
                        emit_expr(out, v, ctx);
                        emitted += 1;
                    }
                    out.push_str(" }");
                    return;
                }
            }
            // Without context: emit a tuple as fallback (rustc will complain).
            out.push_str("todo!()");
        }

        Expr::Path { segments } => {
            // `crate::Type::method` → `crate::Type::method`. Without
            // parentheses — if it is a call, the parser wraps it in
            // `Expr::Call`.
            out.push_str(&segments.join("::"));
        }

        Expr::Binary { op, left, right } => {
            match op {
                BinOp::Add => {
                    // Heuristic: if either operand is a string literal, emit `format!`.
                    if has_string_literal(left) || has_string_literal(right) {
                        out.push_str("format!(\"{}{}\", ");
                        emit_expr(out, left, ctx);
                        out.push_str(", ");
                        emit_expr(out, right, ctx);
                        out.push(')');
                    } else {
                        out.push('(');
                        emit_expr(out, left, ctx);
                        out.push_str(" + ");
                        emit_expr(out, right, ctx);
                        out.push(')');
                    }
                }
                BinOp::Sub => {
                    out.push('(');
                    emit_expr(out, left, ctx);
                    out.push_str(" - ");
                    emit_expr(out, right, ctx);
                    out.push(')');
                }
                BinOp::Mul => {
                    out.push('(');
                    emit_expr(out, left, ctx);
                    out.push_str(" * ");
                    emit_expr(out, right, ctx);
                    out.push(')');
                }
                BinOp::Div => {
                    out.push('(');
                    emit_expr(out, left, ctx);
                    out.push_str(" / ");
                    emit_expr(out, right, ctx);
                    out.push(')');
                }
                BinOp::Mod => {
                    out.push('(');
                    emit_expr(out, left, ctx);
                    out.push_str(" % ");
                    emit_expr(out, right, ctx);
                    out.push(')');
                }
                BinOp::EqEq => {
                    out.push('(');
                    emit_expr(out, left, ctx);
                    out.push_str(" == ");
                    emit_expr(out, right, ctx);
                    out.push(')');
                }
                BinOp::NotEq => {
                    out.push('(');
                    emit_expr(out, left, ctx);
                    out.push_str(" != ");
                    emit_expr(out, right, ctx);
                    out.push(')');
                }
                BinOp::Lt => {
                    out.push('(');
                    emit_expr(out, left, ctx);
                    out.push_str(" < ");
                    emit_expr(out, right, ctx);
                    out.push(')');
                }
                BinOp::Gt => {
                    out.push('(');
                    emit_expr(out, left, ctx);
                    out.push_str(" > ");
                    emit_expr(out, right, ctx);
                    out.push(')');
                }
                BinOp::LtEq => {
                    out.push('(');
                    emit_expr(out, left, ctx);
                    out.push_str(" <= ");
                    emit_expr(out, right, ctx);
                    out.push(')');
                }
                BinOp::GtEq => {
                    out.push('(');
                    emit_expr(out, left, ctx);
                    out.push_str(" >= ");
                    emit_expr(out, right, ctx);
                    out.push(')');
                }
                BinOp::And => {
                    out.push('(');
                    emit_expr(out, left, ctx);
                    out.push_str(" && ");
                    emit_expr(out, right, ctx);
                    out.push(')');
                }
                BinOp::Or => {
                    out.push('(');
                    emit_expr(out, left, ctx);
                    out.push_str(" || ");
                    emit_expr(out, right, ctx);
                    out.push(')');
                }
            }
        }
    }
}

/// Returns `true` if the expression contains a string literal at the top
/// level or nested. Used to decide between `+` and `format!`.
fn has_string_literal(expr: &Expr) -> bool {
    match expr {
        Expr::String(_) => true,
        Expr::Binary { left, right, .. } => has_string_literal(left) || has_string_literal(right),
        Expr::Unary { operand, .. } => has_string_literal(operand),
        Expr::Call { args, .. } => args.iter().any(has_string_literal),
        _ => false,
    }
}

/// Emit the callback call for find/filter/map: `cb(x)`. The level of
/// indirection is already handled by the closure pattern (e.g. `|&&x|` for
/// find/filter, `|&x|` for map), so we pass `x` directly. The callback must
/// be an identifier (a previously-declared named function).
fn emit_callback_call(out: &mut String, cb: &Expr, arg: &str) {
    if let Expr::Ident(name) = cb {
        out.push_str(name);
        out.push('(');
        out.push_str(arg);
        out.push(')');
    } else {
        out.push_str("todo!()");
    }
}

/// Emit the callback call for reduce: `cb(acc, x)`.
fn emit_callback_call2(out: &mut String, cb: &Expr, acc: &str, arg: &str) {
    if let Expr::Ident(name) = cb {
        out.push_str(name);
        out.push('(');
        out.push_str(acc);
        out.push_str(", ");
        out.push_str(arg);
        out.push(')');
    } else {
        out.push_str("todo!()");
    }
}

/// Emit `&<expr>` (a reference) for an argument to a builtin `sys.*` call.
/// `read_to_string` / `Path::new` etc. accept `&S` where `S: AsRef<Path>`,
/// so passing a `String` with `&` triggers the right coercion.
fn emit_arg_ref(out: &mut String, e: &Expr, ctx: &Ctx) {
    out.push('&');
    emit_expr(out, e, ctx);
}

/// Builtins `sys.<fn>(...)` — filesystem / `argv` operations. Translate
/// directly to `std::fs::*` / `std::env::*` without requiring an import.
fn emit_sys_call(out: &mut String, property: &str, args: &[Expr], ctx: &Ctx) {
    match property {
        "readFile" => {
            out.push_str("std::fs::read_to_string(");
            if let Some(a) = args.first() {
                emit_arg_ref(out, a, ctx);
            } else {
                out.push_str("&String::new()");
            }
            out.push_str(").unwrap()");
        }
        "writeFile" => {
            out.push_str("std::fs::write(");
            if let Some(a) = args.first() {
                emit_arg_ref(out, a, ctx);
            } else {
                out.push_str("&String::new()");
            }
            out.push_str(", ");
            if let Some(a) = args.get(1) {
                emit_arg_ref(out, a, ctx);
            } else {
                out.push_str("&String::new()");
            }
            out.push_str(").unwrap()");
        }
        "exists" => {
            out.push_str("std::path::Path::new(");
            if let Some(a) = args.first() {
                emit_arg_ref(out, a, ctx);
            } else {
                out.push_str("&String::new()");
            }
            out.push_str(").exists()");
        }
        "deleteFile" => {
            out.push_str("std::fs::remove_file(");
            if let Some(a) = args.first() {
                emit_arg_ref(out, a, ctx);
            } else {
                out.push_str("&String::new()");
            }
            out.push_str(").unwrap()");
        }
        "mkdir" => {
            out.push_str("std::fs::create_dir(");
            if let Some(a) = args.first() {
                emit_arg_ref(out, a, ctx);
            } else {
                out.push_str("&String::new()");
            }
            out.push_str(").unwrap()");
        }
        "listDir" => {
            out.push_str("std::fs::read_dir(");
            if let Some(a) = args.first() {
                emit_arg_ref(out, a, ctx);
            } else {
                out.push_str("&String::new()");
            }
            out.push_str(
                ").unwrap().map(|__arcis_e| __arcis_e.unwrap().file_name().to_string_lossy().into_owned()).collect::<Vec<String>>()",
            );
        }
        _ => {
            // Unknown `sys.X`: emit as-is (rustc will report).
            out.push_str("sys.");
            out.push_str(property);
            out.push('(');
            for (i, a) in args.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                emit_expr(out, a, ctx);
            }
            out.push(')');
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_modules_produces_empty_rust() {
        // A trivial smoke test: `generate_all` with no modules must not panic.
        let result = generate_all(&[]);
        assert!(result.is_ok());
    }
}