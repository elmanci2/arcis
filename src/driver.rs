//! Driver: orquesta las fases del compilador y delega en `rustc` (o en
//! `cargo` si hay `Cargo.toml` al lado del entry) para producir el binario.
//!
//! Fases:
//!   1. Resolver módulos desde el punto de entrada (por convención `main.tsr`)
//!   2. Validación semántica por módulo
//!   3. Codegen -> un `.rs` por módulo
//!   4. Escribir en `bin/` (rustc directo) o en `bin/<pkg>/src/` (cargo)
//!   5. Invocar `rustc` o `cargo build`/`run` para producir el binario
//!   6. (Opcional) Ejecutar el binario

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::ast::Stmt;
use crate::{codegen, modules, validation};

/// Resultado de `build`: las fuentes generadas y datos del layout elegido.
pub struct BuildOutput {
    /// `(id, fuente_rust)` por módulo.
    pub sources: Vec<(String, String)>,
    /// Id del módulo raíz (entry).
    pub root_id: String,
    /// Si el entry tiene `Cargo.toml` al lado, ruta al proyecto Cargo generado.
    pub cargo_dir: Option<PathBuf>,
    /// Si hay `Cargo.toml`, nombre del paquete; si no, `None`.
    pub pkg_name: Option<String>,
}

/// Resuelve, valida, genera el código y lo escribe en disco.
/// Si el entry tiene `Cargo.toml` al lado, genera un proyecto Cargo en
/// `bin/<pkg>/`; si no, escribe los `.rs` planos en `bin/`.
pub fn build(input: &Path) -> Result<BuildOutput, String> {
    let modules = modules::resolve(input)?;

    for m in &modules {
        let issues = validation::validate(&m.program);
        if !issues.is_empty() {
            return Err(validation::format_issues(
                &issues,
                &m.path.display().to_string(),
            ));
        }
    }

    let generated = codegen::generate_all(&modules)?;

    let bin_dir = PathBuf::from("bin");
    fs::create_dir_all(&bin_dir).map_err(|e| format!("no se pudo crear `bin/`: {}", e))?;

    let root_id = modules[0].id.clone();
    let entry_dir = modules[0].path.parent().ok_or_else(|| {
        format!(
            "no se pudo determinar el directorio de `{}`",
            modules[0].path.display()
        )
    })?;
    let user_cargo_toml = entry_dir.join("Cargo.toml");

    if user_cargo_toml.exists() {
        // Modo Cargo: generar `bin/<pkg>/Cargo.toml` (copia del usuario o
        // generado) + `bin/<pkg>/src/<id>.rs` por módulo.
        // `<pkg>` se deriva del nombre del directorio del entry (no del
        // root_id) para que sea coherente con el `name` del Cargo.toml
        // que genera `arcis init`.
        let pkg_name = entry_dir
            .file_name()
            .and_then(|s| s.to_str())
            .filter(|s| !s.is_empty() && *s != ".")
            .unwrap_or(&root_id)
            .to_string();
        let target_dir = bin_dir.join(&pkg_name);
        // Si quedó un archivo de una build anterior en modo rustc, limpiar.
        if target_dir.exists() && !target_dir.is_dir() {
            fs::remove_file(&target_dir).map_err(|e| {
                format!("no se pudo limpiar `{}`: {}", target_dir.display(), e)
            })?;
        }
        let src_dir = target_dir.join("src");
        fs::create_dir_all(&src_dir)
            .map_err(|e| format!("no se pudo crear `{}`: {}", src_dir.display(), e))?;

        // Copiar el Cargo.toml del usuario (si no, generamos uno mínimo).
        let dest_cargo = target_dir.join("Cargo.toml");
        if user_cargo_toml != dest_cargo {
            fs::copy(&user_cargo_toml, &dest_cargo).map_err(|e| {
                format!(
                    "no se pudo copiar `{}` a `{}`: {}",
                    user_cargo_toml.display(),
                    dest_cargo.display(),
                    e
                )
            })?;
        }

        // Forzar que el sub-proyecto Cargo sea su propio workspace (Cargo
        // autodetecta un workspace a partir del `Cargo.toml` padre del entry
        // y, por defecto, busca los `targets` en el root del workspace — no
        // donde está el `Cargo.toml`. Marcándolo con `[workspace]` evitamos
        // ese comportamiento y los `src/<id>.rs` se buscan en `bin/<pkg>/src/`.
        let mut cargo_contents = fs::read(&dest_cargo)
            .map_err(|e| format!("no se pudo leer `{}`: {}", dest_cargo.display(), e))?;
        if !cargo_contents.ends_with(b"\n") {
            cargo_contents.push(b'\n');
        }
        cargo_contents.extend_from_slice(b"\n[workspace]\n");
        fs::write(&dest_cargo, cargo_contents)
            .map_err(|e| format!("no se pudo escribir `{}`: {}", dest_cargo.display(), e))?;

        for (id, src) in &generated {
            let p = src_dir.join(format!("{}.rs", id));
            fs::write(&p, src)
                .map_err(|e| format!("no se pudo escribir `{}`: {}", p.display(), e))?;
        }

        Ok(BuildOutput {
            sources: generated,
            root_id,
            cargo_dir: Some(target_dir),
            pkg_name: Some(pkg_name),
        })
    } else {
        // Sin `Cargo.toml`: si el programa importa crates, dar un mensaje
        // claro en vez de dejar que rustc falle con un error críptico.
        if has_crate_import(&modules) {
            return Err(
                "para usar crates de Rust (`from \"crate:<nombre>\"`) necesitás un \
                 `Cargo.toml` al lado de `main.tsr`. \
                 Creá uno (o ejecutá `arcis init` de nuevo) y descomentá la dep."
                    .to_string(),
            );
        }
        for (id, src) in &generated {
            let p = bin_dir.join(format!("{}.rs", id));
            fs::write(&p, src)
                .map_err(|e| format!("no se pudo escribir `{}`: {}", p.display(), e))?;
        }
        Ok(BuildOutput {
            sources: generated,
            root_id,
            cargo_dir: None,
            pkg_name: None,
        })
    }
}

