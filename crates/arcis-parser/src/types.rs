//! Type parsing.
//!
//! Supports:
//! - Primitive type keywords: `string`, `number`, `boolean`, `void`
//! - Named (user-defined) types: any identifier
//! - Inline object types: `{ name: type, ... }` — given a deterministic,
//!   hash-based name so the codegen can `pub struct` it once.
//! - Array suffixes: `T[]` (one level in v1).

use arcis_ast::Type;
use arcis_lexer::TokenKind;

use crate::error::ParseError;
use crate::state::Parser;

/// Generate a deterministic identifier for an inline object type, based on
/// the hash of its field shape. Same shape → same name → same struct.
pub(crate) fn object_type_name(fields: &[(String, Box<Type>)]) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut h = DefaultHasher::new();
    for (k, t) in fields {
        k.hash(&mut h);
        t.name.hash(&mut h);
        for (fk, ft) in &t.fields {
            fk.hash(&mut h);
            ft.name.hash(&mut h);
        }
    }
    format!("__Obj{:x}", h.finish() & 0xFFFFFF)
}

impl Parser {
    /// Parse a type annotation.
    pub(crate) fn parse_type(&mut self) -> Result<Type, ParseError> {
        // Inline object type: { name: type, ... }
        if self.check(&TokenKind::LBrace) {
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
                    self.expect(&TokenKind::Colon, "`:` after field name")?;
                    let field_ty = self.parse_type()?;
                    fields.push((key, Box::new(field_ty)));
                    if !self.matches(&TokenKind::Comma) {
                        break;
                    }
                }
            }
            self.expect(&TokenKind::RBrace, "`}` closing object type")?;
            // Generate a deterministic, unique name based on the shape.
            let name = object_type_name(&fields);
            let mut is_array = false;
            // Optional `[]` suffix (array of objects).
            if self.check(&TokenKind::LBracket) {
                self.advance();
                self.expect(&TokenKind::RBracket, "`]` after `[`")?;
                is_array = true;
            }
            return Ok(Type { name, fields, is_array });
        }

        let t = self.advance();
        let name = match &t.kind {
            TokenKind::TypeString => "string".to_string(),
            TokenKind::TypeNumber => "number".to_string(),
            TokenKind::TypeBoolean => "boolean".to_string(),
            TokenKind::TypeVoid => "void".to_string(),
            TokenKind::Ident(s) => s.clone(),
            other => {
                return Err(ParseError {
                    line: t.line,
                    col: t.col,
                    msg: format!("unknown type {}", other),
                });
            }
        };
        let mut is_array = false;
        // Optional `[]` suffix (one level in v1).
        if self.check(&TokenKind::LBracket) {
            self.advance();
            self.expect(&TokenKind::RBracket, "`]` after `[`")?;
            is_array = true;
        }
        Ok(Type {
            name,
            fields: Vec::new(),
            is_array,
        })
    }
}