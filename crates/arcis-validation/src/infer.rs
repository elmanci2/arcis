//! Type inference.
//!
//! Fills in missing type annotations so the user doesn't have to write
//! them everywhere (TS-style inference):
//!
//! - `let x = 5;`               → `let x: number = 5;`
//! - `let o = { a: 1 };`        → `let o: { a: number } = ...` (hash-named
//!   inline object type, deduped with annotation-derived shapes)
//! - `for (let x of arr)`       → element type of `arr`
//! - `function f(n: number) { return n * 2; }` → return type `number`
//!
//! Explicit annotations always win — inference only fills `None` slots (and
//! the parser's `void` default on functions whose bodies clearly return a
//! value). The pass is *best-effort*: anything it can't deduce is simply
//! left as-is for the backend (or `rustc`) to report.
//!
//! Run it AFTER alias/interface resolution so `Type::Named` is already
//! concrete, and after shadowing renaming so a flat per-function scope map
//! is sound.
//!
//! Cross-module inference: build one [`TypeEnv`] over every linked module's
//! program (via [`TypeEnv::add_program`]), then call [`infer_program`] per
//! module. Function return types resolve on demand through the env, with a
//! cycle guard (recursive functions without an annotation stay `void`).

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use arcis_ast::{
    object_type_name, ArrayElement, ArrowBody, Expr, ExportDefault, Function, ObjectField,
    Program, Stmt, Type,
};

/// Global (cross-module) typing facts: function signatures and top-level
/// constant types. Interfaces/aliases are expected to be resolved away
/// before inference (see module docs), so no alias map lives here.
#[derive(Default)]
pub struct TypeEnv {
    /// function name → full declaration. The body is kept so an
    /// unannotated (`void`-defaulted) return type can be inferred on
    /// demand.
    functions: HashMap<String, Function>,
    /// Top-level `let`/`const` (any module) → declared or inferred type.
    consts: HashMap<String, Type>,
    /// Enum names — `Enum.Variant` is a `number`, an enum-typed value too.
    enums: HashSet<String>,
    /// Memoised resolved return types (interior mutability so expression
    /// typing can resolve function returns lazily during a shared walk).
    resolved_returns: RefCell<HashMap<String, Type>>,
    /// Functions currently being resolved (cycle guard).
    resolving: RefCell<HashSet<String>>,
}

impl TypeEnv {
    /// Record every top-level function / const / enum of `program`.
    pub fn add_program(&mut self, program: &Program) {
        for stmt in &program.stmts {
            let inner = match stmt {
                Stmt::ExportDecl(inner) => inner.as_ref(),
                other => other,
            };
            match inner {
                Stmt::Function(f) => self.add_function(f),
                Stmt::Enum { name, .. } => {
                    self.enums.insert(name.clone());
                }
                Stmt::Let { name, ty, value, .. } | Stmt::Const { name, ty, value, .. } => {
                    if let Some(t) = ty {
                        self.consts.insert(name.clone(), t.clone());
                    } else if let Some(t) = literal_type(value) {
                        // Only literal initializers — full expression typing
                        // needs a scope, which top level doesn't have yet.
                        self.consts.insert(name.clone(), t);
                    }
                }
                Stmt::ExportDefault(ExportDefault::Function(f)) => self.add_function(f),
                _ => {}
            }
        }
    }

    fn add_function(&mut self, f: &Function) {
        self.functions.insert(f.name.clone(), f.clone());
    }

    /// The (possibly lazily inferred) return type of `name`.
    fn return_type_of(&self, name: &str) -> Option<Type> {
        let f = self.functions.get(name)?;
        if !is_void(&f.return_type) {
            return Some(f.return_type.clone());
        }
        if let Some(t) = self.resolved_returns.borrow().get(name) {
            return Some(t.clone());
        }
        if !self.resolving.borrow_mut().insert(name.to_string()) {
            // Recursive without an annotation: give up (stays void).
            return Some(Type::void());
        }
        let mut scope: Scope = HashMap::new();
        for p in &f.params {
            scope.insert(p.name.clone(), p.ty.clone());
        }
        let t = infer_body_return(&f.body, &mut scope, self).unwrap_or_else(Type::void);
        self.resolving.borrow_mut().remove(name);
        self.resolved_returns
            .borrow_mut()
            .insert(name.to_string(), t.clone());
        Some(t)
    }

