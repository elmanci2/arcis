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
//!
//! `find`/`filter`/`map`/`reduce` build a real Cranelift loop (blocks +
//! `brif`) rather than splicing the callback body inline the way the Rust
//! backend does — the callback is a genuine indirect call target, resolved
//! by [`resolve_callback`]: either an existing named function, or an inline
//! arrow lambda-lifted **on the spot** into a fresh synthetic top-level
//! function (sound, since Arcis arrows never capture variables).

use std::collections::HashMap;

use arcis_ast::Expr;
use cranelift_codegen::ir::condcodes::{FloatCC, IntCC};
use cranelift_codegen::ir::types::{F64, I32, I64, I8};
use cranelift_codegen::ir::{InstBuilder, MemFlags, Value};
use cranelift_frontend::FunctionBuilder;
use cranelift_module::{FuncId, Linkage, Module as CraneliftModule};
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
    // `find`/`filter`/`map`/`reduce` build their own loop and evaluate
    // `object` themselves — handle them before the generic single-eval path
    // below (which the string/mutation methods share).
    match property {
        "find" if args.len() == 1 => return emit_find(builder, fctx, object, &args[0], runtime, user_fns, module).map(Some),
        "filter" if args.len() == 1 => return emit_filter(builder, fctx, object, &args[0], runtime, user_fns, module).map(Some),
        "map" if args.len() == 1 => return emit_map(builder, fctx, object, &args[0], runtime, user_fns, module).map(Some),
        "reduce" if args.len() == 2 => return emit_reduce(builder, fctx, object, &args[0], &args[1], runtime, user_fns, module).map(Some),
        _ => {}
    }

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

        // ── Not recognised ─────────────────────────────────────────
        _ => Ok(None),
    }
}

// ── Shared array-loop helpers ───────────────────────────────────────────

/// The element `ArcisType` of `object`, if it's an identifier with a
/// tracked array binding (falls back to `Number`, matching `ForOf`'s and
/// `Index`'s existing heuristic in `stmt.rs`/`expr.rs`).
fn elem_type_of(fctx: &FunctionCtx, object: &Expr) -> ArcisType {
    if let Expr::Ident(name) = object {
        fctx.element_ty(name).unwrap_or(ArcisType::Number)
    } else {
        ArcisType::Number
    }
}

fn vec_len(builder: &mut FunctionBuilder, module: &mut ObjectModule, runtime: &Runtime, arr: Value) -> Value {
    let callee = module.declare_func_in_func(runtime.vec_len, builder.func);
    let call = builder.ins().call(callee, &[arr]);
    builder.inst_results(call)[0]
}

fn vec_get(builder: &mut FunctionBuilder, module: &mut ObjectModule, runtime: &Runtime, arr: Value, idx_i32: Value) -> Value {
    let callee = module.declare_func_in_func(runtime.vec_get, builder.func);
    let call = builder.ins().call(callee, &[arr, idx_i32]);
    builder.inst_results(call)[0]
}

/// Coerce a raw `i64` slot (from `vec_get`) into its typed Cranelift
/// representation. Mirrors the identical logic already duplicated in
/// `stmt.rs`'s `ForOf` lowering and `expr.rs`'s `Index` lowering.
fn coerce_from_i64(builder: &mut FunctionBuilder, raw: Value, elem_ty: ArcisType) -> Value {
    match elem_ty {
        ArcisType::Number => builder.ins().bitcast(F64, MemFlags::new(), raw),
        ArcisType::Boolean => builder.ins().ireduce(I8, raw),
        ArcisType::String | ArcisType::Object | ArcisType::Array => raw,
        ArcisType::Void => raw,
    }
}

