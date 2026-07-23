//! Statement lowering.
//!
//! Phase 1 supports:
//! - `let` / `const`
//! - assignment (`x = …`, `obj.x = …`)
//! - `if` / `else`
//! - `while`
//! - `for (init; cond; update)` (desugared to Cranelift blocks)
//! - `return`
//! - `break` / `continue`
//! - expression statements (typically `print(...)` or function calls)
//!
//! The interesting complexity lives in control-flow statements, where each
//! loop is materialised as a triple of Cranelift blocks (cond / body /
//! step) so that `break` and `continue` can branch to the right targets
//! without ad-hoc dataflow tracking.

use std::collections::HashMap;

use arcis_ast::{Expr, Stmt};
use cranelift_codegen::ir::condcodes::{FloatCC, IntCC};
use cranelift_codegen::ir::types::{F64, I32, I64, I8};
use cranelift_codegen::ir::{InstBuilder, MemFlags};
use cranelift_frontend::FunctionBuilder;
use cranelift_module::{DataDescription, FuncId, Linkage, Module as CraneliftModule};
use cranelift_object::ObjectModule;

use crate::context::{FnInfo, FunctionCtx, LoopFrame};
use crate::expr;
use crate::rt::Runtime;
use crate::types::{from_ast, ArcisType};

