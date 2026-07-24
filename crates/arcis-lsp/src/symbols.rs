//! Shared symbol table.
//!
//! A single AST walk that harvests every named definition in a
//! document — variables, constants, functions (+ params), loop
//! variables, imports, type aliases, interfaces, and enums (+
//! variants). [`completion`](crate::completion), [`hover`](crate::hover),
//! and [`definition`](crate::definition) all build on this one walker
//! so the three providers can't drift out of sync with each other or
//! with the AST as the language grows.
//!
//! Arcis has no block scoping the LSP needs to model yet, so every
//! symbol — no matter how deeply nested in `if`/`while`/`for`/`switch`/
//! `try`/arrow-function bodies — is flattened into one file-scoped
//! list. Good enough for "what can I complete / jump to here", not a
//! real scope resolver.

use std::collections::HashMap;

use arcis_ast::{
    ArrayElement, ArrowBody, ExportDefault, Expr, Function, ObjectField, Program, Stmt, Type,
};

use crate::lsp::CompletionItemKind;

/// What a [`Symbol`] represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolKind {
    Variable,
    Constant,
    Function,
    Parameter,
    LoopVar,
    Module,
    TypeAlias,
    Interface,
    Enum,
}

impl SymbolKind {
    /// Map to the closest-matching LSP completion kind.
    pub fn completion_kind(self) -> CompletionItemKind {
        match self {
            SymbolKind::Variable | SymbolKind::Parameter | SymbolKind::LoopVar => {
                CompletionItemKind::VARIABLE
            }
            SymbolKind::Constant => CompletionItemKind::CONSTANT,
            SymbolKind::Function => CompletionItemKind::FUNCTION,
            SymbolKind::Module => CompletionItemKind::MODULE,
            SymbolKind::TypeAlias => CompletionItemKind::CLASS,
            SymbolKind::Interface => CompletionItemKind::INTERFACE,
            SymbolKind::Enum => CompletionItemKind::ENUM,
        }
    }

    /// Friendly label for the hover panel header.
    pub fn label(self) -> &'static str {
        match self {
            SymbolKind::Variable => "variable",
            SymbolKind::Constant => "constant",
            SymbolKind::Function => "function",
            SymbolKind::Parameter => "parameter",
            SymbolKind::LoopVar => "loop variable",
            SymbolKind::Module => "module",
            SymbolKind::TypeAlias => "type alias",
            SymbolKind::Interface => "interface",
            SymbolKind::Enum => "enum",
        }
    }
}

/// One named definition harvested from the AST.
#[derive(Debug, Clone)]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    /// 0-indexed source line/col of the name. `(0, 0)` for symbols the
    /// parser doesn't track a precise position for yet (e.g. `for-of`
    /// loop variables, `catch` bindings) — still enough to land in the
    /// right file.
    pub line: usize,
    pub col: usize,
    /// Short signature/type string shown in completion detail / hover.
    pub detail: String,
    /// `Some((module_path, export_name))` when this name comes from an
    /// import. `export_name` is `None` for namespace imports
    /// (`import foo`), `Some(name)` for `from foo import name`.
    pub import: Option<(Vec<String>, Option<String>)>,
    /// For `SymbolKind::Enum` only: `(variant_name, resolved_value)` in
    /// source order. Empty for every other kind.
    pub enum_variants: Vec<(String, i64)>,
    /// `true` for `export function/const/let/type/interface/enum ...`
    /// and `export default ...` declarations. Used by cross-file
    /// import resolution to tell "this name is visible to other
    /// modules" apart from a same-named local that isn't exported.
    /// `export { a, b as c };` re-exports are marked here too (their
    /// `name` is already the exported alias), but resolving *what they
    /// point to* still requires walking `Stmt::ExportSpec` directly to
    /// recover the original local name.
    pub exported: bool,
    /// The binding's declared or inferred type — `None` for kinds where a
    /// type isn't meaningful (functions, modules, type-level
    /// declarations). Lets `completion`'s member-access dispatch
    /// (`ident.<TAB>`) offer the RIGHT members (object fields, array
    /// methods, string methods, or nothing for a `number`/`boolean`)
    /// instead of always guessing array/string methods. A bare
    /// `Type::Named(name)` (an unresolved interface/alias reference) is
    /// looked up in [`collect_named_shapes`] to reach the actual fields.
    pub ty: Option<Type>,
}