/// A default value for `elem_ty`, used by `find` when no element matches
/// the predicate (mirrors the Rust backend's `.cloned().unwrap_or_default()`).
fn default_value_for(
    builder: &mut FunctionBuilder,
    ty: ArcisType,
    module: &mut ObjectModule,
    runtime: &Runtime,
) -> Value {
    match ty {
        ArcisType::Number => builder.ins().f64const(0.0),
        ArcisType::Boolean => builder.ins().iconst(I8, 0),
        ArcisType::String => {
            let null_ptr = builder.ins().iconst(I64, 0);
            let callee = module.declare_func_in_func(runtime.string_from_cstr, builder.func);
            let call = builder.ins().call(callee, &[null_ptr]);
            builder.inst_results(call)[0]
        }
        ArcisType::Array | ArcisType::Object => builder.ins().iconst(I64, 0),
        ArcisType::Void => builder.ins().iconst(I8, 0),
    }
}

/// Resolve a callback argument to a callable `FuncId`: either an existing
/// named function (`Expr::Ident`, looked up in `user_fns` — this also
/// covers a top-level `let f = (x) => ...;` arrow, since
/// `module.rs::collect_arrow_lets` already lambda-lifted and registered it
/// there), or an inline arrow (`Expr::Arrow`) lambda-lifted right here into
/// a fresh synthetic top-level function. Returns `(FuncId, param types,
/// return type)`.
fn resolve_callback(
    fctx: &mut FunctionCtx,
    cb: &Expr,
    runtime: &Runtime,
    user_fns: &HashMap<String, FnInfo>,
    module: &mut ObjectModule,
) -> Result<(FuncId, Vec<ArcisType>, ArcisType), String> {
    match cb {
        Expr::Ident(name) => {
            let info = user_fns
                .get(name)
                .ok_or_else(|| format!("unknown function `{}` used as a callback", name))?;
            let ret_ty = {
                let decl = module.declarations().get_function_decl(info.id);
                match decl.signature.returns.first() {
                    Some(p) if p.value_type == F64 => ArcisType::Number,
                    Some(p) if p.value_type == I8 => ArcisType::Boolean,
                    Some(p) if p.value_type == I64 => ArcisType::String,
                    _ => ArcisType::Void,
                }
            };
            Ok((info.id, info.params.clone(), ret_ty))
        }
        Expr::Arrow { params, return_type, body } => {
            fctx.arrow_counter += 1;
            let synth_name = format!("__arrow_inline_{}", fctx.arrow_counter);
            let synthetic = crate::module::arrow_to_function(&synth_name, params, return_type, body);
            let enum_names: std::collections::HashSet<String> = fctx.enums().keys().cloned().collect();
            let param_tys = crate::module::param_types(&synthetic, &enum_names);
            let ret_ty = crate::types::from_ast(&synthetic.return_type, &enum_names).unwrap_or(ArcisType::Number);
            let sig = crate::module::function_signature(module, &synthetic, &enum_names);
            let id = module
                .declare_function(&synth_name, Linkage::Local, &sig)
                .map_err(|e| format!("declare inline arrow `{}`: {}", synth_name, e))?;

            // Build + define its body via a fresh `Context`, independent of
            // whatever function is currently mid-build in the caller.
            let mut inner_user_fns = user_fns.clone();
            let synth_ret = crate::module::return_type_of(&synthetic, &fctx.enum_names().clone());
            inner_user_fns.insert(synth_name.clone(), FnInfo { id, params: param_tys.clone(), ret: synth_ret });
            let reassigned = crate::collect::collect_reassigned(&arcis_ast::Program { stmts: synthetic.body.clone() });
            let mut ctx = cranelift_codegen::Context::new();
            ctx.func = cranelift_codegen::ir::Function::with_name_signature(
                cranelift_codegen::ir::UserFuncName::user(0, {
                    use cranelift_codegen::entity::EntityRef;
                    id.index() as u32
                }),
                sig,
            );
            let __gc = fctx.global_consts.clone();
            let __gf = fctx.global_field_types.clone();
            crate::function::emit_function(&mut ctx.func, &synthetic, &inner_user_fns, runtime, &reassigned, fctx.enums(), &__gc, &__gf, module)?;
            module
                .define_function(id, &mut ctx)
                .map_err(|e| format!("define inline arrow `{}`: {}", synth_name, e))?;

            Ok((id, param_tys, ret_ty))
        }
        _ => Err("a callback must be a function name or an arrow function".to_string()),
    }
}

