//! `json(...)` call validation.
//!
//! [`crate::infer`]'s `call_type` treats a `json(...)` call best-effort —
//! silently `None` on any I/O/parse failure, so the LSP stays resilient on
//! every keystroke of a file with a currently-bad path (see that module's
//! doc comment). This pass is the authoritative counterpart run by the
//! compiler driver: it re-validates every `json(...)` call and turns a
//! literal path that can't be read/parsed, or a non-literal path with no
//! explicit type argument, into a real compile error — never a panic or a
//! silently-broken binary.

use arcis_ast::{ArrayElement, Expr, ExportDefault, ObjectField, Program, Stmt};

/// One `json(...)` call validation failure.
#[derive(Debug, Clone)]
pub struct JsonCallIssue {
    pub message: String,
    /// Best-effort position — see [`crate::nullsafety::NullSafetyIssue`]'s
    /// doc comment for why this is often `(0, 0)` (most `Expr` nodes carry
    /// no span in this AST).
    pub line: usize,
    pub col: usize,
}

fn issue(message: String, line: usize, col: usize) -> JsonCallIssue {
    JsonCallIssue { message, line, col }
}

/// Validate every `json(...)` call in `program`.
pub fn check_json_calls(program: &Program) -> Vec<JsonCallIssue> {
    let mut out = Vec::new();
    for stmt in &program.stmts {
        check_stmt(stmt, &mut out);
    }
    out
}

fn check_stmt(stmt: &Stmt, out: &mut Vec<JsonCallIssue>) {
    match stmt {
        Stmt::Let { value, line, col, .. } | Stmt::Const { value, line, col, .. } => {
            check_expr(value, *line, *col, out)
        }
        Stmt::Assign { value, line, col, .. } => check_expr(value, *line, *col, out),
        Stmt::AssignIndex { index, value, line, col, .. } => {
            check_expr(index, *line, *col, out);
            check_expr(value, *line, *col, out);
        }
        Stmt::AssignMember { object, value, line, col, .. } => {
            check_expr(object, *line, *col, out);
            check_expr(value, *line, *col, out);
        }
        Stmt::Function(f) => {
            for s in &f.body {
                check_stmt(s, out);
            }
        }
        Stmt::Return(Some(e)) | Stmt::Throw(e) | Stmt::Expr(e) => check_expr(e, 0, 0, out),
        Stmt::Return(None) | Stmt::Break | Stmt::Continue => {}
        Stmt::If { condition, then_branch, else_branch } => {
            check_expr(condition, 0, 0, out);
            for s in then_branch {
                check_stmt(s, out);
            }
            if let Some(eb) = else_branch {
                for s in eb {
                    check_stmt(s, out);
                }
            }
        }
        Stmt::While { condition, body } => {
            check_expr(condition, 0, 0, out);
            for s in body {
                check_stmt(s, out);
            }
        }
        Stmt::For { init, condition, update, body } => {
            if let Some(i) = init {
                check_stmt(i, out);
            }
            if let Some(c) = condition {
                check_expr(c, 0, 0, out);
            }
            if let Some(u) = update {
                check_stmt(u, out);
            }
            for s in body {
                check_stmt(s, out);
            }
        }
        Stmt::ForOf { iterable, body, .. } => {
            check_expr(iterable, 0, 0, out);
            for s in body {
                check_stmt(s, out);
            }
        }
        Stmt::Switch { discriminant, cases } => {
            check_expr(discriminant, 0, 0, out);
            for case in cases {
                for v in &case.values {
                    check_expr(v, 0, 0, out);
                }
                for s in &case.body {
                    check_stmt(s, out);
                }
            }
        }
        Stmt::Try { body, catch_body, .. } => {
            for s in body {
                check_stmt(s, out);
            }
            for s in catch_body {
                check_stmt(s, out);
            }
        }
        Stmt::Import { .. }
        | Stmt::FromImport { .. }
        | Stmt::ExportSpec(_)
        | Stmt::TypeAlias { .. }
        | Stmt::Interface { .. }
        | Stmt::Enum { .. } => {}
        Stmt::ExportDecl(inner) => check_stmt(inner, out),
        Stmt::ExportDefault(ExportDefault::Function(f)) => {
            for s in &f.body {
                check_stmt(s, out);
            }
        }
        Stmt::ExportDefault(ExportDefault::Expr(e)) => check_expr(e, 0, 0, out),
    }
}

fn check_expr(e: &Expr, line: usize, col: usize, out: &mut Vec<JsonCallIssue>) {
    match e {
        Expr::Call { callee, args, type_args } => {
            if matches!(callee.as_ref(), Expr::Ident(n) if n == "json") {
                check_json_call(args, type_args, line, col, out);
            }
            check_expr(callee, line, col, out);
            for a in args {
                check_expr(a, line, col, out);
            }
        }
        Expr::Unary { operand, .. }
        | Expr::TypeOf(operand)
        | Expr::NonNullAssertion(operand)
        | Expr::AsConst(operand) => check_expr(operand, line, col, out),
        Expr::Binary { left, right, .. } => {
            check_expr(left, line, col, out);
            check_expr(right, line, col, out);
        }
        Expr::Member { object, .. } => check_expr(object, line, col, out),
        Expr::Index { object, index } => {
            check_expr(object, line, col, out);
            check_expr(index, line, col, out);
        }
        Expr::ArrayLiteral { elements } => {
            for el in elements {
                match el {
                    ArrayElement::Item(e) | ArrayElement::Spread(e) => check_expr(e, line, col, out),
                }
            }
        }
        Expr::ObjectLiteral { fields } => {
            for f in fields {
                match f {
                    ObjectField::KV(_, e) | ObjectField::Spread(e) => check_expr(e, line, col, out),
                }
            }
        }
        Expr::AsAssertion { expr, .. } => check_expr(expr, line, col, out),
        Expr::Arrow { body, .. } => match body {
            arcis_ast::ArrowBody::Expr(e) => check_expr(e, line, col, out),
            arcis_ast::ArrowBody::Block(stmts) => {
                for s in stmts {
                    check_stmt(s, out);
                }
            }
        },
        Expr::Number(_)
        | Expr::String(_)
        | Expr::Bool(_)
        | Expr::Ident(_)
        | Expr::Path { .. }
        | Expr::Null
        | Expr::Undefined => {}
    }
}

fn check_json_call(
    args: &[Expr],
    type_args: &[arcis_ast::Type],
    line: usize,
    col: usize,
    out: &mut Vec<JsonCallIssue>,
) {
    match args.first() {
        Some(Expr::String(path)) => {
            if let Err(e) = crate::json::infer_json_type(path) {
                out.push(issue(format!("json(\"{path}\"): {e}"), line, col));
            }
        }
        Some(_) if type_args.len() == 1 => {
            // Non-literal path with an explicit type arg — the type isn't
            // re-validated here (that's `check_types`'s job once the alias
            // is resolved); this pass only guarantees a type IS resolvable.
        }
        Some(_) => out.push(issue(
            "json(path): the path isn't a string literal, so its shape can't be inferred at compile time — use `json<T>(path)` with an explicit type naming a declared interface.".to_string(),
            line,
            col,
        )),
        None => out.push(issue("json(): expected a path argument".to_string(), line, col)),
    }
}
