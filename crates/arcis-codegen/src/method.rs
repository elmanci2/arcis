//! Array and string method dispatch.
//!
//! Recognised methods:
//! - **Array**: `find`, `filter`, `map`, `reduce`, `pop`, `push`, `unshift`.
//! - **String**: `toUpperCase`, `toLowerCase`, `trim`, `substring`,
//!   `indexOf`, `includes`, `charAt`.
//!
//! Each method is translated to the equivalent Rust standard-library call.
//! The receiver and arguments are emitted via [`crate::expr::emit`].
//!
//! Returns `true` if a method matched (and was emitted), `false` if the
//! caller should fall through to the generic call emission.

use arcis_ast::Expr;

use crate::context::Ctx;

/// Try to emit `object.method(args)`. Returns `true` if a method matched.
pub(crate) fn emit(
    out: &mut String,
    object: &Expr,
    property: &str,
    args: &[Expr],
    ctx: &Ctx,
) -> bool {
    // Rust method detail:
    //   Vec<T>::iter() → Iterator<Item = &T>
    //   .find(cb)   expects Fn(&&T) → bool     pattern |&&x|  → x: &T
    //   .filter(cb) expects Fn(&&T) → bool     pattern |&&x|  → x: &T
    //   .map(cb)    expects FnMut(T) → U       pattern |&x|   → x: T
    //   .fold(init, cb) expects FnMut(B, T) → B  pattern |acc, &x|  → x: T
    // User-defined functions take T by value, so we do NOT dereference when
    // passing `x` as an argument.
    match property {
        // ── Array methods ──────────────────────────────────────────────
        "find" => {
            crate::expr::emit(out, object, ctx);
            out.push_str(".iter().find(|&&x| ");
            if let Some(cb) = args.first() {
                crate::builtin::emit_callback_call(out, cb, "x", ctx);
            }
            out.push_str(").cloned().unwrap_or_default()");
            true
        }
        "filter" => {
            crate::expr::emit(out, object, ctx);
            out.push_str(".iter().filter(|&&x| ");
            if let Some(cb) = args.first() {
                crate::builtin::emit_callback_call(out, cb, "x", ctx);
            }
            out.push_str(").cloned().collect()");
            true
        }
        "map" => {
            crate::expr::emit(out, object, ctx);
            out.push_str(".iter().map(|&x| ");
            if let Some(cb) = args.first() {
                crate::builtin::emit_callback_call(out, cb, "x", ctx);
            }
            out.push_str(").collect()");
            true
        }
        "reduce" => {
            crate::expr::emit(out, object, ctx);
            out.push_str(".iter().fold(");
            if let Some(init) = args.get(1) {
                crate::expr::emit(out, init, ctx);
            }
            out.push_str(", |acc, &x| ");
            if let Some(cb) = args.first() {
                crate::builtin::emit_callback_call2(out, cb, "acc", "x", ctx);
            }
            out.push(')');
            true
        }
        "pop" => {
            crate::expr::emit(out, object, ctx);
            out.push_str(".pop().unwrap_or_default()");
            true
        }
        "unshift" => {
            crate::expr::emit(out, object, ctx);
            out.push_str(".insert(0, ");
            if let Some(v) = args.first() {
                crate::expr::emit(out, v, ctx);
            }
            out.push(')');
            true
        }

        // ── String methods ─────────────────────────────────────────────
        "toUpperCase" => {
            crate::expr::emit(out, object, ctx);
            out.push_str(".to_uppercase()");
            true
        }
        "toLowerCase" => {
            crate::expr::emit(out, object, ctx);
            out.push_str(".to_lowercase()");
            true
        }
        "trim" => {
            crate::expr::emit(out, object, ctx);
            out.push_str(".trim().to_string()");
            true
        }
        "substring" => {
            // s[a as usize..b as usize].to_string()
            crate::expr::emit(out, object, ctx);
            out.push('[');
            if let Some(a) = args.first() {
                crate::expr::emit(out, a, ctx);
                out.push_str(" as usize");
            }
            out.push_str("..");
            if let Some(b) = args.get(1) {
                crate::expr::emit(out, b, ctx);
                out.push_str(" as usize");
            }
            out.push_str("].to_string()");
            true
        }
        "indexOf" => {
            // s.find(&sub).map(|i| i as f64).unwrap_or(-1.0)
            crate::expr::emit(out, object, ctx);
            out.push_str(".find(&");
            if let Some(sub) = args.first() {
                crate::expr::emit(out, sub, ctx);
            }
            out.push_str(").map(|i| i as f64).unwrap_or(-1.0)");
            true
        }
        "includes" => {
            // s.contains(&sub) — `&` because String does not implement
            // Pattern but `&str` does.
            crate::expr::emit(out, object, ctx);
            out.push_str(".contains(&");
            if let Some(sub) = args.first() {
                crate::expr::emit(out, sub, ctx);
            }
            out.push(')');
            true
        }
        "charAt" => {
            // s.chars().nth(i as usize).unwrap_or_default().to_string()
            crate::expr::emit(out, object, ctx);
            out.push_str(".chars().nth(");
            if let Some(i) = args.first() {
                crate::expr::emit(out, i, ctx);
                out.push_str(" as usize");
            }
            out.push_str(").unwrap_or_default().to_string()");
            true
        }
        _ => false,
    }
}