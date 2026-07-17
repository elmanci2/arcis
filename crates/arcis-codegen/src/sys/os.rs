//! `sys.os.*` — operating system info.
//!
//! OS-level queries: name/version/arch, hostname, current user, locale,
//! uptime and CPU count. Cross-platform via stdlib where possible
//! (`std::env::consts::ARCH`, `std::thread::available_parallelism`),
//! runtime `cfg!(...)` branches where the API differs, and subprocess
//! calls (`uname`, `hostname`) for things that aren't in stdlib yet.
//!
//! ## Supported operations
//!
//! | Builtin                | Arcis → Rust                                                          | Arcis return |
//! |------------------------|----------------------------------------------------------------------|--------------|
//! | `sys.os.name()`        | runtime cfg literal: `"linux"` / `"macos"` / `"windows"` / `"unknown"` | `string`     |
//! | `sys.os.version()`     | subprocess `uname -r` (unix) or `ver` (windows)                      | `string`     |
//! | `sys.os.arch()`        | `std::env::consts::ARCH.to_string()`                                  | `string`     |
//! | `sys.os.hostname()`    | subprocess `hostname`, trimmed                                        | `string`     |
//! | `sys.os.username()`    | runtime cfg: `USER` (unix) / `USERNAME` (windows)                    | `string`     |
//! | `sys.os.uptime()`      | parse `/proc/uptime` first field on linux, `0` elsewhere             | `number`     |
//! | `sys.os.locale()`      | `LC_ALL` then `LANG`, falls back to empty                            | `string`     |
//! | `sys.os.cpuCount()`    | `std::thread::available_parallelism().unwrap().get() as f64`          | `number`     |

use arcis_ast::Expr;

use crate::context::Ctx;

use super::emit_linux_gated;

/// Try to emit `sys.os.<method>(args)`. Returns `true` if this module
/// handled the method.
pub(crate) fn try_emit_method(
    out: &mut String,
    method: &str,
    args: &[Expr],
    _ctx: &Ctx,
) -> bool {
    match method {
        "name" => {
            out.push_str(
                "(if cfg!(target_os = \"linux\") { \"linux\" } \
                 else if cfg!(target_os = \"macos\") { \"macos\" } \
                 else if cfg!(target_os = \"windows\") { \"windows\" } \
                 else { \"unknown\" }).to_string()",
            );
            true
        }
        "version" => {
            // `uname -r` on unix, `ver` on windows. Use a single
            // runtime branch so the source compiles on every target.
            out.push_str(
                "String::from_utf8_lossy(&std::process::Command::new(\
                 if cfg!(target_os = \"windows\") { \"ver\" } else { \"uname\" })\
                 .arg(if cfg!(target_os = \"windows\") { String::new() } else { \"-r\".to_string() })\
                 .output().unwrap().stdout).into_owned()",
            );
            // Ignore the `args` parameter — version() takes none.
            let _ = args;
            true
        }
        "arch" => {
            out.push_str("std::env::consts::ARCH.to_string()");
            true
        }
        "hostname" => {
            out.push_str(
                "String::from_utf8_lossy(&std::process::Command::new(\"hostname\")\
                 .output().unwrap().stdout).trim().to_string()",
            );
            true
        }
        "username" => {
            out.push_str(
                "std::env::var(if cfg!(target_os = \"windows\") { \"USERNAME\" } else { \"USER\" })\
                 .unwrap_or_default()",
            );
            true
        }
        "uptime" => {
            // Parse /proc/uptime (Linux only). Format:
            //   "<uptime_seconds> <idle_seconds>"
            let linux_impl = "std::fs::read_to_string(\"/proc/uptime\")\
                 .ok().and_then(|s| s.split_whitespace().next().and_then(|v| v.parse::<f64>().ok()))\
                 .unwrap_or(0.0)";
            emit_linux_gated(out, linux_impl, "0.0");
            true
        }
        "locale" => {
            out.push_str(
                "std::env::var(\"LC_ALL\").or_else(|_| std::env::var(\"LANG\"))\
                 .unwrap_or_default()",
            );
            true
        }
        "cpuCount" => {
            out.push_str(
                "std::thread::available_parallelism().map(|n| n.get() as f64).unwrap_or(0.0)",
            );
            true
        }
        _ => false,
    }
}
