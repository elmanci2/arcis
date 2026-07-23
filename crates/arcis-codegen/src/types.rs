//! Type / literal helpers.
//!
//! - [`ts_type_to_rust`] maps an Arcis type to its Rust equivalent.
//! - [`rust_string_literal`] emits a Rust `String::from("…")` literal.
//! - [`infer_expr_rust_type`] returns a Rust type for a literal expression
//!   (used by `export default`).
//! - [`emit_struct_def`] writes a `pub struct` for an inline object type.
//! - [`indent`] is the universal indentation helper.
//!
//! ## Type erasure
//!
//! Arcis has no runtime type checker — annotations are compile-time-only
//! hints, same as TypeScript's own erasure model. Consistently with that:
//! - Literal types (`"left"`, `42`, `true`) erase to their base primitive
//!   (`String`, `f64`, `bool`).
//! - Union and intersection types erase to the Rust type of their first
//!   member — there is no tagged-union runtime, so a real sum type isn't
//!   synthesised in v1. This mirrors how the language already treats
//!   declared types as advisory rather than verified.

use arcis_ast::{Expr, LiteralValue, Type};

use crate::context::Ctx;

/// Map an Arcis [`Type`] to its Rust equivalent.
///
/// `is_root` controls how object types are referenced: from the root module
/// they appear as bare names, from a non-root module they get a `crate::`
/// prefix because the struct lives in the root.
pub(crate) fn ts_type_to_rust(ty: &Type, is_root: bool) -> String {
    match ty {
        Type::Array(inner) => format!("Vec<{}>", ts_type_to_rust(inner, is_root)),
        Type::Object { name, .. } => {
            if is_root {
                name.clone()
            } else {
                format!("crate::{}", name)
            }
        }
        Type::Primitive(name) => match name.as_str() {
            "string" => "String".to_string(),
            "number" => "f64".to_string(),
            "boolean" => "bool".to_string(),
            "void" => "()".to_string(),
            // `any` has no runtime representation to erase to safely; a
            // boxed dynamic value is the closest native equivalent.
            "any" => "Box<dyn std::any::Any>".to_string(),
            "bigint" => "i64".to_string(),
            other => other.to_string(),
        },
        // `null` / `undefined` have no dedicated Arcis runtime value yet;
        // erase to unit, matching `void`.
        Type::Null | Type::Undefined => "()".to_string(),
        Type::Literal(LiteralValue::String(_)) => "String".to_string(),
        Type::Literal(LiteralValue::Number(_)) => "f64".to_string(),
        Type::Literal(LiteralValue::Bool(_)) => "bool".to_string(),
        Type::Union(members) | Type::Intersection(members) => members
            .first()
            .map(|m| ts_type_to_rust(m, is_root))
            .unwrap_or_else(|| "()".to_string()),
        Type::Named(name) => name.clone(),
        Type::Optional(inner) => format!("Option<{}>", ts_type_to_rust(inner, is_root)),
        Type::Function { params, return_type } => {
            let ps: Vec<String> = params.iter().map(|p| ts_type_to_rust(p, is_root)).collect();
            format!("fn({}) -> {}", ps.join(", "), ts_type_to_rust(return_type, is_root))
        }
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
                .iter()
                .find_map(|e| match e {
                    arcis_ast::ArrayElement::Item(e) | arcis_ast::ArrayElement::Spread(e) => {
                        infer_expr_rust_type(e)
                    }
                })
                .unwrap_or_else(|| "f64".into());
            Some(format!("Vec<{}>", inner))
        }
        _ => None,
    }
}

/// Emit a `pub struct <name> { pub field: type, ... }` for an inline
/// object type. Called only from the root module. Optional fields (`?`)
/// become `Option<T>`.
pub(crate) fn emit_struct_def(out: &mut String, ty: &Type) {
    let (name, fields) = match ty {
        Type::Object { name, fields } => (name, fields),
        _ => return,
    };
    // `Default` backs `.find()`'s not-found value and `.pop()` on empty
    // arrays; `Debug` gives `any`-ish printing a fallback.
    out.push_str("#[derive(Clone, Debug, Default)]\n");
    out.push_str("pub struct ");
    out.push_str(name);
    out.push_str(" {\n");
    for (k, fty, optional) in fields {
        out.push_str("    pub ");
        out.push_str(k);
        out.push_str(": ");
        // The parser already folds `field?: T` into `Type::Optional(T)`
        // (see `parse_interface`/`parse_object_type`), so `ts_type_to_rust`
        // alone produces `Option<T>` — wrapping again here would double it.
        if *optional && !fty.is_optional() {
            out.push_str(&format!("Option<{}>", ts_type_to_rust(fty, true)));
        } else {
            out.push_str(&ts_type_to_rust(fty, true));
        }
        out.push_str(",\n");
    }
    out.push_str("}\n\n");
}

/// Emit a `pub enum Name { A, B = 5, C }` for an Arcis `enum` declaration.
/// Called only from the root module, same as [`emit_struct_def`].
/// Arcis enums follow TypeScript *numeric enum* semantics: every variant IS
/// a number (`Category.Food == 10` is true, a `number`-typed field can hold
/// it, printing shows the number). So instead of a Rust `enum` (whose values
/// don't mix with `f64` without casts), emit a unit struct with one `f64`
/// associated const per variant — `Category::Electronics` is then a plain
/// `f64` everywhere. This matches the Cranelift backend, where variants are
/// compile-time `f64const`s.
pub(crate) fn emit_enum_def(out: &mut String, name: &str, variants: &[(String, Option<i64>)]) {
    out.push_str("pub struct ");
    out.push_str(name);
    out.push_str(";\n#[allow(non_upper_case_globals)]\nimpl ");
    out.push_str(name);
    out.push_str(" {\n");
    let mut next = 0i64;
    for (variant, value) in variants {
        let v = value.unwrap_or(next);
        out.push_str(&format!("    pub const {}: f64 = {}f64;\n", variant, v));
        next = v + 1;
    }
    out.push_str("}\n\n");
}

/// `true` for the `any` primitive. `let`/`const` bindings typed `any` skip
/// the explicit Rust annotation (see [`crate::stmt`]) since there is no
/// dynamic-value runtime to erase to — the initializer's own type flows
/// through via ordinary Rust inference instead.
pub(crate) fn is_any(ty: &Type) -> bool {
    matches!(ty, Type::Primitive(n) if n == "any")
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
