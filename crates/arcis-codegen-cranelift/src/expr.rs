//! Expression lowering.
//!
//! Every Arcis expression is lowered into a Cranelift SSA [`Value`] of a
//! predictable IR type: `F64` for `number`, `I64` (handle) for `string`,
//! `I8` (0/1) for `boolean`. The lowering is responsible for:
//!
//! - resolving identifiers against [`FunctionCtx`],
//! - dispatching binary / unary operators with the right Cranelift
//!   instruction,
//! - converting string literals to runtime-allocated handles (via
//!   `arcis_string_from_cstr` over a per-module data object),
//! - dispatching `print(…)` and other builtins to the runtime helpers
//!   declared in [`crate::rt`],
//! - calling user-defined functions whose `FuncId` lives in `user_fns`.

use std::collections::HashMap;

use arcis_ast::{BinOp, Expr, UnaryOp};
use cranelift_codegen::ir::condcodes::{FloatCC, IntCC};
use cranelift_codegen::ir::types::{F64, I32, I64, I8};
use cranelift_codegen::ir::{InstBuilder, MemFlags};
use cranelift_frontend::FunctionBuilder;
use cranelift_module::{DataDescription, FuncId, Linkage, Module as CraneliftModule};
use cranelift_object::ObjectModule;

use crate::context::{FnInfo, FunctionCtx};
use crate::rt::Runtime;
use crate::types::ArcisType;