    /// The declared parameter types of a known top-level function, in
    /// order. Used by the null-safety checker to validate call arguments
    /// against their parameters. `None` for an unknown name (an import
    /// whose target wasn't linked in, a builtin, …) — callers should treat
    /// that permissively (no check), not as an error.
    pub fn function_param_types(&self, name: &str) -> Option<Vec<Type>> {
        Some(self.functions.get(name)?.params.iter().map(|p| p.ty.clone()).collect())
    }

    /// The (possibly lazily inferred) return type of a known top-level
    /// function. Public wrapper around the private on-demand resolver, for
    /// the null-safety checker's `return` sink.
    pub fn resolved_return_type(&self, name: &str) -> Option<Type> {
        self.return_type_of(name)
    }
}

/// Flat name → type scope. Sound because shadowing was alpha-renamed away.
type Scope = HashMap<String, Type>;

fn is_void(t: &Type) -> bool {
    matches!(t, Type::Primitive(p) if p == "void")
}

/// The type of a literal expression (no scope needed).
fn literal_type(e: &Expr) -> Option<Type> {
    match e {
        Expr::Number(_) => Some(Type::number()),
        Expr::String(_) => Some(Type::string()),
        Expr::Bool(_) => Some(Type::boolean()),
        _ => None,
    }
}

/// Infer every missing annotation in `program`, in place.
pub fn infer_program(program: &mut Program, env: &TypeEnv) {
    // Two passes over top-level statements: function return types first
    // (so top-level `let x = f();` sees them), then the statement walk.
    for stmt in &mut program.stmts {
        infer_function_returns(stmt, env);
    }
    let mut scope: Scope = env.consts.clone();
    for stmt in &mut program.stmts {
        infer_stmt(stmt, &mut scope, env);
    }
}

fn infer_function_returns(stmt: &mut Stmt, env: &TypeEnv) {
    let f = match stmt {
        Stmt::Function(f) => f,
        Stmt::ExportDecl(inner) => match inner.as_mut() {
            Stmt::Function(f) => f,
            _ => return,
        },
        Stmt::ExportDefault(ExportDefault::Function(f)) => f,
        _ => return,
    };
    if is_void(&f.return_type) {
        let mut scope: Scope = env.consts.clone();
        for p in &f.params {
            scope.insert(p.name.clone(), p.ty.clone());
        }
        if let Some(t) = infer_body_return(&f.body, &mut scope, env) {
            f.return_type = t;
        }
    }
}

/// Walk `body` in order (so `let` bindings are in scope for later returns)
/// and return the type of the first `return <expr>;` that types.
fn infer_body_return(body: &[Stmt], scope: &mut Scope, env: &TypeEnv) -> Option<Type> {
    let mut found: Option<Type> = None;
    walk_for_return(body, scope, env, &mut found);
    found
}

fn walk_for_return(body: &[Stmt], scope: &mut Scope, env: &TypeEnv, found: &mut Option<Type>) {
    for stmt in body {
        if found.is_some() {
            return;
        }
        match stmt {
            Stmt::Let { name, ty, value, .. } | Stmt::Const { name, ty, value, .. } => {
                let t = ty.clone().or_else(|| expr_type(value, scope, env));
                if let Some(t) = t {
                    scope.insert(name.clone(), t);
                }
            }
            Stmt::Return(Some(e)) => {
                *found = expr_type(e, scope, env);
                return;
            }
            Stmt::If { then_branch, else_branch, .. } => {
                walk_for_return(then_branch, scope, env, found);
                if let Some(eb) = else_branch {
                    walk_for_return(eb, scope, env, found);
                }
            }
            Stmt::While { body, .. } | Stmt::For { body, .. } => {
                walk_for_return(body, scope, env, found);
            }
            Stmt::ForOf { name, ty, iterable, body } => {
                let elem = ty.clone().or_else(|| {
                    expr_type(iterable, scope, env)
                        .and_then(|t| t.array_inner().cloned())
                });
                if let Some(t) = elem {
                    scope.insert(name.clone(), t);
                }
                walk_for_return(body, scope, env, found);
            }
            Stmt::Switch { cases, .. } => {
                for c in cases {
                    walk_for_return(&c.body, scope, env, found);
                }
            }
            Stmt::Try { body, catch_body, .. } => {
                walk_for_return(body, scope, env, found);
                walk_for_return(catch_body, scope, env, found);
            }
            _ => {}
        }
    }
}

