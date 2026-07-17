//! Type / literal helpers.
//!
//! - [`ts_type_to_rust`] maps an Arcis type to its Rust equivalent.
//! - [`rust_string_literal`] emits a Rust `String::from("…")` literal.
//! - [`infer_expr_rust_type`] returns a Rust type for a literal expression
//!   (used by `export default`).
//! - [`emit_struct_def`] writes a `pub struct` for an inline object type.
//! - [`indent`] is the universal indentation helper.

use arcis_ast::{Expr, Type};

use crate::context::Ctx;

/// Map an Arcis [`Type`] to its Rust equivalent.
///
/// `is_root` controls how object types are referenced: from the root module
/// they appear as bare names, from a non-root module they get a `crate::`
/// prefix because the struct lives in the root.
pub(crate) fn ts_type_to_rust(ty: &Type, is_root: bool) -> String {
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

/// Emit a `String::from("…")` literal for a Rust source string. Escapes
/// backslash, double-quote, and the common whitespace sequences.
pub(crate) fn rust_string_literal(s: &str) -> String {
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

/// Simple Rust type inference for literals (used by `export default`).
pub(crate) fn infer_expr_rust_type(e: &Expr) -> Option<String> {
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

/// Emit a `pub struct <name> { pub field: type, ... }` for an inline
/// object type. Called only from the root module.
pub(crate) fn emit_struct_def(out: &mut String, ty: &Type) {
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

/// Append `level` levels (4-space) of indentation to `out`.
pub(crate) fn indent(out: &mut String, level: usize) {
    for _ in 0..level {
        out.push_str("    ");
    }
}

/// Convenience wrapper so other modules can call `emit_struct_def` without
/// having to import [`Ctx`] themselves.
pub(crate) fn _ctx_marker(_: &Ctx) {}