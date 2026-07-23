//! Built-in functions: `print`, `input`.
//!
//! Plus the small helpers they share with the method dispatcher:
//! - [`has_string_literal`] — used by the `+` operator to decide between
//!   `format!` and direct addition.
//! - [`emit_callback_call`] / [`emit_callback_call2`] — used by `find` /
//!   `filter` / `map` / `reduce` to forward a closure parameter to the
//!   user-defined callback function.

use arcis_ast::Expr;

use crate::context::Ctx;

/// Emit `print(expr)` → `println!("{}", expr)`.
pub(crate) fn emit_print(out: &mut String, args: &[Expr], ctx: &Ctx) {
    out.push_str("println!(\"{}\", ");
    if let Some(arg) = args.first() {
        crate::expr::emit(out, arg, ctx);
    }
    out.push(')');
}

/// Emit `input()` → a block that reads one line of stdin and returns it
/// (trimmed) as a `String`.
pub(crate) fn emit_input(out: &mut String) {
    out.push_str(
        "{ let mut __arcis_input = String::new(); \
         std::io::stdin().read_line(&mut __arcis_input).unwrap(); \
         __arcis_input.trim_end().to_string() }",
    );
}

/// Returns `true` if `expr` contains a string literal at the top level or
/// nested. Used to decide between `+` and `format!`.
pub(crate) fn has_string_literal(expr: &Expr) -> bool {
    match expr {
        Expr::String(_) => true,
        Expr::Binary { left, right, .. } => has_string_literal(left) || has_string_literal(right),
        Expr::Unary { operand, .. } => has_string_literal(operand),
        Expr::Call { args, .. } => args.iter().any(has_string_literal),
        _ => false,
    }
}

/// Emit the callback call for `find` / `filter` / `map`: `cb(x)`. The level
/// of indirection is already handled by the closure pattern (e.g. `|&&x|`
/// for find/filter, `|&x|` for map), so we pass `x` directly. The callback
/// is either an identifier (a previously-declared named function) or an
/// inline arrow function, called immediately as `(|params| body)(x)`.
pub(crate) fn emit_callback_call(out: &mut String, cb: &Expr, arg: &str, ctx: &Ctx) {
    match cb {
        Expr::Ident(name) => {
            out.push_str(name);
            out.push('(');
            out.push_str(arg);
            out.push(')');
        }
        Expr::Arrow { .. } => {
            out.push('(');
            crate::expr::emit(out, cb, ctx);
            out.push_str(")(");
            out.push_str(arg);
            out.push(')');
        }
        _ => out.push_str("todo!()"),
    }
}

/// Emit the callback call for `reduce`: `cb(acc, x)`.
pub(crate) fn emit_callback_call2(out: &mut String, cb: &Expr, acc: &str, arg: &str, ctx: &Ctx) {
    match cb {
        Expr::Ident(name) => {
            out.push_str(name);
            out.push('(');
            out.push_str(acc);
            out.push_str(", ");
            out.push_str(arg);
            out.push(')');
        }
        Expr::Arrow { .. } => {
            out.push('(');
            crate::expr::emit(out, cb, ctx);
            out.push_str(")(");
            out.push_str(acc);
            out.push_str(", ");
            out.push_str(arg);
            out.push(')');
        }
        _ => out.push_str("todo!()"),
    }
}