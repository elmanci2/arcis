//! Per-module Cranelift lowering.
//!
//! Each Arcis module compiles into one Cranelift [`ObjectModule`]:
//!
//! - All runtime functions are declared as `Linkage::Import` (they live in
//!   the separately-compiled `arcis_runtime.o`).
//! - Top-level Arcis functions are declared as `Linkage::Export` in their
//!   own module so the linker can resolve cross-module calls.
//! - Functions imported from other Arcis modules are declared as
//!   `Linkage::Import` in the importing module.
//! - The root module additionally defines `arcis_main`, which contains the
//!   lowered top-level statements (variable declarations, calls, control
//!   flow). This is what the `main()` stub in the C runtime calls.
//!
//! Non-root modules define their exported functions but do not emit a
//! `main` — their top-level `let`/`const` statements become module-level
//! initializers (global constants).

use std::collections::HashMap;

use arcis_ast::Stmt;
use arcis_linker::Module;
use cranelift_codegen::entity::EntityRef;
use cranelift_codegen::ir::{AbiParam, Function, InstBuilder, UserFuncName};
use cranelift_codegen::Context;
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::{FuncId, Linkage, Module as CraneliftModule};
use cranelift_object::ObjectModule;

use crate::collect::collect_reassigned;
use crate::context::FunctionCtx;
use crate::function::emit_function;
use crate::rt::Runtime;
use crate::stmt::emit_stmt;

