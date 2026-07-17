//! Codegen: traduce el AST a código fuente Rust.
//!
//! Mapeo de tipos:
//!   string  → String
//!   number  → f64
//!   boolean → bool
//!   void    → ()
//!
//! Concatenación:
//!   El operador `+` se traduce a `format!("{}{}", a, b)` cuando AL MENOS UN
//!   operando es un literal string (heurística simple). En otro caso genera
//!   `a + b` directo (number + number, bool + bool, etc.).
//!
//! Print:
//!   `print(expr)` se traduce a `println!("{}", expr)`. Si la expresión ya
//!   es un `format!`, queda anidado y funciona igual.

use crate::ast::{BinOp, ExportDefault, Expr, Function, Program, Stmt, Type, UnaryOp};
use crate::modules::{resolve_specifier, Module, ModuleTarget};
use std::collections::{HashMap, HashSet};

/// Contexto pasado a las funciones de emisión: variables reasignadas y
/// mapa de tipos declarados (para `.length` sobre Ident).
struct Ctx<'a> {
    reassigned: &'a HashSet<String>,
    types: &'a HashMap<String, String>,
    /// Tipo declarado del let/const que contiene la expresión ObjectLiteral
    /// que se está emitiendo. Es `None` para expresiones sueltas; en ese
    /// caso el codegen emite un error en runtime (rustc).
    current_let_type: Option<&'a Type>,
    /// true si estamos generando el módulo raíz (`main`), donde se definen
    /// los structs de tipos objeto. En módulos no-raíz, las referencias a
    /// esos tipos llevan prefijo `crate::`.
    is_root: bool,
}

/// Genera el código Rust de todos los módulos del programa.
/// Devuelve `(id, fuente_rust)` por módulo; el primer elemento es la raíz.
pub fn generate_all(modules: &[Module]) -> Result<Vec<(String, String)>, String> {
    // Structs de tipos objeto centralizados en la raíz (dedup por nombre).
    let all_obj_types = collect_all_object_types(modules);

    // Mapa path canónico → id, para resolver los `use crate::<id>::...`.
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

    // La raíz declara todos los submódulos y define los structs de tipos objeto.
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

    // Tipos objeto referenciados en este módulo → `use crate::__Obj...;`
    // (necesario en módulos no-root porque los structs viven en el root).
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

    // Funciones (pub si exportadas) y default-export como items top-level.
    for stmt in &m.program.stmts {
        emit_top_item(&mut out, stmt, &ctx);
    }

    if is_root {
        // fn main() con los lets/consts y exprs top-level (las funciones van
        // arriba como items).
        out.push_str("fn main() {\n");
        for stmt in &m.program.stmts {
            emit_stmt(&mut out, stmt, 1, &ctx);
        }
        out.push_str("}\n");
    } else {
        // En un módulo no-raíz los lets/consts top-level se emiten como
        // items `const` (Rust exige const-eval para items; ver README).
        for stmt in &m.program.stmts {
            emit_module_const(&mut out, stmt, &ctx);
        }
    }

    Ok(out)
}

