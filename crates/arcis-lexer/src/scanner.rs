//! Token scanning.
//!
//! Per-token-type readers and the keyword table. Each reader consumes
//! characters from the [`Lexer`](super::state::Lexer) and returns either a
//! fully-decoded [`TokenKind`] or a [`LexError`](super::error::LexError) with
//! the offending position.
//!
//! The dispatcher in [`lex`](super::lex) decides which reader to call by
//! inspecting the first character of the token (identifier-start, digit,
//! `"`, or punctuation).

use std::collections::HashMap;

use crate::error::LexError;
use crate::state::Lexer;
use crate::token::TokenKind;

/// Returns `true` if `c` may start an identifier (letter, `_`, or `$`).
pub(crate) fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_' || c == '$'
}

/// Returns `true` if `c` may continue an identifier (letter, digit, `_`, `$`).
pub(crate) fn is_ident_continue(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '$'
}

/// Build the keyword table (same identifiers as TypeScript, lowercase).
pub(crate) fn keywords() -> HashMap<&'static str, TokenKind> {
    let mut m = HashMap::new();
    m.insert("let", TokenKind::Let);
    m.insert("const", TokenKind::Const);
    m.insert("function", TokenKind::Function);
    m.insert("return", TokenKind::Return);
    m.insert("if", TokenKind::If);
    m.insert("else", TokenKind::Else);
    m.insert("while", TokenKind::While);
    m.insert("for", TokenKind::For);
    m.insert("of", TokenKind::Of);
    m.insert("break", TokenKind::Break);
    m.insert("continue", TokenKind::Continue);
    // Module keywords (ES modules / TS).
    m.insert("import", TokenKind::Import);
    m.insert("export", TokenKind::Export);
    m.insert("from", TokenKind::From);
    m.insert("default", TokenKind::Default);
    m.insert("as", TokenKind::As);
    // `true` and `false` are keywords *and* boolean literals; we emit them as
    // `TokenKind::Bool(_)` so the parser treats them uniformly.
    m.insert("true", TokenKind::Bool(true));
    m.insert("false", TokenKind::Bool(false));
    m.insert("string", TokenKind::TypeString);
    m.insert("number", TokenKind::TypeNumber);
    m.insert("boolean", TokenKind::TypeBoolean);
    m.insert("void", TokenKind::TypeVoid);
    m
}

/// Skip whitespace and `// ...` / `/* ... */` comments, leaving the cursor at
/// the start of the next real token (or at EOF).
pub(crate) fn skip_whitespace_and_comments(lx: &mut Lexer) {
    loop {
        lx.start_token();
        if lx.is_eof() {
            return;
        }
        match lx.peek_char().unwrap() {
            ' ' | '\t' | '\r' | '\n' => {
                lx.advance();
            }
            '/' if lx.peek_char_at(1) == Some('/') => {
                while let Some(c) = lx.peek_char() {
                    if c == '\n' {
                        break;
                    }
                    lx.advance();
                }
            }
            '/' if lx.peek_char_at(1) == Some('*') => {
                lx.advance(); // /
                lx.advance(); // *
                while let Some(c) = lx.peek_char() {
                    if c == '*' && lx.peek_char_at(1) == Some('/') {
                        lx.advance();
                        lx.advance();
                        break;
                    }
                    lx.advance();
                }
            }
            _ => return,
        }
    }
}

/// Read an identifier or keyword starting at the current cursor position.
pub(crate) fn read_ident(lx: &mut Lexer) -> Result<TokenKind, LexError> {
    while let Some(c) = lx.peek_char() {
        if is_ident_continue(c) {
            lx.advance();
        } else {
            break;
        }
    }
    let text = lx.slice_current();
    let kw = keywords();
    Ok(kw.get(text.as_str()).cloned().unwrap_or(TokenKind::Ident(text)))
}