/// Lower an Arcis expression and return its Cranelift SSA value.
pub(crate) fn emit(
    builder: &mut FunctionBuilder,
    fctx: &mut FunctionCtx,
    expr: &Expr,
    runtime: &Runtime,
    user_fns: &HashMap<String, FnInfo>,
    module: &mut ObjectModule,
) -> Result<(cranelift_codegen::ir::Value, ArcisType), String> {
    match expr {
        Expr::Number(n) => {
            let v = builder.ins().f64const(*n);
            Ok((v, ArcisType::Number))
        }
        Expr::String(s) => {
            let handle = emit_string_literal(builder, fctx, module, s, runtime)?;
            Ok((handle, ArcisType::String))
        }
        Expr::Bool(b) => {
            let v = builder.ins().iconst(I8, if *b { 1 } else { 0 });
            Ok((v, ArcisType::Boolean))
        }
        Expr::Ident(name) => {
            if let Some(&var) = fctx.var(name) {
                let v = builder.use_var(var);
                let ty = fctx.ty(name).unwrap_or(ArcisType::Number);
                Ok((v, ty))
            } else if name == "sys" {
                Err("`sys` is a namespace — use sys.X() or sys.ns.method()".to_string())
            } else {
                Err(format!("unknown identifier `{}`", name))
            }
        }
        Expr::Unary { op, operand } => emit_unary(builder, fctx, op, operand, runtime, user_fns, module),
        Expr::Binary { op, left, right } => {
            emit_binary(builder, fctx, op, left, right, runtime, user_fns, module)
        }
        Expr::Call { callee, args } => {
            // Dispatch print, input, method calls, and user functions.
            if let Expr::Ident(fname) = callee.as_ref() {
                if fname == "print" {
                    let v = crate::builtin::emit_print(builder, fctx, args, runtime, user_fns, module)?;
                    return Ok((v, ArcisType::Void));
                }
                if fname == "input" {
                    let callee_r = module.declare_func_in_func(runtime.read_line, builder.func);
                    let call = builder.ins().call(callee_r, &[]);
                    let h = builder.inst_results(call)[0];
                    return Ok((h, ArcisType::String));
                }
                if fname == "parseFloat" {
                    if args.len() != 1 {
                        return Err("parseFloat() takes exactly 1 argument".to_string());
                    }
                    let (arg_val, _) = emit(builder, fctx, &args[0], runtime, user_fns, module)?;
                    let callee = module.declare_func_in_func(runtime.parse_float, builder.func);
                    let call = builder.ins().call(callee, &[arg_val]);
                    let v = builder.inst_results(call)[0];
                    return Ok((v, ArcisType::Number));
                }
                if fname == "str" {
                    if args.len() != 1 {
                        return Err("str() takes exactly 1 argument".to_string());
                    }
                    let (val, ty) = emit(builder, fctx, &args[0], runtime, user_fns, module)?;
                    let handle = promote_to_string(builder, val, ty, module, runtime)?;
                    return Ok((handle, ArcisType::String));
                }
                if fname == "isNaN" {
                    if args.len() != 1 {
                        return Err("isNaN() takes exactly 1 argument".to_string());
                    }
                    let (arg_val, _) = emit(builder, fctx, &args[0], runtime, user_fns, module)?;
                    let callee = module.declare_func_in_func(runtime.is_nan, builder.func);
                    let call = builder.ins().call(callee, &[arg_val]);
                    let raw = builder.inst_results(call)[0];
                    // is_nan returns i32 (0/1), convert to i8 boolean.
                    let one = builder.ins().iconst(I32, 1);
                    let v = builder.ins().icmp(IntCC::Equal, raw, one);
                    return Ok((v, ArcisType::Boolean));
                }
                if let Some(fn_info) = user_fns.get(fname) {
                    let result = emit_user_call(builder, fctx, fn_info, fname, args, runtime, user_fns, module)?;
                    return Ok(result);
                }
            }
            // sys.X(args) — top-level sys call.
            if let Expr::Member { object, property } = callee.as_ref() {
                // sys.ns.method(args) — two-level: object is Member{Ident("sys"), ns}
                if let Expr::Member { object: sys_obj, property: ns } = object.as_ref() {
                    if let Expr::Ident(sys_name) = sys_obj.as_ref() {
                        if sys_name == "sys" {
                            let dispatched = crate::sys::try_emit_subns(
                                builder, fctx, ns, property, args, runtime, user_fns, module,
                            )?;
                            if let Some(result) = dispatched {
                                return Ok(result);
                            }
                        }
                    }
                }
                // sys.X(args) — one-level Member: object is Ident("sys")
                let mut is_sys = false;
                if let Expr::Ident(sys_name) = object.as_ref() {
                    if sys_name == "sys" {
                        is_sys = true;
                        let dispatched = crate::sys::try_emit_call(
                            builder, fctx, property, args, runtime, user_fns, module,
                        )?;
                        if let Some(result) = dispatched {
                            return Ok(result);
                        }
                    }
                }
                // Regular method call: arr.push(x) / s.toUpperCase() / etc.
                if !is_sys {
                    let dispatched = crate::method::emit(
                        builder, fctx, object, property, args, runtime, user_fns, module,
                    )?;
                    if let Some(result) = dispatched {
                        return Ok(result);
                    }
                } else {
                    return Err(format!(
                        "`sys.{}` is not a recognized sys method",
                        property
                    ));
                }
            }
            Err(format!(
                "this call form is not yet supported by the Cranelift backend (callee: {:?})",
                callee
            ))
        }
        Expr::Member { object, property } => {
            // Enum variant access: `Color.Red` -> a compile-time constant.
            // Checked before the sys/field-access logic below since `object`
            // being a plain identifier that names a known enum is otherwise
            // indistinguishable from a variable at this point.
            if let Expr::Ident(name) = object.as_ref() {
                if fctx.is_enum_name(name) {
                    let value = fctx.enum_variant_value(name, property).ok_or_else(|| {
                        format!("enum `{}` has no variant `{}`", name, property)
                    })?;
                    let v = builder.ins().f64const(value);
                    return Ok((v, ArcisType::Number));
                }
            }
            // sys.args / sys.X member access
            let mut is_sys_member = false;
            if let Expr::Ident(sys_name) = object.as_ref() {
                if sys_name == "sys" {
                    is_sys_member = true;
                    if let Some(result) = crate::sys::try_emit_member(
                        builder, fctx, property, runtime, user_fns, module,
                    )? {
                        return Ok(result);
                    }
                }
            }
            if property == "length" {
                if is_sys_member {
                    return Err(format!(
                        "`sys.{}` is not a recognized sys property or method",
                        property
                    ));
                }
                let (obj_val, obj_ty) = emit(builder, fctx, object, runtime, user_fns, module)?;
                let len_val = match obj_ty {
                    ArcisType::String => emit_string_length(builder, obj_val, module, runtime)?,
                    ArcisType::Array => {
                        let callee = module.declare_func_in_func(runtime.vec_len, builder.func);
                        let call = builder.ins().call(callee, &[obj_val]);
                        let raw = builder.inst_results(call)[0];
                        let as_i64 = builder.ins().uextend(I64, raw);
                        as_i64
                    }
                    _ => return Err(format!(".length is not supported on {:?}", obj_ty)),
                };
                let as_f64 = builder.ins().fcvt_from_sint(F64, len_val);
                return Ok((as_f64, ArcisType::Number));
            }
            // sys.X member access that wasn't handled by try_emit_member.
            if is_sys_member {
                return Err(format!(
                    "`sys.{}` is not a recognized sys property or method",
                    property
                ));
            }
            // Object field get: obj.field → arcis_object_get(obj, "field")
            let (obj_val, obj_ty) = emit(builder, fctx, object, runtime, user_fns, module)?;
            if obj_ty == ArcisType::Object {
                // Look up the field type from the object's declared shape.
                let field_ty = if let Expr::Ident(obj_name) = object.as_ref() {
                    fctx.object_field_ty(obj_name, property)
                } else {
                    None
                };
                let key_handle = emit_string_literal(
                    builder, fctx, module, property, runtime,
                )?;
                let callee_get = module.declare_func_in_func(runtime.object_get, builder.func);
                let call_get = builder.ins().call(callee_get, &[obj_val, key_handle]);
                let raw = builder.inst_results(call_get)[0];
                match field_ty {
                    Some(ArcisType::String) => {
                        return Ok((raw, ArcisType::String));
                    }
                    Some(ArcisType::Boolean) => {
                        let as_i8 = builder.ins().ireduce(I8, raw);
                        return Ok((as_i8, ArcisType::Boolean));
                    }
                    _ => {
                        // Default: interpret as f64 number.
                        let as_f64 = builder.ins().bitcast(F64, MemFlags::new(), raw);
                        return Ok((as_f64, ArcisType::Number));
                    }
                }
            }
            Err(format!(
                "member access `.{}` is not yet supported by the Cranelift backend",
                property
            ))
        }
        Expr::Index { object, index } => {
            let (obj_val, _obj_ty) = emit(builder, fctx, object, runtime, user_fns, module)?;
            let (idx_val, _idx_ty) = emit(builder, fctx, index, runtime, user_fns, module)?;
            let idx_i32 = builder.ins().fcvt_to_sint(I32, idx_val);
            let callee = module.declare_func_in_func(runtime.vec_get, builder.func);
            let call = builder.ins().call(callee, &[obj_val, idx_i32]);
            let handle = builder.inst_results(call)[0];
            // Infer element type from the array binding.
            let elem_ty = if let Expr::Ident(obj_name) = object.as_ref() {
                fctx.element_ty(obj_name).unwrap_or(ArcisType::Number)
            } else {
                ArcisType::Number
            };
            let (v, ty) = match elem_ty {
                ArcisType::String => (handle, ArcisType::String),
                ArcisType::Number => {
                    let as_f64 = builder.ins().bitcast(F64, MemFlags::new(), handle);
                    (as_f64, ArcisType::Number)
                }
                ArcisType::Boolean => {
                    let as_i8 = builder.ins().ireduce(I8, handle);
                    (as_i8, ArcisType::Boolean)
                }
                ArcisType::Object | ArcisType::Array => (handle, elem_ty),
                _ => (handle, ArcisType::Number),
            };
            Ok((v, ty))
        }
        Expr::ArrayLiteral { elements } => {
            let callee_new = module.declare_func_in_func(runtime.vec_new, builder.func);
            let call_new = builder.ins().call(callee_new, &[]);
            let vec_handle = builder.inst_results(call_new)[0];
            for elem in elements {
                match elem {
                    arcis_ast::ArrayElement::Item(elem) => {
                        // Promote element to I64 handle. Numbers need bitcast to i64.
                        let (elem_val, elem_ty) = emit(builder, fctx, elem, runtime, user_fns, module)?;
                        let i64_val = promote_to_i64(builder, elem_val, elem_ty);
                        let callee_push = module.declare_func_in_func(runtime.vec_push, builder.func);
                        builder.ins().call(callee_push, &[vec_handle, i64_val]);
                    }
                    arcis_ast::ArrayElement::Spread(src) => {
                        let (src_val, _) = emit(builder, fctx, src, runtime, user_fns, module)?;
                        let callee_extend = module.declare_func_in_func(runtime.vec_extend, builder.func);
                        builder.ins().call(callee_extend, &[vec_handle, src_val]);
                    }
                }
            }
            Ok((vec_handle, ArcisType::Array))
        }
        Expr::ObjectLiteral { fields } => {
            let callee_new = module.declare_func_in_func(runtime.object_new, builder.func);
            let call_new = builder.ins().call(callee_new, &[]);
            let obj_handle = builder.inst_results(call_new)[0];
            for field in fields {
                match field {
                    arcis_ast::ObjectField::KV(key, value) => {
                        let (val_v, val_ty) = emit(builder, fctx, value, runtime, user_fns, module)?;
                        let i64_val = promote_to_i64(builder, val_v, val_ty);
                        // key becomes a string literal data object → call arcis_string_from_cstr
                        let key_handle = emit_string_literal(builder, fctx, module, key, runtime)?;
                        let callee_set = module.declare_func_in_func(runtime.object_set, builder.func);
                        builder.ins().call(callee_set, &[obj_handle, key_handle, i64_val]);
                    }
                    arcis_ast::ObjectField::Spread(src) => {
                        let (src_val, _) = emit(builder, fctx, src, runtime, user_fns, module)?;
                        let callee_merge = module.declare_func_in_func(runtime.object_merge, builder.func);
                        builder.ins().call(callee_merge, &[obj_handle, src_val]);
                    }
                }
            }
            Ok((obj_handle, ArcisType::Object))
        }
        Expr::Path { .. } => Err("static paths are not supported by the Cranelift backend".to_string()),
        Expr::TypeOf(operand) => {
            let (_, ty) = emit(builder, fctx, operand, runtime, user_fns, module)?;
            let type_name = ty.name();
            let handle = emit_string_literal(builder, fctx, module, type_name, runtime)?;
            Ok((handle, ArcisType::String))
        }
        // `null` / `undefined` have no dedicated runtime representation yet
        // in the Cranelift backend; lower to a placeholder void value,
        // matching the Rust backend's `()` erasure.
        Expr::Null | Expr::Undefined => {
            let v = builder.ins().iconst(I8, 0);
            Ok((v, ArcisType::Void))
        }
        // Type assertions and the non-null assertion are compile-time-only
        // in TypeScript: no runtime effect. Lower straight through.
        Expr::AsAssertion { expr, .. } => emit(builder, fctx, expr, runtime, user_fns, module),
        Expr::AsConst(inner) => emit(builder, fctx, inner, runtime, user_fns, module),
        Expr::NonNullAssertion(inner) => emit(builder, fctx, inner, runtime, user_fns, module),
        Expr::Arrow { .. } => {
            Err("arrow functions are not yet supported by the Cranelift backend".to_string())
        }
    }
}