/// `arr.find(cb)` -> the first element for which `cb(elem)` is truthy, or
/// `elem_ty`'s default if none match.
fn emit_find(
    builder: &mut FunctionBuilder,
    fctx: &mut FunctionCtx,
    object: &Expr,
    cb: &Expr,
    runtime: &Runtime,
    user_fns: &HashMap<String, FnInfo>,
    module: &mut ObjectModule,
) -> Result<(Value, ArcisType), String> {
    let (arr_val, _) = expr::emit(builder, fctx, object, runtime, user_fns, module)?;
    let elem_ty = elem_type_of(fctx, object);
    let (cb_id, _, _) = resolve_callback(fctx, cb, runtime, user_fns, module)?;

    let len_val = vec_len(builder, module, runtime, arr_val);
    let len_f64 = builder.ins().fcvt_from_uint(F64, len_val);

    let cond_block = builder.create_block();
    let body_block = builder.create_block();
    let match_block = builder.create_block();
    let next_block = builder.create_block();
    let after_block = builder.create_block();

    let zero_f = builder.ins().f64const(0.0);
    let one_f = builder.ins().f64const(1.0);
    let idx_var = fctx.define("__find_i", ArcisType::Number, zero_f, builder);
    let default_v = default_value_for(builder, elem_ty, module, runtime);
    let result_var = fctx.define("__find_result", elem_ty, default_v, builder);

    builder.ins().jump(cond_block, &[]);

    builder.switch_to_block(cond_block);
    let cur = builder.use_var(idx_var);
    let cmp = builder.ins().fcmp(FloatCC::LessThan, cur, len_f64);
    builder.ins().brif(cmp, body_block, &[], after_block, &[]);

    builder.switch_to_block(body_block);
    builder.seal_block(body_block);
    let cur2 = builder.use_var(idx_var);
    let idx_i32 = builder.ins().fcvt_to_sint(I32, cur2);
    let raw = vec_get(builder, module, runtime, arr_val, idx_i32);
    let elem_v = coerce_from_i64(builder, raw, elem_ty);
    let cb_callee = module.declare_func_in_func(cb_id, builder.func);
    let cb_call = builder.ins().call(cb_callee, &[elem_v]);
    let keep = builder.inst_results(cb_call)[0];
    builder.ins().brif(keep, match_block, &[], next_block, &[]);

    builder.switch_to_block(match_block);
    builder.seal_block(match_block);
    fctx.rebind("__find_result", elem_v, builder);
    builder.ins().jump(after_block, &[]);

    builder.switch_to_block(next_block);
    builder.seal_block(next_block);
    let cur3 = builder.use_var(idx_var);
    let next_idx = builder.ins().fadd(cur3, one_f);
    fctx.rebind("__find_i", next_idx, builder);
    builder.ins().jump(cond_block, &[]);
    builder.seal_block(cond_block);

    builder.switch_to_block(after_block);
    builder.seal_block(after_block);
    let result = builder.use_var(result_var);
    Ok((result, elem_ty))
}