/// Read an integer or decimal number.
pub(crate) fn read_number(lx: &mut Lexer) -> Result<TokenKind, LexError> {
    while let Some(c) = lx.peek_char() {
        if c.is_ascii_digit() {
            lx.advance();
        } else {
            break;
        }
    }
    // Optional fractional part.
    if lx.peek_char() == Some('.')
        && lx.peek_char_at(1).map(|c| c.is_ascii_digit()).unwrap_or(false)
    {
        lx.advance(); // .
        while let Some(c) = lx.peek_char() {
            if c.is_ascii_digit() {
                lx.advance();
            } else {
                break;
            }
        }
    }
    let text = lx.slice_current();
    let n: f64 = text.parse().map_err(|_| LexError {
        line: lx.line,
        col: lx.col,
        msg: format!("invalid number `{}`", text),
    })?;
    Ok(TokenKind::Number(n))
}

/// Read a double-quoted string literal, processing standard escape sequences.
pub(crate) fn read_string(lx: &mut Lexer) -> Result<TokenKind, LexError> {
    // consume the opening quote
    lx.advance(); // "
    let mut s = String::new();
    loop {
        match lx.peek_char() {
            None => {
                return Err(LexError {
                    line: lx.line,
                    col: lx.col,
                    msg: "unterminated string".to_string(),
                });
            }
            Some('"') => {
                lx.advance();
                break;
            }
            Some('\\') => {
                lx.advance();
                match lx.peek_char() {
                    Some('n') => { lx.advance(); s.push('\n'); }
                    Some('t') => { lx.advance(); s.push('\t'); }
                    Some('r') => { lx.advance(); s.push('\r'); }
                    Some('\\') => { lx.advance(); s.push('\\'); }
                    Some('"') => { lx.advance(); s.push('"'); }
                    Some(c) => {
                        return Err(LexError {
                            line: lx.line,
                            col: lx.col,
                            msg: format!("invalid escape `\\{}`", c),
                        });
                    }
                    None => {
                        return Err(LexError {
                            line: lx.line,
                            col: lx.col,
                            msg: "unterminated string".to_string(),
                        });
                    }
                }
            }
            Some(c) => {
                lx.advance();
                s.push(c);
            }
        }
    }
    Ok(TokenKind::String(s))
}

/// Read a punctuation character or operator. Handles multi-character operators
/// like `==`, `!=`, `<=`, `>=`, `&&`, `||`, and `::`.
pub(crate) fn read_punct_or_op(lx: &mut Lexer) -> Result<TokenKind, LexError> {
    let c = lx.advance().unwrap();
    let kind = match c {
        '+' => TokenKind::Plus,
        '-' => TokenKind::Minus,
        '*' => TokenKind::Star,
        '/' => TokenKind::Slash,
        '%' => TokenKind::Percent,
        '(' => TokenKind::LParen,
        ')' => TokenKind::RParen,
        '{' => TokenKind::LBrace,
        '}' => TokenKind::RBrace,
        '[' => TokenKind::LBracket,
        ']' => TokenKind::RBracket,
        ';' => TokenKind::Semi,
        ',' => TokenKind::Comma,
        ':' => {
            if lx.peek_char() == Some(':') {
                lx.advance();
                TokenKind::ColonColon
            } else {
                TokenKind::Colon
            }
        }
        '.' => TokenKind::Dot,
        '=' => {
            if lx.peek_char() == Some('=') {
                lx.advance();
                TokenKind::EqEq
            } else {
                TokenKind::Eq
            }
        }
        '!' => {
            if lx.peek_char() == Some('=') {
                lx.advance();
                TokenKind::NotEq
            } else {
                TokenKind::Bang
            }
        }
        '<' => {
            if lx.peek_char() == Some('=') {
                lx.advance();
                TokenKind::LtEq
            } else {
                TokenKind::Lt
            }
        }
        '>' => {
            if lx.peek_char() == Some('=') {
                lx.advance();
                TokenKind::GtEq
            } else {
                TokenKind::Gt
            }
        }
        '&' if lx.peek_char() == Some('&') => {
            lx.advance();
            TokenKind::And
        }
        '|' if lx.peek_char() == Some('|') => {
            lx.advance();
            TokenKind::Or
        }
        other => {
            return Err(LexError {
                line: lx.line,
                col: lx.col,
                msg: format!("unexpected character `{}`", other),
            });
        }
    };
    Ok(kind)
}