fn emit_unary(
    builder: &mut FunctionBuilder,
    fctx: &mut FunctionCtx,
    op: &UnaryOp,
    operand: &Expr,
    runtime: &Runtime,
    user_fns: &HashMap<String, FnInfo>,
    module: &mut ObjectModule,
) -> Result<(cranelift_codegen::ir::Value, ArcisType), String> {
    let (val, ty) = emit(builder, fctx, operand, runtime, user_fns, module)?;
    match op {
        UnaryOp::Neg => {
            let v = match ty {
                ArcisType::Number => builder.ins().fneg(val),
                _ => return Err("unary `-` only supported on numbers in Phase 1".to_string()),
            };
            Ok((v, ty))
        }
        UnaryOp::Not => {
            let (v, result_ty) = match ty {
                ArcisType::Boolean => {
                    let one = builder.ins().iconst(I8, 1);
                    (builder.ins().bxor(val, one), ArcisType::Boolean)
                }
                ArcisType::Number => {
                    let zero = builder.ins().f64const(0.0);
                    (builder.ins().fcmp(FloatCC::Equal, val, zero), ArcisType::Boolean)
                }
                _ => return Err("unary `!` only supported on booleans/numbers in Phase 1".to_string()),
            };
            Ok((v, result_ty))
        }
    }
}

