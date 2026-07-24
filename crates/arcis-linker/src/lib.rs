//! Arcis linker / module resolver.
//!
//! Receives the entry point (by convention `main.tsr`) and resolves every
//! transitive dependency declared via `import` / `from ... import`, returning
//! the complete program as an ordered list of modules.
//!
//! Path resolution: a module path `["utils"]` or `["lib", "utils"]` is
//! interpreted relative to the directory of the importing file, looking for
//! `<dir>/utils.tsr` or `<dir>/lib/utils.tsr`.
//!
//! The `crate:` prefix on the first segment (e.g. `["crate:serde"]`) marks an
//! external Rust crate — these are not loaded, only validated.

use arcis_ast::{ExportDefault, Program, Stmt};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// A loaded module: its id (= file stem), canonical path, AST, and the
/// exports it offers (used by the codegen to emit cross-module references).
#[derive(Debug, Clone)]
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

/// What an `import` / `from ... import` resolves to:
/// - `Local`: a `.tsr` module in the same program, resolved relatively.
/// - `Crate`: an external Rust crate declared with the `crate:` prefix.
/// - `Builtin`: a builtin namespace provided by the runtime (e.g. `sys`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModuleTarget {
    Local(PathBuf),
    Crate(String),
    Builtin(String),
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
        let module_path = match stmt {
            Stmt::Import { module, .. } => Some(module),
            Stmt::FromImport { module, .. } => Some(module),
            _ => None,
        };
        if let Some(module_path) = module_path {
            match resolve_specifier(&canon, module_path)? {
                ModuleTarget::Local(dep) => load(&dep, modules, order, stack)?,
                ModuleTarget::Crate(_) => {}   // external crates are not loaded
                ModuleTarget::Builtin(_) => {} // builtin namespaces need no file
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

/// Convert a module path (`["utils"]` or `["lib", "utils"]`) to a filesystem
/// path relative to the importing file's directory, looking for
/// `<dir>/<seg0>/<seg1>/.../<segN>.tsr`.
///
/// If the first segment starts with `crate:`, returns `Crate(name)`.
pub fn resolve_specifier(importer: &Path, segments: &[String]) -> Result<ModuleTarget, String> {
    if segments.is_empty() {
        return Err("empty module path".to_string());
    }
    let first = &segments[0];

    // Builtin namespaces provided by the runtime — no file needed.
    if is_builtin_ns(first) && segments.len() == 1 {
        return Ok(ModuleTarget::Builtin(first.clone()));
    }

    if let Some(name) = first.strip_prefix("crate:") {
        if name.is_empty() {
            return Err("empty Rust crate specifier (`crate:`)".to_string());
        }
        // Join remaining segments with `::` for Rust sub-module access.
        let full = if segments.len() > 1 {
            format!("{}::{}", name, &segments[1..].join("::"))
        } else {
            name.to_string()
        };
        return Ok(ModuleTarget::Crate(full));
    }
    let dir = importer.parent().ok_or_else(|| {
        format!(
            "could not determine the directory of `{}`",
            importer.display()
        )
    })?;
    let mut candidate = dir.to_path_buf();
    for seg in segments {
        candidate.push(seg);
    }
    candidate.set_extension("tsr");
    match std::fs::canonicalize(&candidate) {
        Ok(p) => Ok(ModuleTarget::Local(p)),
        Err(_) => Err(format!(
            "module `{}` not found (looked at `{}`)",
            segments.join("."),
            candidate.display()
        )),
    }
}

/// Return the id of a module, derived from the file stem.
fn module_id(path: &Path) -> Result<String, String> {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| format!("invalid file name: {}", path.display()))?;
    Ok(sanitize_module_id(stem))
}

/// Sanitise a string so it is a valid identifier.
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

/// Verify that no two modules share the same id.
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
                Stmt::Let { name, .. }
                | Stmt::Const { name, .. }
                | Stmt::TypeAlias { name, .. }
                | Stmt::Interface { name, .. }
                | Stmt::Enum { name, .. } => {
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

/// Verify that every import references exports that actually exist on the
/// target module.
fn validate_imports(modules: &HashMap<PathBuf, Module>) -> Result<(), String> {
    for m in modules.values() {
        for stmt in &m.program.stmts {
            match stmt {
                Stmt::Import { module, .. } => {
                    // Namespace imports just need the module to exist — already
                    // checked during load. But we still verify the target.
                    validate_target(modules, &m.path, module)?;
                }
                Stmt::FromImport {
                    module,
                    names,
                    wildcard,
                } => {
                    let target = validate_target(modules, &m.path, module)?;
                    if *wildcard {
                        continue; // wildcard imports everything — always valid
                    }
                    match target {
                        ModuleTarget::Crate(_) | ModuleTarget::Builtin(_) => {
                            // External crates validated by `rustc`;
                            // builtins are provided by the runtime.
                        }
                        ModuleTarget::Local(dep_path) => {
                            let target_mod = modules.get(&dep_path).ok_or_else(|| {
                                format!(
                                    "module `{}` was not loaded (internal error)",
                                    module.join(".")
                                )
                            })?;
                            for n in names {
                                // `default` refers to the module's
                                // `export default`, not a named export.
                                if n.name == "default" {
                                    if target_mod.exports.default.is_none() {
                                        return Err(format!(
                                            "`{}` has no default export",
                                            module.join(".")
                                        ));
                                    }
                                    continue;
                                }
                                if !target_mod.exports.named.contains(&n.name) {
                                    return Err(format!(
                                        "`{}` does not export a symbol named `{}`",
                                        module.join("."),
                                        n.name
                                    ));
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
    Ok(())
}

/// Resolve the module target and verify it exists (for namespace imports).
fn validate_target(
    _modules: &HashMap<PathBuf, Module>,
    importer: &Path,
    module: &[String],
) -> Result<ModuleTarget, String> {
    resolve_specifier(importer, module)
}

/// Known builtin namespaces that are provided by the runtime without a file.
/// `json` isn't a namespace in the `sys.*` sense (there's no `json.parse`) —
/// `json(...)` is a bare global builtin, same as `print`/`input`, needing no
/// import at all. Registering it here only makes a redundant `import json;`
/// resolve cleanly instead of erroring as "module not found", matching how
/// `import sys;` is likewise accepted-but-unnecessary today.
fn is_builtin_ns(name: &str) -> bool {
    matches!(name, "sys" | "json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_id_accepts_valid_identifiers() {
        assert_eq!(module_id(Path::new("/tmp/utils.tsr")).unwrap(), "utils");
    }

    #[test]
    fn module_id_sanitises_invalid() {
        assert_eq!(module_id(Path::new("/tmp/my-mod.tsr")).unwrap(), "my_mod");
        assert_eq!(
            module_id(Path::new("/tmp/2starts.tsr")).unwrap(),
            "_2starts"
        );
        assert_eq!(
            module_id(Path::new("/tmp/01-hello_world.tsr")).unwrap(),
            "_01_hello_world"
        );
    }
}
