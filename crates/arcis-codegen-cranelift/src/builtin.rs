//! Lowering of Arcis builtins: `print(…)` for Phase 1.
//!
//! All builtins ultimately dispatch to a function declared in
//! [`crate::rt::Runtime`] (which the C runtime implements). The pattern is
//! the same across builtins: emit the argument values via
//! [`crate::expr::emit`], promote each to whatever shape the runtime
//! expects (e.g. `arcis_println` takes an `ArcisString*`, so any number
//! argument must first be promoted via `arcis_num_to_string`), then call
//! the runtime function.

use std::collections::HashMap;

use arcis_ast::Expr;
use cranelift_codegen::ir::types::{I32, I8};
use cranelift_codegen::ir::InstBuilder;
use cranelift_frontend::FunctionBuilder;
use cranelift_module::FuncId;
use cranelift_module::Module as CraneliftModule;
use cranelift_object::ObjectModule;

use crate::context::{FnInfo, FunctionCtx};
use crate::expr;
use crate::rt::Runtime;
use crate::types::ArcisType;

/// Emit `print(arg)` and return a placeholder SSA value (Cranelift calls
/// with no return produce no results; we fabricate an `I8` zero to keep
/// the call-site signature uniform).
pub(crate) fn emit_print(
    builder: &mut FunctionBuilder,
    fctx: &mut FunctionCtx,
    args: &[Expr],
    runtime: &Runtime,
    user_fns: &HashMap<String, FnInfo>,
    module: &mut ObjectModule,
) -> Result<cranelift_codegen::ir::Value, String> {
    if args.len() != 1 {
        return Err(format!(
            "print() takes exactly 1 argument, got {}",
            args.len()
        ));
    }
    let (v, ty) = expr::emit(builder, fctx, &args[0], runtime, user_fns, module)?;
    // Promote to ArcisString handle.
    let handle = match ty {
        crate::types::ArcisType::String => v,
        crate::types::ArcisType::Number => {
            let callee = module.declare_func_in_func(runtime.num_to_string, builder.func);
            let call = builder.ins().call(callee, &[v]);
            builder.inst_results(call)[0]
        }
        crate::types::ArcisType::Boolean => {
            let widened = builder.ins().uextend(I32, v);
            let callee = module.declare_func_in_func(runtime.bool_to_string, builder.func);
            let call = builder.ins().call(callee, &[widened]);
            builder.inst_results(call)[0]
        }
        crate::types::ArcisType::Void => {
            return Err("cannot print void".to_string());
        }
        ArcisType::Array | ArcisType::Object => {
            // Placeholder handle; in practice Array/Object wouldn't be
            // printed directly but we avoid a panic.
            return Err("cannot print array/object directly (use iteration or field access)".to_string());
        }
    };
    let callee = module.declare_func_in_func(runtime.println, builder.func);
    builder.ins().call(callee, &[handle]);
    // arcis_println returns void; return a poison I8 zero so the caller
    // can treat `print(...)` as if it returned a value.
    Ok(builder.ins().iconst(I8, 0))
}