/// Lower `lv == rv` (both already-evaluated SSA values) to an `I8` 0/1
/// boolean, dispatching on the runtime representation the same way
/// `emit_binary`'s `EqEq` arm does. Shared with `stmt::emit_switch`, whose
/// `case` comparisons are structurally the same operation but performed
/// against a pre-evaluated discriminant rather than a fresh `Expr::Binary`.
pub(crate) fn emit_eq(
    builder: &mut FunctionBuilder,
    module: &mut ObjectModule,
    runtime: &Runtime,
    lv: cranelift_codegen::ir::Value,
    lt: ArcisType,
    rv: cranelift_codegen::ir::Value,
    rt: ArcisType,
) -> Result<cranelift_codegen::ir::Value, String> {
    match (lt, rt) {
        (ArcisType::String, ArcisType::String) => {
            let callee = module.declare_func_in_func(runtime.string_eq, builder.func);
            let call = builder.ins().call(callee, &[lv, rv]);
            let raw = builder.inst_results(call)[0];
            let one = builder.ins().iconst(I32, 1);
            Ok(builder.ins().icmp(IntCC::Equal, raw, one))
        }
        (ArcisType::Number, ArcisType::Number) => {
            Ok(builder.ins().fcmp(FloatCC::Equal, lv, rv))
        }
        (ArcisType::Boolean, ArcisType::Boolean) => {
            Ok(builder.ins().icmp(IntCC::Equal, lv, rv))
        }
        _ => Err(format!(
            "cannot compare `{}` with `{}` using `==`",
            lt.name(),
            rt.name()
        )),
    }
}

