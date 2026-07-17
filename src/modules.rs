//! Linker / resolver de módulos.
//!
//! Recibe el punto de entrada (por convención `main.tsr`) y resuelve en
//! profundidad (DFS) todas las dependencias declaradas con `import`,
//! produciendo la lista completa de módulos del programa.
//!
//! Resolución de rutas: el specifier de un import (`from "utils"`) se
//! interpreta relativo al directorio del archivo que importa, buscando
//! `<dir>/<specifier>.tsr` (sin prefijo `./`, estilo Node/TS simplificado).
//!
//! El codegen genera un `.rs` por módulo declarando `mod <id>;` desde la
//! raíz (`main`) y referenciando items vía `use crate::<id>::...`. Por eso:
//!   - cada `id` (= stem del archivo) debe ser un identificador Rust válido;
//!   - no puede haber dos módulos con el mismo `id`.

use crate::ast::{ExportDefault, Program, Stmt};
use crate::{lexer, parser};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Un módulo cargado: id (= stem del archivo), ruta canónica, su AST y los
/// exports que ofrece (para que el codegen pueda generar `use crate::...`).
#[derive(Debug)]
pub struct Module {
    pub id: String,
    pub path: PathBuf,
    pub program: Program,
    pub exports: Exports,
}

/// Exports públicos de un módulo.
/// `named` es el conjunto de nombres exportados (`export function/const`,
/// `export { a, b as c }`). `default` es el nombre real del símbolo default
/// (`f` si `export default function f`, `__default` en caso anónimo/expr).
#[derive(Debug, Default, Clone)]
pub struct Exports {
    pub named: HashSet<String>,
    pub default: Option<String>,
}

/// Resuelve el programa completo desde `input` y devuelve la lista de
/// módulos con la entrada (`main`) primero.
///
/// `input` puede ser:
/// - un directorio: se busca `<input>/main.tsr`;
/// - un archivo `.tsr`: se usa tal cual (punto de entrada).
pub fn resolve(input: &Path) -> Result<Vec<Module>, String> {
    let entry = resolve_entry(input)?;
    let mut modules: HashMap<PathBuf, Module> = HashMap::new();
    let mut order: Vec<PathBuf> = Vec::new();
    let mut stack: Vec<PathBuf> = Vec::new();

    load(&entry, &mut modules, &mut order, &mut stack)?;

    check_unique_ids(&modules)?;
    validate_imports(&modules)?;

    // Devolver en orden de descubrimiento (entry primero).
    Ok(order.into_iter().filter_map(|p| modules.remove(&p)).collect())
}

/// Decide el archivo de entrada: directorio → `<dir>/main.tsr`, archivo →
/// tal cual. Luego canonicaliza la ruta.
fn resolve_entry(input: &Path) -> Result<PathBuf, String> {
    let candidate = if input.is_dir() {
        input.join("main.tsr")
    } else {
        input.to_path_buf()
    };
    if !candidate.exists() {
        return Err(format!(
            "no se encontró el punto de entrada `{}` (se espera un `main.tsr`)",
            candidate.display()
        ));
    }
    std::fs::canonicalize(&candidate)
        .map_err(|e| format!("no se pudo resolver `{}`: {}", candidate.display(), e))
}

