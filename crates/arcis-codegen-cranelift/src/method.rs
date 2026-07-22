//! Method-call lowering for strings and arrays.
//!
//! The Rust backend dispatches `object.method(args)` in `expr.rs` (lines
//! 189-256) and `method.rs`. This module is the Cranelift equivalent:
//! given an object expression, a property name, and call arguments,
//! it tries to match a known runtime function and emit the call.
//!
//! Returns `Ok(None)` when the property is not recognised (so the caller
//! can either try another dispatcher or return an error). Returns
//! `Ok(Some(...))` on success.

use std::collections::HashMap;

use arcis_ast::Expr;
use cranelift_codegen::ir::types::{F64, I32, I64, I8};
use cranelift_codegen::ir::{InstBuilder, MemFlags};
use cranelift_frontend::FunctionBuilder;
use cranelift_module::{FuncId, Module as CraneliftModule};
use cranelift_object::ObjectModule;

use crate::context::{FnInfo, FunctionCtx};
use crate::expr;
use crate::rt::Runtime;
use crate::types::ArcisType;

/// Try to lower a method call `object.property(args)`. Returns
/// `Ok(Some(result))` when `property` is a recognised string method.
pub(crate) fn emit(
    builder: &mut FunctionBuilder,
    fctx: &mut FunctionCtx,
    object: &Expr,
    property: &str,
    args: &[Expr],
    runtime: &Runtime,
    user_fns: &HashMap<String, FnInfo>,
    module: &mut ObjectModule,
) -> Result<Option<(cranelift_codegen::ir::Value, ArcisType)>, String> {
    // Emit the object expression once.
    let (obj_val, _obj_ty) =
        expr::emit(builder, fctx, object, runtime, user_fns, module)?;

    match property {
        // ── String methods ────────────────────────────────────────
        "toUpperCase" => {
            let callee =
                module.declare_func_in_func(runtime.string_to_uppercase, builder.func);
            let call = builder.ins().call(callee, &[obj_val]);
            let h = builder.inst_results(call)[0];
            Ok(Some((h, ArcisType::String)))
        }
        "toLowerCase" => {
            let callee =
                module.declare_func_in_func(runtime.string_to_lowercase, builder.func);
            let call = builder.ins().call(callee, &[obj_val]);
            let h = builder.inst_results(call)[0];
            Ok(Some((h, ArcisType::String)))
        }
        "trim" => {
            let callee =
                module.declare_func_in_func(runtime.string_trim, builder.func);
            let call = builder.ins().call(callee, &[obj_val]);
            let h = builder.inst_results(call)[0];
            Ok(Some((h, ArcisType::String)))
        }
        "substring" if args.len() == 2 => {
            let (a_val, _) =
                expr::emit(builder, fctx, &args[0], runtime, user_fns, module)?;
            let (b_val, _) =
                expr::emit(builder, fctx, &args[1], runtime, user_fns, module)?;
            let a_i64 = builder.ins().fcvt_to_sint(I64, a_val);
            let b_i64 = builder.ins().fcvt_to_sint(I64, b_val);
            let callee =
                module.declare_func_in_func(runtime.string_substring, builder.func);
            let call = builder.ins().call(callee, &[obj_val, a_i64, b_i64]);
            let h = builder.inst_results(call)[0];
            Ok(Some((h, ArcisType::String)))
        }
        "indexOf" if args.len() == 1 => {
            let (needle_val, _) =
                expr::emit(builder, fctx, &args[0], runtime, user_fns, module)?;
            let callee =
                module.declare_func_in_func(runtime.string_index_of, builder.func);
            let call = builder.ins().call(callee, &[obj_val, needle_val]);
            let raw = builder.inst_results(call)[0];
            // string_index_of returns f64 directly; we need it as f64 for
            // `ArcisType::Number`.
            Ok(Some((raw, ArcisType::Number)))
        }
        "includes" if args.len() == 1 => {
            let (needle_val, _) =
                expr::emit(builder, fctx, &args[0], runtime, user_fns, module)?;
            let callee =
                module.declare_func_in_func(runtime.string_includes, builder.func);
            let call = builder.ins().call(callee, &[obj_val, needle_val]);
            let raw = builder.inst_results(call)[0];
            // string_includes returns i32 (0/1). Compare against 1 to get
            // an i8 boolean.
            let one = builder.ins().iconst(I32, 1);
            let v = builder.ins().icmp(
                cranelift_codegen::ir::condcodes::IntCC::Equal,
                raw,
                one,
            );
            Ok(Some((v, ArcisType::Boolean)))
        }
        "charAt" if args.len() == 1 => {
            let (idx_val, _) =
                expr::emit(builder, fctx, &args[0], runtime, user_fns, module)?;
            let idx_i64 = builder.ins().fcvt_to_sint(I64, idx_val);
            let callee =
                module.declare_func_in_func(runtime.string_char_at, builder.func);
            let call = builder.ins().call(callee, &[obj_val, idx_i64]);
            let h = builder.inst_results(call)[0];
            Ok(Some((h, ArcisType::String)))
        }

        // ── Array methods (mutation) ────────────────────────────────
        "push" if args.len() == 1 => {
            let (val_v, val_ty) =
                expr::emit(builder, fctx, &args[0], runtime, user_fns, module)?;
            let i64_val = crate::expr::promote_to_i64(builder, val_v, val_ty);
            let callee =
                module.declare_func_in_func(runtime.vec_push, builder.func);
            builder.ins().call(callee, &[obj_val, i64_val]);
            // push returns void; provide a poison I8 zero.
            Ok(Some((builder.ins().iconst(I8, 0), ArcisType::Void)))
        }
        "pop" if args.is_empty() => {
            let callee =
                module.declare_func_in_func(runtime.vec_pop, builder.func);
            let call = builder.ins().call(callee, &[obj_val]);
            let h = builder.inst_results(call)[0];
            // vec_pop returns i64 (the raw bits of the f64 stored in the slot).
            // Bitcast back to f64 so the caller sees a Number.
            let as_f64 = builder.ins().bitcast(F64, MemFlags::new(), h);
            Ok(Some((as_f64, ArcisType::Number)))
        }
        "unshift" if args.len() == 1 => {
            let (val_v, val_ty) =
                expr::emit(builder, fctx, &args[0], runtime, user_fns, module)?;
            let i64_val = crate::expr::promote_to_i64(builder, val_v, val_ty);
            let callee =
                module.declare_func_in_func(runtime.vec_unshift, builder.func);
            builder.ins().call(callee, &[obj_val, i64_val]);
            Ok(Some((builder.ins().iconst(I8, 0), ArcisType::Void)))
        }

        // ── Array callback methods (Phase 3 — not yet implemented) ─
        "find" | "filter" | "map" | "reduce" => {
            // These require Cranelift loops calling user functions,
            // which needs careful block control flow. Deferred.
            Ok(None)
        }

        // ── Not recognised ─────────────────────────────────────────
        _ => Ok(None),
    }
}

/// Public re-export for expr.rs to promote values to i64 handles.
pub(crate) fn promote_to_i64(
    builder: &mut FunctionBuilder,
    value: cranelift_codegen::ir::Value,
    ty: ArcisType,
) -> cranelift_codegen::ir::Value {
    match ty {
        ArcisType::Number => builder.ins().bitcast(I64, MemFlags::new(), value),
        ArcisType::Boolean => builder.ins().uextend(I64, value),
        ArcisType::String | ArcisType::Array | ArcisType::Object => value,
        ArcisType::Void => builder.ins().iconst(I64, 0),
    }
}
