//! Validación semántica.
//!
//! Detecta dos clases de errores:
//! - Declaraciones (variables, parámetros) que nunca se usan
//! - Declaraciones duplicadas dentro del mismo scope
//!
//! Análisis **por scope**: main es un scope, cada función es otro. Una
//! declaración es `unused` si dentro de SU scope no hay ningún `Ident`,
//! `Assign` o `Call` que la refiera.
//!
//! Limitación actual: NO se trackean bloques `if`/`while` como scopes
//! separados. Esto significa:
//!   - Falso negativo: una variable de main puede ser "rescatada" por un
//!     identificador usado dentro de una función con el mismo nombre.
//!   - Falso positivo: una variable declarada dentro de un `if` puede
//!     marcarse como unused porque no se usa dentro del mismo `if`.
//!
//! Para el subset actual (sin shadowing, sin closures) los análisis de
//! unused y duplicados son correctos en casos prácticos.

use crate::ast::{Expr, Program, Stmt};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclKind {
    Variable,
    Parameter,
}

impl DeclKind {
    fn label(&self) -> &'static str {
        match self {
            DeclKind::Variable => "variable",
            DeclKind::Parameter => "parámetro",
        }
    }
}

#[derive(Debug, Clone)]
pub struct UnusedDecl {
    pub kind: DeclKind,
    pub name: String,
    pub line: usize,
    pub col: usize,
}

#[derive(Debug, Clone)]
pub struct DuplicateDecl {
    pub kind: DeclKind,
    pub name: String,
    pub first_line: usize,
    pub first_col: usize,
    pub second_line: usize,
    pub second_col: usize,
}

#[derive(Debug, Clone)]
pub enum ValidationIssue {
    Unused(UnusedDecl),
    Duplicate(DuplicateDecl),
    BreakOutsideLoop {
        keyword: &'static str,
        line: usize,
        col: usize,
    },
}

pub fn validate(program: &Program) -> Vec<ValidationIssue> {
    let mut issues = Vec::new();

    // Scope 1: main
    let mut main_decl: Vec<UnusedDecl> = Vec::new();
    let mut main_used: HashSet<String> = HashSet::new();
    let mut main_seen: HashMap<String, UnusedDecl> = HashMap::new();
    for stmt in &program.stmts {
        if matches!(stmt, Stmt::Function(_)) {
            continue;
        }
        collect_decl(stmt, &mut main_decl, &mut main_seen, &mut issues);
        collect_uses(stmt, &mut main_used);
        check_loop_context(stmt, 0, &mut issues);
    }
    push_unused(&main_decl, &main_used, &mut issues);

    // Scope N: cada función (parámetros + cuerpo)
    for stmt in &program.stmts {
        if let Stmt::Function(f) = stmt {
            let mut func_decl: Vec<UnusedDecl> = Vec::new();
            let mut func_used: HashSet<String> = HashSet::new();
            let mut func_seen: HashMap<String, UnusedDecl> = HashMap::new();
            for p in &f.params {
                let decl = UnusedDecl {
                    kind: DeclKind::Parameter,
                    name: p.name.clone(),
                    line: p.line,
                    col: p.col,
                };
                if let Some(prev) = func_seen.get(&p.name) {
                    issues.push(ValidationIssue::Duplicate(DuplicateDecl {
                        kind: DeclKind::Parameter,
                        name: p.name.clone(),
                        first_line: prev.line,
                        first_col: prev.col,
                        second_line: p.line,
                        second_col: p.col,
                    }));
                } else {
                    func_seen.insert(p.name.clone(), decl.clone());
                    func_decl.push(decl);
                }
            }
            for s in &f.body {
                collect_decl(s, &mut func_decl, &mut func_seen, &mut issues);
                collect_uses(s, &mut func_used);
                check_loop_context(s, 0, &mut issues);
            }
            push_unused(&func_decl, &func_used, &mut issues);
        }
    }

    issues
}