/// Fill in annotations INSIDE expressions: an arrow function's missing
/// return type, and the parameter types of inline array-method callbacks
/// (`arr.map(x => ...)` — `x` takes the receiver's element type). Backends
/// lower arrows to real functions and need concrete signatures.
fn infer_expr_annotations(e: &mut Expr, scope: &Scope, env: &TypeEnv) {
    match e {
        Expr::Arrow { params, return_type, body } => {
            match body {
                ArrowBody::Expr(inner) => infer_expr_annotations(inner, scope, env),
                ArrowBody::Block(stmts) => {
                    for s in stmts.iter_mut() {
                        infer_stmt(s, &mut scope.clone(), env);
                    }
                }
            }
            if return_type.is_none() {
                let mut s = scope.clone();
                for p in params.iter() {
                    s.insert(p.name.clone(), p.ty.clone());
                }
                let ret = match body {
                    ArrowBody::Expr(inner) => expr_type(inner, &s, env),
                    ArrowBody::Block(stmts) => infer_body_return(stmts, &mut s, env),
                };
                if let Some(t) = ret {
                    if !is_void(&t) {
                        *return_type = Some(t);
                    }
                }
            }
        }
        Expr::Call { callee, args, .. } => {
            infer_expr_annotations(callee, scope, env);
            for a in args.iter_mut() {
                infer_expr_annotations(a, scope, env);
            }
        }
        Expr::Binary { left, right, .. } => {
            infer_expr_annotations(left, scope, env);
            infer_expr_annotations(right, scope, env);
        }
        Expr::Unary { operand, .. } => infer_expr_annotations(operand, scope, env),
        Expr::Member { object, .. } => infer_expr_annotations(object, scope, env),
        Expr::Index { object, index } => {
            infer_expr_annotations(object, scope, env);
            infer_expr_annotations(index, scope, env);
        }
        Expr::ArrayLiteral { elements } => {
            for el in elements.iter_mut() {
                match el {
                    ArrayElement::Item(x) | ArrayElement::Spread(x) => {
                        infer_expr_annotations(x, scope, env)
                    }
                }
            }
        }
        Expr::ObjectLiteral { fields } => {
            for f in fields.iter_mut() {
                match f {
                    ObjectField::KV(_, v) | ObjectField::Spread(v) => {
                        infer_expr_annotations(v, scope, env)
                    }
                }
            }
        }
        Expr::AsAssertion { expr, .. } => infer_expr_annotations(expr, scope, env),
        Expr::AsConst(inner) | Expr::NonNullAssertion(inner) => {
            infer_expr_annotations(inner, scope, env)
        }
        Expr::TypeOf(inner) => infer_expr_annotations(inner, scope, env),
        _ => {}
    }
}

/// Run [`infer_expr_annotations`] over every expression owned by `stmt`.
fn infer_stmt_exprs(stmt: &mut Stmt, scope: &Scope, env: &TypeEnv) {
    match stmt {
        Stmt::Let { value, .. } | Stmt::Const { value, .. } => {
            infer_expr_annotations(value, scope, env)
        }
        Stmt::Assign { value, .. } => infer_expr_annotations(value, scope, env),
        Stmt::AssignIndex { index, value, .. } => {
            infer_expr_annotations(index, scope, env);
            infer_expr_annotations(value, scope, env);
        }
        Stmt::AssignMember { value, .. } => infer_expr_annotations(value, scope, env),
        Stmt::Return(Some(e)) | Stmt::Throw(e) | Stmt::Expr(e) => {
            infer_expr_annotations(e, scope, env)
        }
        Stmt::If { condition, .. } => infer_expr_annotations(condition, scope, env),
        Stmt::While { condition, .. } => infer_expr_annotations(condition, scope, env),
        Stmt::ForOf { iterable, .. } => infer_expr_annotations(iterable, scope, env),
        Stmt::Switch { discriminant, .. } => infer_expr_annotations(discriminant, scope, env),
        _ => {}
    }
}

