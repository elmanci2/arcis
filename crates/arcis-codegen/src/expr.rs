//! Expression emission.
//!
//! Dispatches on [`Expr`] variants. The `Call` arm is the most complex — it
//! dispatches to:
//!
//! - [`crate::builtin`] for `print` and `input`.
//! - [`crate::sys`] for `sys.*`.
//! - [`crate::method`] for array/string method chains.
//!
//! Everything else (literals, member access, index, binary, unary, paths,
//! array / object literals) is emitted inline.

use arcis_ast::{BinOp, Expr, UnaryOp};

use crate::context::Ctx;

/// Emit one expression.
pub(crate) fn emit(out: &mut String, expr: &Expr, ctx: &Ctx) {
    match expr {
        Expr::Number(n) => emit_number(out, *n),
        Expr::String(s) => out.push_str(&crate::types::rust_string_literal(s)),
        Expr::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Expr::Ident(name) => out.push_str(name),

        Expr::Call { callee, args } => emit_call(out, callee, args, ctx),

        Expr::Unary { op, operand } => emit_unary(out, *op, operand, ctx),
        Expr::Member { object, property } => emit_member(out, object, property, ctx),
        Expr::Index { object, index } => emit_index(out, object, index, ctx),
        Expr::ArrayLiteral { elements } => emit_array_literal(out, elements, ctx),
        Expr::ObjectLiteral { fields } => emit_object_literal(out, fields, ctx),
        Expr::Path { segments } => out.push_str(&segments.join("::")),
        Expr::Binary { op, left, right } => emit_binary(out, *op, left, right, ctx),
    }
}

// ── Atomic forms ──────────────────────────────────────────────────────────

fn emit_number(out: &mut String, n: f64) {
    // Always emit with a decimal point so Rust treats it as f64.
    if n.fract() == 0.0 {
        out.push_str(&format!("{}.0", n as i64));
    } else {
        out.push_str(&format!("{}", n));
    }
}

fn emit_unary(out: &mut String, op: UnaryOp, operand: &Expr, ctx: &Ctx) {
    match op {
        UnaryOp::Not => {
            out.push('!');
            emit(out, operand, ctx);
        }
        UnaryOp::Neg => {
            out.push('-');
            emit(out, operand, ctx);
        }
    }
}