/// Lower a single Arcis statement in the current Cranelift block. After
/// this returns, the builder's current block is in a state suitable for
/// the next sibling statement: either the original block (if `stmt` did
/// not branch) or a fresh block where continuation code should be emitted.
pub(crate) fn emit_stmt(
    builder: &mut FunctionBuilder,
    fctx: &mut FunctionCtx,
    stmt: &Stmt,
    runtime: &Runtime,
    user_fns: &HashMap<String, FnInfo>,
    module: &mut ObjectModule,
) -> Result<(), String> {
    match stmt {
        Stmt::Let { value: Expr::Arrow { .. }, .. } | Stmt::Const { value: Expr::Arrow { .. }, .. } => {
            // `let name = (params) => body;` was already lambda-lifted into
            // a synthetic top-level function and registered in `user_fns`
            // by `module.rs::collect_arrow_lets` — nothing to emit here.
            // `name(...)` calls resolve through the normal `user_fns` path.
            Ok(())
        }
        Stmt::Let { name, ty, value, .. } | Stmt::Const { name, ty, value, .. } => {
            let (v, inferred_ty) = expr::emit(builder, fctx, value, runtime, user_fns, module)?;
            let resolved = match ty {
                Some(t) => from_ast(t, fctx.enum_names())?,
                None => inferred_ty,
            };
            fctx.define(name, resolved, v, builder);
            // Track element type for array bindings and field types for objects.
            if let Some(t) = ty {
                // The "object shape" of `t`, unwrapping one level of `T[]`
                // if present (an array of objects still needs its field
                // types tracked for `arr[i].field` access).
                let object_source = match t.array_inner() {
                    Some(inner) => Some(inner),
                    None => Some(t),
                };
                if let Some(inner) = t.array_inner() {
                    fctx.set_element_ty(name, from_ast(inner, fctx.enum_names())?);
                }
                if let Some(fields) = object_source.and_then(|o| o.object_fields()) {
                    fctx.set_object_fields(name, fields);
                }
            }
            Ok(())
        }
        Stmt::Assign { name, value } => {
            let (v, _) = expr::emit(builder, fctx, value, runtime, user_fns, module)?;
            fctx.rebind(name, v, builder);
            Ok(())
        }
        Stmt::AssignIndex { object, index, value } => {
            let (obj_val, _) = expr::emit(builder, fctx, &Expr::Ident(object.clone()), runtime, user_fns, module)?;
            let (idx_val, _) = expr::emit(builder, fctx, index, runtime, user_fns, module)?;
            let (val_v, val_ty) = expr::emit(builder, fctx, value, runtime, user_fns, module)?;
            let idx_i32 = builder.ins().fcvt_to_sint(I32, idx_val);
            let i64_val = crate::expr::promote_to_i64(builder, val_v, val_ty);
            let callee = module.declare_func_in_func(runtime.vec_set, builder.func);
            builder.ins().call(callee, &[obj_val, idx_i32, i64_val]);
            fctx.rebind(object, obj_val, builder);
            Ok(())
        }
        Stmt::AssignMember { object, property, value } => {
            let (obj_val, _) = expr::emit(builder, fctx, object, runtime, user_fns, module)?;
            let (val_v, val_ty) = expr::emit(builder, fctx, value, runtime, user_fns, module)?;
            let i64_val = crate::expr::promote_to_i64(builder, val_v, val_ty);
            // The key is a string literal we pass as a handle.
            let key_bytes: Vec<u8> = {
                let mut b = Vec::with_capacity(property.len() + 1);
                b.extend_from_slice(property.as_bytes());
                b.push(0);
                b
            };
            let key_id = if let Some(&existing) = fctx.string_literals.get(&key_bytes) {
                existing
            } else {
                let mut hash: u64 = 0xcbf29ce484222325;
                for &b in &key_bytes {
                    hash = hash.wrapping_mul(0x100000001b3).wrapping_add(b as u64);
                }
                let name = format!("arcis_key_{:016x}", hash);
                let id = if let Some(existing) = module.declarations().get_name(&name) {
                    match existing {
                        cranelift_module::FuncOrDataId::Data(did) => did,
                        _ => return Err(format!("name collision: `{}` is a function", name)),
                    }
                } else {
                    let id = module
                        .declare_data(&name, Linkage::Local, false, false)
                        .map_err(|e| format!("declare key data `{}`: {}", name, e))?;
                    let mut dd = DataDescription::new();
                    dd.define(key_bytes.clone().into());
                    dd.set_align(1);
                    module.define_data(id, &dd).map_err(|e| format!("define key data: {}", e))?;
                    id
                };
                fctx.string_literals.insert(key_bytes.clone(), id);
                id
            };
            let key_gv = module.declare_data_in_func(key_id, builder.func);
            let key_addr = builder.ins().global_value(I64, key_gv);
            let key_handle = {
                let callee_s = module.declare_func_in_func(runtime.string_from_cstr, builder.func);
                let call_s = builder.ins().call(callee_s, &[key_addr]);
                builder.inst_results(call_s)[0]
            };
            let callee_set = module.declare_func_in_func(runtime.object_set, builder.func);
            builder.ins().call(callee_set, &[obj_val, key_handle, i64_val]);
            // Re-bind the object variable if it's an Ident.
            if let Expr::Ident(name) = object.as_ref() {
                fctx.rebind(name, obj_val, builder);
            }
            Ok(())
        }
        Stmt::Return(value) => {
            // Leaving the function entirely: every currently-open `try`
            // must call `arcis_try_end()` first so the C runtime's
            // `arcis_try_depth` doesn't leak past this call.
            let v = match value {
                Some(e) => {
                    let (v, _) = expr::emit(builder, fctx, e, runtime, user_fns, module)?;
                    Some(v)
                }
                None => None,
            };
            emit_try_ends(builder, module, runtime, fctx.open_try_count);
            match v {
                Some(v) => builder.ins().return_(&[v]),
                None => builder.ins().return_(&[]),
            };
            Ok(())
        }
        Stmt::If { condition, then_branch, else_branch } => {
            emit_if(builder, fctx, condition, then_branch, else_branch.as_deref(), None, runtime, user_fns, module)
        }
        Stmt::While { condition, body } => {
            emit_while(builder, fctx, condition, body, runtime, user_fns, module)
        }
        Stmt::For { init, condition, update, body } => {
            emit_for(builder, fctx, init.as_deref(), condition.as_ref(), update.as_deref(), body, runtime, user_fns, module)
        }
        Stmt::ForOf { name, ty, iterable, body } => {
            // Lower `for (let name of iterable)` as:
            //   let __i = 0;
            //   loop { if __i >= len(iter) → break; name = iter[__i]; body; __i++ }
            let (arr_val, _) = expr::emit(builder, fctx, iterable, runtime, user_fns, module)?;
            let len_callee = module.declare_func_in_func(runtime.vec_len, builder.func);
            let len_call = builder.ins().call(len_callee, &[arr_val]);
            let len_val = builder.inst_results(len_call)[0]; // i32
            let len_f64 = builder.ins().fcvt_from_uint(F64, len_val);

            let cond_block = builder.create_block();
            let body_block = builder.create_block();
            let after_block = builder.create_block();

            // Index variable: Cranelift SSA Variable for the counter.
            let idx_type = ArcisType::Number;
            let zero = builder.ins().f64const(0.0);
            let one = builder.ins().f64const(1.0);
            let idx_var = fctx.define("__for_i", idx_type, zero, builder);

            if !is_block_terminated(builder) {
                builder.ins().jump(cond_block, &[]);
            }

            builder.switch_to_block(cond_block);
            let cur_idx = builder.use_var(idx_var);
            let cmp = builder.ins().fcmp(FloatCC::LessThan, cur_idx, len_f64);
            builder.ins().brif(cmp, body_block, &[], after_block, &[]);

            builder.switch_to_block(body_block);
            fctx.push_loop(LoopFrame {
                continue_target: cond_block,
                break_target: after_block,
                after: after_block,
                try_depth_at_entry: fctx.open_try_count,
            });

            // Load element: cur_idx (f64) → i32 → vec_get
            let cur_idx_v = builder.use_var(idx_var);
            let idx_i32 = builder.ins().fcvt_to_sint(I32, cur_idx_v);
            let get_callee = module.declare_func_in_func(runtime.vec_get, builder.func);
            let get_call = builder.ins().call(get_callee, &[arr_val, idx_i32]);
            let elem_val = builder.inst_results(get_call)[0];
            // Infer element type from the array binding if possible.
            let (elem_cl_val, elem_ty) = if let Expr::Ident(arr_name) = iterable.as_ref() {
                match fctx.element_ty(arr_name).unwrap_or(ArcisType::Number) {
                    ArcisType::String => (elem_val, ArcisType::String),
                    ArcisType::Number => {
                        let as_f64 = builder.ins().bitcast(F64, MemFlags::new(), elem_val);
                        (as_f64, ArcisType::Number)
                    }
                    ArcisType::Boolean => {
                        let as_i8 = builder.ins().ireduce(I8, elem_val);
                        (as_i8, ArcisType::Boolean)
                    }
                    // Object, Array, and other handle types: pass as raw I64.
                    other @ (ArcisType::Object | ArcisType::Array) => (elem_val, other),
                    other => {
                        let as_f64 = builder.ins().bitcast(F64, MemFlags::new(), elem_val);
                        (as_f64, other)
                    }
                }
            } else {
                let as_f64 = builder.ins().bitcast(F64, MemFlags::new(), elem_val);
                (as_f64, ArcisType::Number)
            };
            fctx.define(name, elem_ty, elem_cl_val, builder);

            for s in body {
                emit_stmt(builder, fctx, s, runtime, user_fns, module)?;
            }
            fctx.pop_loop();

            // Increment counter.
            let cur = builder.use_var(idx_var);
            let next = builder.ins().fadd(cur, one);
            fctx.rebind("__for_i", next, builder);
            if !is_block_terminated(builder) {
                builder.ins().jump(cond_block, &[]);
            }
            builder.seal_block(cond_block);
            builder.seal_block(body_block);

            builder.switch_to_block(after_block);
            builder.seal_block(after_block);
            Ok(())
        }
        Stmt::Break => {
            let frame = *fctx
                .current_loop()
                .ok_or_else(|| "`break` outside of a loop".to_string())?;
            // Close every `try` opened since the loop was entered (a `try`
            // wrapping the whole loop stays open).
            emit_try_ends(builder, module, runtime, fctx.open_try_count - frame.try_depth_at_entry);
            builder.ins().jump(frame.break_target, &[]);
            Ok(())
        }
        Stmt::Continue => {
            let frame = *fctx
                .current_loop()
                .ok_or_else(|| "`continue` outside of a loop".to_string())?;
            emit_try_ends(builder, module, runtime, fctx.open_try_count - frame.try_depth_at_entry);
            builder.ins().jump(frame.continue_target, &[]);
            Ok(())
        }
        Stmt::Expr(e) => {
            let _ = expr::emit(builder, fctx, e, runtime, user_fns, module)?;
            Ok(())
        }
        Stmt::Function(_) => Ok(()),
        Stmt::Import { .. } | Stmt::FromImport { .. } => {
            // Imports are handled at module level (declaring FuncIds as
            // Linkage::Import). Nothing to emit in the function body.
            Ok(())
        }
        Stmt::ExportDecl(inner) => emit_stmt(builder, fctx, inner, runtime, user_fns, module),
        Stmt::ExportSpec(_) => {
            Err("`export { ... }` is not yet supported by the Cranelift backend (Phase 4)".to_string())
        }
        Stmt::ExportDefault(_) => {
            Err("`export default` is not yet supported by the Cranelift backend (Phase 4)".to_string())
        }
        Stmt::TypeAlias { .. } | Stmt::Interface { .. } | Stmt::Enum { .. } => {
            // Type-only / enum declarations have no runtime representation —
            // enum variant access resolves to a constant at the `Expr::Member`
            // call site (see `expr.rs`), driven by `collect::collect_enums`.
            Ok(())
        }
        Stmt::Switch { discriminant, cases } => {
            emit_switch(builder, fctx, discriminant, cases, runtime, user_fns, module)
        }
        Stmt::Try { body, catch_name, catch_body } => {
            emit_try(builder, fctx, body, catch_name.as_deref(), catch_body, runtime, user_fns, module)
        }
        Stmt::Throw(expr) => emit_throw(builder, fctx, expr, runtime, user_fns, module),
    }
}