fn infer_stmt(stmt: &mut Stmt, scope: &mut Scope, env: &TypeEnv) {
    infer_stmt_exprs(stmt, scope, env);
    match stmt {
        Stmt::Let { name, ty, value, .. } | Stmt::Const { name, ty, value, .. } => {
            if ty.is_none() {
                if let Some(t) = expr_type(value, scope, env) {
                    if !is_void(&t) {
                        *ty = Some(t);
                    }
                }
            }
            if let Some(t) = ty.clone() {
                scope.insert(name.clone(), t);
            }
        }
        Stmt::ForOf { name, ty, iterable, body } => {
            if ty.is_none() {
                if let Some(elem) = expr_type(iterable, scope, env)
                    .and_then(|t| t.array_inner().cloned())
                {
                    *ty = Some(elem);
                }
            }
            if let Some(t) = ty.clone() {
                scope.insert(name.clone(), t);
            }
            for s in body {
                infer_stmt(s, scope, env);
            }
        }
        Stmt::Function(f) => infer_function_body(f, scope, env),
        Stmt::ExportDecl(inner) => infer_stmt(inner, scope, env),
        Stmt::ExportDefault(ExportDefault::Function(f)) => infer_function_body(f, scope, env),
        Stmt::If { then_branch, else_branch, .. } => {
            for s in then_branch {
                infer_stmt(s, scope, env);
            }
            if let Some(eb) = else_branch {
                for s in eb {
                    infer_stmt(s, scope, env);
                }
            }
        }
        Stmt::While { body, .. } => {
            for s in body {
                infer_stmt(s, scope, env);
            }
        }
        Stmt::For { init, update, body, .. } => {
            if let Some(i) = init {
                infer_stmt(i, scope, env);
            }
            if let Some(u) = update {
                infer_stmt(u, scope, env);
            }
            for s in body {
                infer_stmt(s, scope, env);
            }
        }
        Stmt::Switch { cases, .. } => {
            for c in cases {
                for s in &mut c.body {
                    infer_stmt(s, scope, env);
                }
            }
        }
        Stmt::Try { body, catch_name, catch_body } => {
            for s in body {
                infer_stmt(s, scope, env);
            }
            if let Some(n) = catch_name {
                scope.insert(n.clone(), Type::string());
            }
            for s in catch_body {
                infer_stmt(s, scope, env);
            }
        }
        _ => {}
    }
}

fn infer_function_body(f: &mut Function, outer: &Scope, env: &TypeEnv) {
    let mut scope = outer.clone();
    for p in &f.params {
        scope.insert(p.name.clone(), p.ty.clone());
    }
    for s in &mut f.body {
        infer_stmt(s, &mut scope, env);
    }
}