fn emit_member(out: &mut String, object: &Expr, property: &str, ctx: &Ctx) {
    // Builtin `sys.X` member accesses (e.g. `sys.args`).
    if let Expr::Ident(module) = object {
        if module == "sys" {
            crate::sys::emit_member(out, property);
            return;
        }
    }
    // `.length` is special-cased based on the type:
    //   string → `.chars().count() as f64` (Unicode codepoints)
    //   T[]    → `.len() as f64` (element count)
    //   other  → `.len() as f64` (default; rustc reports otherwise)
    //
    // We use the type env when the object is an identifier with a declared
    // type.
    emit(out, object, ctx);
    if property == "length" {
        let is_string = matches!(object,
            Expr::Ident(name) if ctx.types.get(name).map(|t| t == "string").unwrap_or(false));
        let is_array = matches!(object,
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

fn emit_index(out: &mut String, object: &Expr, index: &Expr, ctx: &Ctx) {
    emit(out, object, ctx);
    out.push('[');
    emit(out, index, ctx);
    out.push_str(" as usize]");
}

fn emit_array_literal(out: &mut String, elements: &[Expr], ctx: &Ctx) {
    if elements.is_empty() {
        // Without a declared type, default to `Vec<f64>`. For other types,
        // declare the type on the let/const (handled in `Stmt::Let` /
        // `Stmt::Const`).
        out.push_str("Vec::<f64>::new()");
    } else {
        out.push_str("vec![");
        for (i, e) in elements.iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            emit(out, e, ctx);
        }
        out.push(']');
    }
}

fn emit_object_literal(out: &mut String, fields: &[(String, Expr)], ctx: &Ctx) {
    // Requires that the context (let/const) has declared the type, because
    // we emit `StructName { field: value, ... }` directly.
    if let Some(ty) = ctx.current_let_type {
        if !ty.fields.is_empty() {
            out.push_str(&ty.name);
            out.push_str(" { ");
            for (i, (k, v)) in fields.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                out.push_str(k);
                out.push_str(": ");
                emit(out, v, ctx);
            }
            out.push_str(" }");
            return;
        }
    }
    // Without context: emit a tuple as fallback (rustc will complain).
    out.push_str("todo!()");
}

// ── Binary operators ──────────────────────────────────────────────────────

fn emit_binary(out: &mut String, op: BinOp, left: &Expr, right: &Expr, ctx: &Ctx) {
    match op {
        BinOp::Add => {
            // Heuristic: if either operand is a string literal, emit `format!`.
            if crate::builtin::has_string_literal(left) || crate::builtin::has_string_literal(right)
            {
                out.push_str("format!(\"{}{}\", ");
                emit(out, left, ctx);
                out.push_str(", ");
                emit(out, right, ctx);
                out.push(')');
            } else {
                out.push('(');
                emit(out, left, ctx);
                out.push_str(" + ");
                emit(out, right, ctx);
                out.push(')');
            }
        }
        BinOp::Sub => wrap_binary(out, " - ", left, right, ctx),
        BinOp::Mul => wrap_binary(out, " * ", left, right, ctx),
        BinOp::Div => wrap_binary(out, " / ", left, right, ctx),
        BinOp::Mod => wrap_binary(out, " % ", left, right, ctx),
        BinOp::EqEq => wrap_binary(out, " == ", left, right, ctx),
        BinOp::NotEq => wrap_binary(out, " != ", left, right, ctx),
        BinOp::Lt => wrap_binary(out, " < ", left, right, ctx),
        BinOp::Gt => wrap_binary(out, " > ", left, right, ctx),
        BinOp::LtEq => wrap_binary(out, " <= ", left, right, ctx),
        BinOp::GtEq => wrap_binary(out, " >= ", left, right, ctx),
        BinOp::And => wrap_binary(out, " && ", left, right, ctx),
        BinOp::Or => wrap_binary(out, " || ", left, right, ctx),
    }
}

fn wrap_binary(out: &mut String, op: &str, left: &Expr, right: &Expr, ctx: &Ctx) {
    out.push('(');
    emit(out, left, ctx);
    out.push_str(op);
    emit(out, right, ctx);
    out.push(')');
}

// ── Calls: dispatch to builtin / sys / method / generic ──────────────────

fn emit_call(out: &mut String, callee: &Expr, args: &[Expr], ctx: &Ctx) {
    // print / input builtins.
    if let Expr::Ident(name) = callee {
        if name == "print" {
            crate::builtin::emit_print(out, args, ctx);
            return;
        }
        if name == "input" {
            crate::builtin::emit_input(out);
            return;
        }
    }
    // sys.<ns>.<method>(args) sub-namespace call.
    if let Expr::Member { object, property } = callee {
        if let Expr::Member { object: ns_obj, property: ns } = object.as_ref() {
            if let Expr::Ident(module) = ns_obj.as_ref() {
                if module == "sys" {
                    crate::sys::emit_subns_call(out, ns, property, args, ctx);
                    return;
                }
            }
        }
    }
    // sys.* builtins.
    if let Expr::Member { object, property } = callee {
        if let Expr::Ident(module) = object.as_ref() {
            if module == "sys" {
                crate::sys::emit_call(out, property, args, ctx);
                return;
            }
        }
    }
    // Array / string method dispatch.
    if let Expr::Member { object, property } = callee {
        if crate::method::emit(out, object, property, args, ctx) {
            return;
        }
    }
    // Named function call: f(args).
    if let Expr::Ident(name) = callee {
        out.push_str(name);
        out.push('(');
        for (i, a) in args.iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            // Auto-`&` for array-typed identifier arguments.
            if let Expr::Ident(ref n) = a {
                if ctx.types.get(n).map(|t| t.ends_with("[]")).unwrap_or(false) {
                    out.push('&');
                }
            }
            emit(out, a, ctx);
        }
        out.push(')');
        return;
    }
    // Generic call: (callee)(args).
    emit(out, callee, ctx);
    out.push('(');
    for (i, a) in args.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        emit(out, a, ctx);
    }
    out.push(')');
}