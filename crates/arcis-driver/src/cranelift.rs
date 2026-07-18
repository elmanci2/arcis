//! Cranelift backend finalisation: compile `arcis_runtime.c` with the
//! system's `cc`, then link all `.o` files (one per Arcis module plus
//! `arcis_runtime.o`) into a single executable. The Phase 1 entry points
//! mirror [`super::rustc`] but invoke `cc` instead of `rustc`/`cargo`.
//!
//! The output binary depends on **only** libc at runtime — no Rust
//! toolchain, no libstd.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Build, then return the path of the produced binary.
pub(crate) fn compile(input: &Path) -> Result<PathBuf, String> {
    let out = super::build::run(input, super::Backend::Cranelift)?;
    link(&out.root_id)
}

/// Build, then execute the binary. The exit status of the binary is
/// forwarded to the caller.
pub(crate) fn run(input: &Path) -> Result<(), String> {
    let bin_path = compile(input)?;
    run_binary(&bin_path)
}

/// Find the system C compiler. `cc` is the universal entry point (provided
/// by gcc on most distros, by clang on macOS). We do not verify its
/// flavour — any C99-capable compiler handles `arcis_runtime.c`.
fn find_cc() -> Command {
    // Allow override via `ARCIS_CC` env var for sandboxed / cross-build setups.
    if let Ok(custom) = std::env::var("ARCIS_CC") {
        if !custom.is_empty() {
            return Command::new(custom);
        }
    }
    Command::new("cc")
}

/// Step 1: compile `bin/arcis_runtime.c` to `bin/arcis_runtime.o`. Phase 1
/// always recompiles — the runtime is tiny (~150 lines) and the cost of
/// `cc -c` is negligible compared to the Cranelift pass. A content-hash
/// cache is straightforward to add once the rest is stable.
fn compile_runtime() -> Result<PathBuf, String> {
    let c_path = PathBuf::from("bin").join("arcis_runtime.c");
    let o_path = PathBuf::from("bin").join("arcis_runtime.o");
    let status = find_cc()
        .arg("-O2")
        .arg("-c")
        .arg(&c_path)
        .arg("-o")
        .arg(&o_path)
        .arg("-fno-pic")
        .arg("-Wno-unused-function")
        .arg("-Wno-unused-variable")
        .arg("-Wno-unused-but-set-variable")
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| format!("could not invoke `cc`: {} (is a C compiler installed?)", e))?;
    if !status.success() {
        return Err(format!("`cc` failed to compile `{}`", c_path.display()));
    }
    Ok(o_path)
}

/// Step 2: link all `.o` files (root module + runtime) into a single
/// executable at `bin/<root_id>`.
fn link(root_id: &str) -> Result<PathBuf, String> {
    let runtime_o = compile_runtime()?;
    let root_o = PathBuf::from("bin").join(format!("{}.o", root_id));
    let bin_path = PathBuf::from("bin").join(root_id);

    let status = find_cc()
        .arg("-o")
        .arg(&bin_path)
        .arg(&root_o)
        .arg(&runtime_o)
        .arg("-no-pie")
        .arg("-lm")
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| format!("could not invoke `cc` for linking: {}", e))?;
    if !status.success() {
        return Err(format!("`cc` failed to link `{}`", bin_path.display()));
    }
    Ok(bin_path)
}

/// Spawn the produced binary and forward its exit status.
fn run_binary(bin_path: &Path) -> Result<(), String> {
    let status = Command::new(bin_path)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| format!("could not execute `{}`: {}", bin_path.display(), e))?;
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
    Ok(())
}