/// `arr.filter(cb)` -> a new array with every element for which `cb(elem)`
/// is truthy.
fn emit_filter(
    builder: &mut FunctionBuilder,
    fctx: &mut FunctionCtx,
    object: &Expr,
    cb: &Expr,
    runtime: &Runtime,
    user_fns: &HashMap<String, FnInfo>,
    module: &mut ObjectModule,
) -> Result<(Value, ArcisType), String> {
    let (arr_val, _) = expr::emit(builder, fctx, object, runtime, user_fns, module)?;
    let elem_ty = elem_type_of(fctx, object);
    let (cb_id, _, _) = resolve_callback(fctx, cb, runtime, user_fns, module)?;

    let new_vec = {
        let callee = module.declare_func_in_func(runtime.vec_new, builder.func);
        let call = builder.ins().call(callee, &[]);
        builder.inst_results(call)[0]
    };

    let len_val = vec_len(builder, module, runtime, arr_val);
    let len_f64 = builder.ins().fcvt_from_uint(F64, len_val);

    let cond_block = builder.create_block();
    let body_block = builder.create_block();
    let push_block = builder.create_block();
    let next_block = builder.create_block();
    let after_block = builder.create_block();

    let zero_f = builder.ins().f64const(0.0);
    let one_f = builder.ins().f64const(1.0);
    let idx_var = fctx.define("__filter_i", ArcisType::Number, zero_f, builder);

    builder.ins().jump(cond_block, &[]);

    builder.switch_to_block(cond_block);
    let cur = builder.use_var(idx_var);
    let cmp = builder.ins().fcmp(FloatCC::LessThan, cur, len_f64);
    builder.ins().brif(cmp, body_block, &[], after_block, &[]);

    builder.switch_to_block(body_block);
    builder.seal_block(body_block);
    let cur2 = builder.use_var(idx_var);
    let idx_i32 = builder.ins().fcvt_to_sint(I32, cur2);
    let raw = vec_get(builder, module, runtime, arr_val, idx_i32);
    let elem_v = coerce_from_i64(builder, raw, elem_ty);
    let cb_callee = module.declare_func_in_func(cb_id, builder.func);
    let cb_call = builder.ins().call(cb_callee, &[elem_v]);
    let keep = builder.inst_results(cb_call)[0];
    builder.ins().brif(keep, push_block, &[], next_block, &[]);

    builder.switch_to_block(push_block);
    builder.seal_block(push_block);
    let i64_val = crate::expr::promote_to_i64(builder, elem_v, elem_ty);
    let push_callee = module.declare_func_in_func(runtime.vec_push, builder.func);
    builder.ins().call(push_callee, &[new_vec, i64_val]);
    builder.ins().jump(next_block, &[]);

    builder.switch_to_block(next_block);
    builder.seal_block(next_block);
    let cur3 = builder.use_var(idx_var);
    let next_idx = builder.ins().fadd(cur3, one_f);
    fctx.rebind("__filter_i", next_idx, builder);
    builder.ins().jump(cond_block, &[]);
    builder.seal_block(cond_block);

    builder.switch_to_block(after_block);
    builder.seal_block(after_block);
    Ok((new_vec, ArcisType::Array))
}

/// `arr.map(cb)` -> a new array with `cb(elem)` applied to every element.
fn emit_map(
    builder: &mut FunctionBuilder,
    fctx: &mut FunctionCtx,
    object: &Expr,
    cb: &Expr,
    runtime: &Runtime,
    user_fns: &HashMap<String, FnInfo>,
    module: &mut ObjectModule,
) -> Result<(Value, ArcisType), String> {
    let (arr_val, _) = expr::emit(builder, fctx, object, runtime, user_fns, module)?;
    let elem_ty = elem_type_of(fctx, object);
    let (cb_id, _, _ret_ty) = resolve_callback(fctx, cb, runtime, user_fns, module)?;

    let new_vec = {
        let callee = module.declare_func_in_func(runtime.vec_new, builder.func);
        let call = builder.ins().call(callee, &[]);
        builder.inst_results(call)[0]
    };

    let len_val = vec_len(builder, module, runtime, arr_val);
    let len_f64 = builder.ins().fcvt_from_uint(F64, len_val);

    let cond_block = builder.create_block();
    let body_block = builder.create_block();
    let after_block = builder.create_block();

    let zero_f = builder.ins().f64const(0.0);
    let one_f = builder.ins().f64const(1.0);
    let idx_var = fctx.define("__map_i", ArcisType::Number, zero_f, builder);

    builder.ins().jump(cond_block, &[]);

    builder.switch_to_block(cond_block);
    let cur = builder.use_var(idx_var);
    let cmp = builder.ins().fcmp(FloatCC::LessThan, cur, len_f64);
    builder.ins().brif(cmp, body_block, &[], after_block, &[]);

    builder.switch_to_block(body_block);
    builder.seal_block(body_block);
    let cur2 = builder.use_var(idx_var);
    let idx_i32 = builder.ins().fcvt_to_sint(I32, cur2);
    let raw = vec_get(builder, module, runtime, arr_val, idx_i32);
    let elem_v = coerce_from_i64(builder, raw, elem_ty);
    let cb_callee = module.declare_func_in_func(cb_id, builder.func);
    let cb_call = builder.ins().call(cb_callee, &[elem_v]);
    let mapped = builder.inst_results(cb_call)[0];
    // The callback's return type drives how to promote its result to the
    // i64 slot — reuse `promote_to_i64`, which dispatches on `ArcisType`.
    let mapped_i64 = crate::expr::promote_to_i64(builder, mapped, _ret_ty);
    let push_callee = module.declare_func_in_func(runtime.vec_push, builder.func);
    builder.ins().call(push_callee, &[new_vec, mapped_i64]);

    let cur3 = builder.use_var(idx_var);
    let next_idx = builder.ins().fadd(cur3, one_f);
    fctx.rebind("__map_i", next_idx, builder);
    builder.ins().jump(cond_block, &[]);
    builder.seal_block(cond_block);

    builder.switch_to_block(after_block);
    builder.seal_block(after_block);
    Ok((new_vec, ArcisType::Array))
}

