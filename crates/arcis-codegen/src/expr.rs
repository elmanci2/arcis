//! Expression emission.
//!
//! Dispatches on [`Expr`] variants. The `Call` arm is the most complex — it
//! dispatches to:
//!
//! - [`crate::builtin`] for `print` and `input`.
//! - [`crate::sys`] for `sys.*`.
//! - [`crate::method`] for array/string method chains.
//!
//! Everything else (literals, member access, index, binary, unary, paths,
//! array / object literals) is emitted inline.

use arcis_ast::{ArrayElement, ArrowBody, BinOp, Expr, ObjectField, Param, Type, UnaryOp};

use crate::context::Ctx;

/// Emit one expression.
pub(crate) fn emit(out: &mut String, expr: &Expr, ctx: &Ctx) {
    match expr {
        Expr::Number(n) => emit_number(out, *n),
        Expr::String(s) => out.push_str(&crate::types::rust_string_literal(s)),
        Expr::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Expr::Ident(name) => out.push_str(name),

        Expr::Call { callee, args, type_args } => emit_call(out, expr, callee, args, type_args, ctx),

        Expr::Unary { op, operand } => emit_unary(out, *op, operand, ctx),
        Expr::Member { object, property } => emit_member(out, object, property, ctx),
        Expr::Index { object, index } => emit_index(out, object, index, ctx),
        Expr::ArrayLiteral { elements } => emit_array_literal(out, elements, ctx),
        Expr::ObjectLiteral { fields } => emit_object_literal(out, fields, ctx),
        Expr::Path { segments } => out.push_str(&segments.join("::")),
        Expr::Binary { op, left, right } => emit_binary(out, *op, left, right, ctx),
        Expr::TypeOf(operand) => {
            let type_name = infer_type_name(operand, ctx);
            out.push_str(&crate::types::rust_string_literal(type_name));
        }
        // The null-safety checker only lets `null` / `undefined` flow into
        // optional (`Option<T>`) positions.
        Expr::Null | Expr::Undefined => out.push_str("None"),
        // Type assertions (`as Type`, `as const`) and the non-null assertion
        // (`!`) are compile-time-only in TypeScript — they have no runtime
        // effect. Arcis mirrors that: emit just the inner expression.
        Expr::AsAssertion { expr, .. } => emit(out, expr, ctx),
        Expr::AsConst(inner) => emit(out, inner, ctx),
        // `x!` — asserts `x` is present. For a genuinely `Option<T>`-typed
        // expression this must be a REAL `.unwrap()` (the whole point of a
        // sentinel-free `Option<T>` representation is that there is no
        // other way to get a `T` back out); for anything else (already
        // non-optional) it stays a pass-through, matching `as`/`as
        // const`'s compile-time-only semantics. `arcis_validation::expr_type`
        // (not just a by-name lookup) so this also covers `f()!` /
        // `obj.field!`, not only a bare optional identifier.
        Expr::NonNullAssertion(inner) => {
            let unwraps = arcis_validation::expr_type(inner, ctx.type_scope, ctx.env)
                .map(|t| t.is_optional())
                .unwrap_or(false);
            if unwraps {
                out.push('(');
                emit(out, inner, ctx);
                out.push_str(").clone().unwrap()");
            } else {
                emit(out, inner, ctx);
            }
        }
        Expr::Arrow { params, return_type, body } => {
            emit_arrow(out, params, return_type.as_ref(), body, ctx)
        }
    }
}

/// Emit a non-capturing Rust closure for an Arcis arrow function:
/// `|x: f64| -> f64 { x * 2.0 }`. When the source omits the return type,
/// the Rust annotation is omitted too (letting `rustc` infer it) rather
/// than defaulting to `void`/`()`, which would be wrong for the common
/// `(x: number) => x * 2` callback form.
fn emit_arrow(out: &mut String, params: &[Param], return_type: Option<&Type>, body: &ArrowBody, ctx: &Ctx) {
    out.push('|');
    for (i, p) in params.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(&p.name);
        out.push_str(": ");
        out.push_str(&crate::types::ts_type_to_rust(&p.ty, ctx.is_root));
    }
    out.push('|');
    if let Some(rt) = return_type {
        out.push_str(" -> ");
        out.push_str(&crate::types::ts_type_to_rust(rt, ctx.is_root));
    }
    match body {
        ArrowBody::Expr(e) => {
            out.push(' ');
            if return_type.is_some() {
                // An explicit `-> T` on a Rust closure requires a block body.
                out.push_str("{ ");
                emit(out, e, ctx);
                out.push_str(" }");
            } else {
                emit(out, e, ctx);
            }
        }
        ArrowBody::Block(stmts) => {
            out.push_str(" {\n");
            for s in stmts {
                crate::stmt::emit(out, s, 1, ctx);
            }
            out.push('}');
        }
    }
}

