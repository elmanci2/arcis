//! Integration tests for the `arcis-codegen` crate.
//!
//! These tests lex + parse + generate Rust source for small `.tsr` snippets
//! and assert on the emitted code. They exercise the **public** API only.

#[test]
fn empty_modules_list_produces_no_output() {
    let result = arcis_codegen::generate_all(&[]);
    assert!(result.is_ok());
    assert!(result.unwrap().is_empty());
}

#[test]
fn generates_fn_main_for_a_simple_program() {
    // Build the AST by hand from the lexer + parser to keep the test self-contained.
    let tokens = arcis_lexer::lex("print(\"hi\");").expect("lex");
    let program = arcis_parser::parse(tokens).expect("parse");

    let module = arcis_linker::Module {
        id: "main".into(),
        path: std::path::PathBuf::from("main.tsr"),
        program,
        exports: Default::default(),
    };

    let sources = arcis_codegen::generate_all(&[module]).expect("codegen");
    assert_eq!(sources.len(), 1);
    let (_, src) = &sources[0];
    assert!(src.contains("fn main()"), "emitted source must contain `fn main()`: {src}");
    assert!(src.contains("println!"), "emitted source must contain `println!`: {src}");
}

#[test]
fn emits_struct_for_inline_object_type() {
    let tokens = arcis_lexer::lex(
        "let p: { name: string, age: number } = { name: \"alice\", age: 30 };",
    )
    .expect("lex");
    let program = arcis_parser::parse(tokens).expect("parse");

    let module = arcis_linker::Module {
        id: "main".into(),
        path: std::path::PathBuf::from("main.tsr"),
        program,
        exports: Default::default(),
    };

    let sources = arcis_codegen::generate_all(&[module]).expect("codegen");
    let (_, src) = &sources[0];
    assert!(src.contains("pub struct"), "emitted source must define a struct: {src}");
    assert!(src.contains("name"), "emitted struct must have a `name` field: {src}");
}

#[test]
fn string_plus_number_emits_format_macro() {
    let tokens = arcis_lexer::lex("let s: string = \"x=\" + 42;").expect("lex");
    let program = arcis_parser::parse(tokens).expect("parse");

    let module = arcis_linker::Module {
        id: "main".into(),
        path: std::path::PathBuf::from("main.tsr"),
        program,
        exports: Default::default(),
    };

    let sources = arcis_codegen::generate_all(&[module]).expect("codegen");
    let (_, src) = &sources[0];
    assert!(
        src.contains("format!"),
        "`+` of a string literal must use format!: {src}"
    );
}