/// Verifica que `break` y `continue` solo aparezcan dentro de un loop.
/// `depth` cuenta los loops (while/for/for-of) en los que estamos anidados.
fn check_loop_context(stmt: &Stmt, depth: usize, issues: &mut Vec<ValidationIssue>) {
    let new_depth = match stmt {
        Stmt::While { .. } | Stmt::For { .. } | Stmt::ForOf { .. } => depth + 1,
        _ => depth,
    };
    match stmt {
        Stmt::Break => {
            if depth == 0 {
                issues.push(ValidationIssue::BreakOutsideLoop {
                    keyword: "break",
                    line: 0,
                    col: 0,
                });
            }
        }
        Stmt::Continue => {
            if depth == 0 {
                issues.push(ValidationIssue::BreakOutsideLoop {
                    keyword: "continue",
                    line: 0,
                    col: 0,
                });
            }
        }
        Stmt::While { body, .. } | Stmt::For { body, .. } | Stmt::ForOf { body, .. } => {
            for s in body {
                check_loop_context(s, new_depth, issues);
            }
        }
        Stmt::If { then_branch, else_branch, .. } => {
            for s in then_branch {
                check_loop_context(s, new_depth, issues);
            }
            if let Some(eb) = else_branch {
                for s in eb {
                    check_loop_context(s, new_depth, issues);
                }
            }
        }
        Stmt::Function(f) => {
            for s in &f.body {
                check_loop_context(s, 0, issues); // nueva función = nuevo scope de loop
            }
        }
        _ => {}
    }
}

fn push_unused(declared: &[UnusedDecl], used: &HashSet<String>, issues: &mut Vec<ValidationIssue>) {
    for d in declared {
        if !used.contains(&d.name) {
            issues.push(ValidationIssue::Unused(d.clone()));
        }
    }
}

fn collect_decl(
    stmt: &Stmt,
    out: &mut Vec<UnusedDecl>,
    seen: &mut HashMap<String, UnusedDecl>,
    issues: &mut Vec<ValidationIssue>,
) {
    match stmt {
        Stmt::Let { name, line, col, .. } | Stmt::Const { name, line, col, .. } => {
            let decl = UnusedDecl {
                kind: DeclKind::Variable,
                name: name.clone(),
                line: *line,
                col: *col,
            };
            if let Some(prev) = seen.get(name) {
                issues.push(ValidationIssue::Duplicate(DuplicateDecl {
                    kind: DeclKind::Variable,
                    name: name.clone(),
                    first_line: prev.line,
                    first_col: prev.col,
                    second_line: *line,
                    second_col: *col,
                }));
            } else {
                seen.insert(name.clone(), decl.clone());
                out.push(decl);
            }
        }
        Stmt::If { then_branch, else_branch, .. } => {
            // Bloques if/else comparten scope con main en este análisis simple.
            for s in then_branch {
                collect_decl(s, out, seen, issues);
            }
            if let Some(eb) = else_branch {
                for s in eb {
                    collect_decl(s, out, seen, issues);
                }
            }
        }
        Stmt::While { body, .. } | Stmt::For { body, .. } => {
            // El cuerpo del while/for comparte scope con main en este análisis simple.
            for s in body {
                collect_decl(s, out, seen, issues);
            }
        }
        // Las funciones se manejan por separado en validate().
        // Las asignaciones, returns, exprs no declaran nada.
        Stmt::Function(_)
        | Stmt::Assign { .. }
        | Stmt::AssignIndex { .. }
        | Stmt::AssignMember { .. }
        | Stmt::Return(_)
        | Stmt::Break
        | Stmt::Continue
        | Stmt::ForOf { .. }
        | Stmt::Import { .. }
        | Stmt::ExportDecl(_)
        | Stmt::ExportSpec(_)
        | Stmt::ExportDefault(_)
        | Stmt::Expr(_) => {}
    }
}