/// Best-effort expression typing.
pub fn expr_type(e: &Expr, scope: &Scope, env: &TypeEnv) -> Option<Type> {
    match e {
        Expr::Number(_) => Some(Type::number()),
        Expr::String(_) => Some(Type::string()),
        Expr::Bool(_) => Some(Type::boolean()),
        Expr::Null | Expr::Undefined => None,
        Expr::Ident(name) => scope
            .get(name)
            .cloned()
            .or_else(|| env.consts.get(name).cloned()),
        Expr::Unary { op, .. } => match op {
            arcis_ast::UnaryOp::Not => Some(Type::boolean()),
            arcis_ast::UnaryOp::Neg => Some(Type::number()),
        },
        Expr::Binary { op, left, right } => {
            use arcis_ast::BinOp::*;
            match op {
                // `a ?? b`: the result is the fallback's (non-optional)
                // type; fall back to the left side's inner type.
                NullishCoalesce => expr_type(right, scope, env).or_else(|| {
                    expr_type(left, scope, env).map(|t| t.unwrap_optional().clone())
                }),
                Add => {
                    let lt = expr_type(left, scope, env);
                    let rt = expr_type(right, scope, env);
                    let is_str = |t: &Option<Type>| {
                        matches!(t, Some(Type::Primitive(p)) if p == "string")
                    };
                    if is_str(&lt) || is_str(&rt) {
                        Some(Type::string())
                    } else {
                        Some(Type::number())
                    }
                }
                Sub | Mul | Div | Mod => Some(Type::number()),
                EqEq | NotEq | Lt | Gt | LtEq | GtEq | And | Or => Some(Type::boolean()),
            }
        }
        Expr::Call { callee, args, type_args } => call_type(callee, args, type_args, scope, env),
        Expr::Member { object, property } => member_type(object, property, scope, env),
        Expr::Index { object, .. } => expr_type(object, scope, env)
            .and_then(|t| t.array_inner().cloned()),
        Expr::ArrayLiteral { elements } => {
            for el in elements {
                match el {
                    ArrayElement::Item(item) => {
                        if let Some(t) = expr_type(item, scope, env) {
                            return Some(Type::array(t));
                        }
                    }
                    ArrayElement::Spread(src) => {
                        if let Some(t) = expr_type(src, scope, env) {
                            if t.is_array() {
                                return Some(t);
                            }
                        }
                    }
                }
            }
            None // empty / untypeable array — leave for annotation
        }
        Expr::ObjectLiteral { fields } => {
            let mut typed: Vec<(String, Box<Type>, bool)> = Vec::new();
            for f in fields {
                match f {
                    ObjectField::KV(k, v) => {
                        let t = expr_type(v, scope, env)?;
                        typed.push((k.clone(), Box::new(t), false));
                    }
                    ObjectField::Spread(src) => {
                        // Merge the spread source's fields (later fields win).
                        let t = expr_type(src, scope, env)?;
                        let fields = t.object_fields()?.to_vec();
                        for f in fields {
                            typed.retain(|(n, _, _)| n != &f.0);
                            typed.push(f);
                        }
                    }
                }
            }
            let name = object_type_name(&typed);
            Some(Type::Object { name, fields: typed })
        }
        Expr::TypeOf(_) => Some(Type::string()),
        Expr::AsAssertion { ty, .. } => Some(ty.clone()),
        Expr::AsConst(inner) | Expr::NonNullAssertion(inner) => expr_type(inner, scope, env),
        Expr::Arrow { params, return_type, body } => {
            let ret = return_type.clone().or_else(|| match body {
                ArrowBody::Expr(e) => {
                    let mut s = scope.clone();
                    for p in params {
                        s.insert(p.name.clone(), p.ty.clone());
                    }
                    expr_type(e, &s, env)
                }
                ArrowBody::Block(stmts) => {
                    let mut s = scope.clone();
                    for p in params {
                        s.insert(p.name.clone(), p.ty.clone());
                    }
                    infer_body_return(stmts, &mut s, env)
                }
            })?;
            Some(Type::Function {
                params: params.iter().map(|p| p.ty.clone()).collect(),
                return_type: Box::new(ret),
            })
        }
        Expr::Path { .. } => None,
    }
}

