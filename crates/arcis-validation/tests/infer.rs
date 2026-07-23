//! Tests for the type-inference pass.

use arcis_ast::{Stmt, Type};
use arcis_validation::{infer_program, TypeEnv};

fn infer(src: &str) -> arcis_ast::Program {
    let tokens = arcis_lexer::lex(src).expect("lex");
    let mut program = arcis_parser::parse(tokens).expect("parse");
    let mut env = TypeEnv::default();
    env.add_program(&program);
    infer_program(&mut program, &env);
    program
}

fn let_type<'a>(program: &'a arcis_ast::Program, name: &str) -> Option<&'a Type> {
    fn walk<'a>(stmts: &'a [Stmt], name: &str) -> Option<&'a Type> {
        for s in stmts {
            match s {
                Stmt::Let { name: n, ty, .. } | Stmt::Const { name: n, ty, .. } if n == name => {
                    return ty.as_ref()
                }
                Stmt::Function(f) => {
                    if let Some(t) = walk(&f.body, name) {
                        return Some(t);
                    }
                }
                _ => {}
            }
        }
        None
    }
    walk(&program.stmts, name)
}

#[test]
fn infers_primitive_lets() {
    let p = infer("let a = 1;\nlet b = \"x\";\nlet c = true;\n");
    assert_eq!(let_type(&p, "a").unwrap().primitive_name(), "number");
    assert_eq!(let_type(&p, "b").unwrap().primitive_name(), "string");
    assert_eq!(let_type(&p, "c").unwrap().primitive_name(), "boolean");
}

#[test]
fn infers_array_and_element() {
    let p = infer("let xs = [1, 2, 3];\n");
    let t = let_type(&p, "xs").unwrap();
    assert!(t.is_array());
    assert_eq!(t.array_inner().unwrap().primitive_name(), "number");
}

#[test]
fn infers_object_literal_shape() {
    let p = infer("let o = { a: 1, b: \"hi\" };\n");
    let t = let_type(&p, "o").unwrap();
    let fields = t.object_fields().expect("object type");
    assert_eq!(fields.len(), 2);
    assert_eq!(fields[0].0, "a");
    assert_eq!(fields[0].1.primitive_name(), "number");
    assert_eq!(fields[1].1.primitive_name(), "string");
}

#[test]
fn infers_function_return_type() {
    let p = infer("function dbl(n: number) {\n    return n * 2;\n}\n");
    let f = p
        .stmts
        .iter()
        .find_map(|s| match s {
            Stmt::Function(f) => Some(f),
            _ => None,
        })
        .unwrap();
    assert_eq!(f.return_type.primitive_name(), "number");
}

#[test]
fn infers_through_function_calls() {
    let p = infer("function greet(n: string) {\n    return \"hi \" + n;\n}\nlet g = greet(\"ana\");\n");
    assert_eq!(let_type(&p, "g").unwrap().primitive_name(), "string");
}

#[test]
fn explicit_annotation_wins() {
    let p = infer("let x: string = \"a\";\n");
    assert_eq!(let_type(&p, "x").unwrap().primitive_name(), "string");
}

#[test]
fn infers_for_of_element() {
    let p = infer("let xs = [\"a\", \"b\"];\nfor (let s of xs) {\n    print(s);\n}\n");
    let forof_ty = p.stmts.iter().find_map(|s| match s {
        Stmt::ForOf { ty, .. } => ty.as_ref(),
        _ => None,
    });
    assert_eq!(forof_ty.unwrap().primitive_name(), "string");
}

#[test]
fn infers_string_concat_type() {
    let p = infer("let n = 3;\nlet msg = \"total: \" + n;\n");
    assert_eq!(let_type(&p, "msg").unwrap().primitive_name(), "string");
}
