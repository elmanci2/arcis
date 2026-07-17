//! Integration tests for the `arcis-lexer` crate.
//!
//! These tests exercise the **public** API of the lexer (`arcis_lexer::lex`)
//! as a downstream user would. They live outside the crate's `src/` so they
//! stay independent of internal implementation details.

use arcis_lexer::TokenKind;

/// Helper: lex the input and return just the token kinds for compact assertions.
fn kinds(src: &str) -> Vec<TokenKind> {
    arcis_lexer::lex(src)
        .expect("lex must succeed")
        .into_iter()
        .map(|t| t.kind)
        .collect()
}

#[test]
fn lexes_keyword_let() {
    assert!(matches!(kinds("let")[0], TokenKind::Let));
}

#[test]
fn lexes_identifier() {
    let k = kinds("foo_bar");
    assert!(matches!(&k[0], TokenKind::Ident(s) if s == "foo_bar"));
}

#[test]
fn lexes_integer_as_number() {
    let k = kinds("42");
    match &k[0] {
        TokenKind::Number(n) => assert_eq!(*n, 42.0),
        other => panic!("expected number, got {:?}", other),
    }
}

#[test]
fn lexes_decimal_with_fraction() {
    let k = kinds("1.5");
    match &k[0] {
        TokenKind::Number(n) => assert!((*n - 1.5).abs() < 1e-9),
        other => panic!("expected number, got {:?}", other),
    }
}

#[test]
fn lexes_string_with_escapes() {
    let k = kinds(r#""hello\nworld""#);
    match &k[0] {
        TokenKind::String(s) => assert_eq!(s, "hello\nworld"),
        other => panic!("expected string, got {:?}", other),
    }
}

#[test]
fn recognizes_arrow_operators() {
    let k = kinds("== != <= >= && ||");
    assert!(matches!(k[0], TokenKind::EqEq));
    assert!(matches!(k[1], TokenKind::NotEq));
    assert!(matches!(k[2], TokenKind::LtEq));
    assert!(matches!(k[3], TokenKind::GtEq));
    assert!(matches!(k[4], TokenKind::And));
    assert!(matches!(k[5], TokenKind::Or));
}

#[test]
fn skips_line_comments() {
    // The whole `let` then `// foo` then `42` should yield: Let, Number, Eof.
    let k = kinds("// hello\nlet // mid\n42");
    assert!(matches!(k[0], TokenKind::Let));
    assert!(matches!(k[1], TokenKind::Number(_)));
    assert!(matches!(k[k.len() - 1], TokenKind::Eof));
}

#[test]
fn skips_block_comments() {
    let k = kinds("let /* ignore me */ 42");
    assert!(matches!(k[0], TokenKind::Let));
    assert!(matches!(k[1], TokenKind::Number(_)));
}

#[test]
fn unterminated_string_is_an_error() {
    let result = arcis_lexer::lex("\"abc");
    assert!(result.is_err());
}

#[test]
fn unexpected_character_is_an_error() {
    let result = arcis_lexer::lex("@");
    assert!(result.is_err());
}

#[test]
fn lexes_import_and_export_keywords() {
    let k = kinds("import export from default as");
    assert!(matches!(k[0], TokenKind::Import));
    assert!(matches!(k[1], TokenKind::Export));
    assert!(matches!(k[2], TokenKind::From));
    assert!(matches!(k[3], TokenKind::Default));
    assert!(matches!(k[4], TokenKind::As));
}

#[test]
fn lexes_colon_colon_for_paths() {
    let k = kinds("reqwest::Client");
    assert!(matches!(k[0], TokenKind::Ident(ref s) if s == "reqwest"));
    assert!(matches!(k[1], TokenKind::ColonColon));
    assert!(matches!(k[2], TokenKind::Ident(ref s) if s == "Client"));
}
