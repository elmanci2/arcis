//! `sys.*` builtins.
//!
//! The `sys` namespace is dispatched across top-level call handlers
//! (`emit_call`) and sub-namespace method handlers (`emit_subns_call`).
//!
//! ## Top-level `sys.X(args)` builtins
//!
//! - [`fs`] — file-system operations (read, write, create, delete, copy,
//!   move, list).
//! - [`path`] — path queries and transformations (exists, isFile, isDir,
//!   size, absolute, relative, symlink).
//! - [`proc_env`] — process / environment queries (currentDir, tempDir,
//!   homeDir, executablePath, changeDir).
//! - [`process`] — child-process management (process, exec, spawn, kill,
//!   currentPid, parentPid, processes).
//!
//! ## Sub-namespace `sys.<ns>.<method>(args)` builtins
//!
//! - [`env`] — environment variables (get, set, delete, all).
//! - [`os`] — OS info (name, version, arch, hostname, username, uptime,
//!   locale, cpuCount).
//! - [`memory`] — memory info (total, free, used, available).
//! - [`cpu`] — CPU info (model, brand, frequency, usage, cores).
//! - [`gpu`] — GPU info (list, name, vendor, memory).
//! - [`disk`] — disk info (list, free, used, total).
//!
//! Each top-level submodule exposes a `try_emit(...) -> bool`; each
//! sub-namespace module exposes `try_emit_method(...) -> bool`. The
//! dispatchers fall back to emitting the call verbatim so `rustc` can
//! report unknown builtins.
//!
//! `sys.args` is the one `sys.X` member access (not a call) and is
//! handled in [`emit_member`].

use arcis_ast::Expr;

use crate::context::Ctx;

mod cpu;
mod disk;
mod env;
mod fs;
mod gpu;
mod memory;
mod os;
mod path;
mod proc_env;
mod process;

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
    if proc_env::try_emit_call(out, property, args, ctx) {
        return;
    }
    if process::try_emit_call(out, property, args, ctx) {
        return;
    }
    fallback_emit(out, property, args, ctx);
}

/// Dispatch `sys.<ns>.<method>(args)` to the sub-namespace handler.
///
/// Returns after the first handler that recognises the method; if no
/// module recognises the namespace or method, the call is re-emitted
/// verbatim (`sys.<ns>.<method>(...)`) so `rustc` can surface a
/// diagnostic.
pub(crate) fn emit_subns_call(
    out: &mut String,
    ns: &str,
    method: &str,
    args: &[Expr],
    ctx: &Ctx,
) {
    let handled = match ns {
        "env" => env::try_emit_method(out, method, args, ctx),
        "os" => os::try_emit_method(out, method, args, ctx),
        "memory" => memory::try_emit_method(out, method, args, ctx),
        "cpu" => cpu::try_emit_method(out, method, args, ctx),
        "gpu" => gpu::try_emit_method(out, method, args, ctx),
        "disk" => disk::try_emit_method(out, method, args, ctx),
        _ => false,
    };
    if !handled {
        fallback_subns_emit(out, ns, method, args, ctx);
    }
}

/// Emit `(if cfg!(target_os = "linux") { <linux> } else { <default> })`.
///
/// Useful for builtins that have a Linux-specific implementation but
/// must compile (and return `<default>`) on every other target. Keeps
/// the gating implicit at runtime so the emitted source is portable.
pub(super) fn emit_linux_gated(out: &mut String, linux: &str, default: &str) {
    out.push_str("(if cfg!(target_os = \"linux\") { ");
    out.push_str(linux);
    out.push_str(" } else { ");
    out.push_str(default);
    out.push_str(" })");
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

/// Re-emit an unknown `sys.<ns>.<method>(...)` verbatim.
fn fallback_subns_emit(
    out: &mut String,
    ns: &str,
    method: &str,
    args: &[Expr],
    ctx: &Ctx,
) {
    out.push_str("sys.");
    out.push_str(ns);
    out.push('.');
    out.push_str(method);
    out.push('(');
    for (i, a) in args.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        crate::expr::emit(out, a, ctx);
    }
    out.push(')');
}