/// `arr.reduce(cb, init)` -> `cb(...cb(cb(init, arr[0]), arr[1])..., arr[n-1])`.
fn emit_reduce(
    builder: &mut FunctionBuilder,
    fctx: &mut FunctionCtx,
    object: &Expr,
    cb: &Expr,
    init: &Expr,
    runtime: &Runtime,
    user_fns: &HashMap<String, FnInfo>,
    module: &mut ObjectModule,
) -> Result<(Value, ArcisType), String> {
    let (arr_val, _) = expr::emit(builder, fctx, object, runtime, user_fns, module)?;
    let elem_ty = elem_type_of(fctx, object);
    let (cb_id, _, _) = resolve_callback(fctx, cb, runtime, user_fns, module)?;
    let (init_v, init_ty) = expr::emit(builder, fctx, init, runtime, user_fns, module)?;

    let len_val = vec_len(builder, module, runtime, arr_val);
    let len_f64 = builder.ins().fcvt_from_uint(F64, len_val);

    let cond_block = builder.create_block();
    let body_block = builder.create_block();
    let after_block = builder.create_block();

    let zero_f = builder.ins().f64const(0.0);
    let one_f = builder.ins().f64const(1.0);
    let idx_var = fctx.define("__reduce_i", ArcisType::Number, zero_f, builder);
    let acc_var = fctx.define("__reduce_acc", init_ty, init_v, builder);

    builder.ins().jump(cond_block, &[]);

    builder.switch_to_block(cond_block);
    let cur = builder.use_var(idx_var);
    let cmp = builder.ins().fcmp(FloatCC::LessThan, cur, len_f64);
    builder.ins().brif(cmp, body_block, &[], after_block, &[]);

    builder.switch_to_block(body_block);
    builder.seal_block(body_block);
    let cur2 = builder.use_var(idx_var);
    let idx_i32 = builder.ins().fcvt_to_sint(I32, cur2);
    let raw = vec_get(builder, module, runtime, arr_val, idx_i32);
    let elem_v = coerce_from_i64(builder, raw, elem_ty);
    let acc_v = builder.use_var(acc_var);
    let cb_callee = module.declare_func_in_func(cb_id, builder.func);
    let cb_call = builder.ins().call(cb_callee, &[acc_v, elem_v]);
    let new_acc = builder.inst_results(cb_call)[0];
    fctx.rebind("__reduce_acc", new_acc, builder);

    let cur3 = builder.use_var(idx_var);
    let next_idx = builder.ins().fadd(cur3, one_f);
    fctx.rebind("__reduce_i", next_idx, builder);
    builder.ins().jump(cond_block, &[]);
    builder.seal_block(cond_block);

    builder.switch_to_block(after_block);
    builder.seal_block(after_block);
    let result = builder.use_var(acc_var);
    Ok((result, init_ty))
}

/// Public re-export for expr.rs to promote values to i64 handles.
#[allow(dead_code)]
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

// Compile-time check that `IntCC` stays imported (used only in the
// `includes` arm above, but keep the import path intentional/explicit).
#[allow(dead_code)]
fn _unused_intcc_marker(_: IntCC) {}
