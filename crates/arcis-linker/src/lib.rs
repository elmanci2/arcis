//! Arcis linker / module resolver.
//!
//! Receives the entry point (by convention `main.tsr`) and resolves every
//! transitive dependency declared via `import`, returning the complete
//! program as an ordered list of modules.
//!
//! Path resolution: an `import` specifier (`from "utils"`) is interpreted
//! relative to the directory of the importing file, looking for
//! `<dir>/<specifier>.tsr` (no `./` prefix — Node/TS-style convenience).
//!
//! The codegen emits one `.rs` per module, declaring `mod <id>;` from the
//! root (`main`) and referencing items via `use crate::<id>::...`. Because of
//! that:
//!   - each `id` (= file stem) must be a valid Rust identifier;
//!   - no two modules may share the same `id`.

use arcis_ast::{ExportDefault, Program, Stmt};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// A loaded module: its id (= file stem), canonical path, AST, and the
/// exports it offers (used by the codegen to emit `use crate::...`).
#[derive(Debug)]
pub struct Module {
    pub id: String,
    pub path: PathBuf,
    pub program: Program,
    pub exports: Exports,
}

/// Public exports of a module.
/// `named` is the set of exported names (`export function/const` and
/// re-exports via `export { ... }`). `default` is the real symbol name of the
/// default export (`f` when `export default function f`, `__default` for
/// anonymous / expression defaults).
#[derive(Debug, Default, Clone)]
pub struct Exports {
    pub named: HashSet<String>,
    pub default: Option<String>,
}

/// What an `import ... from "<spec>"` resolves to:
/// - `Local`: a `.tsr` module in the same program, resolved relatively.
/// - `Crate`: an external Rust crate declared with the `crate:` prefix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModuleTarget {
    Local(PathBuf),
    Crate(String),
}

/// Resolve the whole program starting at `input` and return the loaded
/// module list (entry module first).
///
/// `input` may be:
/// - a directory: searches for `<input>/main.tsr`
/// - a `.tsr` file: used verbatim as the entry point
pub fn resolve(input: &Path) -> Result<Vec<Module>, String> {
    let entry = resolve_entry(input)?;
    let mut modules: HashMap<PathBuf, Module> = HashMap::new();
    let mut order: Vec<PathBuf> = Vec::new();
    let mut stack: Vec<PathBuf> = Vec::new();

    load(&entry, &mut modules, &mut order, &mut stack)?;

    check_unique_ids(&modules)?;
    validate_imports(&modules)?;

    // Return in discovery order (entry first).
    Ok(order.into_iter().filter_map(|p| modules.remove(&p)).collect())
}

/// Decide the entry file: directory → `<dir>/main.tsr`, file → as-is.
/// Then canonicalize the path.
fn resolve_entry(input: &Path) -> Result<PathBuf, String> {
    let candidate = if input.is_dir() {
        input.join("main.tsr")
    } else {
        input.to_path_buf()
    };
    if !candidate.exists() {
        return Err(format!(
            "entry point `{}` not found (expected `main.tsr`)",
            candidate.display()
        ));
    }
    std::fs::canonicalize(&candidate)
        .map_err(|e| format!("could not resolve `{}`: {}", candidate.display(), e))
}

