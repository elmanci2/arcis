//! `sys.memory.*` — system memory (RAM) info.
//!
//! Reads `/proc/meminfo` on Linux and surfaces totals as `f64` bytes.
//! On non-Linux targets every builtin returns `0.0`. Windows support
//! would need `GlobalMemoryStatusEx` (or a subprocess `wmic`); macOS
//! would use `sysctl hw.memsize`.
//!
//! ## Supported operations
//!
//! | Builtin                   | Arcis → Rust (Linux)                                                       | Arcis return |
//! |---------------------------|---------------------------------------------------------------------------|--------------|
//! | `sys.memory.total()`      | `MemTotal * 1024` from `/proc/meminfo`                                    | `number`     |
//! | `sys.memory.free()`       | `MemFree * 1024` from `/proc/meminfo`                                     | `number`     |
//! | `sys.memory.used()`       | `(MemTotal - MemFree) * 1024`                                             | `number`     |
//! | `sys.memory.available()`  | `MemAvailable * 1024` from `/proc/meminfo` (kernel reclaim estimate)       | `number`     |
//!
//! All values are gated by `cfg!(target_os = "linux")`. On every other
//! target the runtime branch returns `0.0`.

use crate::context::Ctx;

use super::emit_linux_gated;

/// Try to emit `sys.memory.<method>(args)`. Returns `true` if this
/// module handled the method.
pub(crate) fn try_emit_method(
    out: &mut String,
    method: &str,
    _args: &[arcis_ast::Expr],
    _ctx: &Ctx,
) -> bool {
    match method {
        "total" => {
            let linux = parse_meminfo_kb("MemTotal:");
            emit_linux_gated(out, &linux, "0.0");
            true
        }
        "free" => {
            let linux = parse_meminfo_kb("MemFree:");
            emit_linux_gated(out, &linux, "0.0");
            true
        }
        "used" => {
            let linux = format!(
                "(({}) - ({}))",
                parse_meminfo_kb("MemTotal:"),
                parse_meminfo_kb("MemFree:")
            );
            emit_linux_gated(out, &linux, "0.0");
            true
        }
        "available" => {
            let linux = parse_meminfo_kb("MemAvailable:");
            emit_linux_gated(out, &linux, "0.0");
            true
        }
        _ => false,
    }
}

/// Emit a Rust expression that reads `/proc/meminfo` and returns the
/// value of the given field (e.g. `MemTotal:`) as `f64` *bytes*
/// (`/proc/meminfo` reports kB, so we multiply by 1024). Returns 0.0
/// if the file is missing or the field is not present.
fn parse_meminfo_kb(field: &str) -> String {
    format!(
        "std::fs::read_to_string(\"/proc/meminfo\").ok()\
         .and_then(|s| s.lines().find(|l| l.starts_with(\"{field}\"))\
            .and_then(|l| l.split_whitespace().nth(1)\
               .and_then(|v| v.parse::<u64>().ok())))\
         .map(|kb| kb as f64 * 1024.0).unwrap_or(0.0)"
    )
}
