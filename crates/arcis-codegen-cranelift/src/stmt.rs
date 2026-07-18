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
use cranelift_codegen::ir::types::{F64, I8};
use cranelift_codegen::ir::{InstBuilder, Opcode};
use cranelift_frontend::FunctionBuilder;
use cranelift_module::FuncId;
use cranelift_module::Module as CraneliftModule;
use cranelift_object::ObjectModule;

use crate::context::{FunctionCtx, LoopFrame};
use crate::expr;
use crate::rt::Runtime;
use crate::types::from_ast;

/// Lower a single Arcis statement in the current Cranelift block. After
/// this returns, the builder's current block is in a state suitable for
/// the next sibling statement: either the original block (if `stmt` did
/// not branch) or a fresh block where continuation code should be emitted.
pub(crate) fn emit_stmt(
    builder: &mut FunctionBuilder,
    fctx: &mut FunctionCtx,
    stmt: &Stmt,
    runtime: &Runtime,
    user_fns: &HashMap<String, FuncId>,
    module: &mut ObjectModule,
) -> Result<(), String> {
    match stmt {
        Stmt::Let { name, ty, value, .. } | Stmt::Const { name, ty, value, .. } => {
            let (v, inferred_ty) = expr::emit(builder, fctx, value, runtime, user_fns, module)?;
            let resolved = match ty {
                Some(t) => from_ast(&t.name, t.is_array)?,
                None => inferred_ty,
            };
            fctx.define(name, resolved, v, builder);
            Ok(())
        }
        Stmt::Assign { name, value } => {
            let (v, _) = expr::emit(builder, fctx, value, runtime, user_fns, module)?;
            fctx.rebind(name, v, builder);
            Ok(())
        }
        Stmt::AssignIndex { .. } | Stmt::AssignMember { .. } => {
            Err("indexed/field assignment is not yet supported by the Cranelift backend (Phase 2)".to_string())
        }
        Stmt::Return(value) => {
            if let Some(e) = value {
                let (v, _) = expr::emit(builder, fctx, e, runtime, user_fns, module)?;
                builder.ins().return_(&[v]);
            } else {
                builder.ins().return_(&[]);
            }
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
        Stmt::ForOf { .. } => {
            Err("`for-of` is not yet supported by the Cranelift backend (Phase 2)".to_string())
        }
        Stmt::Break => {
            let frame = fctx
                .current_loop()
                .ok_or_else(|| "`break` outside of a loop".to_string())?;
            builder.ins().jump(frame.break_target, &[]);
            Ok(())
        }
        Stmt::Continue => {
            let frame = fctx
                .current_loop()
                .ok_or_else(|| "`continue` outside of a loop".to_string())?;
            builder.ins().jump(frame.continue_target, &[]);
            Ok(())
        }
        Stmt::Expr(e) => {
            let _ = expr::emit(builder, fctx, e, runtime, user_fns, module)?;
            Ok(())
        }
        Stmt::Function(_) => Ok(()),
        Stmt::Import { .. } => {
            Err("imports are not yet supported by the Cranelift backend (Phase 4)".to_string())
        }
        Stmt::ExportDecl(inner) => emit_stmt(builder, fctx, inner, runtime, user_fns, module),
        Stmt::ExportSpec(_) => {
            Err("`export { ... }` is not yet supported by the Cranelift backend (Phase 4)".to_string())
        }
        Stmt::ExportDefault(_) => {
            Err("`export default` is not yet supported by the Cranelift backend (Phase 4)".to_string())
        }
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
    user_fns: &HashMap<String, FuncId>,
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

fn emit_while(
    builder: &mut FunctionBuilder,
    fctx: &mut FunctionCtx,
    condition: &Expr,
    body: &[Stmt],
    runtime: &Runtime,
    user_fns: &HashMap<String, FuncId>,
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
    user_fns: &HashMap<String, FuncId>,
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