//! `sys.gpu.*` — GPU info.
//!
//! Linux-first: enumerates PCI devices via `lspci -vmm`, filtering for
//! VGA / 3D / Display classes. NVIDIA-specific memory queries go
//! through `nvidia-smi`. On other platforms every builtin returns
//! `[]` or `"\"\"".to_string()` or `0.0`.
//!
//! ## Supported operations
//!
//! | Builtin              | Arcis → Rust                                                      | Arcis return |
//! |----------------------|------------------------------------------------------------------|--------------|
//! | `sys.gpu.list()`     | parse `lspci -vmm`, return `Vec<String>` of `vendor=X;name=Y`     | `string[]`   |
//! | `sys.gpu.name()`     | first GPU device name from `lspci -vmm`                           | `string`     |
//! | `sys.gpu.vendor()`   | first GPU vendor from `lspci -vmm`                                | `string`     |
//! | `sys.gpu.memory()`   | first line of `nvidia-smi --query-gpu=memory.total` (MiB → bytes) | `number`     |
//!
//! ## Notes
//!
//! - `name` / `vendor` / `list` all share the same parsing path; the
//!   block is inlined into each emitted callsite. Acceptable cost
//!   given the small number of GPU sites typical in user programs.
//! - `memory()` reads the first NVIDIA GPU; multi-GPU hosts get only
//!   the first. AMD / Intel GPUs always return `0.0` for `memory`
//!   until a vendor-agnostic probe (e.g. parsing VRAM from PCI BAR)
//!   is added.

use crate::context::Ctx;

use super::emit_linux_gated;

/// Try to emit `sys.gpu.<method>(args)`. Returns `true` if this
/// module handled the method.
pub(crate) fn try_emit_method(
    out: &mut String,
    method: &str,
    _args: &[arcis_ast::Expr],
    _ctx: &Ctx,
) -> bool {
    match method {
        "list" => {
            emit_linux_gated(out, &parse_lspci_gpus(), "Vec::<String>::new()");
            true
        }
        "name" => {
            // First entry's `name=X` field, or empty if list is empty.
            // `s.find(";name=")` returns `Option<usize>`; we map over
            // it so the index is non-Option inside the closure.
            let linux = format!(
                "({}.first().cloned()\
                 .and_then(|s| s.find(\";name=\")\
                     .map(|idx| s.get(idx + 6..).unwrap_or(\"\").to_string()))\
                 .unwrap_or_default())",
                parse_lspci_gpus_block()
            );
            emit_linux_gated(out, &linux, "\"\\\"\".to_string()");
            true
        }
        "vendor" => {
            // `vendor=X;name=Y` → take chars [7..name_idx]. If the
            // delimiter isn't present, fall back to the whole string.
            let linux = format!(
                "({}.first().cloned()\
                 .map(|s| {{ \
                     match s.find(\";name=\") {{ \
                         Some(name_idx) => s[7..name_idx].to_string(), \
                         None => s, \
                     }} \
                 }})\
                 .unwrap_or_default())",
                parse_lspci_gpus_block()
            );
            emit_linux_gated(out, &linux, "\"\\\"\".to_string()");
            true
        }
        "memory" => {
            // nvidia-smi --query-gpu=memory.total --format=csv,noheader,nounits
            // Outputs e.g. "6144\n" (MiB). Convert to bytes (×1024²=×1048576).
            let linux = "std::process::Command::new(\"nvidia-smi\")\
                 .arg(\"--query-gpu=memory.total\")\
                 .arg(\"--format=csv,noheader,nounits\")\
                 .output().ok()\
                 .and_then(|o| String::from_utf8_lossy(&o.stdout)\
                    .lines().next().and_then(|l| l.trim().parse::<f64>().ok()))\
                 .map(|mib| mib * 1024.0 * 1024.0).unwrap_or(0.0)";
            emit_linux_gated(out, linux, "0.0");
            true
        }
        _ => false,
    }
}

/// Emit a Rust expression that returns a `Vec<String>` of parsed GPU
/// entries (`vendor=X;name=Y`). Inlined per call site.
fn parse_lspci_gpus() -> String {
    parse_lspci_gpus_block().to_string()
}

/// Inner form usable inside larger expressions.
fn parse_lspci_gpus_block() -> String {
    "({ \
         let __out = std::process::Command::new(\"lspci\").arg(\"-vmm\")\
             .output().ok()\
             .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())\
             .unwrap_or_default(); \
         let mut __result: Vec<String> = Vec::new(); \
         let mut __in_gpu = false; \
         let mut __vendor = String::new(); \
         let mut __device = String::new(); \
         for __line in __out.lines() { \
             if let Some(__c) = __line.strip_prefix(\"Class: \") { \
                 if __in_gpu && (!__vendor.is_empty() || !__device.is_empty()) { \
                     __result.push(format!(\"vendor={};name={}\", __vendor, __device)); \
                 } \
                 let __cl = __c.to_lowercase(); \
                 __in_gpu = __cl.contains(\"vga\") \
                     || __cl.contains(\"3d\") \
                     || __cl.contains(\"display\"); \
                 __vendor.clear(); \
                 __device.clear(); \
             } else if let Some(__v) = __line.strip_prefix(\"Vendor: \") { \
                 if __in_gpu { __vendor = __v.to_string(); } \
             } else if let Some(__d) = __line.strip_prefix(\"Device: \") { \
                 if __in_gpu { __device = __d.to_string(); } \
             } \
         } \
         if __in_gpu && (!__vendor.is_empty() || !__device.is_empty()) { \
             __result.push(format!(\"vendor={};name={}\", __vendor, __device)); \
         } \
         __result \
     })"
        .to_string()
}