/// Parse `program` and collect every symbol declared anywhere in it.
pub fn collect_symbols(program: &Program) -> Vec<Symbol> {
    let mut out = Vec::new();
    for stmt in &program.stmts {
        collect_stmt(stmt, &mut out);
    }
    out
}

/// Resolve every top-level `interface`/`type` declaration in `program` to
/// its field list, keyed by name — so member completion on `let p: Person`
/// can look up `Person`'s actual fields even though the variable's own
/// `Symbol::ty` is just the unresolved `Type::Named("Person")`.
///
/// Single-document only (no cross-module linking, matching the rest of
/// this LSP): `interface Base` referenced via `extends` or `type X = Y`
/// only resolves if `Base`/`Y` is declared in the SAME file. This mirrors,
/// at a much smaller scope, what `arcis_codegen::collect_interfaces` /
/// `resolve_type_aliases` do for the compiler.
pub fn collect_named_shapes(program: &Program) -> HashMap<String, Vec<(String, Type, bool)>> {
    // Raw pass: interface name -> (extends bases, own fields); alias name
    // -> aliased type (only kept if it's itself Named/Object, chains
    // resolved below).
    let mut interfaces: HashMap<String, (Vec<String>, Vec<(String, Type, bool)>)> = HashMap::new();
    let mut aliases: HashMap<String, Type> = HashMap::new();
    for stmt in &program.stmts {
        let inner = match stmt {
            Stmt::ExportDecl(inner) => inner.as_ref(),
            other => other,
        };
        match inner {
            Stmt::Interface { name, extends, fields, .. } => {
                let owned: Vec<(String, Type, bool)> = fields
                    .iter()
                    .map(|(n, t, opt)| (n.clone(), (**t).clone(), *opt))
                    .collect();
                interfaces.insert(name.clone(), (extends.clone(), owned));
            }
            Stmt::TypeAlias { name, ty, .. } => {
                aliases.insert(name.clone(), ty.clone());
            }
            _ => {}
        }
    }

    let mut resolved: HashMap<String, Vec<(String, Type, bool)>> = HashMap::new();
    let names: Vec<String> = interfaces.keys().cloned().collect();
    for name in names {
        let mut visiting = std::collections::HashSet::new();
        resolve_interface(&name, &interfaces, &mut resolved, &mut visiting);
    }
    // `type X = { ... }` (inline object alias) or `type X = Y` (alias to
    // another named shape, resolved transitively).
    for (name, ty) in &aliases {
        if resolved.contains_key(name) {
            continue;
        }
        let mut visiting = std::collections::HashSet::new();
        if let Some(fields) = resolve_alias_shape(ty, &interfaces, &aliases, &mut resolved, &mut visiting) {
            resolved.insert(name.clone(), fields);
        }
    }
    resolved
}

fn resolve_interface(
    name: &str,
    raw: &HashMap<String, (Vec<String>, Vec<(String, Type, bool)>)>,
    resolved: &mut HashMap<String, Vec<(String, Type, bool)>>,
    visiting: &mut std::collections::HashSet<String>,
) -> Vec<(String, Type, bool)> {
    if let Some(fields) = resolved.get(name) {
        return fields.clone();
    }
    let Some((extends, own)) = raw.get(name) else {
        return Vec::new();
    };
    if !visiting.insert(name.to_string()) {
        return own.clone(); // extends cycle: stop recursing
    }
    let mut merged = Vec::new();
    for base in extends {
        for f in resolve_interface(base, raw, resolved, visiting) {
            merged.retain(|(fname, _, _)| fname != &f.0);
            merged.push(f);
        }
    }
    for f in own {
        merged.retain(|(fname, _, _)| fname != &f.0);
        merged.push(f.clone());
    }
    visiting.remove(name);
    resolved.insert(name.to_string(), merged.clone());
    merged
}

fn resolve_alias_shape(
    ty: &Type,
    interfaces: &HashMap<String, (Vec<String>, Vec<(String, Type, bool)>)>,
    aliases: &HashMap<String, Type>,
    resolved: &mut HashMap<String, Vec<(String, Type, bool)>>,
    visiting: &mut std::collections::HashSet<String>,
) -> Option<Vec<(String, Type, bool)>> {
    match ty {
        Type::Object { fields, .. } => Some(
            fields
                .iter()
                .map(|(n, t, opt)| (n.clone(), (**t).clone(), *opt))
                .collect(),
        ),
        Type::Named(n) => {
            if let Some(fields) = resolved.get(n) {
                return Some(fields.clone());
            }
            if interfaces.contains_key(n) {
                return Some(resolve_interface(n, interfaces, resolved, visiting));
            }
            let target = aliases.get(n)?;
            if !visiting.insert(n.clone()) {
                return None; // alias cycle
            }
            let fields = resolve_alias_shape(target, interfaces, aliases, resolved, visiting)?;
            visiting.remove(n);
            Some(fields)
        }
        _ => None,
    }
}

