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

use crate::context::FunctionCtx;
use crate::rt::Runtime;
use crate::types::ArcisType;

/// Lower an Arcis expression and return its Cranelift SSA value.
pub(crate) fn emit(
    builder: &mut FunctionBuilder,
    fctx: &mut FunctionCtx,
    expr: &Expr,
    runtime: &Runtime,
    user_fns: &HashMap<String, FuncId>,
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
            // Two cases: a local binding (Phase 1 default), or a function
            // reference used as a value (Phase 1: only meaningful inside
            // another function call). Bare function-name values that aren't
            // called are not yet supported.
            if let Some(&var) = fctx.var(name) {
                // Use Cranelift's SSA Variable system so that reads after
                // an if-else see a phi node joining the two branches'
                // definitions. Manual tracking would only see one branch's
                // SSA value.
                let v = builder.use_var(var);
                let ty = fctx.ty(name).unwrap_or(ArcisType::Number);
                Ok((v, ty))
            } else {
                Err(format!("unknown identifier `{}`", name))
            }
        }
        Expr::Unary { op, operand } => emit_unary(builder, fctx, op, operand, runtime, user_fns, module),
        Expr::Binary { op, left, right } => {
            emit_binary(builder, fctx, op, left, right, runtime, user_fns, module)
        }
        Expr::Call { callee, args } => {
            // Phase 1 dispatch:
            //  - `print(...)` → builtin (single argument)
            //  - `Ident(name)` where name ∈ user_fns → user-function call
            //  - anything else: error.
            if let Expr::Ident(fname) = callee.as_ref() {
                if fname == "print" {
                    let v = crate::builtin::emit_print(builder, fctx, args, runtime, user_fns, module)?;
                    return Ok((v, ArcisType::Void));
                }
                if let Some(&func_id) = user_fns.get(fname) {
                    let result = emit_user_call(builder, fctx, func_id, args, runtime, user_fns, module)?;
                    return Ok(result);
                }
            }
            Err("this call form is not yet supported by the Cranelift backend".to_string())
        }
        Expr::Member { object, property } => {
            // Phase 1: only `.length` on a string. We map it to a runtime
            // call that returns the byte length as an `i64`, then cast to
            // `f64` to match the existing Arcis semantic (`.length` is
            // always a number).
            if property == "length" {
                if let Expr::Ident(name) = object.as_ref() {
                    if matches!(fctx.ty(name), Some(ArcisType::String)) {
                        let (handle, _) = emit(
                            builder, fctx, object, runtime, user_fns, module,
                        )?;
                        let len_value = emit_string_length(builder, handle, module, runtime)?;
                        let as_f64 = builder.ins().fcvt_from_sint(F64, len_value);
                        return Ok((as_f64, ArcisType::Number));
                    }
                }
                Err("`.length` is only supported on string identifiers in Phase 1".to_string())
            } else {
                Err(format!(
                    "member access `.{}` is not supported by the Cranelift backend",
                    property
                ))
            }
        }
        Expr::Index { .. } => Err("array indexing is not yet supported by the Cranelift backend (Phase 2)".to_string()),
        Expr::ArrayLiteral { .. } => Err("array literals are not yet supported by the Cranelift backend (Phase 2)".to_string()),
        Expr::ObjectLiteral { .. } => Err("object literals are not yet supported by the Cranelift backend (Phase 2)".to_string()),
        Expr::Path { .. } => Err("static paths are not supported by the Cranelift backend".to_string()),
    }
}

fn emit_unary(
    builder: &mut FunctionBuilder,
    fctx: &mut FunctionCtx,
    op: &UnaryOp,
    operand: &Expr,
    runtime: &Runtime,
    user_fns: &HashMap<String, FuncId>,
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
            let v = match ty {
                ArcisType::Boolean => {
                    // Cranelift: `bnot` on i8 flips bits, but for booleans
                    // we want a logical XOR with 1.
                    let one = builder.ins().iconst(I8, 1);
                    builder.ins().bxor(val, one)
                }
                ArcisType::Number => {
                    let zero = builder.ins().f64const(0.0);
                    builder.ins().fcmp(FloatCC::Equal, val, zero)
                }
                _ => return Err("unary `!` only supported on booleans/numbers in Phase 1".to_string()),
            };
            Ok((v, ty))
        }
    }
}

