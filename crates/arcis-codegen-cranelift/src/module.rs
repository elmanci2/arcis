//! Per-module Cranelift lowering.
//!
//! Each Arcis module compiles into one Cranelift [`ObjectModule`]:
//!
//! - All runtime functions are declared as `Linkage::Import` (they live in
//!   the separately-compiled `arcis_runtime.o`).
//! - All top-level Arcis functions become Cranelift functions exported
//!   with their original Arcis name (for the Phase 1 single-file case this
//!   is just the ones declared in the file).
//! - The root module additionally defines `arcis_main`, which contains the
//!   lowered top-level statements (variable declarations, calls, control
//!   flow). This is what the Linux `_start` stub in the C runtime calls.
//!
//! Non-root modules in Phase 1 are emitted as empty object files (the
//! linker still expects one `.o` per module). Cross-module calls land in
//! Phase 4.

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

/// Emit one Arcis module into `module`. Returns the list of function names
/// declared as top-level Arcis functions (used by `expr::emit` when
/// resolving `Call { callee: Ident(name) }`).
pub(crate) fn emit(
    module: &mut ObjectModule,
    m: &Module,
    is_root: bool,
) -> Result<(), String> {
    if !is_root {
        // Phase 4: link non-root modules as archives of FuncIds and resolve
        // cross-module calls in expr::emit. For now we leave the .o empty.
        return Ok(());
    }

    let runtime = Runtime::declare(module)?;
    let reassigned = collect_reassigned(&m.program);

    // Pre-declare every top-level Arcis function so calls can resolve.
    let mut user_fns: HashMap<String, FuncId> = HashMap::new();
    for stmt in &m.program.stmts {
        if let Stmt::Function(f) = stmt {
            let sig = function_signature(module, f);
            let id = module
                .declare_function(f.name.as_str(), Linkage::Export, &sig)
                .map_err(|e| format!("declare `{}`: {}", f.name, e))?;
            user_fns.insert(f.name.clone(), id);
        }
    }

    // Define every top-level function. Each one gets its own Cranelift
    // context (we can't share contexts between functions because Cranelift
    // ties a `Context` to a single function at a time).
    for stmt in &m.program.stmts {
        if let Stmt::Function(f) = stmt {
            let func_id = user_fns[&f.name];
            let sig = function_signature(module, f);
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
                module,
            )?;
            module
                .define_function(func_id, &mut ctx)
                .map_err(|e| format!("define `{}`: {}", f.name, e))?;
        }
    }

    // Build arcis_main for the root module: holds top-level non-function
    // statements.
    let main_sig = {
        let mut s = module.make_signature();
        // Empty parameter list — the C runtime's `_start` calls us with no
        // arguments. argv/argc/envp are not exposed in Phase 1.
        s
    };
    let main_id = module
        .declare_function("arcis_main", Linkage::Export, &main_sig)
        .map_err(|e| format!("declare arcis_main: {}", e))?;
    let mut ctx = Context::new();
    ctx.func = Function::with_name_signature(
        UserFuncName::user(0, main_id.index() as u32),
        main_sig.clone(),
    );
    {
        // Pre-validate arcis_main so a verifier failure produces a
        // readable error instead of `define_function`'s terse "Verifier
        // errors" message.
        let flags = cranelift_codegen::settings::Flags::new(
            cranelift_codegen::settings::builder(),
        );
        if let Err(errs) = cranelift_codegen::verifier::verify_function(&ctx.func, &flags) {
            return Err(format!(
                "verify arcis_main: {} (func: {})",
                errs,
                ctx.func.display()
            ));
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
                Stmt::Function(_) => {
                    // Already emitted above as a separate function.
                }
                _ => {
                    emit_stmt(
                        &mut builder,
                        &mut fctx,
                        stmt,
                        &runtime,
                        &user_fns,
                        module,
                    )?;
                }
            }
        }

        builder.ins().return_(&[]);
        builder.finalize();
    }
    module
        .define_function(main_id, &mut ctx)
        .map_err(|e| {
            format!(
                "define arcis_main: {}\n\n=== IR ===\n{}",
                e,
                ctx.func.display()
            )
        })?;
    Ok(())
}

/// Build a Cranelift signature for an Arcis function. Phase 1: primitive
/// parameter types only (no arrays, no objects, no function types).
fn function_signature(
    module: &mut ObjectModule,
    f: &arcis_ast::Function,
) -> cranelift_codegen::ir::Signature {
    use crate::types::from_ast;

    let mut sig = module.make_signature();
    for p in &f.params {
        let ty = from_ast(&p.ty.name, p.ty.is_array)
            .unwrap_or(crate::types::ArcisType::Number);
        sig.params.push(AbiParam::new(ty.to_cl()));
    }
    if f.return_type.name != "void" {
        let ty = from_ast(&f.return_type.name, f.return_type.is_array)
            .unwrap_or(crate::types::ArcisType::Number);
        sig.returns.push(AbiParam::new(ty.to_cl()));
    }
    sig
}