fn emit_binary(
    builder: &mut FunctionBuilder,
    fctx: &mut FunctionCtx,
    op: &BinOp,
    left: &Expr,
    right: &Expr,
    runtime: &Runtime,
    user_fns: &HashMap<String, FnInfo>,
    module: &mut ObjectModule,
) -> Result<(cranelift_codegen::ir::Value, ArcisType), String> {
    let (lv, lt) = emit(builder, fctx, left, runtime, user_fns, module)?;
    let (rv, rt) = emit(builder, fctx, right, runtime, user_fns, module)?;

    // String concatenation on `+`. If exactly one side is already a string,
    // auto-promote the other (number/boolean) to a string first, matching
    // the Rust backend's `format!("{}{}", a, b)` behavior for
    // `print("label: " + value)` — otherwise this bit-for-bit-matches the
    // most common `print` pattern in every example and would reject it.
    if matches!(op, BinOp::Add) && (lt == ArcisType::String || rt == ArcisType::String) {
        let lv = if lt == ArcisType::String { lv } else { promote_to_string(builder, lv, lt, module, runtime)? };
        let rv = if rt == ArcisType::String { rv } else { promote_to_string(builder, rv, rt, module, runtime)? };
        let callee = module.declare_func_in_func(runtime.string_concat, builder.func);
        let call = builder.ins().call(callee, &[lv, rv]);
        let handle = builder.inst_results(call)[0];
        return Ok((handle, ArcisType::String));
    }

    // String equality.
    if matches!(lt, ArcisType::String) && matches!(rt, ArcisType::String) {
        let callee = module.declare_func_in_func(runtime.string_eq, builder.func);
        let call = builder.ins().call(callee, &[lv, rv]);
        let raw = builder.inst_results(call)[0];
        // `arcis_string_eq` returns i32 (0/1); narrow to i8 and turn into a
        // boolean (1 for true, 0 for false) so the result fits the same
        // I8 boolean shape that other comparisons produce.
        let zero = builder.ins().iconst(I32, 0);
        let one = builder.ins().iconst(I32, 1);
        let v = match op {
            BinOp::EqEq => builder.ins().icmp(IntCC::Equal, raw, one),
            BinOp::NotEq => builder.ins().icmp(IntCC::NotEqual, raw, one),
            _ => return Err(format!(
                "operator `{:?}` is not supported on strings (only `==` and `!=`)",
                op
            )),
        };
        let _ = zero;
        return Ok((v, ArcisType::Boolean));
    }

    // Arithmetic on numbers.
    if matches!(lt, ArcisType::Number) && matches!(rt, ArcisType::Number) {
        let (v, ty) = match op {
            BinOp::Add => (builder.ins().fadd(lv, rv), ArcisType::Number),
            BinOp::Sub => (builder.ins().fsub(lv, rv), ArcisType::Number),
            BinOp::Mul => (builder.ins().fmul(lv, rv), ArcisType::Number),
            BinOp::Div => (builder.ins().fdiv(lv, rv), ArcisType::Number),
            BinOp::Mod => {
                // Cranelift lacks float remainder in older ISAs; emulate.
                let div = builder.ins().fdiv(lv, rv);
                let trunc = builder.ins().floor(div);
                let prod = builder.ins().fmul(trunc, rv);
                (builder.ins().fsub(lv, prod), ArcisType::Number)
            }
            BinOp::EqEq => (builder.ins().fcmp(FloatCC::Equal, lv, rv), ArcisType::Boolean),
            BinOp::NotEq => (builder.ins().fcmp(FloatCC::NotEqual, lv, rv), ArcisType::Boolean),
            BinOp::Lt => (builder.ins().fcmp(FloatCC::LessThan, lv, rv), ArcisType::Boolean),
            BinOp::Gt => (builder.ins().fcmp(FloatCC::GreaterThan, lv, rv), ArcisType::Boolean),
            BinOp::LtEq => (builder.ins().fcmp(FloatCC::LessThanOrEqual, lv, rv), ArcisType::Boolean),
            BinOp::GtEq => (builder.ins().fcmp(FloatCC::GreaterThanOrEqual, lv, rv), ArcisType::Boolean),
            BinOp::And | BinOp::Or => {
                return Err("`&&`/`||` on numbers is not meaningful; Phase 1 expects booleans".to_string());
            }
        };
        return Ok((v, ty));
    }

    // Boolean logic on booleans.
    if matches!(lt, ArcisType::Boolean) && matches!(rt, ArcisType::Boolean) {
        let v = match op {
            BinOp::And => {
                // Both operands non-zero.
                let one = builder.ins().iconst(I8, 1);
                let l_truthy = builder.ins().icmp(IntCC::NotEqual, lv, one);
                let r_truthy = builder.ins().icmp(IntCC::NotEqual, rv, one);
                // Combine via fcmp isn't right; instead, use bitwise AND of
                // the booleans (treating 1 as true) — but Cranelift wants
                // matching integer widths. We widen to i32 for `band`.
                let l32 = builder.ins().uextend(I32, lv);
                let r32 = builder.ins().uextend(I32, rv);
                let band = builder.ins().band(l32, r32);
                // Convert back to boolean (truthy ≠ 0).
                let zero = builder.ins().iconst(I32, 0);
                builder.ins().icmp(IntCC::NotEqual, band, zero)
            }
            BinOp::Or => {
                let l32 = builder.ins().uextend(I32, lv);
                let r32 = builder.ins().uextend(I32, rv);
                let bor = builder.ins().bor(l32, r32);
                let zero = builder.ins().iconst(I32, 0);
                builder.ins().icmp(IntCC::NotEqual, bor, zero)
            }
            BinOp::EqEq => builder.ins().icmp(IntCC::Equal, lv, rv),
            BinOp::NotEq => builder.ins().icmp(IntCC::NotEqual, lv, rv),
            _ => {
                return Err("`</>/<=/>=` on booleans is not supported".to_string());
            }
        };
        return Ok((v, ArcisType::Boolean));
    }

    Err(format!(
        "binary operator `{:?}` not supported between {:?} and {:?}",
        op, lt, rt
    ))
}

