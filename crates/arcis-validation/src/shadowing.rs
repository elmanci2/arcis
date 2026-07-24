//! Alpha-renaming pass for shadowed bindings.
//!
//! Arcis permits shadowing the same name in a nested block scope, like
//! TypeScript/JavaScript (`let x = 1; if (true) { let x = 2; }` is valid).
//! But the rest of the pipeline assumes globally-unique names within a
//! scope: [`crate::duplicate`]'s duplicate-declaration check treats every
//! `if`/`while`/`for` body as sharing the *same* scope as its enclosing
//! function, and `arcis-codegen`'s `collect_types`/`collect_reassigned` are
//! flat `HashMap`s keyed only by name.
//!
//! Rather than teaching all of those consumers to track lexical scope
//! (duplicate detection, two codegen maps, two LSP walkers), this module
//! resolves the conflict once, right after parsing: any inner declaration
//! that shadows an outer one is alpha-renamed (`x` -> `x$shadow1`), and
//! every reference to it within its scope is rewritten to match. After this
//! pass, no two *visible-at-the-same-time* bindings share a name, so
//! downstream code never has to reason about scope at all — a real
//! same-scope duplicate (`let x = 1; let x = 2;` with no nested block in
//! between) is deliberately left untouched so [`crate::duplicate`] still
//! reports it.
//!
//! `main` (top-level statements) is one scope tree; each function's
//! parameters + body form their own, independent scope tree (functions
//! don't close over `main` or each other today — see the crate-level doc
//! comment), matching how [`super::validate`] already partitions scopes.

use std::collections::HashMap;

use arcis_ast::{Expr, ExportDefault, Function, Program, Stmt};

/// One lexical scope: source name -> the (possibly renamed) unique name to
/// use for every reference within this scope and its descendants.
type Scope = HashMap<String, String>;

/// Rewrite `program` in place so shadowed bindings get unique names.
pub fn resolve_shadowing(program: &mut Program) {
    let mut scopes: Vec<Scope> = vec![Scope::new()];
    let mut counters: HashMap<String, u32> = HashMap::new();
    for stmt in &mut program.stmts {
        rewrite_stmt(stmt, &mut scopes, &mut counters);
    }
}

/// Declare `name` in the innermost scope. Returns the name to actually use
/// from here on: unchanged if this is a genuine same-scope duplicate (left
/// for [`crate::duplicate`] to report) or if `name` isn't visible in any
/// ancestor scope; renamed if it shadows an ancestor.
fn declare(scopes: &mut [Scope], counters: &mut HashMap<String, u32>, name: &str) -> String {
    let (current, ancestors) = scopes.split_last_mut().expect("at least one scope");
    if current.contains_key(name) {
        // Real duplicate in the same scope: not shadowing, leave as-is.
        return name.to_string();
    }
    let shadows_ancestor = ancestors.iter().any(|s| s.contains_key(name));
    let final_name = if shadows_ancestor {
        let n = counters.entry(name.to_string()).or_insert(0);
        *n += 1;
        // `$` isn't a valid Rust identifier character (the Cranelift
        // backend's own identifiers must also stay valid), so use a
        // double-underscore separator instead.
        format!("{}__shadow{}", name, n)
    } else {
        name.to_string()
    };
    current.insert(name.to_string(), final_name.clone());
    final_name
}

/// Look up the current mapping for `name`, innermost scope first. Returns
/// `None` if `name` isn't a tracked local (e.g. a function name, `print`,
/// `sys`, or a name from an unrelated scope tree) — such references are
/// left untouched.
fn resolve(scopes: &[Scope], name: &str) -> Option<String> {
    scopes.iter().rev().find_map(|s| s.get(name).cloned())
}

/// Push a fresh child scope, rewrite `stmts` within it, then pop.
fn rewrite_block(stmts: &mut [Stmt], scopes: &mut Vec<Scope>, counters: &mut HashMap<String, u32>) {
    scopes.push(Scope::new());
    for stmt in stmts {
        rewrite_stmt(stmt, scopes, counters);
    }
    scopes.pop();
}