/// DFS recursive loading: load `path`, resolve its imports, then load them.
/// `modules` is keyed by canonical path (dedup); `order` records the
/// pre-order discovery (entry first); `stack` detects cycles.
fn load(
    path: &Path,
    modules: &mut HashMap<PathBuf, Module>,
    order: &mut Vec<PathBuf>,
    stack: &mut Vec<PathBuf>,
) -> Result<(), String> {
    let canon = std::fs::canonicalize(path)
        .map_err(|e| format!("could not resolve `{}`: {}", path.display(), e))?;

    if modules.contains_key(&canon) {
        return Ok(());
    }
    if stack.iter().any(|p| p == &canon) {
        let mut chain: Vec<String> = stack
            .iter()
            .chain(std::iter::once(&canon))
            .map(|p| module_id(p).unwrap_or_else(|_| "?".into()))
            .collect();
        chain.dedup();
        return Err(format!(
            "cyclic module dependency: {}",
            chain.join(" -> ")
        ));
    }

    let source = std::fs::read_to_string(&canon)
        .map_err(|e| format!("could not read `{}`: {}", canon.display(), e))?;
    let tokens = arcis_lexer::lex(&source).map_err(|e| format!("{}: {}", canon.display(), e))?;
    let program =
        arcis_parser::parse(tokens).map_err(|e| format!("{}: {}", canon.display(), e))?;

    let id = module_id(&canon)?;

    stack.push(canon.clone());
    order.push(canon.clone());

    // Resolve and load dependencies before recording the module.
    for stmt in &program.stmts {
        if let Stmt::Import { module: spec, .. } = stmt {
            match resolve_specifier(&canon, spec)? {
                ModuleTarget::Local(dep) => load(&dep, modules, order, stack)?,
                ModuleTarget::Crate(_) => {} // external crates are not loaded
            }
        }
    }

    stack.pop();

    let exports = collect_exports(&program);
    modules.insert(
        canon.clone(),
        Module {
            id,
            path: canon,
            program,
            exports,
        },
    );
    Ok(())
}

/// Resolve the specifier of an `import`.
/// If it starts with `crate:`, returns `Crate(<name>)` without touching the FS.
/// The name may contain `::` (e.g. `crate:reqwest::blocking`) to reach a
/// sub-module of the crate. Otherwise, looks for `<dir>/<spec>.tsr`
/// (canonicalized).
pub fn resolve_specifier(importer: &Path, spec: &str) -> Result<ModuleTarget, String> {
    if let Some(name) = spec.strip_prefix("crate:") {
        if name.is_empty() {
            return Err("empty Rust crate specifier (`from \"crate:\"`)".to_string());
        }
        return Ok(ModuleTarget::Crate(name.to_string()));
    }
    let dir = importer.parent().ok_or_else(|| {
        format!("could not determine the directory of `{}`", importer.display())
    })?;
    let candidate = dir.join(format!("{}.tsr", spec));
    match std::fs::canonicalize(&candidate) {
        Ok(p) => Ok(ModuleTarget::Local(p)),
        Err(_) => Err(format!(
            "module `{}` not found (looked at `{}`)",
            spec,
            candidate.display()
        )),
    }
}

/// Return the Rust-compatible id of a module, derived from the file stem.
///
/// The id is what ends up as `mod <id>;` and the generated `<id>.rs` file
/// name, so it must be a valid Rust identifier. If the file stem starts
/// with a digit (e.g. `01_hello`) or contains a hyphen (e.g. `my-utils`),
/// we prepend `_` until the first character is alphabetic or `_`, then
/// replace any other invalid character with `_`. The file itself is
/// untouched — only the id used inside the generated Rust code is
/// sanitised.
fn module_id(path: &Path) -> Result<String, String> {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| format!("invalid file name: {}", path.display()))?;
    Ok(sanitize_module_id(stem))
}

/// Sanitise a string so it is a valid Rust identifier
/// `[A-Za-z_][A-Za-z0-9_]*`. Prepends `_` until the first character is
/// valid, then replaces any invalid character with `_`.
fn sanitize_module_id(stem: &str) -> String {
    if stem.is_empty() {
        return "_".into();
    }
    let mut out = String::with_capacity(stem.len());
    let mut first = true;
    for c in stem.chars() {
        let valid = if first {
            c.is_ascii_alphabetic() || c == '_'
        } else {
            c.is_ascii_alphanumeric() || c == '_'
        };
        if valid {
            out.push(c);
        } else if first {
            out.push('_');
            // Re-evaluate this char in the non-first position so e.g. a
            // digit that follows gets the normal treatment.
            if c.is_ascii_alphanumeric() {
                out.push(c);
            } else {
                out.push('_');
            }
        } else {
            out.push('_');
        }
        first = false;
    }
    out
}

