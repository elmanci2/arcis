//! `sys.cpu.*` — CPU info.
//!
//! Returns CPU model, brand (vendor), frequency, usage and core count.
//! Linux-first via `/proc/cpuinfo` and `/proc/stat`. On non-Linux
//! targets all builtins return empty strings or `0.0`.
//!
//! ## Supported operations
//!
//! | Builtin                | Arcis → Rust (Linux)                                              | Arcis return |
//! |------------------------|------------------------------------------------------------------|--------------|
//! | `sys.cpu.model()`      | first `model name :` from `/proc/cpuinfo`                         | `string`     |
//! | `sys.cpu.brand()`      | first `vendor_id :` from `/proc/cpuinfo` (e.g. `"GenuineIntel"`) | `string`     |
//! | `sys.cpu.frequency()`  | first `cpu MHz :` from `/proc/cpuinfo`                            | `number` (MHz) |
//! | `sys.cpu.usage()`      | two snapshots of `/proc/stat` 100ms apart, busy percentage        | `number` (0–100) |
//! | `sys.cpu.cores()`      | `std::thread::available_parallelism().get() as f64`               | `number`     |
//!
//! `usage` blocks the calling thread for ~100ms so the second sample is
//! meaningfully different from the first; this is a tradeoff for
//! avoiding a long-running CPU sampler.

use crate::context::Ctx;

use super::emit_linux_gated;

/// Try to emit `sys.cpu.<method>(args)`. Returns `true` if this
/// module handled the method.
pub(crate) fn try_emit_method(
    out: &mut String,
    method: &str,
    _args: &[arcis_ast::Expr],
    _ctx: &Ctx,
) -> bool {
    match method {
        "model" => {
            let linux = parse_cpuinfo_field("model name");
            let linux = format!("({})", linux);
            emit_linux_gated(out, &linux, "\"\\\"\".to_string()");
            true
        }
        "brand" => {
            let linux = parse_cpuinfo_field("vendor_id");
            let linux = format!("({})", linux);
            emit_linux_gated(out, &linux, "\"\\\"\".to_string()");
            true
        }
        "frequency" => {
            // Parse `cpu MHz    : 2400.000` and return f64 MHz.
            let linux = "std::fs::read_to_string(\"/proc/cpuinfo\").ok()\
                 .and_then(|s| s.lines().find(|l| l.starts_with(\"cpu MHz\"))\
                    .and_then(|l| l.split(':').nth(1)\
                       .and_then(|v| v.trim().parse::<f64>().ok())))\
                 .unwrap_or(0.0)";
            emit_linux_gated(out, linux, "0.0");
            true
        }
        "usage" => {
            // Parse the first `cpu <user> <nice> <system> <idle> ...`
            // line of `/proc/stat` at two points separated by a 100ms
            // sleep; busy fraction is `1.0 - idle/total` over the delta.
            //
            // We avoid `return` inside an inner block (which rustc
            // finds ambiguous when combined with IIFEs) and instead
            // bail by panicking on missing data — `cpu.usage()` is
            // non-essential diagnostics, and panic is acceptable.
            let parse = "std::fs::read_to_string(\"/proc/stat\")\
                .ok().and_then(|s| s.lines().find(|l| l.starts_with(\"cpu \"))\
                    .and_then(|line| { \
                        let __v: Vec<f64> = line.split_whitespace()\
                            .skip(1).filter_map(|x| x.parse().ok()).collect(); \
                        if __v.len() >= 4 { \
                            let __total: f64 = __v.iter().sum(); \
                            Some((__total, __v[3])) \
                        } else { None } \
                    })).unwrap_or((0.0, 0.0))";
            let linux = format!(
                "{{ \
                     let __a = {parse}; \
                     std::thread::sleep(std::time::Duration::from_millis(100)); \
                     let __b = {parse}; \
                     let (__ta, __ia) = __a; \
                     let (__tb, __ib) = __b; \
                     let __dtotal = __tb - __ta; \
                     let __didle = __ib - __ia; \
                     if __dtotal <= 0.0 {{ 0.0 }} else {{ ((__dtotal - __didle) / __dtotal) * 100.0 }} \
                 }}",
                parse = parse
            );
            emit_linux_gated(out, &linux, "0.0");
            true
        }
        "cores" => {
            out.push_str(
                "std::thread::available_parallelism().map(|n| n.get() as f64).unwrap_or(0.0)",
            );
            true
        }
        _ => false,
    }
}

/// Emit a Rust expression that reads `/proc/cpuinfo` and returns the
/// value of `field` (e.g. `"model name"`) from the *first* matching
/// line as a `String`. The format is `  model name : Intel(R) ...`.
fn parse_cpuinfo_field(field: &str) -> String {
    format!(
        "std::fs::read_to_string(\"/proc/cpuinfo\").ok()\
         .and_then(|s| s.lines().find(|l| l.starts_with(\"{field}\"))\
            .and_then(|l| l.split(':').nth(1)\
               .map(|v| v.trim().to_string())))\
         .unwrap_or_default()"
    )
}