/// Lex + parse + run type inference over `text`. The returned program has
/// missing annotations (let/const types, for-of element types, function
/// return types) filled in wherever they can be deduced, so symbol details
/// show `let x: number` instead of `let x: unknown` for unannotated bindings.
/// `None` when the document doesn't currently lex/parse.
pub fn parse_and_infer(text: &str) -> Option<Program> {
    let tokens = arcis_lexer::lex(text).ok()?;
    let mut program = arcis_parser::parse(tokens).ok()?;
    infer(&mut program);
    Some(program)
}

/// Like [`parse_and_infer`], but tolerant of a document that is mid-edit:
/// when the parse fails, the offending line is blanked out and the parse
/// retried (up to three times). Completion/hover keep working on the rest
/// of the file while one line is momentarily invalid.
pub fn parse_lenient(text: &str) -> Option<Program> {
    let mut owned = text.to_string();
    for _ in 0..4 {
        let tokens = arcis_lexer::lex(&owned).ok()?;
        match arcis_parser::parse(tokens) {
            Ok(mut program) => {
                infer(&mut program);
                return Some(program);
            }
            Err(e) => {
                let mut lines: Vec<&str> = owned.lines().collect();
                let idx = e.line.saturating_sub(1);
                if idx >= lines.len() || lines[idx].is_empty() {
                    return None;
                }
                lines[idx] = "";
                owned = lines.join("\n");
            }
        }
    }
    None
}

fn infer(program: &mut Program) {
    let mut env = arcis_validation::TypeEnv::default();
    env.add_program(program);
    arcis_validation::infer_program(program, &env);
}

fn push(out: &mut Vec<Symbol>, name: String, kind: SymbolKind, line: usize, col: usize, detail: String) {
    push_ty(out, name, kind, line, col, detail, None);
}

/// Like [`push`] but also records the binding's type (for member-access
/// completion — see [`Symbol::ty`]).
fn push_ty(
    out: &mut Vec<Symbol>,
    name: String,
    kind: SymbolKind,
    line: usize,
    col: usize,
    detail: String,
    ty: Option<Type>,
) {
    push_full(out, name, kind, line, col, detail, false, ty);
}

fn push_ex(
    out: &mut Vec<Symbol>,
    name: String,
    kind: SymbolKind,
    line: usize,
    col: usize,
    detail: String,
    exported: bool,
) {
    push_full(out, name, kind, line, col, detail, exported, None);
}

#[allow(clippy::too_many_arguments)]
fn push_full(
    out: &mut Vec<Symbol>,
    name: String,
    kind: SymbolKind,
    line: usize,
    col: usize,
    detail: String,
    exported: bool,
    ty: Option<Type>,
) {
    out.push(Symbol {
        name,
        kind,
        line,
        col,
        detail,
        import: None,
        enum_variants: Vec::new(),
        exported,
        ty,
    });
}