/// Infer a human-readable type name for an expression, used by `typeof`.
fn infer_type_name(expr: &Expr, ctx: &Ctx) -> &'static str {
    match expr {
        Expr::Number(_) => "number",
        Expr::String(_) => "string",
        Expr::Bool(_) => "boolean",
        Expr::Ident(name) => match ctx.types.get(name).map(|s| s.as_str()) {
            Some("f64") | Some("number") => "number",
            Some("String") | Some("string") => "string",
            Some("bool") | Some("boolean") => "boolean",
            Some("Vec<f64>") | Some("Vec<String>") => "array",
            Some("()") => "void",
            _ => "object",
        },
        Expr::ArrayLiteral { .. } => "array",
        Expr::ObjectLiteral { .. } => "object",
        Expr::Call { .. } => "string", // default for calls
        _ => "object",
    }
}

// ── Atomic forms ──────────────────────────────────────────────────────────

fn emit_number(out: &mut String, n: f64) {
    // Always emit with a decimal point so Rust treats it as f64.
    if n.fract() == 0.0 {
        out.push_str(&format!("{}.0", n as i64));
    } else {
        out.push_str(&format!("{}", n));
    }
}

fn emit_unary(out: &mut String, op: UnaryOp, operand: &Expr, ctx: &Ctx) {
    match op {
        UnaryOp::Not => {
            out.push('!');
            emit(out, operand, ctx);
        }
        UnaryOp::Neg => {
            out.push('-');
            emit(out, operand, ctx);
        }
    }
}

fn emit_member(out: &mut String, object: &Expr, property: &str, ctx: &Ctx) {
    // Builtin `sys.X` member accesses (e.g. `sys.args`).
    if let Expr::Ident(module) = object {
        if module == "sys" {
            crate::sys::emit_member(out, property);
            return;
        }
        // Enum variant access: `Color.Red` -> `Color::Red`. Distinguished
        // from a field access by `module` naming a known `enum`, never a
        // variable (Arcis identifiers are lowerCamelCase by convention but
        // this isn't enforced, so we go by the declared-enum-names set
        // rather than casing).
        if ctx.enum_names.contains(module) {
            out.push_str(module);
            out.push_str("::");
            out.push_str(property);
            return;
        }
        // Namespace-import member access: `utils.item` -> `utils::item`.
        if ctx.namespace_names.contains(module) {
            out.push_str(module);
            out.push_str("::");
            out.push_str(property);
            return;
        }
    }
    // `.length` is special-cased based on the type:
    //   string → `.chars().count() as f64` (Unicode codepoints)
    //   T[]    → `.len() as f64` (element count)
    //   other  → `.len() as f64` (default; rustc reports otherwise)
    //
    // We use the type env when the object is an identifier with a declared
    // type.
    emit(out, object, ctx);
    if property == "length" {
        let is_string = matches!(object,
            Expr::Ident(name) if ctx.types.get(name).map(|t| t == "string").unwrap_or(false));
        let is_array = matches!(object,
            Expr::Ident(name) if ctx.types.get(name).map(|t| t.ends_with("[]")).unwrap_or(false));
        if is_string {
            out.push_str(".chars().count() as f64");
        } else {
            // array or other type: `.len() as f64`
            out.push_str(".len() as f64");
        }
        let _ = is_array; // currently we only branch on `is_string`
    } else {
        out.push('.');
        out.push_str(property);
    }
}