fn emit_binary(
    builder: &mut FunctionBuilder,
    fctx: &mut FunctionCtx,
    op: &BinOp,
    left: &Expr,
    right: &Expr,
    runtime: &Runtime,
    user_fns: &HashMap<String, FuncId>,
    module: &mut ObjectModule,
) -> Result<(cranelift_codegen::ir::Value, ArcisType), String> {
    let (lv, lt) = emit(builder, fctx, left, runtime, user_fns, module)?;
    let (rv, rt) = emit(builder, fctx, right, runtime, user_fns, module)?;

    // String concatenation on `+` short-circuit.
    if matches!(op, BinOp::Add) && (lt == ArcisType::String || rt == ArcisType::String) {
        let a = if lt == ArcisType::String { lv } else {
            // Promote the non-string operand to a string handle via runtime.
            promote_to_string(builder, lv, lt, module, runtime)?
        };
        let b = if rt == ArcisType::String { rv } else {
            promote_to_string(builder, rv, rt, module, runtime)?
        };
        let callee = module.declare_func_in_func(runtime.string_concat, builder.func);
        let call = builder.ins().call(callee, &[a, b]);
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
    func_id: FuncId,
    args: &[Expr],
    runtime: &Runtime,
    user_fns: &HashMap<String, FuncId>,
    module: &mut ObjectModule,
) -> Result<(cranelift_codegen::ir::Value, ArcisType), String> {
    let mut arg_vals: Vec<cranelift_codegen::ir::Value> = Vec::with_capacity(args.len());
    for a in args {
        let (v, _) = emit(builder, fctx, a, runtime, user_fns, module)?;
        arg_vals.push(v);
    }
    let callee = module.declare_func_in_func(func_id, builder.func);
    let call = builder.ins().call(callee, &arg_vals);
    let results = builder.inst_results(call);
    let result_val = results.first().copied();
    // Infer the return type from the callee's signature. We only care
    // about the first return value (Arcis functions return at most one
    // value today). The signature lives in the module's declaration
    // table; we read it via `module.declarations().get_function_decl`.
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
        // Build a unique, valid C identifier name derived from the bytes
        // so two functions emitting the same literal share a `DataId`. The
        // counter at the end disambiguates collisions in pathological
        // cases (e.g. two distinct strings whose hash maps collide).
        let mut name = String::from("arcis_str_");
        for &b in &bytes {
            if b.is_ascii_alphanumeric() {
                name.push(b as char);
            } else {
                name.push('_');
            }
        }
        // Trim trailing '_' to avoid colliding on trailing NUL escapes.
        while name.ends_with('_') {
            name.pop();
        }
        fctx.string_literal_counter += 1;
        name.push_str(&format!("_{}", fctx.string_literal_counter));
        let id = module
            .declare_data(&name, Linkage::Local, false, false)
            .map_err(|e| format!("declare string literal data `{}`: {}", name, e))?;
        let mut dd = DataDescription::new();
        dd.define(bytes.clone().into());
        dd.set_align(1);
        module
            .define_data(id, &dd)
            .map_err(|e| format!("define string literal data: {}", e))?;
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
    // Read the `len` field of the runtime record: the layout is
    //   struct { char* ptr; uint64_t len; uint64_t cap; }
    // On all supported targets, `int64_t` reads of a properly-aligned slot
    // are fine. `len` lives at offset 8 from the handle.
    let offset = builder.ins().iconst(I64, 8);
    let addr = builder.ins().iadd(handle, offset);
    let len = builder
        .ins()
        .load(I64, MemFlags::trusted(), addr, 0);
    Ok(len)
}

fn promote_to_string(
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
    }
}