fn emit_user_call(
    builder: &mut FunctionBuilder,
    fctx: &mut FunctionCtx,
    fn_info: &crate::context::FnInfo,
    fn_name: &str,
    args: &[Expr],
    runtime: &Runtime,
    user_fns: &HashMap<String, FnInfo>,
    module: &mut ObjectModule,
) -> Result<(cranelift_codegen::ir::Value, ArcisType), String> {
    // Type-check arguments against declared parameter types.
    if args.len() != fn_info.params.len() {
        return Err(format!(
            "`{}` expects {} argument(s), got {}",
            fn_name,
            fn_info.params.len(),
            args.len()
        ));
    }
    let mut arg_vals: Vec<cranelift_codegen::ir::Value> = Vec::with_capacity(args.len());
    for (i, a) in args.iter().enumerate() {
        let (v, arg_ty) = emit(builder, fctx, a, runtime, user_fns, module)?;
        let expected = fn_info.params[i];
        if arg_ty != expected && arg_ty != ArcisType::Void {
            return Err(format!(
                "`{}` parameter {} expects `{}`, got `{}`",
                fn_name,
                i + 1,
                expected.name(),
                arg_ty.name()
            ));
        }
        arg_vals.push(v);
    }
    let func_id = fn_info.id;
    let callee = module.declare_func_in_func(func_id, builder.func);
    let call = builder.ins().call(callee, &arg_vals);
    let results = builder.inst_results(call);
    let result_val = results.first().copied();
    let result_ty = {
        let decl = module.declarations().get_function_decl(func_id);
        match decl.signature.returns.first() {
            Some(param) if param.value_type == F64 => ArcisType::Number,
            Some(param) if param.value_type == I8 => ArcisType::Boolean,
            Some(param) if param.value_type == I64 => ArcisType::String,
            _ => ArcisType::Void,
        }
    };
    let v = result_val.unwrap_or_else(|| {
        // Void: provide a poison I8 zero so the return tuple has a Value.
        // The caller is expected to discard this when the type is Void.
        builder.ins().iconst(I8, 0)
    });
    Ok((v, result_ty))
}

