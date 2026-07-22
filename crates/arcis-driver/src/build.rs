//! Build pipeline: link → validate → codegen → write to disk.
//!
//! Two output layouts are supported for the Rust backend, decided by
//! whether the entry's directory has a `Cargo.toml` beside it:
//!
//! - **Direct rustc**: write `<id>.rs` files directly under `bin/`.
//! - **Cargo project**: write `bin/<pkg>/Cargo.toml` + `bin/<pkg>/src/<id>.rs`
//!   for use with `cargo build`.
//!
//! The Cranelift backend always writes object files directly to `bin/`
//! (one `.o` per module + `arcis_runtime.o` + `arcis_runtime.c`).

use std::fs;
use std::path::{Path, PathBuf};

use arcis_ast::Stmt;
use arcis_linker::Module;

/// Run the full pipeline: resolve modules, validate them, generate sources
/// for the chosen backend, and write them to disk.
pub(crate) fn run(input: &Path, backend: super::Backend) -> Result<super::BuildOutput, String> {
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

    let bin_dir = PathBuf::from("bin");
    fs::create_dir_all(&bin_dir).map_err(|e| format!("could not create `bin/`: {}", e))?;
    let root_id = modules[0].id.clone();

    match backend {
        super::Backend::Rust => {
            let generated = arcis_codegen::generate_all(&modules)?;
            let entry_dir = modules[0].path.parent().ok_or_else(|| {
                format!(
                    "could not determine the directory of `{}`",
                    modules[0].path.display()
                )
            })?;
            let user_cargo_toml = entry_dir.join("Cargo.toml");

            if user_cargo_toml.exists() {
                Ok(write_cargo_layout(
                    &modules,
                    &generated,
                    &bin_dir,
                    &user_cargo_toml,
                    &root_id,
                )?)
            } else {
                write_rustc_layout(&modules, &generated, &bin_dir, &root_id)
            }
        }
        super::Backend::Cranelift => {
            // Clean stale .o files from previous builds.
            if let Ok(entries) = fs::read_dir(&bin_dir) {
                for e in entries.flatten() {
                    let p = e.path();
                    if p.extension().map_or(false, |x| x == "o") {
                        let _ = fs::remove_file(&p);
                    }
                }
            }
            let triple = target_lexicon::Triple::host();
            arcis_codegen_cranelift::compile_to_object(&modules, &bin_dir, &triple)?;
            // Write the runtime source alongside so the linker step can
            // also use `cc` on it.
            let runtime_c_path = bin_dir.join("arcis_runtime.c");
            fs::write(
                &runtime_c_path,
                arcis_codegen_cranelift::runtime::RUNTIME_C_SOURCE,
            )
            .map_err(|e| {
                format!("could not write `{}`: {}", runtime_c_path.display(), e)
            })?;
            Ok(super::BuildOutput {
                sources: Vec::new(),
                root_id,
                cargo_dir: None,
                pkg_name: None,
                backend: super::Backend::Cranelift,
            })
        }
    }
}

/// Write the files for the **Cargo** layout (`bin/<pkg>/Cargo.toml` +
/// `bin/<pkg>/src/<id>.rs`).
fn write_cargo_layout(
    _modules: &[Module],
    generated: &[(String, String)],
    bin_dir: &Path,
    user_cargo_toml: &Path,
    root_id: &str,
) -> Result<super::BuildOutput, String> {
    // `<pkg>` is derived from the entry directory's name (not from the
    // root_id) so it stays consistent with `arcis init`'s Cargo.toml.
    let entry_dir = user_cargo_toml.parent().ok_or_else(|| {
        format!(
            "could not determine the directory of `{}`",
            user_cargo_toml.display()
        )
    })?;
    let pkg_name = entry_dir
        .file_name()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty() && *s != ".")
        .unwrap_or(root_id)
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
        fs::copy(user_cargo_toml, &dest_cargo).map_err(|e| {
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

    for (id, src) in generated {
        let p = src_dir.join(format!("{}.rs", id));
        fs::write(&p, src)
            .map_err(|e| format!("could not write `{}`: {}", p.display(), e))?;
    }

    Ok(super::BuildOutput {
        sources: generated.to_vec(),
        root_id: root_id.to_string(),
        cargo_dir: Some(target_dir),
        pkg_name: Some(pkg_name),
        backend: super::Backend::Rust,
    })
}

/// Write the files for the **direct rustc** layout (`bin/<id>.rs`).
fn write_rustc_layout(
    modules: &[Module],
    generated: &[(String, String)],
    bin_dir: &Path,
    root_id: &str,
) -> Result<super::BuildOutput, String> {
    // Without a Cargo.toml: if the program imports external crates, give a
    // clear message instead of letting rustc fail cryptically.
    if has_crate_import(modules) {
        return Err(
            "to use Rust crates (`from \"crate:<name>\"`) you need a \
             `Cargo.toml` next to `main.tsr`. \
             Create one (or run `arcis init` again) and uncomment the dependency."
                .to_string(),
        );
    }
    for (id, src) in generated {
        let p = bin_dir.join(format!("{}.rs", id));
        fs::write(&p, src)
            .map_err(|e| format!("could not write `{}`: {}", p.display(), e))?;
    }
    Ok(super::BuildOutput {
        sources: generated.to_vec(),
        root_id: root_id.to_string(),
        cargo_dir: None,
        pkg_name: None,
        backend: super::Backend::Rust,
    })
}

/// Returns `true` if any module imports a specifier with the `crate:` prefix.
fn has_crate_import(modules: &[Module]) -> bool {
    modules.iter().any(|m| {
        m.program.stmts.iter().any(|s| match s {
            Stmt::Import { module, .. } | Stmt::FromImport { module, .. } => {
                module.first().map_or(false, |seg| seg.starts_with("crate:"))
            }
            _ => false,
        })
    })
}
