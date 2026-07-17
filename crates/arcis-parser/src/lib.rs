//! Arcis parser: token stream → AST.
//!
//! Recursive-descent parser with **precedence climbing** for expressions.
//!
//! Grammar (simplified):
//! ```text
//!   program     = stmt*
//!   stmt        = let_stmt | const_stmt | fn_stmt | if_stmt | while_stmt
//!               | for_stmt | return_stmt | break_stmt | continue_stmt
//!               | assign_stmt | expr_stmt
//!   let_stmt    = "let"  IDENT (":" type)? "=" expr ";"
//!   const_stmt  = "const" IDENT (":" type)? "=" expr ";"
//!   fn_stmt     = "function" IDENT "(" params? ")" (":" type)? "{" stmt* "}"
//!   if_stmt     = "if" "(" expr ")" "{" stmt* "}" ("else" "{" stmt* "}")?
//!   while_stmt  = "while" "(" expr ")" "{" stmt* "}"
//!   for_stmt    = "for" "(" (let_stmt | expr_stmt | ";") expr? ";" expr? ")" "{" stmt* "}"
//!   return_stmt = "return" expr? ";"
//!   break_stmt  = "break" ";"
//!   continue_stmt = "continue" ";"
//!   expr_stmt   = expr ";"
//!
//!   expr        = or
//!   or          = and ( "||" and )*
//!   and         = equality ( "&&" equality )*
//!   equality    = comparison ( ("==" | "!=") comparison )*
//!   comparison  = additive ( ("<" | ">" | "<=" | ">=") additive )*
//!   additive    = multiplicative ( ("+" | "-") multiplicative )*
//!   multiplicative = unary ( ("*" | "/" | "%") unary )*
//!   unary       = ("!" | "-") unary | postfix
//!   postfix     = atom ( "." IDENT | "[" expr "]" )*
//!   atom        = NUMBER | STRING | "true" | "false" | IDENT ("(" args ")")? | "(" expr ")"
//! ```
//!
//! ## Layout
//!
//! - [`state`](self::state) — `Parser` cursor + look-ahead / advance helpers.
//! - [`error`](self::error) — `ParseError` type.
//! - [`stmt`](self::stmt) — `parse_stmt`, `parse_let`, `parse_function`, etc.
//! - [`expr`](self::expr) — `parse_expr` and the precedence-climbing chain.
//! - [`types`](self::types) — `parse_type` and `object_type_name`.
//! - [`modules`](self::modules) — `parse_import` and `parse_export`.

use arcis_ast::Program;
use arcis_lexer::{Token, TokenKind};

mod error;
mod expr;
mod modules;
mod state;
mod stmt;
mod types;

pub use error::ParseError;

/// Parse a complete token stream into a [`Program`].
pub fn parse(tokens: Vec<Token>) -> Result<Program, ParseError> {
    use crate::state::Parser;

    let mut p = Parser { tokens, pos: 0 };
    let mut stmts = Vec::new();
    while !p.check(&TokenKind::Eof) {
        stmts.push(p.parse_stmt()?);
    }
    Ok(Program { stmts })
}

#[cfg(test)]
mod tests {
    use super::*;
    use arcis_lexer::lex;

    #[test]
    fn parses_simple_let_and_print() {
        let toks = lex("let x: number = 42; print(x);").expect("lex must succeed");
        let prog = parse(toks).expect("parse must succeed");
        assert_eq!(prog.stmts.len(), 2);
    }

    #[test]
    fn parses_module_import() {
        let toks = lex("import { add } from \"utils\";").expect("lex must succeed");
        let prog = parse(toks).expect("parse must succeed");
        assert!(matches!(prog.stmts[0], arcis_ast::Stmt::Import { .. }));
    }
}