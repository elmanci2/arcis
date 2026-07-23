//! Per-module emission.
//!
//! Generates the Rust source for one module at a time:
//!
//! 1. Pre-amble (`#![allow(...)]`).
//! 2. Root-only `mod <other>;` declarations and `pub struct` definitions.
//! 3. `use crate::<id>::...` for every imported binding.
//! 4. `use crate::__Obj...;` for every referenced object type (non-root).
//! 5. `pub use self::...` for every `export { ... }` entry.
//! 6. Top-level items (functions, default export).
//! 7. Either `fn main() { ... }` (root) or `const` items (non-root).

use std::collections::{HashMap, HashSet};

use arcis_ast::{ExportDefault, Expr, Program, Stmt, Type};
use arcis_linker::{resolve_specifier, Module, ModuleTarget};

use crate::context::Ctx;

/// Generate the Rust source for a single module.
pub(crate) fn generate(
    m: &Module,
    is_root: bool,
    modules: &[Module],
    all_obj_types: &[Type],
    enums: &[(String, Vec<(String, Option<i64>)>)],
    enum_names: &HashSet<String>,
    path_to_id: &HashMap<std::path::PathBuf, String>,
) -> Result<String, String> {
    let mut out = String::new();
    out.push_str("#![allow(unused_parens, non_snake_case, while_true, unused_imports, dead_code)]\n\n");

    let reassigned = crate::collect::collect_reassigned(&m.program);
    let types = crate::collect::collect_types(&m.program);
    let ctx = Ctx {
        reassigned: &reassigned,
        types: &types,
        current_let_type: None,
        current_return_type: None,
        is_root,
        enum_names,
    };

    // The root declares every sub-module and defines the object-type
    // structs and enums.
    if is_root {
        for other in modules.iter().skip(1) {
            out.push_str(&format!("mod {};\n", other.id));
        }
        out.push('\n');
        for ty in all_obj_types {
            crate::types::emit_struct_def(&mut out, ty);
        }
        for (name, variants) in enums {
            crate::types::emit_enum_def(&mut out, name, variants);
        }
    }

    // imports → `use crate::<id>::...`
    emit_imports(&mut out, &m.program, &m.path, modules, path_to_id)?;

    // Object-type references in this module → `use crate::__Obj...;`
    // (necessary in non-root modules because the structs live in the root).
    if !is_root {
        let mut seen: HashSet<String> = HashSet::new();
        for stmt in &m.program.stmts {
            crate::collect::collect_object_type_names(stmt, &mut seen);
        }
        for name in seen {
            out.push_str(&format!("use crate::{};\n", name));
        }
    }

    // `export { a, b as c }` → `pub use self::a; pub use self::b as c;`
    emit_export_specs(&mut out, &m.program);

    // Top-level functions (pub if exported) and default export.
    for stmt in &m.program.stmts {
        emit_top_item(&mut out, stmt, &ctx);
    }

    if is_root {
        // fn main() wrapping the lets/consts and expression statements.
        // Functions were emitted above as items.
        out.push_str("fn main() {\n");
        for stmt in &m.program.stmts {
            crate::stmt::emit(&mut out, stmt, 1, &ctx);
        }
        out.push_str("}\n");
    } else {
        // In a non-root module, top-level lets/consts become `const` items
        // (Rust requires const-evaluable values for items; see the README).
        for stmt in &m.program.stmts {
            emit_module_const(&mut out, stmt, &ctx);
        }
    }

    Ok(out)
}

