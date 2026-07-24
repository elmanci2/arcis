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

use crate::collect::{collect_enums, collect_reassigned};
use crate::context::{FnInfo, FunctionCtx};
use crate::function::emit_function;
use crate::rt::Runtime;
use crate::stmt::emit_stmt;
use crate::types::{from_ast, ArcisType};

/// Emit one Arcis module into `obj_module`.
pub(crate) fn emit(
    obj_module: &mut ObjectModule,
    m: &Module,
    is_root: bool,
    all_modules: &[Module],
) -> Result<(), String> {
    let runtime = Runtime::declare(obj_module)?;
    let reassigned = collect_reassigned(&m.program);
    // Enums resolve to compile-time constants, so collect them from EVERY
    // linked module — `Category.Electronics` must work in any module, not
    // just the one that declares the enum.
    let mut enums = collect_enums(&m.program);
    for other in all_modules {
        for (name, table) in collect_enums(&other.program) {
            enums.entry(name).or_insert(table);
        }
    }
    let enum_names: std::collections::HashSet<String> = enums.keys().cloned().collect();
    // Type-level names (interfaces / type aliases / enums, any module):
    // importing one is valid Arcis but there is no function symbol behind
    // it — skip them when declaring imported functions.
    let type_level_names = collect_type_level_names(all_modules);
    // Module-level constants visible inside function bodies: the module's
    // own top-level `let`/`const` initializers (non-root — in the root they
    // are ordinary `arcis_main` locals) plus imported constants, collected
    // below while walking the import statements.
    let mut global_consts: HashMap<String, arcis_ast::Expr> = HashMap::new();
    if !is_root {
        for stmt in &m.program.stmts {
            let inner = match stmt {
                Stmt::ExportDecl(inner) => inner.as_ref(),
                other => other,
            };
            match inner {
                Stmt::Let { name, value, .. } | Stmt::Const { name, value, .. }
                    if !matches!(value, arcis_ast::Expr::Arrow { .. }) =>
                {
                    global_consts.insert(name.clone(), value.clone());
                }
                _ => {}
            }
        }
    }
    // Program-wide field-name → type fallback for member accesses whose
    // receiver has no tracked per-variable shape.
    let global_field_types = collect_global_field_types(all_modules, &enum_names);
    // Top-level `let name = (params) => body;` — lambda-lifted into a
    // synthetic top-level function (sound because Arcis arrows never
    // capture), declared/defined via the exact same path as `function`.
    let arrow_fns = collect_arrow_lets(&m.program.stmts);

    // ── Build a lookup: (exported_name) → (module that exports it, signature) ──
    let export_info = build_export_map(all_modules);

    // ── Declare own top-level functions as Export ──────────────────────────
    let mut user_fns: HashMap<String, FnInfo> = HashMap::new();
    for (name, f) in &arrow_fns {
        let params = param_types(f, &enum_names);
        let ret = return_type_of(f, &enum_names);
        let sig = function_signature(obj_module, f, &enum_names);
        let id = obj_module
            .declare_function(name, Linkage::Export, &sig)
            .map_err(|e| format!("declare arrow `{}`: {}", name, e))?;
        user_fns.insert(name.clone(), FnInfo { id, params, ret });
    }
    for stmt in &m.program.stmts {
        match stmt {
            Stmt::Function(f) => {
                let params = param_types(f, &enum_names);
                let ret = return_type_of(f, &enum_names);
                let sig = function_signature(obj_module, f, &enum_names);
                let id = obj_module
                    .declare_function(f.name.as_str(), Linkage::Export, &sig)
                    .map_err(|e| format!("declare `{}`: {}", f.name, e))?;
                user_fns.insert(f.name.clone(), FnInfo { id, params, ret });
            }
            Stmt::ExportDecl(inner) => {
                if let Stmt::Function(f) = inner.as_ref() {
                    let params = param_types(f, &enum_names);
                    let ret = return_type_of(f, &enum_names);
                    let sig = function_signature(obj_module, f, &enum_names);
                    let id = obj_module
                        .declare_function(f.name.as_str(), Linkage::Export, &sig)
                        .map_err(|e| format!("declare export `{}`: {}", f.name, e))?;
                    user_fns.insert(f.name.clone(), FnInfo { id, params, ret });
                }
            }
            Stmt::ExportDefault(arcis_ast::ExportDefault::Function(f)) => {
                let name = if f.name.is_empty() { "__default".into() } else { f.name.clone() };
                let params = param_types(f, &enum_names);
                let ret = return_type_of(f, &enum_names);
                let sig = function_signature(obj_module, f, &enum_names);
                let id = obj_module
                    .declare_function(&name, Linkage::Export, &sig)
                    .map_err(|e| format!("declare export default `{}`: {}", name, e))?;
                user_fns.insert(name, FnInfo { id, params, ret });
            }
            _ => {}
        }
    }

    // ── Process imports: declare imported functions as Import ─────────────
    for stmt in &m.program.stmts {
        match stmt {
            Stmt::Import { module: path, alias } => {
                if !path.first().map_or(false, |s| s == "sys") {
                    declare_imported_module(obj_module, &m.path, path, &export_info, &mut user_fns, &enum_names)?;
                    // Qualified access through the namespace binding:
                    // `u.f()` resolves via a `"u.f"` key in `user_fns`,
                    // `u.CONST` via a `"u.CONST"` key in `global_consts`.
                    let local_ns = alias
                        .clone()
                        .or_else(|| path.last().cloned())
                        .unwrap_or_default();
                    if !local_ns.is_empty() {
                        if let Ok(arcis_linker::ModuleTarget::Local(dep_path)) =
                            arcis_linker::resolve_specifier(&m.path, path)
                        {
                            if let Some(mod_exports) = export_info.get(&dep_path) {
                                for (name, info) in mod_exports {
                                    let sig = import_sig_from_func(&info.func, &enum_names);
                                    let params = param_types(&info.func, &enum_names);
                                    let ret = return_type_of(&info.func, &enum_names);
                                    let id = obj_module
                                        .declare_function(&info.symbol, Linkage::Import, &sig)
                                        .map_err(|e| {
                                            format!("declare import `{}`: {}", info.symbol, e)
                                        })?;
                                    user_fns.insert(
                                        format!("{}.{}", local_ns, name),
                                        FnInfo { id, params, ret },
                                    );
                                }
                            }
                        }
                        for (cname, cinit) in
                            collect_exported_consts(&m.path, path, all_modules)
                        {
                            global_consts
                                .entry(format!("{}.{}", local_ns, cname))
                                .or_insert(cinit);
                        }
                    }
                }
            }
            Stmt::FromImport { module: path, names, wildcard } => {
                if !path.first().map_or(false, |s| s == "sys") {
                    if *wildcard {
                        declare_imported_module(obj_module, &m.path, path, &export_info, &mut user_fns, &enum_names)?;
                        // Wildcard also brings every exported constant in.
                        for (cname, cinit) in
                            collect_exported_consts(&m.path, path, all_modules)
                        {
                            global_consts.entry(cname).or_insert(cinit);
                        }
                    } else {
                        for n in names {
                            if type_level_names.contains(&n.name) {
                                continue;
                            }
                            let local = n.alias.clone().unwrap_or_else(|| n.name.clone());
                            // `default` refers to the target module's
                            // `export default` — resolve its real name.
                            let lookup = if n.name == "default" {
                                match resolve_default_name(&m.path, path, all_modules) {
                                    Some(real) => real,
                                    None => {
                                        return Err(format!(
                                            "`{}` has no default export",
                                            path.join(".")
                                        ))
                                    }
                                }
                            } else {
                                n.name.clone()
                            };
                            // Exported constants have no function symbol —
                            // record the initializer for in-place re-emission.
                            if let Some(init) =
                                find_exported_const(&m.path, path, &lookup, all_modules)
                            {
                                global_consts.insert(local, init);
                                continue;
                            }
                            let (sig, params, symbol, ret) = lookup_export_sig_and_params(&m.path, path, &lookup, &export_info, &enum_names)?;
                            // Declare under the REAL exported symbol name —
                            // that's what the target module's `.o` exports
                            // (aliased re-exports keep the original symbol).
                            // The local (possibly aliased) name only lives
                            // in `user_fns` for call resolution.
                            let id = obj_module
                                .declare_function(&symbol, Linkage::Import, &sig)
                                .map_err(|e| format!("declare import `{}`: {}", symbol, e))?;
                            user_fns.insert(local, FnInfo { id, params, ret });
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
            let fn_info = user_fns.get(lookup_name).ok_or_else(|| {
                format!("function `{}` not found in user_fns (internal error)", lookup_name)
            })?;
            let func_id = fn_info.id;
            let sig = function_signature(obj_module, f, &enum_names);
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
                &enums,
                &global_consts,
                &global_field_types,
                obj_module,
            )?;
            obj_module
                .define_function(func_id, &mut ctx)
                .map_err(|e| format!("define `{}`: {}", lookup_name, e))?;
        }
    }

    // ── Define each lambda-lifted arrow's body ───────────────────────────
    for (name, f) in &arrow_fns {
        let fn_info = user_fns.get(name).ok_or_else(|| {
            format!("arrow `{}` not found in user_fns (internal error)", name)
        })?;
        let func_id = fn_info.id;
        let sig = function_signature(obj_module, f, &enum_names);
        let mut ctx = Context::new();
        ctx.func = Function::with_name_signature(UserFuncName::user(0, func_id.index() as u32), sig);
        emit_function(&mut ctx.func, f, &user_fns, &runtime, &reassigned, &enums, &global_consts, &global_field_types, obj_module)?;
        obj_module
            .define_function(func_id, &mut ctx)
            .map_err(|e| format!("define arrow `{}`: {}", name, e))?;
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

            let mut fctx = FunctionCtx::new(&reassigned, &enums);
            fctx.global_consts = global_consts.clone();
            fctx.global_field_types = global_field_types.clone();

            for stmt in &m.program.stmts {
                match stmt {
                    Stmt::Function(_) | Stmt::Import { .. } | Stmt::FromImport { .. } => {}
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
        if let Err(e) = obj_module.define_function(main_id, &mut ctx) {
            // Re-run the verifier standalone to surface WHICH instruction
            // is invalid — `define_function`'s error alone just says
            // "Verifier errors".
            let flags = cranelift_codegen::settings::Flags::new(
                cranelift_codegen::settings::builder(),
            );
            let detail = match cranelift_codegen::verifier::verify_function(&ctx.func, &flags) {
                Err(errs) => format!("\n{}\n{}", errs, ctx.func.display()),
                Ok(()) => String::new(),
            };
            return Err(format!("define arcis_main: {}{}", e, detail));
        }
    }

    Ok(())
}

// ── Helpers ────────────────────────────────────────────────────────────────

/// Top-level `let name = (params) [: RT] => body;` becomes a synthetic
/// top-level function named `name`, declared/defined through the same
/// two-phase machinery as a real `function` declaration — sound because
/// Arcis arrows never capture variables. Returns `(var_name, synthetic
/// Function)` pairs in source order. Arrows nested inside function bodies or
/// passed inline as call arguments (e.g. `.map(x => x*2)`) are not handled
/// here — see `method.rs` for inline-callback lowering.
fn collect_arrow_lets(stmts: &[Stmt]) -> Vec<(String, arcis_ast::Function)> {
    let mut out = Vec::new();
    for stmt in stmts {
        let (name, value) = match stmt {
            Stmt::Let { name, value, .. } | Stmt::Const { name, value, .. } => (name, value),
            _ => continue,
        };
        if let arcis_ast::Expr::Arrow { params, return_type, body } = value {
            out.push((name.clone(), arrow_to_function(name, params, return_type, body)));
        }
    }
    out
}

/// Lower an `Expr::Arrow` into a synthetic top-level `arcis_ast::Function`
/// named after its binding. `return_type: None` (source omitted it)
/// defaults to `number` — Cranelift has no `rustc`-style type inference to
/// fall back on the way the Rust backend does (it just omits the closure's
/// `-> T` and lets `rustc` infer it). Documented limitation: covers the
/// common case (most inferred-return arrows return a number) but is wrong
/// for e.g. an inferred-string-returning arrow.
pub(crate) fn arrow_to_function(
    name: &str,
    params: &[arcis_ast::Param],
    return_type: &Option<arcis_ast::Type>,
    body: &arcis_ast::ArrowBody,
) -> arcis_ast::Function {
    let body_stmts = match body {
        arcis_ast::ArrowBody::Expr(e) => vec![Stmt::Return(Some((**e).clone()))],
        arcis_ast::ArrowBody::Block(stmts) => stmts.clone(),
    };
    arcis_ast::Function {
        name: name.to_string(),
        type_params: Vec::new(), // arrows can't be generic
        params: params.to_vec(),
        return_type: return_type.clone().unwrap_or_else(arcis_ast::Type::number),
        body: body_stmts,
        line: 0,
        col: 0,
    }
}

/// The declared Arcis return type of `f`, lowered (void when unannotated).
pub(crate) fn return_type_of(f: &arcis_ast::Function, enum_names: &std::collections::HashSet<String>) -> ArcisType {
    if f.return_type.primitive_name() == "void" {
        return ArcisType::Void;
    }
    from_ast(&f.return_type, enum_names).unwrap_or(ArcisType::Number)
}

pub(crate) fn param_types(f: &arcis_ast::Function, enum_names: &std::collections::HashSet<String>) -> Vec<ArcisType> {
    f.params
        .iter()
        .map(|p| from_ast(&p.ty, enum_names).unwrap_or(ArcisType::Number))
        .collect()
}

// ── Import helpers ──────────────────────────────────────────────────────────

struct ExportInfo {
    func: arcis_ast::Function,
    /// The linker-level symbol name — the function's ORIGINAL name, which
    /// is what its defining module exports. Differs from the map key for
    /// aliased re-exports (`export { gritar as gritarFuerte }`).
    symbol: String,
}

type ExportMap = HashMap<std::path::PathBuf, HashMap<String, ExportInfo>>;

fn build_export_map(all_modules: &[Module]) -> ExportMap {
    let mut map: ExportMap = HashMap::new();
    for m in all_modules {
        let mut exports: HashMap<String, ExportInfo> = HashMap::new();
        for stmt in &m.program.stmts {
            match stmt {
                Stmt::Function(f) => {
                    if m.exports.named.contains(&f.name) || m.exports.default.as_deref() == Some(&f.name) {
                        exports.insert(f.name.clone(), ExportInfo { func: f.clone(), symbol: f.name.clone() });
                    }
                }
                Stmt::ExportDecl(inner) => {
                    if let Stmt::Function(f) = inner.as_ref() {
                        exports.insert(f.name.clone(), ExportInfo { func: f.clone(), symbol: f.name.clone() });
                    }
                }
                Stmt::ExportSpec(items) => {
                    for it in items {
                        if let Some(orig) = find_function_in_module(m, &it.name) {
                            let alias = it.alias.clone().unwrap_or_else(|| it.name.clone());
                            let symbol = orig.name.clone();
                            exports.insert(alias, ExportInfo { func: orig.clone(), symbol });
                        }
                    }
                }
                Stmt::ExportDefault(ed) => {
                    let (name, func) = match ed {
                        arcis_ast::ExportDefault::Function(f) => {
                            let n: String = if f.name.is_empty() { "__default".into() } else { f.name.clone() };
                            (n, f.clone())
                        }
                        _ => continue,
                    };
                    let symbol = name.clone();
                    exports.insert(name, ExportInfo { func, symbol });
                }
                _ => {}
            }
        }
        map.insert(m.path.clone(), exports);
    }
    map
}

/// Names of every interface / type alias / enum declared in any module
/// (including `export`-wrapped ones). Importing one of these is valid at
/// the Arcis level but has no function symbol behind it.
fn collect_type_level_names(all_modules: &[Module]) -> std::collections::HashSet<String> {
    let mut out = std::collections::HashSet::new();
    for m in all_modules {
        for stmt in &m.program.stmts {
            let inner = match stmt {
                Stmt::ExportDecl(inner) => inner.as_ref(),
                other => other,
            };
            match inner {
                Stmt::TypeAlias { name, .. }
                | Stmt::Interface { name, .. }
                | Stmt::Enum { name, .. } => {
                    out.insert(name.clone());
                }
                _ => {}
            }
        }
    }
    out
}

/// Program-wide `field name → ArcisType` map, collected from every object
/// type reachable from any type annotation in any module. Fallback typing
/// for member accesses on receivers without a tracked shape.
fn collect_global_field_types(
    all_modules: &[Module],
    enum_names: &std::collections::HashSet<String>,
) -> HashMap<String, ArcisType> {
    fn from_type(
        t: &arcis_ast::Type,
        enum_names: &std::collections::HashSet<String>,
        out: &mut HashMap<String, ArcisType>,
    ) {
        match t {
            arcis_ast::Type::Object { fields, .. } => {
                for (fname, fty, _) in fields {
                    if let Ok(at) = from_ast(fty, enum_names) {
                        out.insert(fname.clone(), at);
                    }
                    from_type(fty, enum_names, out);
                }
            }
            arcis_ast::Type::Array(inner) => from_type(inner, enum_names, out),
            arcis_ast::Type::Union(ms) | arcis_ast::Type::Intersection(ms) => {
                for m in ms {
                    from_type(m, enum_names, out);
                }
            }
            _ => {}
        }
    }
    fn from_stmt(
        stmt: &Stmt,
        enum_names: &std::collections::HashSet<String>,
        out: &mut HashMap<String, ArcisType>,
    ) {
        match stmt {
            Stmt::Let { ty: Some(t), .. } | Stmt::Const { ty: Some(t), .. } => {
                from_type(t, enum_names, out)
            }
            Stmt::ForOf { ty, body, .. } => {
                if let Some(t) = ty {
                    from_type(t, enum_names, out);
                }
                for s in body {
                    from_stmt(s, enum_names, out);
                }
            }
            Stmt::Function(f) => {
                for p in &f.params {
                    from_type(&p.ty, enum_names, out);
                }
                from_type(&f.return_type, enum_names, out);
                for s in &f.body {
                    from_stmt(s, enum_names, out);
                }
            }
            Stmt::Interface { fields, .. } => {
                for (fname, fty, _) in fields {
                    if let Ok(at) = from_ast(fty, enum_names) {
                        out.insert(fname.clone(), at);
                    }
                    from_type(fty, enum_names, out);
                }
            }
            Stmt::TypeAlias { ty, .. } => from_type(ty, enum_names, out),
            Stmt::ExportDecl(inner) => from_stmt(inner, enum_names, out),
            Stmt::ExportDefault(arcis_ast::ExportDefault::Function(f)) => {
                for p in &f.params {
                    from_type(&p.ty, enum_names, out);
                }
                from_type(&f.return_type, enum_names, out);
                for s in &f.body {
                    from_stmt(s, enum_names, out);
                }
            }
            Stmt::If { then_branch, else_branch, .. } => {
                for s in then_branch {
                    from_stmt(s, enum_names, out);
                }
                if let Some(eb) = else_branch {
                    for s in eb {
                        from_stmt(s, enum_names, out);
                    }
                }
            }
            Stmt::While { body, .. } | Stmt::For { body, .. } => {
                for s in body {
                    from_stmt(s, enum_names, out);
                }
            }
            _ => {}
        }
    }
    let mut out = HashMap::new();
    for m in all_modules {
        for stmt in &m.program.stmts {
            from_stmt(stmt, enum_names, &mut out);
        }
    }
    out
}

/// The initializer expression of an exported top-level `let`/`const` named
/// `name` in the module referenced by `path`, if any.
fn find_exported_const(
    importer: &std::path::Path,
    path: &[String],
    name: &str,
    all_modules: &[Module],
) -> Option<arcis_ast::Expr> {
    collect_exported_consts(importer, path, all_modules)
        .into_iter()
        .find(|(n, _)| n == name)
        .map(|(_, e)| e)
}

/// Every exported top-level `let`/`const` of the module referenced by
/// `path`, as `(name, initializer)` pairs.
fn collect_exported_consts(
    importer: &std::path::Path,
    path: &[String],
    all_modules: &[Module],
) -> Vec<(String, arcis_ast::Expr)> {
    let Ok(arcis_linker::ModuleTarget::Local(dep_path)) =
        arcis_linker::resolve_specifier(importer, path)
    else {
        return Vec::new();
    };
    let Some(dep) = all_modules.iter().find(|m| m.path == dep_path) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for stmt in &dep.program.stmts {
        if let Stmt::ExportDecl(inner) = stmt {
            match inner.as_ref() {
                Stmt::Let { name, value, .. } | Stmt::Const { name, value, .. }
                    if !matches!(value, arcis_ast::Expr::Arrow { .. }) =>
                {
                    out.push((name.clone(), value.clone()));
                }
                _ => {}
            }
        }
    }
    out
}

/// Resolve the real symbol name of the target module's default export.
fn resolve_default_name(
    importer: &std::path::Path,
    path: &[String],
    all_modules: &[Module],
) -> Option<String> {
    match arcis_linker::resolve_specifier(importer, path).ok()? {
        arcis_linker::ModuleTarget::Local(dep_path) => all_modules
            .iter()
            .find(|m| m.path == dep_path)
            .and_then(|m| m.exports.default.clone()),
        _ => None,
    }
}

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

fn lookup_export_sig_and_params(
    importer: &std::path::Path,
    path: &[String],
    name: &str,
    export_map: &ExportMap,
    enum_names: &std::collections::HashSet<String>,
) -> Result<(cranelift_codegen::ir::Signature, Vec<ArcisType>, String, ArcisType), String> {
    let target = arcis_linker::resolve_specifier(importer, path).map_err(|e| e)?;
    match target {
        arcis_linker::ModuleTarget::Local(dep_path) => {
            let mod_exports = export_map.get(&dep_path).ok_or_else(|| {
                format!("module `{}` not found in export map", path.join("."))
            })?;
            let info = mod_exports.get(name).ok_or_else(|| {
                format!("`{}` does not export `{}`", path.join("."), name)
            })?;
            let sig = import_sig_from_func(&info.func, enum_names);
            let params = param_types(&info.func, enum_names);
            let ret = return_type_of(&info.func, enum_names);
            Ok((sig, params, info.symbol.clone(), ret))
        }
        _ => Err(format!("`{}` is not a local module", path.join("."))),
    }
}

#[allow(dead_code)]
fn lookup_export_sig(
    importer: &std::path::Path,
    path: &[String],
    name: &str,
    export_map: &ExportMap,
    enum_names: &std::collections::HashSet<String>,
) -> Result<cranelift_codegen::ir::Signature, String> {
    lookup_export_sig_and_params(importer, path, name, export_map, enum_names).map(|(s, _, _, _)| s)
}

fn declare_imported_module(
    obj_module: &mut ObjectModule,
    importer: &std::path::Path,
    path: &[String],
    export_map: &ExportMap,
    user_fns: &mut HashMap<String, FnInfo>,
    enum_names: &std::collections::HashSet<String>,
) -> Result<(), String> {
    let target = arcis_linker::resolve_specifier(importer, path).map_err(|e| e)?;
    match target {
        arcis_linker::ModuleTarget::Local(dep_path) => {
            let mod_exports = export_map.get(&dep_path).ok_or_else(|| {
                format!("module `{}` not found in export map", path.join("."))
            })?;
            for (name, info) in mod_exports {
                if user_fns.contains_key(name) { continue; }
                let sig = import_sig_from_func(&info.func, enum_names);
                let params = param_types(&info.func, enum_names);
                let ret = return_type_of(&info.func, enum_names);
                let id = obj_module
                    .declare_function(&info.symbol, Linkage::Import, &sig)
                    .map_err(|e| format!("declare import `{}`: {}", info.symbol, e))?;
                user_fns.insert(name.clone(), FnInfo { id, params, ret });
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

fn import_sig_from_func(f: &arcis_ast::Function, enum_names: &std::collections::HashSet<String>) -> cranelift_codegen::ir::Signature {
    let mut sig = cranelift_codegen::ir::Signature::new(
        cranelift_codegen::isa::CallConv::SystemV,
    );
    for p in &f.params {
        let ty = from_ast(&p.ty, enum_names).unwrap_or(ArcisType::Number);
        sig.params.push(AbiParam::new(ty.to_cl()));
    }
    if f.return_type.primitive_name() != "void" {
        let ty = from_ast(&f.return_type, enum_names).unwrap_or(ArcisType::Number);
        sig.returns.push(AbiParam::new(ty.to_cl()));
    }
    sig
}

pub(crate) fn function_signature(
    module: &mut ObjectModule,
    f: &arcis_ast::Function,
    enum_names: &std::collections::HashSet<String>,
) -> cranelift_codegen::ir::Signature {
    let mut sig = module.make_signature();
    for p in &f.params {
        let ty = from_ast(&p.ty, enum_names).unwrap_or(ArcisType::Number);
        sig.params.push(AbiParam::new(ty.to_cl()));
    }
    if f.return_type.primitive_name() != "void" {
        let ty = from_ast(&f.return_type, enum_names).unwrap_or(ArcisType::Number);
        sig.returns.push(AbiParam::new(ty.to_cl()));
    }
    sig
}
