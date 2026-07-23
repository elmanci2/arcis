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
//! ## Layout
//!
//! - [`context`](self::context) — `Ctx` shared by every emitter.
//! - [`collect`](self::collect) — pre-passes: reassigned vars, type map,
//!   object-type names.
//! - [`types`](self::types) — type / literal helpers (`ts_type_to_rust`,
//!   `rust_string_literal`, `infer_expr_rust_type`, struct emit).
//! - [`module`](self::module) — per-module emission (imports, exports, top
//!   items, `fn main()` body or `const` items).
//! - [`function`](self::function) — `fn` emission.
//! - [`stmt`](self::stmt) — statement emission.
//! - [`expr`](self::expr) — expression emission (literals, calls, member,
//!   index, binary, etc.).
//! - [`builtin`](self::builtin) — `print`, `input`, callback helpers.
//! - [`method`](self::method) — array / string method dispatch.
//! - [`sys`](self::sys) — `sys.*` builtins, split across
//!   [`sys::fs`](self::sys::fs) (file/dir IO), [`sys::path`](self::sys::path)
//!   (path queries), and [`sys::env`](self::sys::env) (process/argv).

use arcis_linker::Module;

mod builtin;
mod collect;
mod context;
mod expr;
mod function;
mod method;
mod module;
mod stmt;
mod sys;
mod types;

/// Resolve every interface name, type alias, and enum name used in *type
/// position* across all modules, rewriting `Type::Named(...)` to the
/// underlying type (`Type::Object` for interfaces/object aliases, `number`
/// for enums, etc.). Backend-independent: the Rust backend runs this inside
/// [`generate_all`], and the driver runs it before handing modules to the
/// Cranelift backend so both see the same fully-resolved ASTs.
pub fn resolve_program_types(modules: &[Module]) -> Vec<Module> {
    use std::collections::HashMap;

    let interfaces = collect::collect_interfaces(modules);
    let interfaces_map: HashMap<String, arcis_ast::Type> = interfaces
        .iter()
        .map(|t| (t.struct_name().unwrap_or_default().to_string(), t.clone()))
        .collect();
    let global_aliases = collect::collect_global_aliases(modules);
    let mut extra = interfaces_map;
    for (name, ty) in global_aliases {
        extra.entry(name).or_insert(ty);
    }
    for (name, _) in collect::collect_enums(modules) {
        extra.entry(name).or_insert_with(arcis_ast::Type::number);
    }
    modules
        .iter()
        .map(|m| {
            let mut m = m.clone();
            collect::resolve_type_aliases(&mut m.program, &extra);
            m
        })
        .collect()
}

/// Generate the Rust source code for every module of the program.
/// Returns `(id, rust_source)` per module; the first element is the root.
pub fn generate_all(modules: &[Module]) -> Result<Vec<(String, String)>, String> {
    use std::collections::HashMap;

    // `interface` declarations become `pub struct`s in the root, named after
    // the interface itself (not hash-based), with `extends` chains already
    // merged. Computed from the *original* modules since interface field
    // shapes don't depend on alias resolution.
    let interfaces = collect::collect_interfaces(modules);
    let interfaces_map: HashMap<String, arcis_ast::Type> = interfaces
        .iter()
        .map(|t| (t.struct_name().unwrap_or_default().to_string(), t.clone()))
        .collect();

    // `type X = ...;` has no runtime representation of its own — resolve
    // every reference to a type alias, AND every reference to an interface
    // name, to its underlying `Type::Object` before any other pass runs, so
    // the rest of codegen never has to special-case `Type::Named` pointing
    // at either kind of declaration (object-literal emission only knows how
    // to read fields off `Type::Object`). Top-level aliases are collected
    // globally so they resolve across module boundaries; a module's own
    // (function-local) aliases still take priority on a name collision.
    let global_aliases = collect::collect_global_aliases(modules);
    let mut extra = interfaces_map.clone();
    for (name, ty) in &global_aliases {
        extra.entry(name.clone()).or_insert_with(|| ty.clone());
    }
    // Enum names used in TYPE position erase to `number` (TS numeric-enum
    // semantics; the variants themselves are `f64` consts — see
    // `types::emit_enum_def`). Collected from the original modules since
    // enum declarations aren't affected by alias resolution.
    for (name, _) in collect::collect_enums(modules) {
        extra.entry(name).or_insert_with(arcis_ast::Type::number);
    }
    let modules: Vec<Module> = modules
        .iter()
        .map(|m| {
            let mut m = m.clone();
            collect::resolve_type_aliases(&mut m.program, &extra);
            m
        })
        .collect();
    let modules = modules.as_slice();

    // Object-type structs are centralised in the root module. `interfaces`
    // and `collect_all_object_types` are collected independently and can
    // both surface the same interface shape (e.g. `let x: Dog = ...` makes
    // the resolved `Type::Object { name: "Dog", .. }` visible to the latter
    // too), so dedup the combined list by struct name, keeping the first
    // occurrence — the interface's own field order and shape.
    let mut all_obj_types = interfaces;
    let mut seen_struct_names: std::collections::HashSet<String> =
        all_obj_types.iter().filter_map(|t| t.struct_name()).map(str::to_string).collect();
    for ty in collect::collect_all_object_types(modules) {
        if let Some(name) = ty.struct_name() {
            if seen_struct_names.insert(name.to_string()) {
                all_obj_types.push(ty);
            }
        }
    }

    // `sys.process(cmd, args)` returns an `ArcisProcess { stdout, stderr, exitCode }`
    // value. Codegen always emits a literal of this type, so the struct definition
    // must always be present in the root module — even for programs that don't
    // reference it (the linker will discard the unused type).
    all_obj_types.push(arcis_ast::Type::Object {
        name: "ArcisProcess".to_string(),
        fields: vec![
            ("stdout".to_string(), Box::new(arcis_ast::Type::string()), false),
            ("stderr".to_string(), Box::new(arcis_ast::Type::string()), false),
            ("exitCode".to_string(), Box::new(arcis_ast::Type::number()), false),
        ],
    });

    // `enum` declarations become `pub enum`s in the root module.
    let enums = collect::collect_enums(modules);
    let enum_names: std::collections::HashSet<String> =
        enums.iter().map(|(name, _)| name.clone()).collect();

    // Every type-level name (interface / alias / enum): importing one is
    // valid Arcis, but emits no Rust `use` (they live in the root or are
    // erased entirely).
    let mut type_level_names: std::collections::HashSet<String> =
        interfaces_map.keys().cloned().collect();
    type_level_names.extend(global_aliases.keys().cloned());
    type_level_names.extend(enum_names.iter().cloned());

    // Canonical-path → id map, for resolving `use crate::<id>::...`.
    let path_to_id: HashMap<std::path::PathBuf, String> = modules
        .iter()
        .map(|m| (m.path.clone(), m.id.clone()))
        .collect();

    let mut out = Vec::with_capacity(modules.len());
    for (i, m) in modules.iter().enumerate() {
        let is_root = i == 0;
        let src = module::generate(
            m,
            is_root,
            modules,
            &all_obj_types,
            &enums,
            &enum_names,
            &type_level_names,
            &path_to_id,
        )?;
        out.push((m.id.clone(), src));
    }
    Ok(out)
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