/// Lower an `if` statement.
///
/// `continuation`: when this `if` is the `else` branch of a parent `if`,
/// the continuation is the parent's after_block — all `if`/`else if`
/// leaves jump there directly, so no extra block is needed. When
/// `continuation` is `None`, we create a fresh after_block.
fn emit_if(
    builder: &mut FunctionBuilder,
    fctx: &mut FunctionCtx,
    condition: &Expr,
    then_branch: &[Stmt],
    else_branch: Option<&[Stmt]>,
    continuation: Option<cranelift_codegen::ir::Block>,
    runtime: &Runtime,
    user_fns: &HashMap<String, FnInfo>,
    module: &mut ObjectModule,
) -> Result<(), String> {
    // Evaluate the condition in the current block. Whatever block is
    // active here becomes the source of the brif; it must therefore
    // have no terminator yet — we just emitted user code or were just
    // switched here.
    let (cond_v, _) = expr::emit(builder, fctx, condition, runtime, user_fns, module)?;
    let then_block = builder.create_block();
    let else_block = builder.create_block();
    let after_block = match continuation {
        Some(b) => b,
        None => builder.create_block(),
    };

    let cond_bool = to_truthy(builder, cond_v);
    builder.ins().brif(cond_bool, then_block, &[], else_block, &[]);

    builder.switch_to_block(then_block);
    builder.seal_block(then_block);
    for s in then_branch {
        emit_stmt(builder, fctx, s, runtime, user_fns, module)?;
    }
    if !is_block_terminated(builder) {
        builder.ins().jump(after_block, &[]);
    }

    builder.switch_to_block(else_block);
    builder.seal_block(else_block);
    match else_branch {
        // Plain else: run the statements in `else_block`. The last
        // statement leaves us in `else_block` again (unless it itself
        // branches); fall through to `after_block` if not yet terminated.
        Some([]) => {
            if !is_block_terminated(builder) {
                builder.ins().jump(after_block, &[]);
            }
        }
        Some([single]) => {
            // Detect `else if`: recurse with the same `after_block`.
            if let Stmt::If { condition: c2, then_branch: tb2, else_branch: eb2 } = single {
                emit_if(
                    builder,
                    fctx,
                    c2,
                    tb2,
                    eb2.as_deref(),
                    Some(after_block),
                    runtime,
                    user_fns,
                    module,
                )?;
            } else {
                emit_stmt(builder, fctx, single, runtime, user_fns, module)?;
                if !is_block_terminated(builder) {
                    builder.ins().jump(after_block, &[]);
                }
            }
        }
        Some(many) => {
            for s in many {
                emit_stmt(builder, fctx, s, runtime, user_fns, module)?;
            }
            if !is_block_terminated(builder) {
                builder.ins().jump(after_block, &[]);
            }
        }
        None => {
            // No else branch: the `else_block` simply falls through to
            // `after_block`.
            builder.ins().jump(after_block, &[]);
        }
    }

    if continuation.is_none() {
        builder.switch_to_block(after_block);
        builder.seal_block(after_block);
    }
    Ok(())
}