fn emit_string_literal(
    builder: &mut FunctionBuilder,
    fctx: &mut FunctionCtx,
    module: &mut ObjectModule,
    s: &str,
    runtime: &Runtime,
) -> Result<cranelift_codegen::ir::Value, String> {
    // Bytes + NUL terminator (arcis_string_from_cstr uses strlen).
    let mut bytes = Vec::with_capacity(s.len() + 1);
    bytes.extend_from_slice(s.as_bytes());
    bytes.push(0);

    let data_id = if let Some(&existing) = fctx.string_literals.get(&bytes) {
        existing
    } else {
        let mut hash: u64 = 0xcbf29ce484222325;
        for &b in &bytes {
            hash = hash.wrapping_mul(0x100000001b3).wrapping_add(b as u64);
        }
        let name = format!("arcis_str_{:016x}", hash);
        // Check if this name already exists globally (e.g., declared by
        // another function in the same module).
        let id = if let Some(existing) = module.declarations().get_name(&name) {
            match existing {
                cranelift_module::FuncOrDataId::Data(did) => did,
                _ => {
                    return Err(format!("name collision: `{}` is a function", name));
                }
            }
        } else {
            let id = module
                .declare_data(&name, Linkage::Local, false, false)
                .map_err(|e| format!("declare string literal data `{}`: {}", name, e))?;
            let mut dd = DataDescription::new();
            dd.define(bytes.clone().into());
            dd.set_align(1);
            module
                .define_data(id, &dd)
                .map_err(|e| format!("define string literal data: {}", e))?;
            id
        };
        fctx.string_literals.insert(bytes.clone(), id);
        id
    };

    let gv = module.declare_data_in_func(data_id, builder.func);
    let addr = builder.ins().global_value(I64, gv);
    let callee = module.declare_func_in_func(runtime.string_from_cstr, builder.func);
    let call = builder.ins().call(callee, &[addr]);
    Ok(builder.inst_results(call)[0])
}

fn emit_string_length(
    builder: &mut FunctionBuilder,
    handle: cranelift_codegen::ir::Value,
    _module: &mut ObjectModule,
    _runtime: &Runtime,
) -> Result<cranelift_codegen::ir::Value, String> {
    let offset = builder.ins().iconst(I64, 8);
    let addr = builder.ins().iadd(handle, offset);
    let len = builder
        .ins()
        .load(I64, MemFlags::trusted(), addr, 0i32);
    Ok(len)
}

pub(crate) fn promote_to_string(
    builder: &mut FunctionBuilder,
    value: cranelift_codegen::ir::Value,
    ty: ArcisType,
    module: &mut ObjectModule,
    runtime: &Runtime,
) -> Result<cranelift_codegen::ir::Value, String> {
    match ty {
        ArcisType::String => Ok(value),
        ArcisType::Number => {
            let callee = module.declare_func_in_func(runtime.num_to_string, builder.func);
            let call = builder.ins().call(callee, &[value]);
            Ok(builder.inst_results(call)[0])
        }
        ArcisType::Boolean => {
            // Boolean stored as i8 (0/1) in Cranelift; widen to i32 to
            // match the C runtime signature `int32_t`.
            let widened = builder.ins().uextend(I32, value);
            let callee = module.declare_func_in_func(runtime.bool_to_string, builder.func);
            let call = builder.ins().call(callee, &[widened]);
            Ok(builder.inst_results(call)[0])
        }
        ArcisType::Void => Err("cannot promote void to string".to_string()),
        ArcisType::Array => Err("cannot promote array to string".to_string()),
        ArcisType::Object => Err("cannot promote object to string".to_string()),
    }
}

/// Promote any Arcis value to an `i64` handle.
/// - Numbers: bitcast the f64 bits to i64.
/// - Booleans: widen to i64 (0 or 1).
/// - Already-I64 handles (String, Array, Object): pass through.
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