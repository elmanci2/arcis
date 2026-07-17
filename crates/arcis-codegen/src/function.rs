//! `fn` emission.

use arcis_ast::Function;

use crate::context::Ctx;

/// Emit `fn NAME(params) -> RET { body }` or `pub fn NAME(...)` when
/// `pub_` is `true`.
pub(crate) fn emit(out: &mut String, f: &Function, ctx: &Ctx, pub_: bool) {
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
        out.push_str(&crate::types::ts_type_to_rust(&p.ty, ctx.is_root));
    }
    out.push(')');
    out.push_str(" -> ");
    out.push_str(&crate::types::ts_type_to_rust(&f.return_type, ctx.is_root));
    out.push_str(" {\n");
    for stmt in &f.body {
        crate::stmt::emit(out, stmt, 1, ctx);
    }
    out.push_str("}\n\n");
}