/// `switch (d) { case v1: A case v2: B default: C }` -> a chain of
/// equality comparisons against the discriminant, deliberately NOT a
/// Cranelift jump-table/`br_table`: case values are arbitrary expressions
/// (not required to be compile-time constants — the parser allows
/// `case someExpr():`), so each one must go through `expr::emit` like any
/// other expression. Non-fallthrough: every case body unconditionally
/// jumps to `after_block`; `break` inside a case is a no-op (see
/// `Stmt::Break`'s handling in `emit_switch_case_body`).
fn emit_switch(
    builder: &mut FunctionBuilder,
    fctx: &mut FunctionCtx,
    discriminant: &Expr,
    cases: &[arcis_ast::SwitchCase],
    runtime: &Runtime,
    user_fns: &HashMap<String, FnInfo>,
    module: &mut ObjectModule,
) -> Result<(), String> {
    let (disc_v, disc_ty) = expr::emit(builder, fctx, discriminant, runtime, user_fns, module)?;
    // Stash the discriminant in a Cranelift Variable so each case can
    // compare against it without re-evaluating `discriminant` (which may
    // have side effects, e.g. a function call).
    let disc_var = fctx.define("__switch_disc", disc_ty, disc_v, builder);

    let after_block = builder.create_block();
    let default_case = cases.iter().find(|c| c.is_default);

    for case in cases.iter().filter(|c| !c.is_default) {
        let cur_disc = builder.use_var(disc_var);
        let mut cond: Option<cranelift_codegen::ir::Value> = None;
        for value_expr in &case.values {
            let (val_v, val_ty) = expr::emit(builder, fctx, value_expr, runtime, user_fns, module)?;
            let eq = crate::expr::emit_eq(builder, module, runtime, cur_disc, disc_ty, val_v, val_ty)?;
            cond = Some(match cond {
                None => eq,
                Some(prev) => {
                    // OR two I8 0/1 booleans via a widen/bor/narrow-back
                    // round-trip (mirrors `emit_binary`'s boolean `||`).
                    let prev32 = builder.ins().uextend(I32, prev);
                    let eq32 = builder.ins().uextend(I32, eq);
                    let bor = builder.ins().bor(prev32, eq32);
                    let zero = builder.ins().iconst(I32, 0);
                    builder.ins().icmp(IntCC::NotEqual, bor, zero)
                }
            });
        }
        let cond = cond.ok_or_else(|| "`case` with no values".to_string())?;

        let case_block = builder.create_block();
        let next_check_block = builder.create_block();
        builder.ins().brif(cond, case_block, &[], next_check_block, &[]);

        builder.switch_to_block(case_block);
        builder.seal_block(case_block);
        emit_switch_case_body(builder, fctx, &case.body, runtime, user_fns, module)?;
        if !is_block_terminated(builder) {
            builder.ins().jump(after_block, &[]);
        }

        builder.switch_to_block(next_check_block);
        builder.seal_block(next_check_block);
    }

    // Reached only when no case matched.
    if let Some(case) = default_case {
        emit_switch_case_body(builder, fctx, &case.body, runtime, user_fns, module)?;
    }
    if !is_block_terminated(builder) {
        builder.ins().jump(after_block, &[]);
    }

    builder.switch_to_block(after_block);
    builder.seal_block(after_block);
    Ok(())
}