/// Devuelve true si algún módulo importa un specifier con prefijo `crate:`.
fn has_crate_import(modules: &[modules::Module]) -> bool {
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
            .map_err(|e| format!("no se pudo invocar `cargo`: {} (¿está instalado?)", e))?;
        if !status.success() {
            return Err(format!(
                "`cargo build` falló para {}",
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
            .map_err(|e| format!("no se pudo invocar `rustc`: {} (¿está instalado?)", e))?;
        if !status.success() {
            return Err(format!("`rustc` falló al compilar {}", rs_root.display()));
        }
        Ok(bin_path)
    }
}

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
            .map_err(|e| format!("no se pudo invocar `cargo`: {} (¿está instalado?)", e))?;
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
            .map_err(|e| format!("no se pudo ejecutar `{}`: {}", bin_path.display(), e))?;
        if !status.success() {
            std::process::exit(status.code().unwrap_or(1));
        }
        Ok(())
    }
}

/// Crea un proyecto Arcis mínimo en `dir`: un `main.tsr` con un ejemplo
/// "Hola" listo para correr, más un `Cargo.toml` con deps de Rust comentadas
/// como plantilla. Si el directorio ya tiene un `main.tsr`, aborta sin
/// sobreescribir.
pub fn init(dir: &Path) -> Result<(), String> {
    let main_tsr = dir.join("main.tsr");
    if main_tsr.exists() {
        return Err(format!(
            "`{}` ya existe; `arcis init` no sobreescribe proyectos existentes",
            main_tsr.display()
        ));
    }
    if !dir.exists() {
        fs::create_dir_all(dir)
            .map_err(|e| format!("no se pudo crear `{}`: {}", dir.display(), e))?;
    }
    fs::write(&main_tsr, INIT_MAIN_TSR)
        .map_err(|e| format!("no se pudo escribir `{}`: {}", main_tsr.display(), e))?;

    let cargo_toml = dir.join("Cargo.toml");
    let cargo_line = if cargo_toml.exists() {
        format!("ya existía, se omitió `{}`", cargo_toml.display())
    } else {
        let body = init_cargo_toml(dir);
        fs::write(&cargo_toml, body)
            .map_err(|e| format!("no se pudo escribir `{}`: {}", cargo_toml.display(), e))?;
        format!("{}", cargo_toml.display())
    };

    let abs = fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    println!("✓ Proyecto Arcis creado en {}", abs.display());
    println!("  - {}{}", abs.join("main.tsr").display(), "");
    println!("  - Cargo.toml: {}", cargo_line);
    println!();
    println!("Próximos pasos:");
    println!("  cd {}", abs.display());
    println!("  arcis run");
    println!();
    println!("Para usar dependencias de Rust, edita `Cargo.toml` descomentando");
    println!("las líneas de `[dependencies]` y usa:");
    println!("  import {{ Client }} from \"crate:reqwest\";");
    Ok(())
}

/// Plantilla de `Cargo.toml` con el nombre derivado del directorio.
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

# Descomenta las dependencias de Rust que necesites. Arcis las usa con
# `import {{ ... }} from "crate:<nombre>";` y las compila con `cargo build`.
#
# [dependencies]
# serde = {{ version = "1", features = ["derive"] }}
# serde_json = "1"
# reqwest = {{ version = "0.12", default-features = false, features = ["blocking", "rustls-tls"] }}
"#
    )
}

const INIT_MAIN_TSR: &str = r#"// main.tsr — punto de entrada de tu proyecto Arcis.
//
// Por convención, Arcis busca `main.tsr` en el directorio que le pases.
//
// Comandos:
//   arcis run     compila y ejecuta este proyecto
//   arcis build   compila y deja el binario en ./bin/main
//   arcis check   solo genera el código Rust sin invocar rustc
//
// Para importar módulos locales, crea otros archivos .tsr en este directorio
// y usa `import` con la misma sintaxis de TypeScript:
//   import { sumar, PI } from "utiles";
//   import calcula from "utiles";   // default export
//
// Para usar crates de Rust, edita `Cargo.toml` descomentando las deps:
//   import { Client } from "crate:reqwest";

print("Hola, Arcis!");
"#;
