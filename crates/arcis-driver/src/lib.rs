//! Arcis driver: orchestrates the compilation pipeline.
//!
//! Phases:
//!   1. Resolve modules from the entry point (by convention `main.tsr`)
//!   2. Run semantic validation per module
//!   3. Run codegen — one `.rs` per module
//!   4. Write to `bin/` (direct rustc) or `bin/<pkg>/` (Cargo project)
//!   5. Invoke `rustc` or `cargo build`/`run` to produce the binary
//!   6. (Optional) Execute the binary
//!
//! In phase 2 this single file will be split into `build.rs` (the pipeline),
//! `rustc.rs` (rustc/cargo invocation), and `init.rs` (project scaffolding).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use arcis_ast::Stmt;
use arcis_linker::{Module, ModuleTarget};

/// Result of [`build`]: the generated sources and the layout chosen.
pub struct BuildOutput {
    /// `(id, rust_source)` per module.
    pub sources: Vec<(String, String)>,
    /// The root module id (the entry point).
    pub root_id: String,
    /// When the entry has a `Cargo.toml` beside it, this is the path to the
    /// generated Cargo project. `None` for the direct-rustc path.
    pub cargo_dir: Option<PathBuf>,
    /// When a Cargo project was used, the package name. `None` otherwise.
    pub pkg_name: Option<String>,
}

/// Resolve, validate, generate Rust sources and write them to disk.
/// If the entry has a `Cargo.toml` beside it, generate a Cargo project in
/// `bin/<pkg>/`; otherwise write the `.rs` files directly to `bin/`.
pub fn build(input: &Path) -> Result<BuildOutput, String> {
    let modules = arcis_linker::resolve(input)?;

    for m in &modules {
        let issues = arcis_validation::validate(&m.program);
        if !issues.is_empty() {
            return Err(arcis_validation::format_issues(
                &issues,
                &m.path.display().to_string(),
            ));
        }
    }

    let generated = arcis_codegen::generate_all(&modules)?;

    let bin_dir = PathBuf::from("bin");
    fs::create_dir_all(&bin_dir).map_err(|e| format!("could not create `bin/`: {}", e))?;

    let root_id = modules[0].id.clone();
    let entry_dir = modules[0].path.parent().ok_or_else(|| {
        format!(
            "could not determine the directory of `{}`",
            modules[0].path.display()
        )
    })?;
    let user_cargo_toml = entry_dir.join("Cargo.toml");

    if user_cargo_toml.exists() {
        // Cargo mode: generate `bin/<pkg>/Cargo.toml` (copy of the user's
        // or a minimal one) + `bin/<pkg>/src/<id>.rs` per module.
        // `<pkg>` is derived from the entry directory's name (not from the
        // root_id) so it stays consistent with `arcis init`'s Cargo.toml.
        let pkg_name = entry_dir
            .file_name()
            .and_then(|s| s.to_str())
            .filter(|s| !s.is_empty() && *s != ".")
            .unwrap_or(&root_id)
            .to_string();
        let target_dir = bin_dir.join(&pkg_name);
        // If a stale file from a previous rustc-mode build sits here, clean it.
        if target_dir.exists() && !target_dir.is_dir() {
            fs::remove_file(&target_dir).map_err(|e| {
                format!("could not clean `{}`: {}", target_dir.display(), e)
            })?;
        }
        let src_dir = target_dir.join("src");
        fs::create_dir_all(&src_dir)
            .map_err(|e| format!("could not create `{}`: {}", src_dir.display(), e))?;

        // Copy the user's Cargo.toml (otherwise generate a minimal one).
        let dest_cargo = target_dir.join("Cargo.toml");
        if user_cargo_toml != dest_cargo {
            fs::copy(&user_cargo_toml, &dest_cargo).map_err(|e| {
                format!(
                    "could not copy `{}` to `{}`: {}",
                    user_cargo_toml.display(),
                    dest_cargo.display(),
                    e
                )
            })?;
        }

        // Force the Cargo sub-project to be its own workspace. Cargo
        // auto-detects a workspace by walking up from the Cargo.toml, and
        // by default searches for `src/*.rs` at the workspace root — not
        // where the Cargo.toml lives. Adding an empty `[workspace]` section
        // makes the sub-project self-contained.
        let mut cargo_contents = fs::read(&dest_cargo)
            .map_err(|e| format!("could not read `{}`: {}", dest_cargo.display(), e))?;
        if !cargo_contents.ends_with(b"\n") {
            cargo_contents.push(b'\n');
        }
        cargo_contents.extend_from_slice(b"\n[workspace]\n");
        fs::write(&dest_cargo, cargo_contents)
            .map_err(|e| format!("could not write `{}`: {}", dest_cargo.display(), e))?;

        for (id, src) in &generated {
            let p = src_dir.join(format!("{}.rs", id));
            fs::write(&p, src)
                .map_err(|e| format!("could not write `{}`: {}", p.display(), e))?;
        }

        Ok(BuildOutput {
            sources: generated,
            root_id,
            cargo_dir: Some(target_dir),
            pkg_name: Some(pkg_name),
        })
    } else {
        // Without a Cargo.toml: if the program imports external crates,
        // give a clear message instead of letting rustc fail cryptically.
        if has_crate_import(&modules) {
            return Err(
                "to use Rust crates (`from \"crate:<name>\"`) you need a \
                 `Cargo.toml` next to `main.tsr`. \
                 Create one (or run `arcis init` again) and uncomment the dependency."
                    .to_string(),
            );
        }
        for (id, src) in &generated {
            let p = bin_dir.join(format!("{}.rs", id));
            fs::write(&p, src)
                .map_err(|e| format!("could not write `{}`: {}", p.display(), e))?;
        }
        Ok(BuildOutput {
            sources: generated,
            root_id,
            cargo_dir: None,
            pkg_name: None,
        })
    }
}