/// Emit a switch case's body, treating a top-level `break;` as a no-op —
/// mirrors the Rust backend's `emit_switch_case_body` (there is no Cranelift
/// loop/labeled-block wrapping each case, so a real `break` would have
/// nowhere valid to jump to; a `break` inside a loop *nested* within the
/// case is unaffected, since that loop pushes its own `LoopFrame`).
fn emit_switch_case_body(
    builder: &mut FunctionBuilder,
    fctx: &mut FunctionCtx,
    body: &[Stmt],
    runtime: &Runtime,
    user_fns: &HashMap<String, FnInfo>,
    module: &mut ObjectModule,
) -> Result<(), String> {
    for s in body {
        if matches!(s, Stmt::Break) {
            continue;
        }
        emit_stmt(builder, fctx, s, runtime, user_fns, module)?;
    }
    Ok(())
}

fn emit_while(
    builder: &mut FunctionBuilder,
    fctx: &mut FunctionCtx,
    condition: &Expr,
    body: &[Stmt],
    runtime: &Runtime,
    user_fns: &HashMap<String, FnInfo>,
    module: &mut ObjectModule,
) -> Result<(), String> {
    let cond_block = builder.create_block();
    let body_block = builder.create_block();
    let after_block = builder.create_block();

    // Initial jump into the cond block.
    if !is_block_terminated(builder) {
        builder.ins().jump(cond_block, &[]);
    }

    builder.switch_to_block(cond_block);
    let (cond_v, _) = expr::emit(builder, fctx, condition, runtime, user_fns, module)?;
    let cond_bool = to_truthy(builder, cond_v);
    builder.ins().brif(cond_bool, body_block, &[], after_block, &[]);

    builder.switch_to_block(body_block);
    fctx.push_loop(LoopFrame {
        continue_target: cond_block,
        break_target: after_block,
        after: after_block,
        try_depth_at_entry: fctx.open_try_count,
    });
    for s in body {
        emit_stmt(builder, fctx, s, runtime, user_fns, module)?;
    }
    fctx.pop_loop();
    if !is_block_terminated(builder) {
        builder.ins().jump(cond_block, &[]);
    }
    // After all predecessors are known (entry block + body block),
    // seal cond_block. This is where Cranelift inserts any phi nodes
    // required by SSA value flows from those predecessors.
    builder.seal_block(cond_block);
    builder.seal_block(body_block);

    builder.switch_to_block(after_block);
    builder.seal_block(after_block);
    Ok(())
}