/// DFS recursivo: carga `path`, resuelve sus imports y los carga también.
/// `modules` indexa por ruta canónica (dedup); `order` es pre-order (entry
/// primero); `stack` detecta ciclos.
fn load(
    path: &Path,
    modules: &mut HashMap<PathBuf, Module>,
    order: &mut Vec<PathBuf>,
    stack: &mut Vec<PathBuf>,
) -> Result<(), String> {
    let canon = std::fs::canonicalize(path)
        .map_err(|e| format!("no se pudo resolver `{}`: {}", path.display(), e))?;

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
            "dependencia cíclica entre módulos: {}",
            chain.join(" -> ")
        ));
    }

    let source = std::fs::read_to_string(&canon)
        .map_err(|e| format!("no se pudo leer `{}`: {}", canon.display(), e))?;
    let tokens = lexer::lex(&source)
        .map_err(|e| format!("{}: {}", canon.display(), e))?;
    let program = parser::parse(tokens)
        .map_err(|e| format!("{}: {}", canon.display(), e))?;

    let id = module_id(&canon)?;

    stack.push(canon.clone());
    order.push(canon.clone());

    // Resolver y cargar dependencias antes de registrar el módulo.
    for stmt in &program.stmts {
        if let Stmt::Import { module: spec, .. } = stmt {
            match resolve_specifier(&canon, spec)? {
                ModuleTarget::Local(dep) => load(&dep, modules, order, stack)?,
                ModuleTarget::Crate(_) => {} // las crates externas no se cargan
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

/// A qué apunta un `import ... from "<spec>"`:
/// - `Local`: un módulo `.tsr` del programa, resuelto relativamente.
/// - `Crate`: una dependencia de Rust declarada con el prefijo `crate:`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModuleTarget {
    Local(PathBuf),
    Crate(String),
}

/// Resuelve el specifier de un import.
/// Si empieza con `crate:`, devuelve `Crate(<nombre>)` y NO toca el FS.
/// El nombre puede contener `::` (p.ej. `crate:reqwest::blocking`) para
/// apuntar a un submódulo del crate. En caso contrario, busca
/// `<dir>/<spec>.tsr` (canonicalizado).
pub fn resolve_specifier(importer: &Path, spec: &str) -> Result<ModuleTarget, String> {
    if let Some(name) = spec.strip_prefix("crate:") {
        if name.is_empty() {
            return Err("crate de Rust sin nombre (`from \"crate:\"`)".to_string());
        }
        return Ok(ModuleTarget::Crate(name.to_string()));
    }
    let dir = importer.parent().ok_or_else(|| {
        format!("no se pudo determinar el directorio de `{}`", importer.display())
    })?;
    let candidate = dir.join(format!("{}.tsr", spec));
    match std::fs::canonicalize(&candidate) {
        Ok(p) => Ok(ModuleTarget::Local(p)),
        Err(_) => Err(format!(
            "módulo no encontrado `{}` (buscado en `{}`)",
            spec,
            candidate.display()
        )),
    }
}

/// Devuelve el id de un módulo (= stem del archivo) validado como
/// identificador Rust: `[A-Za-z_][A-Za-z0-9_]*`.
fn module_id(path: &Path) -> Result<String, String> {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| format!("nombre de archivo inválido: {}", path.display()))?;
    let valid_start = stem
        .chars()
        .next()
        .map(|c| c.is_ascii_alphabetic() || c == '_')
        .unwrap_or(false);
    let valid_rest = stem.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !(valid_start && valid_rest) {
        return Err(format!(
            "nombre de módulo inválido `{}`: debe ser un identificador (letras, dígitos o `_`, sin empezar por dígito y sin guiones)",
            stem
        ));
    }
    Ok(stem.to_string())
}

/// Verifica que no haya dos módulos con el mismo id (colisionarían en el
/// `mod <id>;` y en el archivo `.rs` generado).
fn check_unique_ids(modules: &HashMap<PathBuf, Module>) -> Result<(), String> {
    let mut seen: HashMap<String, PathBuf> = HashMap::new();
    for m in modules.values() {
        if let Some(prev) = seen.get(&m.id) {
            return Err(format!(
                "dos módulos con el mismo nombre `{}`: {} y {}",
                m.id,
                prev.display(),
                m.path.display()
            ));
        }
        seen.insert(m.id.clone(), m.path.clone());
    }
    Ok(())
}

/// Recolecta los exports de un programa.
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
                    // El nombre exportado es el alias si existe, si no, el local.
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

/// Nombre real del símbolo default en el código Rust generado.
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

/// Valida que cada import referencie exports que existen en el módulo destino.
fn validate_imports(modules: &HashMap<PathBuf, Module>) -> Result<(), String> {
    for m in modules.values() {
        for stmt in &m.program.stmts {
            if let Stmt::Import { default, named, module: spec } = stmt {
                let target = resolve_specifier(&m.path, spec)?;
                match target {
                    ModuleTarget::Crate(_) => {
                        // Los crates de Rust son externos; el compilador de Rust
                        // validará al compilar. Solo restringimos el default.
                        if default.is_some() {
                            return Err(format!(
                                "`import x from \"crate:{}\"` no está soportado: \
                                 los crates de Rust no tienen `default export`",
                                spec.strip_prefix("crate:").unwrap_or(spec)
                            ));
                        }
                    }
                    ModuleTarget::Local(dep_path) => {
                        let target_mod = modules.get(&dep_path).ok_or_else(|| {
                            format!("módulo `{}` no fue cargado (error interno)", spec)
                        })?;

                        if let Some(_local) = default {
                            if target_mod.exports.default.is_none() {
                                return Err(format!(
                                    "`{}` no exporta un valor por defecto (default export)",
                                    spec
                                ));
                            }
                        }
                        for n in named {
                            if !target_mod.exports.named.contains(&n.name) {
                                return Err(format!(
                                    "`{}` no exporta ningún símbolo llamado `{}`",
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
