//! rustc / cargo invocation.
//!
//! Wraps the two ways of producing a binary from the generated Rust sources:
//! - **Direct rustc**: compile a single `.rs` with `rustc`.
//! - **Cargo**: run `cargo build` or `cargo run` against the generated
//!   project at `bin/<pkg>/`.
//!
//! [`compile`] returns the path of the produced binary; [`run`] builds then
//! executes it and forwards the binary's exit status.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Build, then return the path of the produced binary.
pub(crate) fn compile(input: &Path) -> Result<PathBuf, String> {
    let out = super::build::run(input)?;
    if let Some(cargo_dir) = &out.cargo_dir {
        compile_with_cargo(cargo_dir, out.pkg_name.as_deref().unwrap_or("arcis-app"))
    } else {
        compile_with_rustc(&out.root_id)
    }
}

/// Build, then execute the binary. The exit status of the binary is
/// forwarded to the caller.
pub(crate) fn run(input: &Path) -> Result<(), String> {
    let out = super::build::run(input)?;
    if let Some(cargo_dir) = &out.cargo_dir {
        run_with_cargo(cargo_dir)
    } else {
        let bin_path = compile_with_rustc(&out.root_id)?;
        run_binary(&bin_path)
    }
}

/// Invoke `cargo build` against the generated project. Returns the binary
/// path on success.
fn compile_with_cargo(cargo_dir: &Path, pkg: &str) -> Result<PathBuf, String> {
    let manifest = cargo_dir.join("Cargo.toml");
    let status = Command::new("cargo")
        .arg("build")
        .arg("--manifest-path")
        .arg(&manifest)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| format!("could not invoke `cargo`: {} (is it installed?)", e))?;
    if !status.success() {
        return Err(format!("`cargo build` failed for {}", manifest.display()));
    }
    Ok(cargo_dir.join("target").join("debug").join(pkg))
}

/// Invoke `cargo run` against the generated project. The exit status of the
/// underlying binary is propagated by `exit`-ing this process.
fn run_with_cargo(cargo_dir: &Path) -> Result<(), String> {
    let manifest = cargo_dir.join("Cargo.toml");
    let status = Command::new("cargo")
        .arg("run")
        .arg("--manifest-path")
        .arg(&manifest)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| format!("could not invoke `cargo`: {} (is it installed?)", e))?;
    if !status.success() {
        std::process::exit(status.code().unwrap_or(1));
    }
    Ok(())
}

/// Compile a single `.rs` with `rustc`, producing a binary at
/// `bin/<root_id>`.
fn compile_with_rustc(root_id: &str) -> Result<PathBuf, String> {
    let rs_root = PathBuf::from("bin").join(format!("{}.rs", root_id));
    let bin_path = PathBuf::from("bin").join(root_id);
    let status = Command::new("rustc")
        .arg(&rs_root)
        .arg("-o")
        .arg(&bin_path)
        .arg("--edition=2021")
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| format!("could not invoke `rustc`: {} (is it installed?)", e))?;
    if !status.success() {
        return Err(format!("`rustc` failed to compile {}", rs_root.display()));
    }
    Ok(bin_path)
}

/// Spawn `bin_path` and forward its exit status to the calling process.
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