fn emit_for(
    builder: &mut FunctionBuilder,
    fctx: &mut FunctionCtx,
    init: Option<&Stmt>,
    condition: Option<&Expr>,
    update: Option<&Stmt>,
    body: &[Stmt],
    runtime: &Runtime,
    user_fns: &HashMap<String, FnInfo>,
    module: &mut ObjectModule,
) -> Result<(), String> {
    if let Some(init_stmt) = init {
        emit_stmt(builder, fctx, init_stmt, runtime, user_fns, module)?;
    }
    let cond_block = builder.create_block();
    let body_block = builder.create_block();
    let update_block = builder.create_block();
    let after_block = builder.create_block();

    if !is_block_terminated(builder) {
        builder.ins().jump(cond_block, &[]);
    }

    builder.switch_to_block(cond_block);
    let cond_bool = if let Some(c) = condition {
        let (cond_v, _) = expr::emit(builder, fctx, c, runtime, user_fns, module)?;
        to_truthy(builder, cond_v)
    } else {
        // `for (;;)` => always true.
        builder.ins().iconst(I8, 1)
    };
    builder.ins().brif(cond_bool, body_block, &[], after_block, &[]);

    builder.switch_to_block(body_block);
    fctx.push_loop(LoopFrame {
        continue_target: update_block,
        break_target: after_block,
        after: after_block,
        try_depth_at_entry: fctx.open_try_count,
    });
    for s in body {
        emit_stmt(builder, fctx, s, runtime, user_fns, module)?;
    }
    fctx.pop_loop();
    if !is_block_terminated(builder) {
        builder.ins().jump(update_block, &[]);
    }

    builder.switch_to_block(update_block);
    if let Some(upd) = update {
        emit_stmt(builder, fctx, upd, runtime, user_fns, module)?;
    }
    if !is_block_terminated(builder) {
        builder.ins().jump(cond_block, &[]);
    }
    // Seal cond_block now that its predecessors (entry + update) are
    // known and phi nodes can be inserted. Body is sealed after the
    // jump to update_block has been emitted.
    builder.seal_block(body_block);
    builder.seal_block(cond_block);
    builder.seal_block(update_block);

    builder.switch_to_block(after_block);
    builder.seal_block(after_block);
    Ok(())
}

