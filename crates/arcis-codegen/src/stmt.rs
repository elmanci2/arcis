//! Statement emission.
//!
//! Dispatches on the [`Stmt`] variant and emits the corresponding Rust
//! source at the requested indentation level.

use arcis_ast::Stmt;

use crate::context::Ctx;

/// Emit one statement at `level` indentation.
pub(crate) fn emit(out: &mut String, stmt: &Stmt, level: usize, ctx: &Ctx) {
    match stmt {
        Stmt::Let { name, ty, value, .. } => emit_let(out, name, ty.as_ref(), value, level, ctx),
        Stmt::Const { name, ty, value, .. } => {
            emit_const(out, name, ty.as_ref(), value, level, ctx)
        }
        Stmt::Assign { name, value } => emit_assign(out, name, value, level, ctx),
        Stmt::AssignIndex { object, index, value } => {
            emit_assign_index(out, object, index, value, level, ctx)
        }
        Stmt::AssignMember { object, property, value } => {
            emit_assign_member(out, object, property, value, level, ctx)
        }
        Stmt::Function(_) => {
            // Functions were emitted before `main`; ignore them here.
        }
        Stmt::Return(expr) => emit_return(out, expr.as_ref(), level, ctx),
        Stmt::If { condition, then_branch, else_branch } => {
            emit_if(out, condition, then_branch, else_branch.as_deref(), level, ctx)
        }
        Stmt::While { condition, body } => emit_while(out, condition, body, level, ctx),
        Stmt::ForOf { name, ty, iterable, body } => {
            emit_for_of(out, name, ty.as_ref(), iterable, body, level, ctx)
        }
        Stmt::For { init, condition, update, body } => {
            emit_for(out, init.as_deref(), condition.as_ref(), update.as_deref(), body, level, ctx)
        }
        Stmt::Break => {
            crate::types::indent(out, level);
            out.push_str("break;\n");
        }
        Stmt::Continue => {
            crate::types::indent(out, level);
            out.push_str("continue;\n");
        }
        Stmt::Expr(expr) => {
            crate::types::indent(out, level);
            crate::expr::emit(out, expr, ctx);
            out.push_str(";\n");
        }
        Stmt::ExportDecl(inner) => {
            // In the root, exported lets/consts live in `main()` just like
            // unexported ones; functions / defaults were emitted as items
            // by `emit_top_item` in module.rs.
            match inner.as_ref() {
                s @ (Stmt::Let { .. } | Stmt::Const { .. }) => emit(out, s, level, ctx),
                _ => {}
            }
        }
        Stmt::Import { .. } | Stmt::FromImport { .. } | Stmt::ExportSpec(_) | Stmt::ExportDefault(_) => {
            // Module structure (mod/use/pub) was emitted outside `emit_stmt`.
        }
    }
}

// ── Helpers for each statement kind ───────────────────────────────────────

fn emit_let(
    out: &mut String,
    name: &str,
    ty: Option<&arcis_ast::Type>,
    value: &arcis_ast::Expr,
    level: usize,
    ctx: &Ctx,
) {
    // `let` in TS allows reassignment, so we only emit `mut` when this
    // variable is reassigned somewhere in the program.
    crate::types::indent(out, level);
    if ctx.reassigned.contains(name) {
        out.push_str("let mut ");
    } else {
        out.push_str("let ");
    }
    out.push_str(name);
    if let Some(t) = ty {
        out.push_str(": ");
        out.push_str(&crate::types::ts_type_to_rust(t, ctx.is_root));
    }
    out.push_str(" = ");
    // Special case: `let x: T[] = []` — `vec![]` does not infer T, so we
    // emit `Vec::new()` and let the declared type guide inference (Rust
    // fills in the generic parameter).
    if let (Some(t), arcis_ast::Expr::ArrayLiteral { elements }) = (ty, value) {
        if elements.is_empty() && t.is_array {
            out.push_str("Vec::new()");
            out.push_str(";\n");
            return;
        }
    }
    let nested = Ctx {
        reassigned: ctx.reassigned,
        types: ctx.types,
        current_let_type: ty,
        current_return_type: None,
        is_root: ctx.is_root,
    };
    crate::expr::emit(out, value, &nested);
    out.push_str(";\n");
}

fn emit_const(
    out: &mut String,
    name: &str,
    ty: Option<&arcis_ast::Type>,
    value: &arcis_ast::Expr,
    level: usize,
    ctx: &Ctx,
) {
    // Rust `const` requires a compile-time constant value, so it fails for
    // values computed at runtime. We use `let` with the name in UPPER-CASE
    // as a simple approximation (immutable).
    crate::types::indent(out, level);
    out.push_str("let ");
    out.push_str(name);
    if let Some(t) = ty {
        out.push_str(": ");
        out.push_str(&crate::types::ts_type_to_rust(t, ctx.is_root));
    }
    out.push_str(" = ");
    if let (Some(t), arcis_ast::Expr::ArrayLiteral { elements }) = (ty, value) {
        if elements.is_empty() && t.is_array {
            out.push_str("Vec::new()");
            out.push_str(";\n");
            return;
        }
    }
    let nested = Ctx {
        reassigned: ctx.reassigned,
        types: ctx.types,
        current_let_type: ty,
        current_return_type: None,
        is_root: ctx.is_root,
    };
    crate::expr::emit(out, value, &nested);
    out.push_str(";\n");
}

fn emit_assign(out: &mut String, name: &str, value: &arcis_ast::Expr, level: usize, ctx: &Ctx) {
    crate::types::indent(out, level);
    out.push_str(name);
    out.push_str(" = ");
    crate::expr::emit(out, value, ctx);
    out.push_str(";\n");
}