/// Emit one Arcis module into `obj_module`.
pub(crate) fn emit(
    obj_module: &mut ObjectModule,
    m: &Module,
    is_root: bool,
    all_modules: &[Module],
) -> Result<(), String> {
    let runtime = Runtime::declare(obj_module)?;
    let reassigned = collect_reassigned(&m.program);

    // ── Build a lookup: (exported_name) → (module that exports it, signature) ──
    let export_info = build_export_map(all_modules);

    // ── Declare own top-level functions as Export ──────────────────────────
    let mut user_fns: HashMap<String, FuncId> = HashMap::new();
    for stmt in &m.program.stmts {
        match stmt {
            Stmt::Function(f) => {
                let sig = function_signature(obj_module, f);
                let id = obj_module
                    .declare_function(f.name.as_str(), Linkage::Export, &sig)
                    .map_err(|e| format!("declare `{}`: {}", f.name, e))?;
                user_fns.insert(f.name.clone(), id);
            }
            Stmt::ExportDecl(inner) => {
                if let Stmt::Function(f) = inner.as_ref() {
                    let sig = function_signature(obj_module, f);
                    let id = obj_module
                        .declare_function(f.name.as_str(), Linkage::Export, &sig)
                        .map_err(|e| format!("declare export `{}`: {}", f.name, e))?;
                    user_fns.insert(f.name.clone(), id);
                }
            }
            Stmt::ExportDefault(arcis_ast::ExportDefault::Function(f)) => {
                let name = if f.name.is_empty() { "__default".into() } else { f.name.clone() };
                let sig = function_signature(obj_module, f);
                let id = obj_module
                    .declare_function(&name, Linkage::Export, &sig)
                    .map_err(|e| format!("declare export default `{}`: {}", name, e))?;
                user_fns.insert(name, id);
            }
            _ => {}
        }
    }

    // ── Process imports: declare imported functions as Import ─────────────
    for stmt in &m.program.stmts {
        match stmt {
            Stmt::Import { module: path, .. } => {
                // Namespace import: `import test` — declare all its exports.
                if !path.first().map_or(false, |s| s == "sys") {
                    declare_imported_module(obj_module, &m.path, path, &export_info, &mut user_fns)?;
                }
            }
            Stmt::FromImport { module: path, names, wildcard } => {
                if !path.first().map_or(false, |s| s == "sys") {
                    if *wildcard {
                        declare_imported_module(obj_module, &m.path, path, &export_info, &mut user_fns)?;
                    } else {
                        for n in names {
                            let local = n.alias.clone().unwrap_or_else(|| n.name.clone());
                            let sig = lookup_export_sig(&m.path, path, &n.name, &export_info)?;
                            let id = obj_module
                                .declare_function(&local, Linkage::Import, &sig)
                                .map_err(|e| format!("declare import `{}`: {}", local, e))?;
                            user_fns.insert(local, id);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    // ── Define each own function body ────────────────────────────────────
    for stmt in &m.program.stmts {
        let func_ast: Option<&arcis_ast::Function> = match stmt {
            Stmt::Function(f) => Some(f),
            Stmt::ExportDecl(inner) => match inner.as_ref() {
                Stmt::Function(f) => Some(f),
                _ => None,
            },
            Stmt::ExportDefault(arcis_ast::ExportDefault::Function(f)) => Some(f),
            _ => None,
        };
        if let Some(f) = func_ast {
            let lookup_name = if f.name.is_empty() { "__default" } else { &f.name };
            let func_id = *user_fns.get(lookup_name).ok_or_else(|| {
                format!("function `{}` not found in user_fns (internal error)", lookup_name)
            })?;
            let sig = function_signature(obj_module, f);
            let mut ctx = Context::new();
            ctx.func = Function::with_name_signature(
                UserFuncName::user(0, func_id.index() as u32),
                sig,
            );
            emit_function(
                &mut ctx.func,
                f,
                &user_fns,
                &runtime,
                &reassigned,
                obj_module,
            )?;
            obj_module
                .define_function(func_id, &mut ctx)
                .map_err(|e| format!("define `{}`: {}", lookup_name, e))?;
        }
    }

    // ── arcis_main — only for root ───────────────────────────────────────
    if is_root {
        let main_sig = { obj_module.make_signature() };
        let main_id = obj_module
            .declare_function("arcis_main", Linkage::Export, &main_sig)
            .map_err(|e| format!("declare arcis_main: {}", e))?;
        let mut ctx = Context::new();
        ctx.func = Function::with_name_signature(
            UserFuncName::user(0, main_id.index() as u32),
            main_sig.clone(),
        );
        {
            // Pre-validate.
            let flags = cranelift_codegen::settings::Flags::new(
                cranelift_codegen::settings::builder(),
            );
            if let Err(errs) = cranelift_codegen::verifier::verify_function(&ctx.func, &flags) {
                eprintln!("=== VERIFY ERROR: {} ===", errs);
                eprintln!("{}", ctx.func.display());
                return Err(format!("verify arcis_main: {}", errs));
            }

            let mut builder_ctx = FunctionBuilderContext::new();
            let mut builder = FunctionBuilder::new(&mut ctx.func, &mut builder_ctx);
            let entry = builder.create_block();
            builder.append_block_params_for_function_params(entry);
            builder.switch_to_block(entry);
            builder.seal_block(entry);

            let mut fctx = FunctionCtx::new(&reassigned);

            for stmt in &m.program.stmts {
                match stmt {
                    Stmt::Function(_) | Stmt::Import { .. } | Stmt::FromImport { .. } => {
                        // Already handled above.
                    }
                    _ => {
                        emit_stmt(
                            &mut builder,
                            &mut fctx,
                            stmt,
                            &runtime,
                            &user_fns,
                            obj_module,
                        )?;
                    }
                }
            }

            builder.ins().return_(&[]);
            builder.finalize();
        }
        obj_module
            .define_function(main_id, &mut ctx)
            .map_err(|e| format!("define arcis_main: {}", e))?;
    }

    Ok(())
}

// ── Import helpers ──────────────────────────────────────────────────────────

/// Info about one exported symbol from a module.
struct ExportInfo {
    /// The Arcis function AST (params + return type).
    func: arcis_ast::Function,
}

/// Map: (module_path_canonical) → (exported_name → ExportInfo)
type ExportMap = HashMap<std::path::PathBuf, HashMap<String, ExportInfo>>;

/// Build a lookup table of every exported function in every module.
fn build_export_map(all_modules: &[Module]) -> ExportMap {
    let mut map: ExportMap = HashMap::new();
    for m in all_modules {
        let mut exports: HashMap<String, ExportInfo> = HashMap::new();
        for stmt in &m.program.stmts {
            match stmt {
                Stmt::Function(f) => {
                    if m.exports.named.contains(&f.name) || m.exports.default.as_deref() == Some(&f.name) {
                        exports.insert(f.name.clone(), ExportInfo { func: f.clone() });
                    }
                }
                Stmt::ExportDecl(inner) => {
                    if let Stmt::Function(f) = inner.as_ref() {
                        exports.insert(f.name.clone(), ExportInfo { func: f.clone() });
                    }
                }
                Stmt::ExportSpec(items) => {
                    for it in items {
                        // Re-exports must point to a function declared in the same module.
                        // Look up the original function.
                        if let Some(orig) = find_function_in_module(m, &it.name) {
                            let alias = it.alias.clone().unwrap_or_else(|| it.name.clone());
                            exports.insert(alias, ExportInfo { func: orig.clone() });
                        }
                    }
                }
                Stmt::ExportDefault(ed) => {
                    let (name, func) = match ed {
                        arcis_ast::ExportDefault::Function(f) => {
                            let n = if f.name.is_empty() { "__default".into() } else { f.name.clone() };
                            (n, f.clone())
                        }
                        _ => continue,
                    };
                    exports.insert(name, ExportInfo { func });
                }
                _ => {}
            }
        }
        map.insert(m.path.clone(), exports);
    }
    map
}

/// Find a function AST in a module by name.
fn find_function_in_module(m: &Module, name: &str) -> Option<arcis_ast::Function> {
    for stmt in &m.program.stmts {
        match stmt {
            Stmt::Function(f) if f.name == name => return Some(f.clone()),
            Stmt::ExportDecl(inner) => {
                if let Stmt::Function(f) = inner.as_ref() {
                    if f.name == name { return Some(f.clone()); }
                }
            }
            Stmt::ExportDefault(arcis_ast::ExportDefault::Function(f)) => {
                if f.name == name || (f.name.is_empty() && name == "__default") {
                    return Some(f.clone());
                }
            }
            _ => {}
        }
    }
    None
}

/// Look up the signature of an exported function from another module.
fn lookup_export_sig(
    importer: &std::path::Path,
    path: &[String],
    name: &str,
    export_map: &ExportMap,
) -> Result<cranelift_codegen::ir::Signature, String> {
    let target = arcis_linker::resolve_specifier(importer, path).map_err(|e| e)?;
    match target {
        arcis_linker::ModuleTarget::Local(dep_path) => {
            let mod_exports = export_map.get(&dep_path).ok_or_else(|| {
                format!("module `{}` not found in export map", path.join("."))
            })?;
            let info = mod_exports.get(name).ok_or_else(|| {
                format!(
                    "`{}` does not export `{}`",
                    path.join("."),
                    name
                )
            })?;
            Ok(import_sig_from_func(&info.func))
        }
        _ => Err(format!(
            "`{}` is not a local module — only file-based imports are supported",
            path.join(".")
        )),
    }
}

/// Declare all exports from a module as imports (for namespace or wildcard).
fn declare_imported_module(
    obj_module: &mut ObjectModule,
    importer: &std::path::Path,
    path: &[String],
    export_map: &ExportMap,
    user_fns: &mut HashMap<String, FuncId>,
) -> Result<(), String> {
    let target = arcis_linker::resolve_specifier(importer, path).map_err(|e| e)?;
    match target {
        arcis_linker::ModuleTarget::Local(dep_path) => {
            let mod_exports = export_map.get(&dep_path).ok_or_else(|| {
                format!("module `{}` not found in export map", path.join("."))
            })?;
            for (name, info) in mod_exports {
                if user_fns.contains_key(name) { continue; }
                let sig = import_sig_from_func(&info.func);
                let id = obj_module
                    .declare_function(name, Linkage::Import, &sig)
                    .map_err(|e| format!("declare import `{}`: {}", name, e))?;
                user_fns.insert(name.clone(), id);
            }
        }
        _ => {
            return Err(format!(
                "`{}` is not a local module — only file-based imports are supported",
                path.join(".")
            ));
        }
    }
    Ok(())
}

/// Convert an Arcis function AST to a Cranelift import signature.
fn import_sig_from_func(f: &arcis_ast::Function) -> cranelift_codegen::ir::Signature {
    use crate::types::{from_ast, ArcisType};

    let mut sig = cranelift_codegen::ir::Signature::new(
        cranelift_codegen::isa::CallConv::SystemV,
    );
    for p in &f.params {
        let ty = from_ast(&p.ty.name, p.ty.is_array).unwrap_or(ArcisType::Number);
        sig.params.push(AbiParam::new(ty.to_cl()));
    }
    if f.return_type.name != "void" {
        let ty = from_ast(&f.return_type.name, f.return_type.is_array)
            .unwrap_or(ArcisType::Number);
        sig.returns.push(AbiParam::new(ty.to_cl()));
    }
    sig
}

/// Build a Cranelift signature for an Arcis function defined in this module.
fn function_signature(
    module: &mut ObjectModule,
    f: &arcis_ast::Function,
) -> cranelift_codegen::ir::Signature {
    use crate::types::{from_ast, ArcisType};

    let mut sig = module.make_signature();
    for p in &f.params {
        let ty = from_ast(&p.ty.name, p.ty.is_array).unwrap_or(ArcisType::Number);
        sig.params.push(AbiParam::new(ty.to_cl()));
    }
    if f.return_type.name != "void" {
        let ty = from_ast(&f.return_type.name, f.return_type.is_array)
            .unwrap_or(ArcisType::Number);
        sig.returns.push(AbiParam::new(ty.to_cl()));
    }
    sig
}
