//! `sys.*` builtins.
//!
//! The `sys` namespace is dispatched across three focused submodules:
//!
//! - [`fs`] — file-system operations (read, write, create, delete, copy,
//!   move, list).
//! - [`path`] — path queries and transformations (exists, isFile, isDir,
//!   size, absolute, relative, symlink).
//! - [`env`] — process / environment queries (args, currentDir, tempDir,
//!   homeDir, executablePath, changeDir).
//!
//! Each submodule exposes a `try_emit(...) -> bool` that returns `true`
//! when it handled a call; the dispatcher below tries each in order, then
//! falls back to emitting the call verbatim so `rustc` can report the
//! error for unknown `sys.X` invocations.
//!
//! `sys.args` is the one `sys.X` member access (not a call) and is
//! handled in [`emit_member`].

use arcis_ast::Expr;

use crate::context::Ctx;

mod env;
mod fs;
mod path;

/// Dispatch `sys.<property>(args)` to the appropriate submodule.
///
/// Returns after the first submodule that handles the property; if none
/// match, the call is re-emitted verbatim (`sys.X(...)`) so `rustc` can
/// surface a diagnostic.
pub(crate) fn emit_call(out: &mut String, property: &str, args: &[Expr], ctx: &Ctx) {
    if fs::try_emit(out, property, args, ctx) {
        return;
    }
    if path::try_emit(out, property, args, ctx) {
        return;
    }
    if env::try_emit_call(out, property, args, ctx) {
        return;
    }
    fallback_emit(out, property, args, ctx);
}

/// Dispatch `sys.<property>` as a member access (no call). Currently
/// only `sys.args` is recognised.
pub(crate) fn emit_member(out: &mut String, property: &str) {
    match property {
        "args" => out.push_str("std::env::args().collect::<Vec<String>>()"),
        // Unknown member access — emit as `sys.<property>` so rustc reports.
        _ => {
            out.push_str("sys.");
            out.push_str(property);
        }
    }
}

/// Emit `&<expr>` for an argument to a `sys.*` call that expects
/// `&S` where `S: AsRef<Path>` (or a similar borrowed string/bytes view).
/// If the argument is missing, emit `&String::new()` as a placeholder.
pub(super) fn emit_arg_ref(out: &mut String, arg: Option<&Expr>, ctx: &Ctx) {
    if let Some(a) = arg {
        out.push('&');
        crate::expr::emit(out, a, ctx);
    } else {
        out.push_str("&String::new()");
    }
}

/// Re-emit an unknown `sys.X(...)` verbatim so `rustc` can produce a
/// useful diagnostic on the runtime side.
fn fallback_emit(out: &mut String, property: &str, args: &[Expr], ctx: &Ctx) {
    out.push_str("sys.");
    out.push_str(property);
    out.push('(');
    for (i, a) in args.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        crate::expr::emit(out, a, ctx);
    }
    out.push(')');
}