fn collect_stmt(stmt: &Stmt, out: &mut Vec<Symbol>) {
    match stmt {
        // ── let / const ────────────────────────────────────────────
        Stmt::Let { name, ty, value, line, col } => {
            push_ty(out, name.clone(), SymbolKind::Variable, *line, *col, describe_binding(ty), ty.clone());
            collect_expr(value, out);
        }
        Stmt::Const { name, ty, value, line, col } => {
            push_ty(out, name.clone(), SymbolKind::Constant, *line, *col, describe_binding(ty), ty.clone());
            collect_expr(value, out);
        }

        // ── function ──────────────────────────────────────────────
        Stmt::Function(f) => collect_function(f, out, None),

        // ── for-of ────────────────────────────────────────────────
        Stmt::ForOf { name, ty, iterable, body } => {
            let detail = ty.as_ref().map(type_label).unwrap_or_else(|| "unknown".to_string());
            push_ty(out, name.clone(), SymbolKind::LoopVar, 0, 0, detail, ty.clone());
            collect_expr(iterable, out);
            for s in body {
                collect_stmt(s, out);
            }
        }

        // ── imports ───────────────────────────────────────────────
        Stmt::Import { module, alias } => {
            let local = alias.clone().unwrap_or_else(|| module.last().cloned().unwrap_or_default());
            out.push(Symbol {
                name: local,
                kind: SymbolKind::Module,
                line: 0,
                col: 0,
                detail: format!("module \"{}\"", module.join(".")),
                import: Some((module.clone(), None)),
                enum_variants: Vec::new(),
                exported: false,
                ty: None,
            });
        }
        Stmt::FromImport { module, names, wildcard: false } => {
            for n in names {
                let local = n.alias.clone().unwrap_or_else(|| n.name.clone());
                out.push(Symbol {
                    name: local,
                    kind: SymbolKind::Variable,
                    line: 0,
                    col: 0,
                    detail: format!("from \"{}\"", module.join(".")),
                    import: Some((module.clone(), Some(n.name.clone()))),
                    enum_variants: Vec::new(),
                    exported: false,
                    ty: None,
                });
            }
        }
        Stmt::FromImport { wildcard: true, .. } => {}

        // ── export decl (inline) ──────────────────────────────────
        Stmt::ExportDecl(inner) => collect_export_decl(inner, out),

        // ── export spec / export default ──────────────────────────
        Stmt::ExportSpec(items) => {
            for item in items {
                let name = item.alias.clone().unwrap_or_else(|| item.name.clone());
                push_ex(out, name, SymbolKind::Variable, 0, 0, "re-export".to_string(), true);
            }
        }
        Stmt::ExportDefault(ExportDefault::Function(f)) => {
            collect_function(f, out, Some("default export"));
        }
        Stmt::ExportDefault(ExportDefault::Expr(e)) => {
            push_ex(out, "default".to_string(), SymbolKind::Variable, 0, 0, "default export".to_string(), true);
            collect_expr(e, out);
        }

        // ── compound statements: recurse (+ nested expressions) ───
        Stmt::Assign { value, .. } => collect_expr(value, out),
        Stmt::AssignIndex { index, value, .. } => {
            collect_expr(index, out);
            collect_expr(value, out);
        }
        Stmt::AssignMember { object, value, .. } => {
            collect_expr(object, out);
            collect_expr(value, out);
        }
        Stmt::Return(Some(e)) => collect_expr(e, out),
        Stmt::Return(None) => {}
        Stmt::If { condition, then_branch, else_branch } => {
            collect_expr(condition, out);
            for s in then_branch {
                collect_stmt(s, out);
            }
            if let Some(els) = else_branch {
                for s in els {
                    collect_stmt(s, out);
                }
            }
        }
        Stmt::While { condition, body } => {
            collect_expr(condition, out);
            for s in body {
                collect_stmt(s, out);
            }
        }
        Stmt::For { init, condition, update, body } => {
            if let Some(i) = init {
                collect_stmt(i, out);
            }
            if let Some(c) = condition {
                collect_expr(c, out);
            }
            if let Some(u) = update {
                collect_stmt(u, out);
            }
            for s in body {
                collect_stmt(s, out);
            }
        }
        Stmt::Switch { discriminant, cases } => {
            collect_expr(discriminant, out);
            for case in cases {
                for v in &case.values {
                    collect_expr(v, out);
                }
                for s in &case.body {
                    collect_stmt(s, out);
                }
            }
        }
        Stmt::Try { body, catch_name, catch_body } => {
            for s in body {
                collect_stmt(s, out);
            }
            if let Some(n) = catch_name {
                push(out, n.clone(), SymbolKind::Variable, 0, 0, "caught error".to_string());
            }
            for s in catch_body {
                collect_stmt(s, out);
            }
        }
        Stmt::Throw(e) => collect_expr(e, out),
        Stmt::Expr(e) => collect_expr(e, out),
        Stmt::Break | Stmt::Continue => {}

        // ── type-level declarations ────────────────────────────────
        Stmt::TypeAlias { name, ty, line, col } => {
            push(out, name.clone(), SymbolKind::TypeAlias, *line, *col, type_label(ty));
        }
        Stmt::Interface { name, extends, fields, line, col } => {
            push(out, name.clone(), SymbolKind::Interface, *line, *col, describe_interface(extends, fields));
        }
        Stmt::Enum { name, variants, line, col } => {
            let resolved = resolve_enum_variants(variants);
            let detail = format!(
                "{{ {} }}",
                resolved.iter().map(|(n, v)| format!("{n} = {v}")).collect::<Vec<_>>().join(", ")
            );
            out.push(Symbol {
                name: name.clone(),
                kind: SymbolKind::Enum,
                line: *line,
                col: *col,
                detail,
                import: None,
                enum_variants: resolved,
                exported: false,
                ty: None,
            });
        }
    }
}

