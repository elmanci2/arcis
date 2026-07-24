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
        Stmt::Assign { name, value, .. } => emit_assign(out, name, value, level, ctx),
        Stmt::AssignIndex { object, index, value, .. } => {
            emit_assign_index(out, object, index, value, level, ctx)
        }
        Stmt::AssignMember { object, property, value, .. } => {
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
        Stmt::Switch { discriminant, cases } => emit_switch(out, discriminant, cases, level, ctx),
        Stmt::Try { body, catch_name, catch_body } => {
            emit_try(out, body, catch_name.as_deref(), catch_body, level, ctx)
        }
        Stmt::Throw(expr) => emit_throw(out, expr, level, ctx),
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
        Stmt::TypeAlias { .. } | Stmt::Interface { .. } | Stmt::Enum { .. } => {
            // Type-only / enum declarations were emitted as top-level items
            // by `module.rs`; nothing to do at statement position.
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
        if !crate::types::is_any(t) {
            out.push_str(": ");
            out.push_str(&crate::types::ts_type_to_rust(t, ctx.is_root));
        }
    }
    out.push_str(" = ");
    // Special case: `let x: T[] = []` — `vec![]` does not infer T, so we
    // emit `Vec::new()` and let the declared type guide inference (Rust
    // fills in the generic parameter).
    if let (Some(t), arcis_ast::Expr::ArrayLiteral { elements }) = (ty, value) {
        if elements.is_empty() && t.is_array() {
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
        enum_names: ctx.enum_names,
        namespace_names: ctx.namespace_names,
        env: ctx.env,
        type_scope: ctx.type_scope,
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
        if !crate::types::is_any(t) {
            out.push_str(": ");
            out.push_str(&crate::types::ts_type_to_rust(t, ctx.is_root));
        }
    }
    out.push_str(" = ");
    if let (Some(t), arcis_ast::Expr::ArrayLiteral { elements }) = (ty, value) {
        if elements.is_empty() && t.is_array() {
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
        enum_names: ctx.enum_names,
        namespace_names: ctx.namespace_names,
        env: ctx.env,
        type_scope: ctx.type_scope,
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
            enum_names: ctx.enum_names,
            namespace_names: ctx.namespace_names,
                env: ctx.env,
            type_scope: ctx.type_scope,
        };
        // `return` a genuinely definite value where the function's return
        // type is `T?` (`Option<T>` in Rust) — wrap it. A value that's
        // ALREADY optional (an `Option<T>`-typed ident, another call
        // returning `T?`, …) is passed straight through unchanged.
        let target_is_optional = ctx.current_return_type.map(|t| t.is_optional()).unwrap_or(false);
        // `return null;` already emits a bare `None` (see `Expr::Null`'s
        // own codegen) — must NOT also get `Some(...)`-wrapped.
        let is_null_lit = matches!(e, arcis_ast::Expr::Null | arcis_ast::Expr::Undefined);
        let value_is_optional = is_null_lit
            || arcis_validation::expr_type(e, ctx.type_scope, ctx.env)
                .map(|t| t.is_optional())
                .unwrap_or(false);
        if target_is_optional && !value_is_optional {
            out.push_str("Some(");
            crate::expr::emit(out, e, &nested);
            out.push(')');
        } else {
            crate::expr::emit(out, e, &nested);
        }
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

/// `switch (d) { case v1: A case v2: B default: C }` -> an `if`/`else if`
/// chain comparing `d` with `==` against each case's values, ending in a
/// bare `else` for `default`. Deliberately NOT a Rust `match`: Arcis
/// `number` is `f64`, and Rust match patterns reject float literals — an
/// `==`-based chain sidesteps that entirely and works uniformly for
/// numbers, strings, booleans, and enum variants.
fn emit_switch(
    out: &mut String,
    discriminant: &arcis_ast::Expr,
    cases: &[arcis_ast::SwitchCase],
    level: usize,
    ctx: &Ctx,
) {
    crate::types::indent(out, level);
    out.push_str("{\n");
    crate::types::indent(out, level + 1);
    out.push_str("let __arcis_switch = ");
    crate::expr::emit(out, discriminant, ctx);
    out.push_str(";\n");

    let mut first = true;
    let mut default_case: Option<&arcis_ast::SwitchCase> = None;
    for case in cases {
        if case.is_default {
            default_case = Some(case);
            continue;
        }
        crate::types::indent(out, level + 1);
        out.push_str(if first { "if " } else { "else if " });
        first = false;
        for (i, v) in case.values.iter().enumerate() {
            if i > 0 {
                out.push_str(" || ");
            }
            out.push_str("__arcis_switch == ");
            crate::expr::emit(out, v, ctx);
        }
        out.push_str(" {\n");
        emit_switch_case_body(out, &case.body, level + 2, ctx);
        crate::types::indent(out, level + 1);
        out.push_str("}\n");
    }
    if let Some(case) = default_case {
        crate::types::indent(out, level + 1);
        out.push_str(if first { "{\n" } else { "else {\n" });
        emit_switch_case_body(out, &case.body, level + 2, ctx);
        crate::types::indent(out, level + 1);
        out.push_str("}\n");
    }
    crate::types::indent(out, level);
    out.push_str("}\n");
}

/// `throw expr;` -> `panic!("{}", expr);`. Every Arcis primitive
/// (`string`/`number`/`boolean`) implements Rust's `Display`, so this works
/// regardless of the thrown value's type — no need for the `+`-operator's
/// string-literal heuristic.
fn emit_throw(out: &mut String, expr: &arcis_ast::Expr, level: usize, ctx: &Ctx) {
    crate::types::indent(out, level);
    out.push_str("panic!(\"{}\", ");
    crate::expr::emit(out, expr, ctx);
    out.push_str(");\n");
}

/// `try { body } catch (e) { catch_body }` -> `std::panic::catch_unwind`.
///
/// Caveats (documented in `docs/language-reference.md`, not hidden):
/// - Uses `AssertUnwindSafe`, which sidesteps Rust's `UnwindSafe` check —
///   genuinely unsound if `body` mutates state observed after the catch.
///   Low-risk for Arcis's typical `String`/`Vec`/primitive locals.
/// - `body` is wrapped in a closure, so `return`/`break`/`continue` inside
///   a `try` block do not propagate to the enclosing function/loop the way
///   they would in TypeScript — `rustc` will reject `break`/`continue`
///   there outright, and a `return` would (silently, incorrectly) only
///   return from the closure.
fn emit_try(
    out: &mut String,
    body: &[Stmt],
    catch_name: Option<&str>,
    catch_body: &[Stmt],
    level: usize,
    ctx: &Ctx,
) {
    crate::types::indent(out, level);
    out.push_str("match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {\n");
    for s in body {
        emit(out, s, level + 1, ctx);
    }
    crate::types::indent(out, level);
    out.push_str("})) {\n");
    crate::types::indent(out, level + 1);
    out.push_str("Ok(_) => {}\n");
    crate::types::indent(out, level + 1);
    out.push_str("Err(__arcis_panic) => {\n");
    if let Some(name) = catch_name {
        crate::types::indent(out, level + 2);
        out.push_str(&format!(
            "let {name} = __arcis_panic.downcast_ref::<String>().cloned().or_else(|| __arcis_panic.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_else(|| String::from(\"unknown error\"));\n"
        ));
    }
    for s in catch_body {
        emit(out, s, level + 2, ctx);
    }
    crate::types::indent(out, level + 1);
    out.push_str("}\n");
    crate::types::indent(out, level);
    out.push_str("}\n");
}

/// Emit a switch case's body, treating a top-level `break;` as a no-op:
/// each case already ends its own `if`/`else` arm (no Rust loop or labeled
/// block wraps it), so a literal `break;` there would be invalid Rust
/// ("cannot break outside of a loop"). A `break` inside a loop *nested*
/// within the case is unaffected — that loop emits its own body via `emit`,
/// not this function.
fn emit_switch_case_body(out: &mut String, body: &[Stmt], level: usize, ctx: &Ctx) {
    for s in body {
        if matches!(s, Stmt::Break) {
            continue;
        }
        emit(out, s, level, ctx);
    }
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