fn emit_assign_index(
    out: &mut String,
    object: &str,
    index: &arcis_ast::Expr,
    value: &arcis_ast::Expr,
    level: usize,
    ctx: &Ctx,
) {
    crate::types::indent(out, level);
    out.push_str(object);
    out.push('[');
    crate::expr::emit(out, index, ctx);
    out.push_str(" as usize] = ");
    crate::expr::emit(out, value, ctx);
    out.push_str(";\n");
}

fn emit_assign_member(
    out: &mut String,
    object: &arcis_ast::Expr,
    property: &str,
    value: &arcis_ast::Expr,
    level: usize,
    ctx: &Ctx,
) {
    crate::types::indent(out, level);
    crate::expr::emit(out, object, ctx);
    out.push('.');
    out.push_str(property);
    out.push_str(" = ");
    crate::expr::emit(out, value, ctx);
    out.push_str(";\n");
}

fn emit_return(
    out: &mut String,
    expr: Option<&arcis_ast::Expr>,
    level: usize,
    ctx: &Ctx,
) {
    crate::types::indent(out, level);
    out.push_str("return");
    if let Some(e) = expr {
        out.push(' ');
        // For `return { ... };` inside a function whose declared return
        // type is an inline object, propagate that type as the expected
        // shape so the ObjectLiteral can be resolved against it.
        let nested = Ctx {
            reassigned: ctx.reassigned,
            types: ctx.types,
            current_let_type: ctx.current_return_type,
            current_return_type: None,
            is_root: ctx.is_root,
        };
        crate::expr::emit(out, e, &nested);
    }
    out.push_str(";\n");
}

fn emit_if(
    out: &mut String,
    condition: &arcis_ast::Expr,
    then_branch: &[Stmt],
    else_branch: Option<&[Stmt]>,
    level: usize,
    ctx: &Ctx,
) {
    crate::types::indent(out, level);
    out.push_str("if ");
    crate::expr::emit(out, condition, ctx);
    out.push_str(" {\n");
    for s in then_branch {
        emit(out, s, level + 1, ctx);
    }
    crate::types::indent(out, level);
    out.push('}');
    if let Some(eb) = else_branch {
        out.push_str(" else {\n");
        for s in eb {
            emit(out, s, level + 1, ctx);
        }
        crate::types::indent(out, level);
        out.push('}');
    }
    out.push('\n');
}

fn emit_while(
    out: &mut String,
    condition: &arcis_ast::Expr,
    body: &[Stmt],
    level: usize,
    ctx: &Ctx,
) {
    crate::types::indent(out, level);
    out.push_str("while ");
    crate::expr::emit(out, condition, ctx);
    out.push_str(" {\n");
    for s in body {
        emit(out, s, level + 1, ctx);
    }
    crate::types::indent(out, level);
    out.push_str("}\n");
}

fn emit_for_of(
    out: &mut String,
    name: &str,
    _ty: Option<&arcis_ast::Type>,
    iterable: &arcis_ast::Expr,
    body: &[Stmt],
    level: usize,
    ctx: &Ctx,
) {
    // For an identifier (local variable of type `Vec`) we use
    // `.iter().cloned()` so we don't consume it. For calls, member
    // accesses or index accesses (typical of crate-produced iterators
    // like `server.incoming_requests()`) we use `.into_iter()` because
    // `Iterator` does not have `.iter()`.
    crate::types::indent(out, level);
    out.push_str("for ");
    out.push_str(name);
    out.push_str(" in ");
    let use_iter = matches!(iterable, arcis_ast::Expr::Ident(_));
    crate::expr::emit(out, iterable, ctx);
    if use_iter {
        out.push_str(".iter().cloned()");
    } else {
        out.push_str(".into_iter()");
    }
    out.push_str(" {\n");
    for s in body {
        emit(out, s, level + 1, ctx);
    }
    crate::types::indent(out, level);
    out.push_str("}\n");
}

fn emit_for(
    out: &mut String,
    init: Option<&Stmt>,
    condition: Option<&arcis_ast::Expr>,
    update: Option<&Stmt>,
    body: &[Stmt],
    level: usize,
    ctx: &Ctx,
) {
    // Desugared using a `loop` with a flag so that `continue` runs the
    // update before the next iteration:
    //   {
    //     init;
    //     let mut __for_first = true;
    //     loop {
    //         if !__for_first { update; }
    //         __for_first = false;
    //         if !(cond) { break; }
    //         body;
    //     }
    //   }
    crate::types::indent(out, level);
    out.push_str("{\n");
    if let Some(init) = init {
        emit(out, init, level + 1, ctx);
    } else {
        crate::types::indent(out, level + 1);
        out.push_str(";\n");
    }
    crate::types::indent(out, level + 1);
    out.push_str("let mut __for_first = true;\n");
    crate::types::indent(out, level + 1);
    out.push_str("loop {\n");
    crate::types::indent(out, level + 2);
    out.push_str("if !__for_first {\n");
    if let Some(upd) = update {
        emit(out, upd, level + 3, ctx);
    }
    crate::types::indent(out, level + 2);
    out.push_str("}\n");
    crate::types::indent(out, level + 2);
    out.push_str("__for_first = false;\n");
    crate::types::indent(out, level + 2);
    out.push_str("if !(");
    if let Some(cond) = condition {
        crate::expr::emit(out, cond, ctx);
    } else {
        out.push_str("true");
    }
    out.push_str(") { break; }\n");
    for s in body {
        emit(out, s, level + 2, ctx);
    }
    crate::types::indent(out, level + 1);
    out.push_str("}\n");
    crate::types::indent(out, level);
    out.push_str("}\n");
}