/// Collect a function's own symbol, its parameters, and everything
/// declared in its body. `export_tag`, if set, is appended to the
/// function's detail string (e.g. `"(default export)"`) and the
/// symbol is marked [`Symbol::exported`].
fn collect_function(f: &Function, out: &mut Vec<Symbol>, export_tag: Option<&str>) {
    let name = if f.name.is_empty() { "default".to_string() } else { f.name.clone() };
    let detail = match export_tag {
        Some(tag) => format!("{} ({tag})", func_sig(f)),
        None => func_sig(f),
    };
    push_ex(out, name, SymbolKind::Function, f.line, f.col, detail, export_tag.is_some());
    for p in &f.params {
        push_ty(out, p.name.clone(), SymbolKind::Parameter, p.line, p.col, type_label(&p.ty), Some(p.ty.clone()));
    }
    for s in &f.body {
        collect_stmt(s, out);
    }
}

/// `export function/const/let/type/interface/enum ...` — the inline
/// declaration is a `Box<Stmt>` wrapping one of those kinds; tag its
/// symbol as exported.
fn collect_export_decl(inner: &Stmt, out: &mut Vec<Symbol>) {
    match inner {
        Stmt::Let { name, ty, value, line, col } => {
            push_full(out, name.clone(), SymbolKind::Variable, *line, *col, format!("{} (exported)", describe_binding(ty)), true, ty.clone());
            collect_expr(value, out);
        }
        Stmt::Const { name, ty, value, line, col } => {
            push_full(out, name.clone(), SymbolKind::Constant, *line, *col, format!("{} (exported)", describe_binding(ty)), true, ty.clone());
            collect_expr(value, out);
        }
        Stmt::Function(f) => collect_function(f, out, Some("exported")),
        Stmt::TypeAlias { name, ty, line, col } => {
            push_ex(out, name.clone(), SymbolKind::TypeAlias, *line, *col, format!("{} (exported)", type_label(ty)), true);
        }
        Stmt::Interface { name, extends, fields, line, col } => {
            push_ex(out, name.clone(), SymbolKind::Interface, *line, *col, format!("{} (exported)", describe_interface(extends, fields)), true);
        }
        Stmt::Enum { name, variants, line, col } => {
            let resolved = resolve_enum_variants(variants);
            let detail = format!(
                "{{ {} }} (exported)",
                resolved.iter().map(|(n, v)| format!("{n} = {v}")).collect::<Vec<_>>().join(", ")
            );
            out.push(Symbol {
                name: name.clone(),
                kind: SymbolKind::Enum,
                line: *line,
                col: *col,
                detail,
                import: None,
                enum_variants: resolved,
                exported: true,
                ty: None,
            });
        }
        _ => {}
    }
}

/// Recurse into an expression looking for nested statement lists —
/// today that's only arrow-function bodies (`Expr::Arrow`), but every
/// expression variant is walked so an arrow buried in a call argument,
/// array literal, or object field value is still found.
fn collect_expr(expr: &Expr, out: &mut Vec<Symbol>) {
    match expr {
        Expr::Arrow { params, return_type: _, body } => {
            for p in params {
                push_ty(out, p.name.clone(), SymbolKind::Parameter, p.line, p.col, type_label(&p.ty), Some(p.ty.clone()));
            }
            match body {
                ArrowBody::Expr(e) => collect_expr(e, out),
                ArrowBody::Block(stmts) => {
                    for s in stmts {
                        collect_stmt(s, out);
                    }
                }
            }
        }
        Expr::Call { callee, args } => {
            collect_expr(callee, out);
            for a in args {
                collect_expr(a, out);
            }
        }
        Expr::Unary { operand, .. } => collect_expr(operand, out),
        Expr::Binary { left, right, .. } => {
            collect_expr(left, out);
            collect_expr(right, out);
        }
        Expr::Member { object, .. } => collect_expr(object, out),
        Expr::Index { object, index } => {
            collect_expr(object, out);
            collect_expr(index, out);
        }
        Expr::ArrayLiteral { elements } => {
            for el in elements {
                match el {
                    ArrayElement::Item(e) | ArrayElement::Spread(e) => collect_expr(e, out),
                }
            }
        }
        Expr::ObjectLiteral { fields } => {
            for f in fields {
                match f {
                    ObjectField::KV(_, e) => collect_expr(e, out),
                    ObjectField::Spread(e) => collect_expr(e, out),
                }
            }
        }
        Expr::TypeOf(e) | Expr::AsConst(e) | Expr::NonNullAssertion(e) => collect_expr(e, out),
        Expr::AsAssertion { expr, .. } => collect_expr(expr, out),
        Expr::Number(_)
        | Expr::String(_)
        | Expr::Bool(_)
        | Expr::Ident(_)
        | Expr::Path { .. }
        | Expr::Null
        | Expr::Undefined => {}
    }
}