fn collect_uses(stmt: &Stmt, used: &mut HashSet<String>) {
    match stmt {
        Stmt::Let { value, .. } | Stmt::Const { value, .. } => {
            collect_uses_expr(value, used);
        }
        Stmt::Assign { name, value } => {
            used.insert(name.clone());
            collect_uses_expr(value, used);
        }
        Stmt::AssignIndex { object, index, value } => {
            used.insert(object.clone());
            collect_uses_expr(index, used);
            collect_uses_expr(value, used);
        }
        Stmt::AssignMember { object, value, .. } => {
            collect_uses_expr(object, used);
            collect_uses_expr(value, used);
        }
        Stmt::Function(f) => {
            for s in &f.body {
                collect_uses(s, used);
            }
        }
        Stmt::Return(Some(expr)) => collect_uses_expr(expr, used),
        Stmt::Return(None) => {}
        Stmt::If { condition, then_branch, else_branch } => {
            collect_uses_expr(condition, used);
            for s in then_branch {
                collect_uses(s, used);
            }
            if let Some(eb) = else_branch {
                for s in eb {
                    collect_uses(s, used);
                }
            }
        }
        Stmt::While { condition, body } => {
            collect_uses_expr(condition, used);
            for s in body {
                collect_uses(s, used);
            }
        }
        Stmt::For { init, condition, update, body } => {
            if let Some(init) = init {
                collect_uses(init, used);
            }
            if let Some(cond) = condition {
                collect_uses_expr(cond, used);
            }
            if let Some(upd) = update {
                collect_uses(upd, used);
            }
            for s in body {
                collect_uses(s, used);
            }
        }
        Stmt::ForOf { iterable, body, .. } => {
            collect_uses_expr(iterable, used);
            for s in body {
                collect_uses(s, used);
            }
        }
        Stmt::Break | Stmt::Continue => {}
        Stmt::Import { .. } | Stmt::ExportDecl(_) | Stmt::ExportSpec(_) | Stmt::ExportDefault(_) => {}
        Stmt::Expr(expr) => collect_uses_expr(expr, used),
    }
}

fn collect_uses_expr(expr: &Expr, used: &mut HashSet<String>) {
    match expr {
        Expr::Ident(name) => {
            used.insert(name.clone());
        }
        Expr::Call { callee, args } => {
            collect_uses_expr(callee, used);
            for a in args {
                collect_uses_expr(a, used);
            }
        }
        Expr::Binary { left, right, .. } => {
            collect_uses_expr(left, used);
            collect_uses_expr(right, used);
        }
        Expr::Unary { operand, .. } => {
            collect_uses_expr(operand, used);
        }
        Expr::Member { object, .. } => {
            collect_uses_expr(object, used);
        }
        Expr::Index { object, index } => {
            collect_uses_expr(object, used);
            collect_uses_expr(index, used);
        }
        Expr::ArrayLiteral { elements } => {
            for e in elements {
                collect_uses_expr(e, used);
            }
        }
        Expr::ObjectLiteral { fields } => {
            for (_, v) in fields {
                collect_uses_expr(v, used);
            }
        }
        Expr::Path { segments } => {
            // Marca los segmentos como usados: permite que Arcis detecte
            // imports no usados cuando el path sólo referencia nombres ya
            // importados vía `crate:`.
            for s in segments {
                used.insert(s.clone());
            }
        }
        Expr::Number(_) | Expr::String(_) | Expr::Bool(_) => {}
    }
}

/// Formatea los issues para mostrarlos al usuario.
pub fn format_issues(issues: &[ValidationIssue], source_path: &str) -> String {
    let mut out = String::new();
    for issue in issues {
        match issue {
            ValidationIssue::Unused(u) => {
                out.push_str(&format!(
                    "error: {} `{}` declarado pero no usado\n --> {}:{}:{}\n",
                    u.kind.label(),
                    u.name,
                    source_path,
                    u.line,
                    u.col,
                ));
            }
            ValidationIssue::Duplicate(d) => {
                out.push_str(&format!(
                    "error: {} `{}` ya fue declarado en {}:{}:{}\n --> {}:{}:{}\n",
                    d.kind.label(),
                    d.name,
                    source_path,
                    d.first_line,
                    d.first_col,
                    source_path,
                    d.second_line,
                    d.second_col,
                ));
            }
            ValidationIssue::BreakOutsideLoop { keyword, .. } => {
                out.push_str(&format!(
                    "error: `{}` usado fuera de un loop (while/for)\n",
                    keyword,
                ));
            }
        }
    }
    if !issues.is_empty() {
        let unused = issues.iter().filter(|i| matches!(i, ValidationIssue::Unused(_))).count();
        let dup = issues.iter().filter(|i| matches!(i, ValidationIssue::Duplicate(_))).count();
        let outside = issues.iter().filter(|i| matches!(i, ValidationIssue::BreakOutsideLoop { .. })).count();
        out.push('\n');
        if unused > 0 {
            out.push_str(&format!("{} declaración(es) sin uso.\n", unused));
        }
        if dup > 0 {
            out.push_str(&format!("{} declaración(es) duplicada(s).\n", dup));
        }
        if outside > 0 {
            out.push_str(&format!("{} uso(s) de break/continue fuera de loop.\n", outside));
        }
    }
    out
}