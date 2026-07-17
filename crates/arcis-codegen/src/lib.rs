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

/// Generate the Rust source code for every module of the program.
/// Returns `(id, rust_source)` per module; the first element is the root.
pub fn generate_all(modules: &[Module]) -> Result<Vec<(String, String)>, String> {
    use std::collections::HashMap;

    // Object-type structs are centralised in the root module (deduped by name).
    let mut all_obj_types = collect::collect_all_object_types(modules);

    // `sys.process(cmd, args)` returns an `ArcisProcess { stdout, stderr, exitCode }`
    // value. Codegen always emits a literal of this type, so the struct definition
    // must always be present in the root module — even for programs that don't
    // reference it (the linker will discard the unused type).
    all_obj_types.push(arcis_ast::Type {
        name: "ArcisProcess".to_string(),
        fields: vec![
            (
                "stdout".to_string(),
                Box::new(arcis_ast::Type {
                    name: "string".to_string(),
                    fields: Vec::new(),
                    is_array: false,
                }),
            ),
            (
                "stderr".to_string(),
                Box::new(arcis_ast::Type {
                    name: "string".to_string(),
                    fields: Vec::new(),
                    is_array: false,
                }),
            ),
            (
                "exitCode".to_string(),
                Box::new(arcis_ast::Type {
                    name: "number".to_string(),
                    fields: Vec::new(),
                    is_array: false,
                }),
            ),
        ],
        is_array: false,
    });

    // Canonical-path → id map, for resolving `use crate::<id>::...`.
    let path_to_id: HashMap<std::path::PathBuf, String> = modules
        .iter()
        .map(|m| (m.path.clone(), m.id.clone()))
        .collect();

    let mut out = Vec::with_capacity(modules.len());
    for (i, m) in modules.iter().enumerate() {
        let is_root = i == 0;
        let src = module::generate(m, is_root, modules, &all_obj_types, &path_to_id)?;
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