fn call_type(callee: &Expr, args: &[Expr], type_args: &[Type], scope: &Scope, env: &TypeEnv) -> Option<Type> {
    if let Expr::Ident(name) = callee {
        return match name.as_str() {
            "print" => Some(Type::void()),
            "input" => Some(Type::string()),
            "str" => Some(Type::string()),
            "parseFloat" | "parseInt" | "Number" => Some(Type::number()),
            "isNaN" => Some(Type::boolean()),
            // `json("./data.json")` — a literal path is read and its shape
            // inferred at compile time (see `crate::json`), same as an
            // object literal's own type is inferred. `json<T>(path)` — a
            // non-literal path falls back to the explicit turbofish type
            // arg (resolved to a real interface shape later, in
            // `arcis-codegen`'s alias/interface substitution pass — here it
            // may still be an unresolved `Type::Named`, which is fine for
            // best-effort inference purposes).
            "json" => match args.first() {
                Some(Expr::String(path)) => crate::json::infer_json_type(path).ok(),
                _ => type_args.first().cloned(),
            },
            _ => {
                // Arrow-typed local (function value) or a declared function.
                if let Some(Type::Function { return_type, .. }) = scope.get(name) {
                    return Some((**return_type).clone());
                }
                env.return_type_of(name)
            }
        };
    }
    if let Expr::Member { object, property } = callee {
        // Array / string methods.
        let recv = expr_type(object, scope, env);
        return match property.as_str() {
            // Array methods.
            "map" => {
                // Element type = callback return.
                let cb_ret = args.first().and_then(|cb| callback_return(cb, scope, env));
                cb_ret.map(Type::array)
            }
            "filter" => recv.filter(|t| t.is_array()),
            "find" => recv.as_ref().and_then(|t| t.array_inner().cloned()),
            "reduce" => args
                .get(1)
                .and_then(|init| expr_type(init, scope, env))
                .or_else(|| args.first().and_then(|cb| callback_return(cb, scope, env))),
            "pop" => recv.as_ref().and_then(|t| t.array_inner().cloned()),
            "push" | "unshift" => Some(Type::number()),
            // String methods.
            "toUpperCase" | "toLowerCase" | "trim" | "substring" | "charAt" => {
                Some(Type::string())
            }
            "indexOf" => Some(Type::number()),
            "includes" => Some(Type::boolean()),
            // `sys.*` and namespace calls — unknown here.
            _ => None,
        };
    }
    None
}

/// The return type of a `.map`/`.reduce` callback: a named function's
/// (declared or inferred) return type, or an inline arrow's.
fn callback_return(cb: &Expr, scope: &Scope, env: &TypeEnv) -> Option<Type> {
    match cb {
        Expr::Ident(fname) => env.return_type_of(fname).filter(|t| !is_void(t)),
        Expr::Arrow { params, return_type, body } => {
            if let Some(rt) = return_type {
                return Some(rt.clone());
            }
            let mut s = scope.clone();
            for p in params {
                s.insert(p.name.clone(), p.ty.clone());
            }
            match body {
                ArrowBody::Expr(e) => expr_type(e, &s, env),
                ArrowBody::Block(stmts) => infer_body_return(stmts, &mut s, env),
            }
        }
        _ => None,
    }
}

fn member_type(object: &Expr, property: &str, scope: &Scope, env: &TypeEnv) -> Option<Type> {
    if property == "length" {
        return Some(Type::number());
    }
    // Enum variant: `Color.Red` — a number.
    if let Expr::Ident(name) = object {
        if env.enums.contains(name) {
            return Some(Type::number());
        }
    }
    // `sys.process(...)` result fields (runtime-fixed shape).
    match property {
        "stdout" | "stderr" => {
            if object_is_process_like(object, scope, env) {
                return Some(Type::string());
            }
        }
        "exitCode" => {
            if object_is_process_like(object, scope, env) {
                return Some(Type::number());
            }
        }
        _ => {}
    }
    // Object field access via the object's (resolved) shape.
    let obj_ty = expr_type(object, scope, env)?;
    let fields = obj_ty.object_fields()?;
    fields
        .iter()
        .find(|(n, _, _)| n == property)
        .map(|(_, t, opt)| {
            let t = (**t).clone();
            let _ = opt;
            t
        })
}

fn object_is_process_like(object: &Expr, scope: &Scope, env: &TypeEnv) -> bool {
    match expr_type(object, scope, env) {
        Some(Type::Object { name, .. }) => name == "ArcisProcess",
        Some(Type::Named(name)) => name == "ArcisProcess",
        // Untyped receiver: assume yes only if it's a direct sys.process call.
        None => matches!(
            object,
            Expr::Call { callee, .. }
                if matches!(callee.as_ref(), Expr::Member { object: o, property: p }
                    if p == "process" && matches!(o.as_ref(), Expr::Ident(s) if s == "sys"))
        ),
        _ => false,
    }
}
