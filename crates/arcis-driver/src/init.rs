//! Project scaffolding: `arcis init`.
//!
//! Creates a minimal Arcis project in `dir`: a `main.tsr` with a "Hello"
//! example ready to run, plus a `Cargo.toml` with commented-out Rust
//! dependencies as a starter template. If `main.tsr` already exists, abort
//! without overwriting.

use std::fs;
use std::path::Path;

/// Run the scaffolding step. See [module docs](self).
pub(crate) fn run(dir: &Path) -> Result<(), String> {
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
        cargo_toml.display().to_string()
    };

    let abs = fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    println!("✓ Arcis project created at {}", abs.display());
    println!("  - {}", abs.join("main.tsr").display());
    println!("  - Cargo.toml: {cargo_line}");
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