fn rewrite_stmt(stmt: &mut Stmt, scopes: &mut Vec<Scope>, counters: &mut HashMap<String, u32>) {
    match stmt {
        Stmt::Let { name, value, .. } | Stmt::Const { name, value, .. } => {
            // The initializer resolves against the scope BEFORE this
            // declaration takes effect (`let x = x + 1;` reads the outer
            // `x` on the RHS, then may shadow it on the LHS).
            rewrite_expr(value, scopes);
            *name = declare(scopes, counters, name);
        }
        Stmt::Assign { name, value, .. } => {
            rewrite_expr(value, scopes);
            if let Some(mapped) = resolve(scopes, name) {
                *name = mapped;
            }
        }
        Stmt::AssignIndex { object, index, value, .. } => {
            rewrite_expr(index, scopes);
            rewrite_expr(value, scopes);
            if let Some(mapped) = resolve(scopes, object) {
                *object = mapped;
            }
        }
        Stmt::AssignMember { object, value, .. } => {
            rewrite_expr(object, scopes);
            rewrite_expr(value, scopes);
        }
        Stmt::Function(f) => rewrite_function(f),
        Stmt::Return(Some(expr)) => rewrite_expr(expr, scopes),
        Stmt::Return(None) | Stmt::Break | Stmt::Continue => {}
        Stmt::If { condition, then_branch, else_branch } => {
            rewrite_expr(condition, scopes);
            rewrite_block(then_branch, scopes, counters);
            if let Some(eb) = else_branch {
                rewrite_block(eb, scopes, counters);
            }
        }
        Stmt::While { condition, body } => {
            rewrite_expr(condition, scopes);
            rewrite_block(body, scopes, counters);
        }
        Stmt::For { init, condition, update, body } => {
            // The whole `for (init; cond; update) { body }` header shares
            // one scope (so `init`'s `let i` is visible to `condition` and
            // `update`), with `body` as its own nested child scope.
            scopes.push(Scope::new());
            if let Some(init_stmt) = init {
                rewrite_stmt(init_stmt, scopes, counters);
            }
            if let Some(cond) = condition {
                rewrite_expr(cond, scopes);
            }
            rewrite_block(body, scopes, counters);
            if let Some(upd) = update {
                rewrite_stmt(upd, scopes, counters);
            }
            scopes.pop();
        }
        Stmt::ForOf { name, iterable, body, .. } => {
            rewrite_expr(iterable, scopes);
            scopes.push(Scope::new());
            *name = declare(scopes, counters, name);
            rewrite_block(body, scopes, counters);
            scopes.pop();
        }
        Stmt::Switch { discriminant, cases } => {
            rewrite_expr(discriminant, scopes);
            for case in cases {
                for v in &mut case.values {
                    rewrite_expr(v, scopes);
                }
                rewrite_block(&mut case.body, scopes, counters);
            }
        }
        Stmt::Try { body, catch_name, catch_body } => {
            rewrite_block(body, scopes, counters);
            scopes.push(Scope::new());
            if let Some(name) = catch_name {
                *name = declare(scopes, counters, name);
            }
            for s in catch_body {
                rewrite_stmt(s, scopes, counters);
            }
            scopes.pop();
        }
        Stmt::Throw(expr) => rewrite_expr(expr, scopes),
        Stmt::Expr(expr) => rewrite_expr(expr, scopes),
        Stmt::ExportDecl(inner) => rewrite_stmt(inner, scopes, counters),
        Stmt::ExportDefault(ExportDefault::Function(f)) => rewrite_function(f),
        Stmt::ExportDefault(ExportDefault::Expr(expr)) => rewrite_expr(expr, scopes),
        Stmt::Import { .. }
        | Stmt::FromImport { .. }
        | Stmt::ExportSpec(_)
        | Stmt::TypeAlias { .. }
        | Stmt::Interface { .. }
        | Stmt::Enum { .. } => {}
    }
}

/// A function's parameters and top-level body share one scope (matching
/// [`super::validate`]'s `func_seen`), independent of any enclosing scope —
/// functions don't close over `main` or other functions.
fn rewrite_function(f: &mut Function) {
    let mut scopes: Vec<Scope> = vec![Scope::new()];
    let mut counters: HashMap<String, u32> = HashMap::new();
    for p in &mut f.params {
        // Renaming never actually fires here (a fresh scope tree has no
        // ancestors to shadow); `declare` is still used for a duplicate
        // param name to correctly fall through as a real duplicate.
        p.name = declare(&mut scopes, &mut counters, &p.name);
    }
    for s in &mut f.body {
        rewrite_stmt(s, &mut scopes, &mut counters);
    }
}

