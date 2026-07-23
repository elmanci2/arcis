//! Type parsing.
//!
//! Supports the "Everyday Types" subset of TypeScript:
//! - Primitive type keywords: `string`, `number`, `boolean`, `void`, `any`
//! - `null` and `undefined`
//! - Named (user-defined) types: any identifier (type alias / interface name)
//! - Inline object types: `{ name: type, opt?: type, ... }` — given a
//!   deterministic, hash-based name so the codegen can `pub struct` it once.
//!   Fields marked with `?` are optional.
//! - Array suffixes: `T[]` (one level in v1).
//! - Union types: `A | B | C`.
//! - Intersection types: `A & B`.
//! - Literal types: `"hello"`, `42`, `true`, `false`.
//! - Function types: `(a: T, b: U) => R`.
//!
//! Precedence (loosest to tightest): union > intersection > postfix-array >
//! primary. This mirrors TypeScript's own type-grammar precedence.

use arcis_ast::{LiteralValue, Type};
use arcis_lexer::TokenKind;

use crate::error::ParseError;
use crate::state::Parser;

// Deterministic inline-object-type naming — shared with the type-inference
// pass, so it lives in `arcis-ast`.
pub(crate) use arcis_ast::object_type_name;

impl Parser {
    /// Parse a type annotation. Entry point: union types (the loosest
    /// binding type-level operator).
    pub(crate) fn parse_type(&mut self) -> Result<Type, ParseError> {
        self.parse_union_type()
    }

    /// `intersection ( "|" intersection )*`
    ///
    /// Unions with `null` / `undefined` members normalize to
    /// `Optional(rest)` — `string | null` IS `string?`, so the null-safety
    /// checker only ever has to reason about one optional shape.
    fn parse_union_type(&mut self) -> Result<Type, ParseError> {
        // Allow a leading `|` before the first member (TS permits this to
        // make multi-line union declarations easier to format).
        self.matches(&TokenKind::Bar);
        let first = self.parse_intersection_type()?;
        if !self.check(&TokenKind::Bar) {
            return Ok(first);
        }
        let mut members = vec![first];
        while self.matches(&TokenKind::Bar) {
            members.push(self.parse_intersection_type()?);
        }
        let had_null = members
            .iter()
            .any(|m| matches!(m, Type::Null | Type::Undefined));
        members.retain(|m| !matches!(m, Type::Null | Type::Undefined));
        let base = match members.len() {
            0 => Type::Null,
            1 => members.into_iter().next().unwrap(),
            _ => Type::Union(members),
        };
        Ok(if had_null { Type::optional(base) } else { base })
    }

    /// `postfix_type ( "&" postfix_type )*`
    fn parse_intersection_type(&mut self) -> Result<Type, ParseError> {
        let first = self.parse_postfix_type()?;
        if !self.check(&TokenKind::Ampersand) {
            return Ok(first);
        }
        let mut members = vec![first];
        while self.matches(&TokenKind::Ampersand) {
            members.push(self.parse_postfix_type()?);
        }
        Ok(Type::Intersection(members))
    }

    /// `primary_type ( "[" "]" )* "?"?` — repeated `[]` suffixes build
    /// nested `Type::Array`; a trailing `?` makes the type optional.
    fn parse_postfix_type(&mut self) -> Result<Type, ParseError> {
        let mut ty = self.parse_primary_type()?;
        while self.check(&TokenKind::LBracket) {
            self.advance();
            self.expect(&TokenKind::RBracket, "`]` after `[`")?;
            ty = Type::Array(Box::new(ty));
        }
        if self.check(&TokenKind::QuestionQuestion) {
            let t = self.peek();
            return Err(ParseError {
                line: t.line,
                col: t.col,
                msg: "`T??` is not a type — one `?` already covers \"may be missing\"".to_string(),
            });
        }
        if self.matches(&TokenKind::Question) {
            ty = Type::optional(ty);
        }
        Ok(ty)
    }