// ── Descriptive-string helpers (shared by completion / hover) ──────────

/// `"<type>"` if annotated, `"unknown"` otherwise (matches the wording used
/// for un-annotated `let`/`const` bindings).
fn describe_binding(ty: &Option<Type>) -> String {
    ty.as_ref().map(type_label).unwrap_or_else(|| "unknown".to_string())
}

fn describe_interface(extends: &[String], fields: &[(String, Box<Type>, bool)]) -> String {
    let ext = if extends.is_empty() {
        String::new()
    } else {
        format!(" extends {}", extends.join(", "))
    };
    let fs: Vec<String> = fields
        .iter()
        // The `?` is already folded into `t` (`Type::Optional`), so
        // `type_label` alone renders `n: T?` — no separate marker needed.
        .map(|(n, t, _opt)| format!("{n}: {}", type_label(t)))
        .collect();
    format!("interface{ext} {{ {} }}", fs.join(", "))
}

/// Resolve implicit enum variant values: unset means "previous value +
/// 1" (or `0` for the first variant) — matching TypeScript numeric enum
/// rules (see [`arcis_ast::Stmt::Enum`]'s doc comment).
fn resolve_enum_variants(variants: &[(String, Option<i64>)]) -> Vec<(String, i64)> {
    let mut out = Vec::with_capacity(variants.len());
    let mut next = 0i64;
    for (name, explicit) in variants {
        let val = explicit.unwrap_or(next);
        out.push((name.clone(), val));
        next = val + 1;
    }
    out
}

/// Human-readable label for a type annotation.
pub fn type_label(ty: &Type) -> String {
    use arcis_ast::LiteralValue;
    match ty {
        Type::Optional(inner) => format!("{}?", type_label(inner)),
        Type::Array(inner) => format!("{}[]", type_label(inner)),
        Type::Object { fields, .. } => {
            let fields: Vec<String> = fields
                .iter()
                .map(|(n, t, opt)| {
                    if *opt {
                        format!("{n}?: {}", type_label(t))
                    } else {
                        format!("{n}: {}", type_label(t))
                    }
                })
                .collect();
            format!("{{ {} }}", fields.join(", "))
        }
        Type::Primitive(name) | Type::Named(name) => name.clone(),
        Type::Null => "null".to_string(),
        Type::Undefined => "undefined".to_string(),
        Type::Union(members) => members.iter().map(type_label).collect::<Vec<_>>().join(" | "),
        Type::Intersection(members) => members.iter().map(type_label).collect::<Vec<_>>().join(" & "),
        Type::Literal(LiteralValue::String(s)) => format!("\"{s}\""),
        Type::Literal(LiteralValue::Number(n)) => format!("{n}"),
        Type::Literal(LiteralValue::Bool(b)) => format!("{b}"),
        Type::Function { params, return_type } => {
            let ps: Vec<String> = params.iter().map(type_label).collect();
            format!("({}) => {}", ps.join(", "), type_label(return_type))
        }
    }
}

/// Build a short function signature string like `(a: number, b: number) -> number`.
pub fn func_sig(f: &Function) -> String {
    let params: Vec<String> = f
        .params
        .iter()
        .map(|p| format!("{}: {}", p.name, type_label(&p.ty)))
        .collect();
    format!("({}) -> {}", params.join(", "), type_label(&f.return_type))
}