fn rewrite_expr(expr: &mut Expr, scopes: &[Scope]) {
    match expr {
        Expr::Ident(name) => {
            if let Some(mapped) = resolve(scopes, name) {
                *name = mapped;
            }
        }
        Expr::Call { callee, args, .. } => {
            rewrite_expr(callee, scopes);
            for a in args {
                rewrite_expr(a, scopes);
            }
        }
        Expr::Unary { operand, .. } => rewrite_expr(operand, scopes),
        Expr::Binary { left, right, .. } => {
            rewrite_expr(left, scopes);
            rewrite_expr(right, scopes);
        }
        Expr::Member { object, .. } => rewrite_expr(object, scopes),
        Expr::Index { object, index } => {
            rewrite_expr(object, scopes);
            rewrite_expr(index, scopes);
        }
        Expr::ArrayLiteral { elements } => {
            for e in elements {
                match e {
                    arcis_ast::ArrayElement::Item(e) | arcis_ast::ArrayElement::Spread(e) => {
                        rewrite_expr(e, scopes)
                    }
                }
            }
        }
        Expr::ObjectLiteral { fields } => {
            for f in fields {
                match f {
                    arcis_ast::ObjectField::KV(_, v) | arcis_ast::ObjectField::Spread(v) => {
                        rewrite_expr(v, scopes)
                    }
                }
            }
        }
        Expr::TypeOf(inner) | Expr::AsConst(inner) | Expr::NonNullAssertion(inner) => {
            rewrite_expr(inner, scopes)
        }
        Expr::AsAssertion { expr, .. } => rewrite_expr(expr, scopes),
        Expr::Arrow { params, body, .. } => {
            // A fresh child scope for the arrow's own parameters, extending
            // (not mutating) the enclosing stack: this scope must not leak
            // into sibling expressions once we return. A local counter map
            // is fine even though it doesn't share state with the caller —
            // two arrows independently minting the same `x__shadow1` name
            // can't collide, since each lives in its own Rust closure.
            let mut extended: Vec<Scope> = scopes.to_vec();
            extended.push(Scope::new());
            let mut local_counters: HashMap<String, u32> = HashMap::new();
            for p in params.iter_mut() {
                p.name = declare(&mut extended, &mut local_counters, &p.name);
            }
            match body {
                arcis_ast::ArrowBody::Expr(e) => rewrite_expr(e, &extended),
                arcis_ast::ArrowBody::Block(stmts) => {
                    for s in stmts {
                        rewrite_stmt(s, &mut extended, &mut local_counters);
                    }
                }
            }
        }
        Expr::Number(_)
        | Expr::String(_)
        | Expr::Bool(_)
        | Expr::Path { .. }
        | Expr::Null
        | Expr::Undefined => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arcis_ast::{Expr, Stmt};

    fn let_stmt(name: &str, value: Expr) -> Stmt {
        Stmt::Let { name: name.to_string(), ty: None, value, line: 0, col: 0 }
    }

    #[test]
    fn renames_inner_shadow_and_rewrites_its_uses() {
        // let x = 1; if (true) { let x = 2; print(x); }
        let inner_let = let_stmt("x", Expr::Number(2.0));
        let inner_print = Stmt::Expr(Expr::Call {
            callee: Box::new(Expr::Ident("print".to_string())),
            args: vec![Expr::Ident("x".to_string())],
            type_args: Vec::new(),
        });
        let mut program = Program {
            stmts: vec![
                let_stmt("x", Expr::Number(1.0)),
                Stmt::If {
                    condition: Expr::Bool(true),
                    then_branch: vec![inner_let, inner_print],
                    else_branch: None,
                },
            ],
        };
        resolve_shadowing(&mut program);

        let Stmt::Let { name: outer_name, .. } = &program.stmts[0] else { panic!() };
        assert_eq!(outer_name, "x");

        let Stmt::If { then_branch, .. } = &program.stmts[1] else { panic!() };
        let Stmt::Let { name: inner_name, .. } = &then_branch[0] else { panic!() };
        assert_ne!(inner_name, "x");
        let Stmt::Expr(Expr::Call { args, .. }) = &then_branch[1] else { panic!() };
        let Expr::Ident(used_name) = &args[0] else { panic!() };
        assert_eq!(used_name, inner_name, "the print(x) inside the block must reference the renamed inner x");
    }

    #[test]
    fn leaves_same_scope_duplicate_untouched_for_the_validator() {
        // let x = 1; let x = 2;  (no nested block — a real duplicate)
        let mut program = Program {
            stmts: vec![
                let_stmt("x", Expr::Number(1.0)),
                let_stmt("x", Expr::Number(2.0)),
            ],
        };
        resolve_shadowing(&mut program);
        for stmt in &program.stmts {
            let Stmt::Let { name, .. } = stmt else { panic!() };
            assert_eq!(name, "x", "same-scope duplicates must stay named `x` so `arcis-validation` still flags them");
        }
    }

    #[test]
    fn does_not_rename_unrelated_names() {
        let mut program = Program {
            stmts: vec![let_stmt("x", Expr::Number(1.0)), let_stmt("y", Expr::Number(2.0))],
        };
        resolve_shadowing(&mut program);
        let Stmt::Let { name: a, .. } = &program.stmts[0] else { panic!() };
        let Stmt::Let { name: b, .. } = &program.stmts[1] else { panic!() };
        assert_eq!(a, "x");
        assert_eq!(b, "y");
    }
}