fn emit_index(out: &mut String, object: &Expr, index: &Expr, ctx: &Ctx) {
    emit(out, object, ctx);
    out.push('[');
    emit(out, index, ctx);
    out.push_str(" as usize]");
}

fn emit_array_literal(out: &mut String, elements: &[ArrayElement], ctx: &Ctx) {
    // A literal array of objects (`[{ a: 1 }, { a: 2 }]`) needs
    // `current_let_type` narrowed to the ELEMENT type before recursing into
    // each item — same reasoning as `emit_object_literal`'s per-field
    // narrowing just above: without it, a nested object-literal element
    // would still see the outer `T[]` type and emit garbage.
    let elem_ty = ctx.current_let_type.and_then(|t| t.array_inner());
    let nested;
    let ctx = match elem_ty {
        Some(t) => {
            nested = Ctx { current_let_type: Some(t), ..*ctx };
            &nested
        }
        None => ctx,
    };
    let has_spread = elements.iter().any(|e| matches!(e, ArrayElement::Spread(_)));
    if elements.is_empty() {
        // Without a declared type, default to `Vec<f64>`. For other types,
        // declare the type on the let/const (handled in `Stmt::Let` /
        // `Stmt::Const`).
        out.push_str("Vec::<f64>::new()");
    } else if !has_spread {
        out.push_str("vec![");
        for (i, e) in elements.iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            let ArrayElement::Item(e) = e else { unreachable!("checked above") };
            emit(out, e, ctx);
        }
        out.push(']');
    } else {
        // `vec![]` can't splice in a runtime `Vec`, so build one instead:
        // `{ let mut __v = Vec::new(); __v.push(x); __v.extend(base.iter().cloned()); __v }`.
        out.push_str("{ let mut __arcis_spread = Vec::new(); ");
        for e in elements {
            match e {
                ArrayElement::Item(e) => {
                    out.push_str("__arcis_spread.push(");
                    emit(out, e, ctx);
                    out.push_str("); ");
                }
                ArrayElement::Spread(e) => {
                    out.push_str("__arcis_spread.extend(");
                    emit(out, e, ctx);
                    out.push_str(".iter().cloned()); ");
                }
            }
        }
        out.push_str("__arcis_spread }");
    }
}

fn emit_object_literal(out: &mut String, fields: &[ObjectField], ctx: &Ctx) {
    // Requires that the context (let/const) has declared the type, because
    // we emit `StructName { field: value, ... }` directly. When the
    // declared type is `T[]`, unwrap one level of `Array` first — each
    // element literal is shaped like `T`, not `T[]`.
    if let Some(outer_ty) = ctx.current_let_type {
        let ty = outer_ty.array_inner().unwrap_or(outer_ty);
        // A generic interface/alias usage (`Box<number>`) is never inlined
        // back to `Type::Object` (see `Ctx::struct_fields`'s doc comment) —
        // fall back to the field-shape TEMPLATE looked up by struct name.
        // The concrete field types (`T` -> `number`) are left for `rustc`'s
        // own inference to fill in from `ty`, same as a non-generic struct
        // literal already relies on context for its field types.
        let generic_fields = match ty {
            arcis_ast::Type::Generic { name, .. } => ctx.struct_fields.get(name),
            _ => None,
        };
        if let Some(obj_fields) = ty.object_fields().or_else(|| generic_fields.map(|v| v.as_slice())) {
            out.push_str(ty.struct_name().unwrap_or(""));
            out.push_str(" { ");
            let mut first = true;
            // Rust struct-update syntax (`..base`) only accepts one trailing
            // expression; the last `...spread` wins if the source has more
            // than one (a rare case we don't try to resolve field-by-field).
            let mut last_spread: Option<&Expr> = None;
            for f in fields {
                match f {
                    ObjectField::KV(k, v) => {
                        if !first {
                            out.push_str(", ");
                        }
                        first = false;
                        let field_ty = obj_fields.iter().find(|(fname, _, _)| fname == k);
                        let optional = field_ty.map(|(_, _, opt)| *opt).unwrap_or(false);
                        out.push_str(k);
                        out.push_str(": ");
                        // A nested object/array literal (`{ a: { b: 1 } }`)
                        // needs `current_let_type` narrowed to THIS field's
                        // own declared type — otherwise the recursive
                        // `emit` call for `v` would still see the OUTER
                        // struct's type and emit the wrong struct name /
                        // bogus missing-field `todo!()`s for the inner
                        // literal (same "context drives struct-literal
                        // emission" mechanism `emit_let`/`emit_const` use
                        // for the top-level case, just needed here too for
                        // nesting).
                        let nested;
                        let field_ctx = match field_ty.map(|(_, t, _)| t.as_ref()) {
                            Some(t) => {
                                nested = Ctx { current_let_type: Some(t), ..*ctx };
                                &nested
                            }
                            None => ctx,
                        };
                        if optional {
                            out.push_str("Some(");
                            emit(out, v, field_ctx);
                            out.push(')');
                        } else {
                            emit(out, v, field_ctx);
                        }
                    }
                    ObjectField::Spread(e) => last_spread = Some(e),
                }
            }
            if let Some(base) = last_spread {
                if !first {
                    out.push_str(", ");
                }
                out.push_str("..");
                emit(out, base, ctx);
            } else {
                // No spread: every declared field not given explicitly must
                // still appear (optional -> `None`, required -> `todo!()`)
                // — a partial struct literal wouldn't compile.
                for (fname, _, optional) in obj_fields {
                    if fields.iter().any(|f| matches!(f, ObjectField::KV(k, _) if k == fname)) {
                        continue;
                    }
                    if !first {
                        out.push_str(", ");
                    }
                    first = false;
                    out.push_str(fname);
                    out.push_str(": ");
                    out.push_str(if *optional { "None" } else { "todo!()" });
                }
            }
            out.push_str(" }");
            return;
        }
    }
    // Without context: emit a tuple as fallback (rustc will complain).
    out.push_str("todo!()");
}

