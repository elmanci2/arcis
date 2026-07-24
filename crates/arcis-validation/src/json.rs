//! Compile-time JSON → [`Type`] shape inference, for the `json(path)`
//! builtin.
//!
//! When a `json(...)` call's path argument is a string literal, the
//! compiler reads that real file off disk (at both driver-build time and
//! LSP-typing time) and infers an Arcis type from its actual structure —
//! the same way `let o = { a: 1 };` already infers `o`'s type from a
//! literal object expression. This module is the JSON-specific half of
//! that: it turns a `serde_json::Value` into the same `Vec<(String,
//! Box<Type>, bool)>` shape [`crate::infer`]'s `Expr::ObjectLiteral`
//! handling already builds from AST literals, and reuses
//! [`arcis_ast::object_type_name`] for the hash-based struct naming so a
//! JSON-inferred shape dedupes with an identically-shaped object literal or
//! another JSON file with the same structure.
//!
//! Resolution is **CWD-relative**, deliberately matching the existing
//! runtime convention `sys.readFile` already uses (see
//! `arcis-codegen/src/sys/fs.rs`) rather than the module-import convention
//! (relative to the `.tsr` file's own directory) — this needs no path
//! plumbing through the LSP or codegen, since the process's current
//! directory is already ambient everywhere.

use arcis_ast::{object_type_name, Type};

/// Read and parse the JSON file at `path` (resolved relative to the
/// process's current directory) and infer an Arcis [`Type`] from its
/// top-level value. Never panics — any I/O or parse failure is an `Err`,
/// letting callers (the LSP, in particular) degrade gracefully rather than
/// crashing on every keystroke of a file with a currently-bad path.
pub fn infer_json_type(path: &str) -> Result<Type, String> {
    let full_path = std::env::current_dir()
        .map_err(|e| format!("could not determine the current directory: {e}"))?
        .join(path);
    let text = std::fs::read_to_string(&full_path)
        .map_err(|e| format!("could not read `{}`: {e}", full_path.display()))?;
    let value: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| format!("`{}` is not valid JSON: {e}", full_path.display()))?;
    Ok(value_to_type(&value))
}

/// Recursively convert a parsed JSON value into an Arcis [`Type`].
fn value_to_type(value: &serde_json::Value) -> Type {
    match value {
        serde_json::Value::Null => {
            // A null sample reveals nothing about the field's real type —
            // `string?` is a documented best-guess placeholder, not a
            // meaningful inference.
            Type::optional(Type::string())
        }
        serde_json::Value::Bool(_) => Type::boolean(),
        serde_json::Value::Number(_) => Type::number(),
        serde_json::Value::String(_) => Type::string(),
        serde_json::Value::Array(items) => {
            // Element type from the first item, matching the existing
            // empty-array-literal convention (defaults to `number`, see
            // `arcis-codegen/src/expr.rs`'s `emit_array_literal`).
            let elem = items.first().map(value_to_type).unwrap_or_else(Type::number);
            Type::array(elem)
        }
        serde_json::Value::Object(fields) => {
            let typed: Vec<(String, Box<Type>, bool)> =
                fields.iter().map(|(k, v)| (k.clone(), Box::new(value_to_type(v)), false)).collect();
            let name = object_type_name(&typed);
            Type::Object { name, fields: typed }
        }
    }
}