/// Verify that no two modules share the same id (which would clash in
/// `mod <id>;` and in the generated `.rs` files).
fn check_unique_ids(modules: &HashMap<PathBuf, Module>) -> Result<(), String> {
    let mut seen: HashMap<String, PathBuf> = HashMap::new();
    for m in modules.values() {
        if let Some(prev) = seen.get(&m.id) {
            return Err(format!(
                "two modules share the same name `{}`: {} and {}",
                m.id,
                prev.display(),
                m.path.display()
            ));
        }
        seen.insert(m.id.clone(), m.path.clone());
    }
    Ok(())
}

/// Collect the exports offered by a program.
fn collect_exports(program: &Program) -> Exports {
    let mut exp = Exports::default();
    for stmt in &program.stmts {
        match stmt {
            Stmt::ExportDecl(inner) => match inner.as_ref() {
                Stmt::Function(f) => {
                    exp.named.insert(f.name.clone());
                }
                Stmt::Let { name, .. } | Stmt::Const { name, .. } => {
                    exp.named.insert(name.clone());
                }
                _ => {}
            },
            Stmt::ExportSpec(items) => {
                for it in items {
                    exp.named
                        .insert(it.alias.clone().unwrap_or_else(|| it.name.clone()));
                }
            }
            Stmt::ExportDefault(ed) => {
                exp.default = Some(default_real_name(ed));
            }
            _ => {}
        }
    }
    exp
}

/// Real Rust-side name of a default export.
pub fn default_real_name(ed: &ExportDefault) -> String {
    match ed {
        ExportDefault::Function(f) => {
            if f.name.is_empty() {
                "__default".into()
            } else {
                f.name.clone()
            }
        }
        ExportDefault::Expr(_) => "__default".into(),
    }
}

/// Verify that every `import` references exports that actually exist on the
/// target module.
fn validate_imports(modules: &HashMap<PathBuf, Module>) -> Result<(), String> {
    for m in modules.values() {
        for stmt in &m.program.stmts {
            if let Stmt::Import { default, named, module: spec } = stmt {
                let target = resolve_specifier(&m.path, spec)?;
                match target {
                    ModuleTarget::Crate(_) => {
                        // External Rust crates are validated by `rustc`. We
                        // only reject the `default import` form here.
                        if default.is_some() {
                            return Err(format!(
                                "`import x from \"crate:{}\"` is unsupported: \
                                 Rust crates have no `default export`",
                                spec.strip_prefix("crate:").unwrap_or(spec)
                            ));
                        }
                    }
                    ModuleTarget::Local(dep_path) => {
                        let target_mod = modules.get(&dep_path).ok_or_else(|| {
                            format!("module `{}` was not loaded (internal error)", spec)
                        })?;

                        if let Some(_local) = default {
                            if target_mod.exports.default.is_none() {
                                return Err(format!(
                                    "`{}` does not export a default value",
                                    spec
                                ));
                            }
                        }
                        for n in named {
                            if !target_mod.exports.named.contains(&n.name) {
                                return Err(format!(
                                    "`{}` does not export a symbol named `{}`",
                                    spec, n.name
                                ));
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_id_accepts_valid_identifiers() {
        assert!(module_id(Path::new("/tmp/utils.tsr")).is_ok());
        assert_eq!(module_id(Path::new("/tmp/utils.tsr")).unwrap(), "utils");
    }

    #[test]
    fn module_id_sanitises_invalid() {
        // Hyphens are replaced by underscores; the function never errors.
        assert_eq!(module_id(Path::new("/tmp/my-mod.tsr")).unwrap(), "my_mod");
        assert_eq!(module_id(Path::new("/tmp/2starts.tsr")).unwrap(), "_2starts");
        // Hyphens mixed with digits and underscores.
        assert_eq!(
            module_id(Path::new("/tmp/01-hello_world.tsr")).unwrap(),
            "_01_hello_world"
        );
    }
}