/// `try { body } catch (e) { catch_body }` -> `arcis_try_begin`/`longjmp`.
///
/// `arcis_try_begin()` returns `0` on the direct call (about to run `body`)
/// or `1` when control resumed via `longjmp` from a `throw` somewhere in
/// `body` (possibly several calls deep). Both paths converge on
/// `after_block`, each having called `arcis_try_end()` exactly once first —
/// see `runtime.rs`'s module doc comment for why the C-side depth counter
/// isn't decremented by `longjmp` itself.
fn emit_try(
    builder: &mut FunctionBuilder,
    fctx: &mut FunctionCtx,
    body: &[Stmt],
    catch_name: Option<&str>,
    catch_body: &[Stmt],
    runtime: &Runtime,
    user_fns: &HashMap<String, FnInfo>,
    module: &mut ObjectModule,
) -> Result<(), String> {
    // Reserve a `jmp_buf` slot (a normal call — safe to wrap and return
    // from) ...
    let callee_push = module.declare_func_in_func(runtime.try_push, builder.func);
    let call_push = builder.ins().call(callee_push, &[]);
    let buf_ptr = builder.inst_results(call_push)[0];
    // ... then call `setjmp` on it *directly*: its "containing function" is
    // this Cranelift-compiled function, which does not return between here
    // and any later `longjmp` (control just moves between this function's
    // own blocks while the `try` body runs) — see `runtime.rs`'s comment
    // for why a wrapper that itself returns is unsound here.
    let callee_setjmp = module.declare_func_in_func(runtime.raw_setjmp, builder.func);
    let call_setjmp = builder.ins().call(callee_setjmp, &[buf_ptr]);
    let r = builder.inst_results(call_setjmp)[0];

    let try_block = builder.create_block();
    let catch_block = builder.create_block();
    let after_block = builder.create_block();

    let zero = builder.ins().iconst(I32, 0);
    let is_normal = builder.ins().icmp(IntCC::Equal, r, zero);
    builder.ins().brif(is_normal, try_block, &[], catch_block, &[]);

    builder.switch_to_block(try_block);
    builder.seal_block(try_block);
    fctx.open_try_count += 1;
    for s in body {
        emit_stmt(builder, fctx, s, runtime, user_fns, module)?;
    }
    fctx.open_try_count -= 1;
    if !is_block_terminated(builder) {
        emit_try_ends(builder, module, runtime, 1);
        builder.ins().jump(after_block, &[]);
    }

    builder.switch_to_block(catch_block);
    builder.seal_block(catch_block);
    emit_try_ends(builder, module, runtime, 1);
    if let Some(name) = catch_name {
        let callee_val = module.declare_func_in_func(runtime.thrown_value, builder.func);
        let call_val = builder.ins().call(callee_val, &[]);
        let handle = builder.inst_results(call_val)[0];
        fctx.define(name, ArcisType::String, handle, builder);
    }
    for s in catch_body {
        emit_stmt(builder, fctx, s, runtime, user_fns, module)?;
    }
    if !is_block_terminated(builder) {
        builder.ins().jump(after_block, &[]);
    }

    builder.switch_to_block(after_block);
    builder.seal_block(after_block);
    Ok(())
}

/// `throw expr;` -> coerce `expr` to a string message and call `arcis_throw`.
fn emit_throw(
    builder: &mut FunctionBuilder,
    fctx: &mut FunctionCtx,
    expr_node: &Expr,
    runtime: &Runtime,
    user_fns: &HashMap<String, FnInfo>,
    module: &mut ObjectModule,
) -> Result<(), String> {
    let (val, ty) = expr::emit(builder, fctx, expr_node, runtime, user_fns, module)?;
    let handle = crate::expr::promote_to_string(builder, val, ty, module, runtime)?;
    let callee = module.declare_func_in_func(runtime.throw, builder.func);
    builder.ins().call(callee, &[handle]);
    Ok(())
}

/// Emit `count` calls to `arcis_try_end()`, keeping the C runtime's
/// `arcis_try_depth` balanced across an early exit (`return`/`break`/
/// `continue`) out of one or more open `try` blocks.
fn emit_try_ends(builder: &mut FunctionBuilder, module: &mut ObjectModule, runtime: &Runtime, count: u32) {
    for _ in 0..count {
        let callee = module.declare_func_in_func(runtime.try_end, builder.func);
        builder.ins().call(callee, &[]);
    }
}

/// Coerce a value of any Arcis type into an `i8` truthy for `brif`.
/// Cranelift's `brif` takes any integer value and branches to the first
/// target when it is non-zero. Boolean SSA values from Cranelift's
/// `fcmp`/`icmp` are already `0`/`1` `I8`s that can be passed directly;
/// floats need a non-zero comparison; string handles need a NULL check.
fn to_truthy(builder: &mut FunctionBuilder, value: cranelift_codegen::ir::Value) -> cranelift_codegen::ir::Value {
    let ty = builder.func.dfg.value_type(value);
    if ty == F64 {
        let zero = builder.ins().f64const(0.0);
        return builder.ins().fcmp(FloatCC::NotEqual, value, zero);
    }
    if ty == I8 {
        // Already 0/1 from fcmp/icmp; pass through.
        return value;
    }
    // I64 (string handle): non-null = truthy.
    let zero = builder.ins().iconst(cranelift_codegen::ir::types::I64, 0);
    builder.ins().icmp(IntCC::NotEqual, value, zero)
}

/// Heuristic: is the current block already terminated? We look at the last
/// instruction Cranelift placed in the block and check whether it is a
/// terminator (jump, brif, return, …).
fn is_block_terminated(builder: &FunctionBuilder) -> bool {
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