    fn parse_primary_type(&mut self) -> Result<Type, ParseError> {
        // Inline object type: { name: type, opt?: type, ... }
        if self.check(&TokenKind::LBrace) {
            return self.parse_object_type();
        }

        // Function type: ( params ) => ReturnType
        // Only attempted when we can see `(` — the caller doesn't need to
        // disambiguate from a parenthesised type since Arcis v1 doesn't
        // support parenthesised type grouping.
        if self.check(&TokenKind::LParen) {
            return self.parse_function_type();
        }

        let t = self.advance();
        let ty = match &t.kind {
            TokenKind::TypeString => Type::Primitive("string".to_string()),
            TokenKind::TypeNumber => Type::Primitive("number".to_string()),
            TokenKind::TypeBoolean => Type::Primitive("boolean".to_string()),
            TokenKind::TypeVoid => Type::Primitive("void".to_string()),
            TokenKind::TypeAny => Type::Primitive("any".to_string()),
            TokenKind::Null => Type::Null,
            TokenKind::Undefined => Type::Undefined,
            TokenKind::String(s) => Type::Literal(LiteralValue::String(s.clone())),
            TokenKind::Number(n) => Type::Literal(LiteralValue::Number(*n)),
            TokenKind::Bool(b) => Type::Literal(LiteralValue::Bool(*b)),
            // Signed numeric literal type: `-1`.
            TokenKind::Minus => {
                let next = self.advance();
                match next.kind {
                    TokenKind::Number(n) => Type::Literal(LiteralValue::Number(-n)),
                    other => {
                        return Err(ParseError {
                            line: next.line,
                            col: next.col,
                            msg: format!("expected a number after `-` in type position, found {}", other),
                        });
                    }
                }
            }
            TokenKind::Ident(s) => Type::Named(s.clone()),
            other => {
                return Err(ParseError {
                    line: t.line,
                    col: t.col,
                    msg: format!("unknown type {}", other),
                });
            }
        };
        Ok(ty)
    }

    /// `{ name: type, opt?: type, ... }` — the `{` has not been consumed yet.
    fn parse_object_type(&mut self) -> Result<Type, ParseError> {
        self.advance(); // {
        let mut fields = Vec::new();
        // Permit `{}` (empty object type)
        if !self.check(&TokenKind::RBrace) {
            loop {
                let key_tok = self.expect(
                    &TokenKind::Ident(String::new()),
                    "field name in object type",
                )?;
                let key = match &key_tok.kind {
                    TokenKind::Ident(s) => s.clone(),
                    _ => unreachable!(),
                };
                let optional = self.matches(&TokenKind::Question);
                self.expect(&TokenKind::Colon, "`:` after field name")?;
                let field_ty = self.parse_type()?;
                // `field?: T` == `field: T?` — see the matching comment in
                // `stmt.rs::parse_interface`.
                let stored_ty = if optional { Type::optional(field_ty) } else { field_ty };
                fields.push((key, Box::new(stored_ty), optional));
                // `,` or `;` both separate fields (TS allows either).
                if !self.matches(&TokenKind::Comma) && !self.matches(&TokenKind::Semi) {
                    break;
                }
            }
        }
        self.expect(&TokenKind::RBrace, "`}` closing object type")?;
        let name = object_type_name(&fields);
        Ok(Type::Object { name, fields })
    }

    /// `( params ) => ReturnType` — the `(` has not been consumed yet.
    fn parse_function_type(&mut self) -> Result<Type, ParseError> {
        self.advance(); // (
        let mut params = Vec::new();
        if !self.check(&TokenKind::RParen) {
            loop {
                // Named parameter form: `name: type`. The name is discarded
                // for the type representation (Arcis function types are
                // structural), but we still require it syntactically to
                // match TypeScript's function type grammar.
                self.expect(&TokenKind::Ident(String::new()), "parameter name in function type")?;
                self.expect(&TokenKind::Colon, "`:` after parameter name")?;
                params.push(self.parse_type()?);
                if !self.matches(&TokenKind::Comma) {
                    break;
                }
            }
        }
        self.expect(&TokenKind::RParen, "`)` after function type parameters")?;
        self.expect(&TokenKind::FatArrow, "`=>` in function type")?;
        let return_type = self.parse_type()?;
        Ok(Type::Function {
            params,
            return_type: Box::new(return_type),
        })
    }
}