// ── Binary operators ──────────────────────────────────────────────────────

fn emit_binary(out: &mut String, op: BinOp, left: &Expr, right: &Expr, ctx: &Ctx) {
    // Null tests: `x == null` → `.is_none()`, `x != null` → `.is_some()`.
    let null_side = |e: &Expr| matches!(e, Expr::Null | Expr::Undefined);
    if matches!(op, BinOp::EqEq | BinOp::NotEq) && (null_side(left) ^ null_side(right)) {
        let value = if null_side(left) { right } else { left };
        out.push('(');
        emit(out, value, ctx);
        out.push_str(if matches!(op, BinOp::EqEq) { ").is_none()" } else { ").is_some()" });
        return;
    }
    match op {
        BinOp::NullishCoalesce => {
            // `a ?? b` — the checker guarantees `b` is a solid fallback.
            out.push('(');
            emit(out, left, ctx);
            out.push_str(").clone().unwrap_or_else(|| ");
            emit(out, right, ctx);
            out.push(')');
        }
        BinOp::Add => {
            // Heuristic: if either operand is a string literal, emit `format!`.
            if crate::builtin::has_string_literal(left) || crate::builtin::has_string_literal(right)
            {
                out.push_str("format!(\"{}{}\", ");
                emit(out, left, ctx);
                out.push_str(", ");
                emit(out, right, ctx);
                out.push(')');
            } else {
                out.push('(');
                emit(out, left, ctx);
                out.push_str(" + ");
                emit(out, right, ctx);
                out.push(')');
            }
        }
        BinOp::Sub => wrap_binary(out, " - ", left, right, ctx),
        BinOp::Mul => wrap_binary(out, " * ", left, right, ctx),
        BinOp::Div => wrap_binary(out, " / ", left, right, ctx),
        BinOp::Mod => wrap_binary(out, " % ", left, right, ctx),
        BinOp::EqEq => wrap_binary(out, " == ", left, right, ctx),
        BinOp::NotEq => wrap_binary(out, " != ", left, right, ctx),
        BinOp::Lt => wrap_binary(out, " < ", left, right, ctx),
        BinOp::Gt => wrap_binary(out, " > ", left, right, ctx),
        BinOp::LtEq => wrap_binary(out, " <= ", left, right, ctx),
        BinOp::GtEq => wrap_binary(out, " >= ", left, right, ctx),
        BinOp::And => wrap_binary(out, " && ", left, right, ctx),
        BinOp::Or => wrap_binary(out, " || ", left, right, ctx),
    }
}

