//! Integration tests for the `arcis-parser` crate.
//!
//! These tests round-trip `.tsr` source through the lexer and parser and
//! assert against the resulting AST. They live outside the crate so they
//! exercise the **public** API only.

use arcis_ast::{Expr, Stmt};

fn parse(src: &str) -> Vec<Stmt> {
    let tokens = arcis_lexer::lex(src).expect("lex must succeed");
    let prog = arcis_parser::parse(tokens).expect("parse must succeed");
    prog.stmts
}

#[test]
fn parses_let_with_type_annotation() {
    let stmts = parse("let x: number = 42;");
    assert_eq!(stmts.len(), 1);
    match &stmts[0] {
        Stmt::Let { name, value, .. } => {
            assert_eq!(name, "x");
            assert!(matches!(value, Expr::Number(42.0)));
        }
        _ => panic!("expected Stmt::Let"),
    }
}

#[test]
fn parses_print_call() {
    let stmts = parse("print(\"hello\");");
    assert!(matches!(&stmts[0],
        Stmt::Expr(Expr::Call { callee, .. })
        if matches!(callee.as_ref(), Expr::Ident(n) if n == "print")
    ));
}

#[test]
fn parses_if_else() {
    let src = "if (true) { print(\"a\"); } else { print(\"b\"); }";
    let stmts = parse(src);
    match &stmts[0] {
        Stmt::If { else_branch, .. } => assert!(else_branch.is_some()),
        _ => panic!("expected Stmt::If"),
    }
}

#[test]
fn parses_for_of_loop() {
    let src = "for (let v of arr) { print(v); }";
    let stmts = parse(src);
    assert!(matches!(&stmts[0], Stmt::ForOf { .. }));
}

#[test]
fn parses_named_function() {
    let src = "function add(a: number, b: number): number { return a + b; }";
    let stmts = parse(src);
    match &stmts[0] {
        Stmt::Function(f) => {
            assert_eq!(f.name, "add");
            assert_eq!(f.params.len(), 2);
        }
        _ => panic!("expected Stmt::Function"),
    }
}

#[test]
fn parses_named_import() {
    let stmts = parse("import { add } from \"utils\";");
    match &stmts[0] {
        Stmt::Import { named, module, .. } => {
            assert_eq!(module, "utils");
            assert_eq!(named.len(), 1);
            assert_eq!(named[0].name, "add");
        }
        _ => panic!("expected Stmt::Import"),
    }
}

#[test]
fn parses_default_import() {
    let stmts = parse("import compute from \"utils\";");
    match &stmts[0] {
        Stmt::Import { default, named, .. } => {
            assert_eq!(default.as_deref(), Some("compute"));
            assert!(named.is_empty());
        }
        _ => panic!("expected Stmt::Import"),
    }
}

#[test]
fn parses_export_default_function() {
    let stmts = parse("export default function () { return 42; }");
    match &stmts[0] {
        Stmt::ExportDefault(_) => {}
        _ => panic!("expected Stmt::ExportDefault"),
    }
}

#[test]
fn parses_object_literal() {
    let stmts = parse("let p = { name: \"alice\", age: 30 };");
    match &stmts[0] {
        Stmt::Let { value, .. } => match value {
            Expr::ObjectLiteral { fields } => {
                assert_eq!(fields.len(), 2);
            }
            _ => panic!("expected object literal"),
        },
        _ => panic!("expected Stmt::Let"),
    }
}

#[test]
fn operator_precedence_is_respected() {
    // 1 + 2 * 3 should parse as `1 + (2 * 3)`, NOT `(1 + 2) * 3`.
    let stmts = parse("let x: number = 1 + 2 * 3;");
    match &stmts[0] {
        Stmt::Let { value, .. } => match value {
            Expr::Binary {
                op: arcis_ast::BinOp::Add,
                left,
                right,
            } => {
                assert!(matches!(left.as_ref(), Expr::Number(1.0)));
                assert!(matches!(
                    right.as_ref(),
                    Expr::Binary { op: arcis_ast::BinOp::Mul, .. }
                ));
            }
            _ => panic!("expected outer + with inner *"),
        },
        _ => panic!("expected Stmt::Let"),
    }
}
