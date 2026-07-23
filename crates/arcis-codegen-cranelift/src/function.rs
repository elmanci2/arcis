//! Per-function Cranelift body emission.
//!
//! Two entry points work identically for the body traversal: `arcis_main`
//! in [`crate::module`] and the user functions here. Both walk a `Vec<Stmt>`
//! and produce Cranelift IR via `stmt::emit`.

use std::collections::{HashMap, HashSet};

use arcis_ast::Function;
use cranelift_codegen::ir::types::{I32, I64, I8};
use cranelift_codegen::ir::{Function as ClifFunction, InstBuilder, UserFuncName};
use cranelift_frontend::{FunctionBuilder, FunctionBuilderContext};
use cranelift_module::{FuncId, Module as CraneliftModule};
use cranelift_object::ObjectModule;

use crate::context::{FnInfo, FunctionCtx};
use crate::rt::Runtime;
use crate::stmt::emit_stmt;
use crate::types::{from_ast, ArcisType};

/// Lower the body of an Arcis function into Cranelift IR (writing into
/// `func`). The corresponding `FuncId` lives in `user_fns`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_function(
    func: &mut ClifFunction,
    f: &Function,
    user_fns: &HashMap<String, FnInfo>,
    runtime: &Runtime,
    reassigned: &HashSet<String>,
    enums: &HashMap<String, HashMap<String, f64>>,
    global_consts: &HashMap<String, arcis_ast::Expr>,
    global_field_types: &HashMap<String, ArcisType>,
    module: &mut ObjectModule,
) -> Result<(), String> {
    let mut builder_ctx = FunctionBuilderContext::new();
    let mut builder = FunctionBuilder::new(func, &mut builder_ctx);
    let entry = builder.create_block();
    builder.append_block_params_for_function_params(entry);
    builder.switch_to_block(entry);
    let mut fctx = FunctionCtx::new(reassigned, enums);
    fctx.global_consts = global_consts.clone();
    fctx.global_field_types = global_field_types.clone();
    let enum_names: std::collections::HashSet<String> = enums.keys().cloned().collect();

    // Bind each Arcis parameter to a Cranelift variable populated from the
    // entry block's parameters.
    let block_params = builder.block_params(entry).to_vec();
    for (i, p) in f.params.iter().enumerate() {
        let ty = from_ast(&p.ty, &enum_names).unwrap_or(ArcisType::Number);
        fctx.define(&p.name, ty, block_params[i], &mut builder);
        // Track array element types and object field shapes so member /
        // index accesses on the parameter are correctly typed.
        if let Some(inner) = p.ty.array_inner() {
            if let Ok(elem) = from_ast(inner, &enum_names) {
                fctx.set_element_ty(&p.name, elem);
            }
        }
        let object_source = p.ty.array_inner().unwrap_or(&p.ty);
        if let Some(fields) = object_source.object_fields() {
            fctx.set_object_fields(&p.name, fields);
        }
    }

    for stmt in &f.body {
        emit_stmt(
            &mut builder,
            &mut fctx,
            stmt,
            runtime,
            user_fns,
            module,
        )?;
    }

    // Fall-off-the-end: every Cranelift block must end in a terminator.
    // For void Arcis functions we emit `return_()`; for non-void functions
    // we emit a default of the type. Skip the emission when the block is
    // already terminated by an explicit `return` (or a control-flow jump).
    if !block_already_terminated(&builder) {
        let return_ty = from_ast(&f.return_type, &enum_names).unwrap_or(ArcisType::Void);
        match return_ty {
            ArcisType::Void => {
                builder.ins().return_(&[]);
            }
            ArcisType::Number => {
                let zero = builder.ins().f64const(0.0);
                builder.ins().return_(&[zero]);
            }
            ArcisType::Boolean => {
                let zero = builder.ins().iconst(cranelift_codegen::ir::types::I8, 0);
                builder.ins().return_(&[zero]);
            }
            ArcisType::String => {
                let null_ptr = builder.ins().iconst(I64, 0);
                let callee =
                    module.declare_func_in_func(runtime.string_from_cstr, builder.func);
                let call = builder.ins().call(callee, &[null_ptr]);
                let handle = builder.inst_results(call)[0];
                builder.ins().return_(&[handle]);
            }
            ArcisType::Array | ArcisType::Object => {
                let zero = builder.ins().iconst(I64, 0);
                builder.ins().return_(&[zero]);
            }
        }
    }

    builder.seal_block(entry);
    builder.finalize();
    Ok(())
}

/// Returns true if the current block already ends in a terminator
/// (return / jump / brif). Mirrors [`crate::stmt::is_block_terminated`].
fn block_already_terminated(builder: &FunctionBuilder) -> bool {
    let block = match builder.current_block() {
        Some(b) => b,
        None => return false,
    };
    let func = &builder.func;
    if let Some(last_inst) = func.layout.last_inst(block) {
        let opcode = func.dfg.insts[last_inst].opcode();
        opcode.is_branch() || opcode.is_return()
    } else {
        false
    }
}