/// Emit Rust `use` statements for all import declarations.
fn emit_imports(
    out: &mut String,
    program: &Program,
    importer_path: &std::path::Path,
    modules: &[Module],
    path_to_id: &HashMap<std::path::PathBuf, String>,
) -> Result<(), String> {
    for stmt in &program.stmts {
        match stmt {
            // `import utils [as u]` — namespace import
            Stmt::Import { module, alias } => {
                match resolve_specifier(importer_path, module)? {
                    ModuleTarget::Builtin(_) => {
                        // Builtin namespaces (e.g. `sys`) are provided by the
                        // runtime — nothing to emit at the Rust level.
                    }
                    ModuleTarget::Crate(crate_name) => {
                        let local = alias
                            .clone()
                            .unwrap_or_else(|| crate_name.replace("::", "_"));
                        out.push_str(&format!(
                            "use {} as {};\n",
                            crate_name, local
                        ));
                    }
                    ModuleTarget::Local(dep_path) => {
                        let dep_id = path_to_id.get(&dep_path).ok_or_else(|| {
                            format!(
                                "module `{}` not resolved to an id (internal error)",
                                module.join(".")
                            )
                        })?;
                        let local = alias.clone().unwrap_or_else(|| dep_id.clone());
                        out.push_str(&format!(
                            "use crate::{} as {};\n",
                            dep_id, local
                        ));
                    }
                }
            }
            // `from utils import a, b as c` or `from utils import *`
            Stmt::FromImport {
                module,
                names,
                wildcard,
            } => {
                match resolve_specifier(importer_path, module)? {
                    ModuleTarget::Builtin(_) => {
                        // Builtin symbols (e.g. `from sys import gpu`) are
                        // provided by the runtime — nothing to emit.
                    }
                    ModuleTarget::Crate(crate_name) => {
                        if *wildcard {
                            out.push_str(&format!("use {}::*;\n", crate_name));
                        } else {
                            for n in names {
                                let local =
                                    n.alias.clone().unwrap_or_else(|| n.name.clone());
                                if local == n.name {
                                    out.push_str(&format!(
                                        "use {}::{};\n",
                                        crate_name, n.name
                                    ));
                                } else {
                                    out.push_str(&format!(
                                        "use {}::{} as {};\n",
                                        crate_name, n.name, local
                                    ));
                                }
                            }
                        }
                    }
                    ModuleTarget::Local(dep_path) => {
                        let dep_id = path_to_id.get(&dep_path).ok_or_else(|| {
                            format!(
                                "module `{}` not resolved to an id (internal error)",
                                module.join(".")
                            )
                        })?;
                        if *wildcard {
                            out.push_str(&format!("use crate::{}::*;\n", dep_id));
                        } else {
                            for n in names {
                                let local =
                                    n.alias.clone().unwrap_or_else(|| n.name.clone());
                                if local == n.name {
                                    out.push_str(&format!(
                                        "use crate::{}::{};\n",
                                        dep_id, n.name
                                    ));
                                } else {
                                    out.push_str(&format!(
                                        "use crate::{}::{} as {};\n",
                                        dep_id, n.name, local
                                    ));
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// `export { a, b as c };` → `pub use self::a;` / `pub use self::b as c;`
fn emit_export_specs(out: &mut String, program: &Program) {
    for stmt in &program.stmts {
        if let Stmt::ExportSpec(items) = stmt {
            for it in items {
                match &it.alias {
                    Some(alias) => out.push_str(&format!("pub use self::{} as {};\n", it.name, alias)),
                    None => out.push_str(&format!("pub use self::{};\n", it.name)),
                }
            }
        }
    }
}

/// Emit top-level functions (free or exported) and the default export as
/// items. Lets / consts are handled separately.
fn emit_top_item(out: &mut String, stmt: &Stmt, ctx: &Ctx) {
    match stmt {
        Stmt::Function(f) => crate::function::emit(out, f, ctx, false),
        Stmt::ExportDecl(inner) => {
            if let Stmt::Function(f) = inner.as_ref() {
                crate::function::emit(out, f, ctx, true);
            }
            // exported lets/consts: in the root they live in `main()`;
            // in non-root they become `const` items (`emit_module_const`).
        }
        Stmt::ExportDefault(ed) => match ed {
            ExportDefault::Function(f) => {
                let mut f = f.clone();
                if f.name.is_empty() {
                    f.name = "__default".into();
                }
                crate::function::emit(out, &f, ctx, true);
            }
            ExportDefault::Expr(e) => emit_default_const(out, e, ctx),
        },
        _ => {}
    }
}

/// Top-level lets / consts in a non-root module become `const` items.
fn emit_module_const(out: &mut String, stmt: &Stmt, ctx: &Ctx) {
    let (name, ty, value, pub_) = match stmt {
        Stmt::Const { name, ty, value, .. } => (name, ty, value, false),
        Stmt::Let { name, ty, value, .. } => (name, ty, value, false),
        Stmt::ExportDecl(inner) => match inner.as_ref() {
            Stmt::Const { name, ty, value, .. } | Stmt::Let { name, ty, value, .. } => {
                (name, ty, value, true)
            }
            _ => return,
        },
        _ => return,
    };
    out.push_str(if pub_ { "pub const " } else { "const " });
    out.push_str(name);
    if let Some(t) = ty {
        out.push_str(": ");
        out.push_str(&crate::types::ts_type_to_rust(t, ctx.is_root));
    }
    out.push_str(" = ");
    if let (Some(t), Expr::ArrayLiteral { elements }) = (ty, value) {
        if elements.is_empty() && t.is_array() {
            out.push_str("Vec::new();\n\n");
            return;
        }
    }
    let nested = Ctx {
        reassigned: ctx.reassigned,
        types: ctx.types,
        current_let_type: ty.as_ref(),
        current_return_type: None,
        is_root: ctx.is_root,
        enum_names: ctx.enum_names,
    };
    crate::expr::emit(out, value, &nested);
    out.push_str(";\n\n");
}

/// `export default <expr>;` → `pub const __default: T = expr;`
/// Only works if `expr` is const-evaluable (rustc will enforce this).
fn emit_default_const(out: &mut String, e: &Expr, ctx: &Ctx) {
    match crate::types::infer_expr_rust_type(e) {
        Some(ty) => {
            out.push_str(&format!("pub const __default: {} = ", ty));
        }
        None => out.push_str("pub const __default = "),
    }
    crate::expr::emit(out, e, ctx);
    out.push_str(";\n\n");
}