//! `sys.disk.*` — disk / filesystem info.
//!
//! Wraps POSIX `df(1)` with `-B1 -P` (1-byte blocks, portable
//! single-line format). `df` ships on Linux and macOS; Windows returns
//! empty `Vec` / `0.0` until a `wmic`/`Get-PSDrive` backend lands.
//!
//! ## Supported operations
//!
//! | Builtin                  | Arcis → Rust                                                          | Arcis return |
//! |--------------------------|----------------------------------------------------------------------|--------------|
//! | `sys.disk.list()`        | `df -B1 -P` → `Vec<String>` of `mount=…;size=…;used=…;avail=…`        | `string[]`   |
//! | `sys.disk.free(p)`       | `df -B1 -P p` → available column in bytes (last containing FS)        | `number`     |
//! | `sys.disk.used(p)`       | `df -B1 -P p` → used column in bytes                                 | `number`     |
//! | `sys.disk.total(p)`      | `df -B1 -P p` → size column in bytes                                 | `number`     |
//!
//! Each call inline-parses the full `df` output and picks the last
//! entry (the most specific filesystem containing the path). This is
//! verbose (~30 lines per call) but keeps each call site self-contained
//! without a prelude helper function.

use arcis_ast::Expr;

use crate::context::Ctx;

use super::emit_arg_ref;

/// Try to emit `sys.disk.<method>(args)`. Returns `true` if this
/// module handled the method.
pub(crate) fn try_emit_method(
    out: &mut String,
    method: &str,
    args: &[Expr],
    ctx: &Ctx,
) -> bool {
    match method {
        "list" => emit_list(out, None, ctx),
        "free" => emit_column(out, args, 2, ctx),
        "used" => emit_column(out, args, 1, ctx),
        "total" => emit_column(out, args, 0, ctx),
        _ => false,
    }
}

/// Emit `sys.disk.list()` (or `sys.disk.list("path")`) returning a
/// `Vec<String>` of `mount=…;size=…;used=…;avail=…` rows.
fn emit_list(out: &mut String, path: Option<&Expr>, ctx: &Ctx) -> bool {
    out.push_str(
        "(if cfg!(target_os = \"windows\") { Vec::<String>::new() } else { \
         (|| -> Vec<String> { \
         let __out = String::from_utf8_lossy(&std::process::Command::new(\"df\")\
         .arg(\"-B1\").arg(\"-P\")",
    );
    if let Some(p) = path {
        // .arg(<path-expr>) — emit_arg_ref already includes the
        // leading `&` and the path expression (which may itself
        // include a closing paren, e.g. for `String::from("…")`).
        // We just close `.arg(` here.
        out.push_str(".arg(");
        emit_arg_ref(out, Some(p), ctx);
        out.push(')');
    }
    out.push_str(
        ".output().unwrap().stdout).into_owned(); \
         let mut __result: Vec<String> = Vec::new(); \
         let mut __it = __out.lines(); \
         let _ = __it.next(); /* skip header */ \
         for __line in __it { \
             let mut __f = __line.split_whitespace(); \
             let __fs = __f.next().unwrap_or(\"\"); \
             let __size = __f.next().unwrap_or(\"0\"); \
             let __used = __f.next().unwrap_or(\"0\"); \
             let __avail = __f.next().unwrap_or(\"0\"); \
             let _use_pct = __f.next(); \
             let __mount = __f.next().unwrap_or(\"\"); \
             let _ = __fs; \
             __result.push(format!(\
                 \"mount={};size={};used={};avail={}\", __mount, __size, __used, __avail)); \
         } \
         __result \
         })() })",
    );
    true
}

/// Emit `sys.disk.{used,total,free}(path)` returning the requested
/// column (0=size, 1=used, 2=avail) of the last (most-specific) row as
/// a `f64`.
fn emit_column(
    out: &mut String,
    args: &[Expr],
    col: usize,
    ctx: &Ctx,
) -> bool {
    out.push_str(
        "(if cfg!(target_os = \"windows\") { 0.0 } else { \
         (|| -> f64 { \
         let __out = String::from_utf8_lossy(&std::process::Command::new(\"df\")\
         .arg(\"-B1\").arg(\"-P\").arg(",
    );
    // emit_arg_ref pushes `&<path-expr>` where <path-expr> already
    // includes its own closing parens (e.g. for a string literal
    // it emits `&String::from("…")`). Then `.arg(` is already open
    // above, so we only need to close it once here.
    emit_arg_ref(out, args.first(), ctx);
    out.push_str(
        ").output().unwrap().stdout).into_owned(); \
         let mut __rows: Vec<(f64, f64, f64)> = Vec::new(); \
         let mut __it = __out.lines(); \
         let _ = __it.next(); /* skip header */ \
         for __line in __it { \
             let mut __f = __line.split_whitespace(); \
             let _fs = __f.next(); \
             let __size: f64 = __f.next().and_then(|v| v.parse().ok()).unwrap_or(0.0); \
             let __used: f64 = __f.next().and_then(|v| v.parse().ok()).unwrap_or(0.0); \
             let __avail: f64 = __f.next().and_then(|v| v.parse().ok()).unwrap_or(0.0); \
             __rows.push((__size, __used, __avail)); \
         } \
         match __rows.last() { \
             Some(r) => [r.0, r.1, r.2][",
    );
    out.push_str(&col.to_string());
    out.push_str(
        "], \
             None => 0.0, \
         } \
         })() })",
    );
    true
}
