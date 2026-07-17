//! `sys.net.*` — network info.
//!
//! Linux-first implementation. Reads hostname via the `hostname`
//! command, enumerates interfaces via `ip -o -4 addr show`, derives
//! the local IP from `hostname -I`, queries public IP via
//! `curl https://ifconfig.me`, and probes connectivity via
//! `ping -c 1 -W 3 1.1.1.1`. Other platforms get `""` / `[]` /
//! `false` until a backend lands.
//!
//! ## Supported operations
//!
//! | Builtin                | Arcis → Rust                                                       | Arcis return |
//! |------------------------|-------------------------------------------------------------------|--------------|
//! | `sys.net.hostname()`   | subprocess `hostname`, trimmed                                    | `string`     |
//! | `sys.net.interfaces()` | parse `ip -o -4 addr show` → `iface=X;ip=Y` rows                  | `string[]`   |
//! | `sys.net.ip()`         | first token of `hostname -I` (Linux)                              | `string`     |
//! | `sys.net.publicIp()`   | `curl -s --max-time 5 https://ifconfig.me`, trimmed               | `string`     |
//! | `sys.net.online()`     | `ping -c 1 -W 3 1.1.1.1` exit-success check                        | `boolean`    |
//!
//! ## Notes
//!
//! - `publicIp()` and `online()` depend on external resources
//!   (`ifconfig.me` / `1.1.1.1`). They are best-effort diagnostics
//!   that may legitimately fail in air-gapped environments.
//! - `hostname()` here is the same data as `sys.os.hostname()` —
//!   duplicated so the namespace is self-contained.

use crate::context::Ctx;

use super::emit_linux_gated;

/// Try to emit `sys.net.<method>(args)`. Returns `true` if this
/// module handled the method.
pub(crate) fn try_emit_method(
    out: &mut String,
    method: &str,
    _args: &[arcis_ast::Expr],
    _ctx: &Ctx,
) -> bool {
    match method {
        "hostname" => {
            out.push_str(
                "String::from_utf8_lossy(&std::process::Command::new(\"hostname\")\
                 .output().unwrap().stdout).trim().to_string()",
            );
            true
        }
        "interfaces" => {
            // `ip -o -4 addr show` emits one line per address, e.g.:
            //   "2: eth0    inet 192.168.1.100/24 brd 192.168.1.255 ..."
            // We only care about the iface name and the address (without
            // the CIDR suffix).
            let linux = "{ \
                 let __out = String::from_utf8_lossy(&std::process::Command::new(\"ip\")\
                     .arg(\"-o\").arg(\"-4\").arg(\"addr\").arg(\"show\")\
                     .output().unwrap().stdout).into_owned(); \
                 let mut __result: Vec<String> = Vec::new(); \
                 for __line in __out.lines() { \
                     let mut __parts = __line.split_whitespace(); \
                     let __idx = __parts.next(); \
                     let __iface = __parts.next().unwrap_or(\"\"); \
                     let __inet = __parts.next(); \
                     let __ip = __parts.next().unwrap_or(\"\"); \
                     let __ip = __ip.split('/').next().unwrap_or(\"\"); \
                     if !__iface.is_empty() && !__ip.is_empty() { \
                         __result.push(format!(\"iface={};ip={}\", __iface, __ip)); \
                     } \
                     let _ = (__idx, __inet); \
                 } \
                 __result \
             }";
            emit_linux_gated(out, linux, "Vec::<String>::new()");
            true
        }
        "ip" => {
            // `hostname -I` prints all local IPs space-separated.
            // We take the first.
            let linux = "{ \
                 let __out = String::from_utf8_lossy(&std::process::Command::new(\"hostname\")\
                     .arg(\"-I\").output().unwrap().stdout).into_owned(); \
                 __out.split_whitespace().next().unwrap_or(\"\").to_string() \
             }";
            emit_linux_gated(out, linux, "\"\\\"\".to_string()");
            true
        }
        "publicIp" => {
            let linux = "String::from_utf8_lossy(&std::process::Command::new(\"curl\")\
                 .arg(\"-s\").arg(\"--max-time\").arg(\"5\")\
                 .arg(\"https://ifconfig.me\")\
                 .output().unwrap().stdout).trim().to_string()";
            emit_linux_gated(out, linux, "\"\\\"\".to_string()");
            true
        }
        "online" => {
            // `ping -c 1 -W 3 1.1.1.1` exits 0 on success, non-zero
            // otherwise. We map the result to a bool; any I/O error
            // (e.g. ping not installed) maps to false. Both stdout
            // and stderr are silenced so ping's chatter doesn't leak
            // into the program's output.
            let linux = "std::process::Command::new(\"ping\")\
                 .arg(\"-c\").arg(\"1\").arg(\"-W\").arg(\"3\")\
                 .arg(\"1.1.1.1\")\
                 .stdout(std::process::Stdio::null())\
                 .stderr(std::process::Stdio::null())\
                 .status()\
                 .map(|__s| __s.success()).unwrap_or(false)";
            emit_linux_gated(out, linux, "false");
            true
        }
        _ => false,
    }
}