/// `use crate::<id>::<name> [as <local>];` por cada binding importado.
/// Para crates externos (`crate:<name>`) emite `use <crate>::<name>;`.
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
                    // El linker ya rechazó `default.is_some()` para crates.
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
                        format!("módulo `{}` no resuelto a un id (error interno)", spec)
                    })?;
                    if let Some(local) = default {
                        let target = modules
                            .iter()
                            .find(|m| m.path == dep_path)
                            .expect("módulo destino cargado");
                        let default_name =
                            target.exports.default.clone().ok_or_else(|| {
                                format!("`{}` no tiene default export", spec)
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

/// Emite funciones top-level (suelta o exportadas) y el default-export,
/// como items. Lets/consts no se tratan aquí.
fn emit_top_item(out: &mut String, stmt: &Stmt, ctx: &Ctx) {
    match stmt {
        Stmt::Function(f) => emit_function(out, f, ctx, false),
        Stmt::ExportDecl(inner) => {
            if let Stmt::Function(f) = inner.as_ref() {
                emit_function(out, f, ctx, true);
            }
            // lets/consts exportados: en raíz van a main(), en no-raíz son
            // const items (emit_module_const).
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

/// Lets/consts top-level de un módulo no-raíz como items `const`.
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
/// Solo funciona si `expr` es const-evaluable (rustc lo exigirá).
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

/// Inferencia simple de tipos Rust para literales (usado en `export default`).
fn infer_expr_rust_type(e: &Expr) -> Option<String> {
    match e {
        Expr::Number(_) => Some("f64".into()),
        Expr::String(_) => Some("String".into()),
        Expr::Bool(_) => Some("bool".into()),
        Expr::ArrayLiteral { elements } => {
            let inner = elements.first().and_then(infer_expr_rust_type).unwrap_or_else(|| "f64".into());
            Some(format!("Vec<{}>", inner))
        }
        _ => None,
    }
}

/// Recolecta los NOMBRES de tipos objeto referenciados en una stmt (recorre
/// el árbol, deduplicando). Usado para emitir `use crate::__Obj...;` en
/// módulos no-root.
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

/// Recoge los tipos objeto de TODOS los módulos (para centralizarlos en la
/// raíz). Dedup por nombre (el nombre ya es determinista por hash).
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

/// Recorre el programa y devuelve el conjunto de nombres que se reasignan
/// al menos una vez (sea en main o dentro de una función).
fn collect_reassigned(program: &Program) -> HashSet<String> {
    let mut set = HashSet::new();
    for stmt in &program.stmts {
        collect_in_stmt(stmt, &mut set);
    }
    set
}

/// Construye un mapa nombre → tipo declarado. Se usa en `.length` para
/// distinguir entre string y array cuando el objeto es un identificador.
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
            for s in then_branch { collect_types_stmt(s, map); }
            if let Some(eb) = else_branch {
                for s in eb { collect_types_stmt(s, map); }
            }
        }
        Stmt::While { body, .. } | Stmt::For { body, .. } => {
            for s in body { collect_types_stmt(s, map); }
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
            // Marcar el Ident raíz (descendiendo por Index/Member) como
            // reasignado. Para `arr[i].x = v` también marcamos `arr`.
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
            for s in then_branch { collect_in_stmt(s, set); }
            if let Some(eb) = else_branch {
                for s in eb { collect_in_stmt(s, set); }
            }
        }
        Stmt::While { body, .. } => {
            for s in body { collect_in_stmt(s, set); }
        }
        Stmt::For { init, update, body, .. } => {
            if let Some(init) = init {
                collect_in_stmt(init, set);
            }
            if let Some(update) = update {
                collect_in_stmt(update, set);
            }
            for s in body { collect_in_stmt(s, set); }
        }
        Stmt::ForOf { iterable, body, .. } => {
            collect_mutation_in_expr(iterable, set);
            for s in body { collect_in_stmt(s, set); }
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

/// Detecta llamadas a métodos que mutan el array (`pop`, `unshift`) para
/// marcar el receptor como `let mut`. Los métodos sin efecto (`find`,
/// `filter`, `map`, `reduce`) NO mutan, así que no se marcan.
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

/// Desciende por Index/Member hasta encontrar el Ident más externo de una
/// expresión de asignación, marcándolo como reasignado.
fn mark_ident_root_mutated(expr: &Expr, set: &mut HashSet<String>) {
    match expr {
        Expr::Ident(name) => { set.insert(name.clone()); }
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
        // Array de T: `Vec<T>` donde T es el tipo interno (con sus fields).
        let inner = Type {
            name: ty.name.clone(),
            fields: ty.fields.clone(),
            is_array: false,
        };
        return format!("Vec<{}>", ts_type_to_rust(&inner, is_root));
    }
    // Tipo objeto: el struct vive en la raíz; desde un submódulo se referencia
    // como `crate::__ObjNAME`.
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
        if i > 0 { out.push_str(", "); }
        out.push_str(&p.name);
        out.push_str(": ");
        // Arrays se pasan por referencia para no consumir el argumento
        // (semántica de TS: pasar un array a una función no lo invalida).
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
            // `let` en TS permite reasignación. Solo emitimos `mut` si esta
            // variable se reasigna en algún punto del programa.
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
            // Caso especial: `let x: T[] = []` — `vec![]` no infiere tipo,
            // así que emitimos `Vec::new()` y dejamos que el tipo declarado
            // guíe la inferencia (Rust deduce el parámetro genérico).
            if let (Some(t), Expr::ArrayLiteral { elements }) = (ty, value) {
                if elements.is_empty() && t.is_array {
                    out.push_str("Vec::new()");
                    out.push_str(";\n");
                    return;
                }
            }
            // Pasar el tipo declarado al contexto para ObjectLiteral.
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
            // Rust `const` requiere valor constante en tiempo de compilación,
            // no funciona para valores calculados en runtime. Usamos `let`
            // con nombre en MAYÚSCULAS como aproximación simple (inmutable).
            indent(out, level);
            out.push_str("let ");
            out.push_str(name);
            if let Some(t) = ty {
                out.push_str(": ");
                out.push_str(&ts_type_to_rust(t, ctx.is_root));
            }
            out.push_str(" = ");
            // Mismo caso especial que Let: array literal vacío con tipo.
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
            // Las funciones se emitieron antes de main; las ignoramos aquí.
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
            out.push_str("}");
            if let Some(eb) = else_branch {
                out.push_str(" else {\n");
                for s in eb {
                    emit_stmt(out, s, level + 1, ctx);
                }
                indent(out, level);
                out.push_str("}");
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
            // Para un identificador (variable local tipo Vec) usamos
            // `.iter().cloned()` para no consumirlo. Para llamadas, accesos a
            // miembro o accesos por índice (típicamente iteradores de crates
            // como `server.incoming_requests()`) usamos `.into_iter()` porque
            // los `Iterator` no tienen `.iter()`.
            indent(out, level);
            out.push_str("for ");
            out.push_str(name);
            out.push_str(" in ");
            let use_iter = matches!(
                iterable.as_ref(),
                Expr::Ident(_)
            );
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
            let _ = ty; // el tipo declarado en `let x: T of arr` se ignora por ahora
        }
        Stmt::For { init, condition, update, body } => {
            // Desazucarado usando `loop` con flag para que `continue` ejecute
            // el update antes de la siguiente iteración:
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
            // En la raíz, los lets/consts exportados viven en main() igual que
            // los no exportados; las funciones/default se emiten aparte como
            // items (emit_top_item).
            match inner.as_ref() {
                s @ (Stmt::Let { .. } | Stmt::Const { .. }) => emit_stmt(out, s, level, ctx),
                _ => {}
            }
        }
        Stmt::Import { .. } | Stmt::ExportSpec(_) | Stmt::ExportDefault(_) => {
            // La estructura de módulos (mod/use/pub) se genera fuera de emit_stmt.
        }
    }
}

fn emit_expr(out: &mut String, expr: &Expr, ctx: &Ctx) {
    match expr {
        Expr::Number(n) => {
            // Siempre emitimos con punto decimal para que Rust lo tome como f64.
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
                // input() -> lee una línea de stdin y la devuelve como String.
                // (Sin argumentos; si se quiere prompt, usar `print(...)` antes.)
                if name == "input" {
                    out.push_str(
                        "{ let mut __arcis_input = String::new(); \
                         std::io::stdin().read_line(&mut __arcis_input).unwrap(); \
                         __arcis_input.trim_end().to_string() }",
                    );
                    return;
                }
            }
            // Builtins `sys.*` (operaciones de sistema, sin import).
            if let Expr::Member { object, property } = callee.as_ref() {
                if let Expr::Ident(module) = object.as_ref() {
                    if module == "sys" {
                        emit_sys_call(out, property, args, ctx);
                        return;
                    }
                }
            }
            // Métodos array encadenados: arr.find(cb), arr.filter(cb), etc.
// Detalle de tipos de Rust:
//   Vec<T>::iter() → Iterator<Item = &T>
//   .find(cb)  espera Fn(&&T) → bool     patrón |&&x|  → x: &T
//   .filter(cb) espera Fn(&&T) → bool    patrón |&&x|  → x: &T
//   .map(cb)   espera FnMut(T) → U       patrón |&x|   → x: T
//   .fold(init, cb) espera FnMut(B, T) → B  patrón |acc, &x|  → x: T
// Las funciones de usuario reciben T por valor, así que NO desreferenciamos
// al pasar `x` como argumento.
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
        // ===== Métodos de string =====
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
            // s.contains(&sub) — `&` porque String no implementa Pattern,
            // pero &str sí.
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
            // Llamada a función con nombre: f(args)
            if let Expr::Ident(name) = callee.as_ref() {
                out.push_str(name);
                out.push('(');
                for (i, a) in args.iter().enumerate() {
                    if i > 0 { out.push_str(", "); }
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
            // Llamada genérica: (callee)(args)
            emit_expr(out, callee, ctx);
            out.push('(');
            for (i, a) in args.iter().enumerate() {
                if i > 0 { out.push_str(", "); }
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
            // Builtin `sys.args` → argumentos del programa (Vec<String>).
            if let Expr::Ident(module) = object.as_ref() {
                if module == "sys" && property == "args" {
                    out.push_str("std::env::args().collect::<Vec<String>>()");
                    return;
                }
            }
            // Caso especial: `.length` se traduce distinto según el tipo:
            //   string → `.chars().count() as f64` (caracteres Unicode)
            //   T[]    → `.len() as f64` (cantidad de elementos)
            //   otro   → `.len() as f64` (default; rustc reportará si no aplica)
            //
            // Para decidir string vs array usamos el type_env cuando el
            // objeto es un identificador con tipo declarado.
            emit_expr(out, object, ctx);
            if property == "length" {
                let is_string = matches!(object.as_ref(), Expr::Ident(name) if ctx.types.get(name).map(|t| t == "string").unwrap_or(false));
                let is_array = matches!(object.as_ref(), Expr::Ident(name) if ctx.types.get(name).map(|t| t.ends_with("[]")).unwrap_or(false));
                if is_string {
                    out.push_str(".chars().count() as f64");
                } else {
                    // array u otro tipo: `.len() as f64`
                    out.push_str(".len() as f64");
                }
                let _ = is_array; // por ahora solo usamos is_string; is_array cae al else
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
                // Sin contexto de tipo, default a Vec<f64>. Para otro tipo,
                // declarar el tipo en el let/const (ya manejado en Stmt::Let/Const).
                out.push_str("Vec::<f64>::new()");
            } else {
                out.push_str("vec![");
                for (i, e) in elements.iter().enumerate() {
                    if i > 0 { out.push_str(", "); }
                    emit_expr(out, e, ctx);
                }
                out.push(']');
            }
        }
        Expr::ObjectLiteral { fields } => {
            // Requiere que el contexto (let/const) haya declarado el tipo,
            // porque emitimos `StructName { campo: valor, ... }` directo.
            if let Some(ty) = ctx.current_let_type {
                if !ty.fields.is_empty() {
                    out.push_str(&ty.name);
                    out.push_str(" { ");
                    let mut emitted = 0;
                    for (k, v) in fields {
                        if emitted > 0 { out.push_str(", "); }
                        out.push_str(k);
                        out.push_str(": ");
                        emit_expr(out, v, ctx);
                        emitted += 1;
                    }
                    out.push_str(" }");
                    return;
                }
            }
            // Sin contexto: emite una tupla como fallback (rustc se quejará).
            out.push_str("todo!()");
        }

        Expr::Path { segments } => {
            // `crate::Type::method` → `crate::Type::method`. Sin paréntesis;
            // si es una llamada, el parser lo envolvió en `Expr::Call`.
            out.push_str(&segments.join("::"));
        }

        Expr::Binary { op, left, right } => {
            match op {
                BinOp::Add => {
                    // Heurística: si alguno es literal string, generar format!
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

/// Devuelve true si la expresión contiene un literal string en su nivel
/// superior o anidada. Usado para decidir entre `+` y `format!`.
fn has_string_literal(expr: &Expr) -> bool {
    match expr {
        Expr::String(_) => true,
        Expr::Binary { left, right, .. } => has_string_literal(left) || has_string_literal(right),
        Expr::Unary { operand, .. } => has_string_literal(operand),
        Expr::Call { args, .. } => args.iter().any(has_string_literal),
        _ => false,
    }
}

/// Emite la llamada al callback para find/filter/map: `cb(x)`.
/// El nivel de indirección ya quedó resuelto en el patrón de la closure
/// (p.ej. `|&&x|` para find/filter, `|&x|` para map), así que pasamos `x`
/// directo. El callback debe ser un Ident (función con nombre ya definida).
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

/// Emite la llamada al callback para reduce: `cb(acc, x)`.
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

/// Emite `&<expr>` (ref) para un argumento de los builtins `sys.*`.
/// `read_to_string`/`Path::new`/etc. aceptan `&S` donde `S: AsRef<Path>`,
/// así que pasar un `String` con `&` fuerza la coerción correcta.
fn emit_arg_ref(out: &mut String, e: &Expr, ctx: &Ctx) {
    out.push('&');
    emit_expr(out, e, ctx);
}

/// Builtins `sys.<fn>(...)` — operaciones del sistema de archivos y argv.
/// Se traducen directamente a `std::fs::*` / `std::env::*` sin requerir import.
fn emit_sys_call(
    out: &mut String,
    property: &str,
    args: &[Expr],
    ctx: &Ctx,
) {
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
            // sys.X desconocido: emitimos tal cual (rustc se quejará).
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