fn wrap_binary(out: &mut String, op: &str, left: &Expr, right: &Expr, ctx: &Ctx) {
    out.push('(');
    emit(out, left, ctx);
    out.push_str(op);
    emit(out, right, ctx);
    out.push(')');
}

// ── Calls: dispatch to builtin / sys / method / generic ──────────────────

fn emit_call(out: &mut String, call_expr: &Expr, callee: &Expr, args: &[Expr], type_args: &[arcis_ast::Type], ctx: &Ctx) {
    // print / input / json builtins.
    if let Expr::Ident(name) = callee {
        if name == "print" {
            crate::builtin::emit_print(out, args, ctx);
            return;
        }
        if name == "input" {
            crate::builtin::emit_input(out);
            return;
        }
        if name == "json" {
            crate::builtin::emit_json(out, call_expr, args, type_args, ctx);
            return;
        }
    }
    // sys.<ns>.<method>(args) sub-namespace call.
    if let Expr::Member { object, property } = callee {
        if let Expr::Member { object: ns_obj, property: ns } = object.as_ref() {
            if let Expr::Ident(module) = ns_obj.as_ref() {
                if module == "sys" {
                    crate::sys::emit_subns_call(out, ns, property, args, ctx);
                    return;
                }
            }
        }
    }
    // sys.* builtins.
    if let Expr::Member { object, property } = callee {
        if let Expr::Ident(module) = object.as_ref() {
            if module == "sys" {
                crate::sys::emit_call(out, property, args, ctx);
                return;
            }
        }
    }
    // Array / string method dispatch.
    if let Expr::Member { object, property } = callee {
        if crate::method::emit(out, object, property, args, ctx) {
            return;
        }
    }
    // Named function call: f(args), or f::<T>(args) with explicit turbofish
    // type args. Omitted (the common, inferred case) emits exactly as
    // before and lets `rustc`'s own inference fill in the generic params.
    if let Expr::Ident(name) = callee {
        out.push_str(name);
        if !type_args.is_empty() {
            out.push_str("::<");
            for (i, t) in type_args.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                out.push_str(&crate::types::ts_type_to_rust(t, ctx.is_root));
            }
            out.push('>');
        }
        out.push('(');
        for (i, a) in args.iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            emit_arg(out, a, ctx);
        }
        out.push(')');
        return;
    }
    // Generic call: (callee)(args).
    emit(out, callee, ctx);
    out.push('(');
    for (i, a) in args.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        emit_arg(out, a, ctx);
    }
    out.push(')');
}

/// Emit one call argument. TS passes variables to functions without giving
/// them up, so a *place* expression (`x`, `obj.field`, `arr[i]`) must not be
/// moved out of — Rust would reject any later use of the source
/// (use-after-move / E0507). Arrays are auto-borrowed (`&arr`, matching the
/// `&Vec<T>` parameter convention); other places are cloned unless their
/// declared type is `Copy` (`number` / `boolean`). Non-place expressions
/// (literals, call results, arithmetic) are emitted as-is.
fn emit_arg(out: &mut String, a: &Expr, ctx: &Ctx) {
    match a {
        Expr::Ident(n) => {
            let declared = ctx.types.get(n).map(|s| s.as_str());
            if declared.map(|t| t.ends_with("[]")).unwrap_or(false) {
                out.push('&');
                emit(out, a, ctx);
            } else if matches!(declared, Some("number") | Some("boolean")) {
                emit(out, a, ctx);
            } else {
                emit(out, a, ctx);
                out.push_str(".clone()");
            }
        }
        // `.length` lowers to `.len() as f64` (a value, and `as` binds
        // looser than a trailing method call) — never clone it.
        Expr::Member { property, .. } if property == "length" => emit(out, a, ctx),
        Expr::Member { .. } | Expr::Index { .. } => {
            out.push('(');
            emit(out, a, ctx);
            out.push_str(").clone()");
        }
        _ => emit(out, a, ctx),
    }
}