/// Returns `true` if any module imports a specifier with the `crate:` prefix.
fn has_crate_import(modules: &[Module]) -> bool {
    modules.iter().any(|m| {
        m.program.stmts.iter().any(|s| {
            if let Stmt::Import { module: spec, .. } = s {
                spec.starts_with("crate:")
            } else {
                false
            }
        })
    })
}

/// Compile, then return the path of the produced binary.
pub fn compile(input: &Path) -> Result<PathBuf, String> {
    let out = build(input)?;
    if let Some(cargo_dir) = &out.cargo_dir {
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
            return Err(format!(
                "`cargo build` failed for {}",
                manifest.display()
            ));
        }
        let pkg = out.pkg_name.as_deref().unwrap_or("arcis-app");
        Ok(cargo_dir.join("target").join("debug").join(pkg))
    } else {
        let rs_root = PathBuf::from("bin").join(format!("{}.rs", out.root_id));
        let bin_path = PathBuf::from("bin").join(&out.root_id);
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
}

/// Build, then execute the binary. The exit status of the binary is
/// forwarded to the caller.
pub fn run(input: &Path) -> Result<(), String> {
    let out = build(input)?;
    if let Some(cargo_dir) = &out.cargo_dir {
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
    } else {
        let bin_path = compile(input)?;
        let status = Command::new(&bin_path)
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit())
            .status()
            .map_err(|e| format!("could not execute `{}`: {}", bin_path.display(), e))?;
        if !status.success() {
            std::process::exit(status.code().unwrap_or(1));
        }
        Ok(())
    }
}

/// Create a minimal Arcis project in `dir`: a `main.tsr` with a "Hello"
/// example ready to run, plus a `Cargo.toml` with commented-out Rust
/// dependencies as a starter template. If the directory already has a
/// `main.tsr`, abort without overwriting.
pub fn init(dir: &Path) -> Result<(), String> {
    let main_tsr = dir.join("main.tsr");
    if main_tsr.exists() {
        return Err(format!(
            "`{}` already exists; `arcis init` does not overwrite existing projects",
            main_tsr.display()
        ));
    }
    if !dir.exists() {
        fs::create_dir_all(dir)
            .map_err(|e| format!("could not create `{}`: {}", dir.display(), e))?;
    }
    fs::write(&main_tsr, INIT_MAIN_TSR)
        .map_err(|e| format!("could not write `{}`: {}", main_tsr.display(), e))?;

    let cargo_toml = dir.join("Cargo.toml");
    let cargo_line = if cargo_toml.exists() {
        format!("already existed, `{}` was skipped", cargo_toml.display())
    } else {
        let body = init_cargo_toml(dir);
        fs::write(&cargo_toml, body)
            .map_err(|e| format!("could not write `{}`: {}", cargo_toml.display(), e))?;
        format!("{}", cargo_toml.display())
    };

    let abs = fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    println!("✓ Arcis project created at {}", abs.display());
    println!("  - {}{}", abs.join("main.tsr").display(), "");
    println!("  - Cargo.toml: {}", cargo_line);
    println!();
    println!("Next steps:");
    println!("  cd {}", abs.display());
    println!("  arcis run");
    println!();
    println!("To use Rust dependencies, edit `Cargo.toml` and uncomment the");
    println!("lines under `[dependencies]`, then use:");
    println!("  import {{ Client }} from \"crate:reqwest\";");
    Ok(())
}

/// Scaffold a `Cargo.toml` whose name is derived from the directory name.
fn init_cargo_toml(dir: &Path) -> String {
    let name = dir
        .file_name()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty() && *s != ".")
        .unwrap_or("arcis-app");
    format!(
        r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2021"

# Uncomment the Rust dependencies you need. Arcis uses them with
# `import {{ ... }} from "crate:<name>";` and compiles with `cargo build`.
#
# [dependencies]
# serde = {{ version = "1", features = ["derive"] }}
# serde_json = "1"
# reqwest = {{ version = "0.12", default-features = false, features = ["blocking", "rustls-tls"] }}
"#
    )
}

const INIT_MAIN_TSR: &str = r#"// main.tsr — entry point of your Arcis project.
//
// By convention Arcis looks for `main.tsr` in the directory you pass.
//
// Commands:
//   arcis run     build and run this project
//   arcis build   build the binary and leave it at ./bin/main
//   arcis check   only generate Rust code without invoking rustc
//
// To import local modules, add other .tsr files in this directory and use
// `import` with the same TypeScript syntax:
//   import { add, PI } from "utils";
//   import calculate from "utils";   // default export
//
// To use Rust crates, edit `Cargo.toml` and uncomment the deps:
//   import { Client } from "crate:reqwest";

print("Hello, Arcis!");
"#;

// Suppress an "unused import" warning if `ModuleTarget` ever becomes unused
// in this module after refactors.
#[allow(dead_code)]
fn _marker(_: ModuleTarget) {}