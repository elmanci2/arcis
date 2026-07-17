//! Lexer: convierte el texto fuente en una secuencia de [`Token`].
//!
//! Implementación hand-rolled estilo "dragon book" — un solo paso sobre los
//! caracteres, con lookahead implícito (`peek_char`).
//!
//! Soporta:
//! - Comentarios de línea `//` y bloque `/* ... */`
//! - Strings con comillas dobles y escapes `\n`, `\t`, `\\`, `\"`
//! - Números enteros y decimales
//! - Identificadores y las palabras clave de TS

use crate::token::{Token, TokenKind};
use std::collections::HashMap;

/// Tabla de palabras clave (mismas que TypeScript, en minúsculas).
fn keywords() -> HashMap<&'static str, TokenKind> {
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
    // Módulos (ES modules / TS).
    m.insert("import", TokenKind::Import);
    m.insert("export", TokenKind::Export);
    m.insert("from", TokenKind::From);
    m.insert("default", TokenKind::Default);
    m.insert("as", TokenKind::As);
    // `true` y `false` son keywords pero también literales booleanos: los
    // emitimos como `TokenKind::Bool(_)` para que el parser los trate igual.
    m.insert("true", TokenKind::Bool(true));
    m.insert("false", TokenKind::Bool(false));
    m.insert("string", TokenKind::TypeString);
    m.insert("number", TokenKind::TypeNumber);
    m.insert("boolean", TokenKind::TypeBoolean);
    m.insert("void", TokenKind::TypeVoid);
    m
}

pub fn lex(source: &str) -> Result<Vec<Token>, LexError> {
    let mut lx = Lexer::new(source);
    let mut tokens = Vec::new();

    loop {
        lx.skip_whitespace_and_comments();
        if lx.is_eof() {
            tokens.push(Token {
                kind: TokenKind::Eof,
                lexeme: String::new(),
                line: lx.line,
                col: lx.col,
            });
            return Ok(tokens);
        }

        let line = lx.line;
        let col = lx.col;
        let c = lx.peek_char().unwrap();

        let kind = if is_ident_start(c) {
            lx.read_ident()?
        } else if c.is_ascii_digit() {
            lx.read_number()?
        } else if c == '"' {
            lx.read_string()?
        } else {
            lx.read_punct_or_op()?
        };

        let lexeme = lx.slice_current();
        tokens.push(Token { kind, lexeme, line, col });
    }
}

struct Lexer<'a> {
    chars: Vec<char>,
    pos: usize,   // posición de "inicio" del token actual
    cur: usize,   // posición del siguiente char a consumir
    line: usize,
    col: usize,
    _src: &'a str,
}

impl<'a> Lexer<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            chars: source.chars().collect(),
            pos: 0,
            cur: 0,
            line: 1,
            col: 1,
            _src: source,
        }
    }

    fn is_eof(&self) -> bool {
        self.cur >= self.chars.len()
    }

    fn peek_char(&self) -> Option<char> {
        self.chars.get(self.cur).copied()
    }

    fn peek_char_at(&self, offset: usize) -> Option<char> {
        self.chars.get(self.cur + offset).copied()
    }

    fn advance(&mut self) -> Option<char> {
        let c = self.chars.get(self.cur).copied()?;
        self.cur += 1;
        if c == '\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        Some(c)
    }

    fn slice_current(&self) -> String {
        self.chars[self.pos..self.cur].iter().collect()
    }

    fn start_token(&mut self) {
        self.pos = self.cur;
    }

    fn skip_whitespace_and_comments(&mut self) {
        loop {
            self.start_token();
            if self.is_eof() {
                return;
            }
            match self.peek_char().unwrap() {
                ' ' | '\t' | '\r' | '\n' => {
                    self.advance();
                }
                '/' if self.peek_char_at(1) == Some('/') => {
                    while let Some(c) = self.peek_char() {
                        if c == '\n' {
                            break;
                        }
                        self.advance();
                    }
                }
                '/' if self.peek_char_at(1) == Some('*') => {
                    self.advance(); // /
                    self.advance(); // *
                    while let Some(c) = self.peek_char() {
                        if c == '*' && self.peek_char_at(1) == Some('/') {
                            self.advance();
                            self.advance();
                            break;
                        }
                        self.advance();
                    }
                }
                _ => return,
            }
        }
    }

    fn read_ident(&mut self) -> Result<TokenKind, LexError> {
        while let Some(c) = self.peek_char() {
            if is_ident_continue(c) {
                self.advance();
            } else {
                break;
            }
        }
        let text = self.slice_current();
        let kw = keywords();
        Ok(kw.get(text.as_str()).cloned().unwrap_or(TokenKind::Ident(text)))
    }

    fn read_number(&mut self) -> Result<TokenKind, LexError> {
        while let Some(c) = self.peek_char() {
            if c.is_ascii_digit() {
                self.advance();
            } else {
                break;
            }
        }
        // Parte decimal opcional
        if self.peek_char() == Some('.') && self.peek_char_at(1).map(|c| c.is_ascii_digit()).unwrap_or(false) {
            self.advance(); // .
            while let Some(c) = self.peek_char() {
                if c.is_ascii_digit() {
                    self.advance();
                } else {
                    break;
                }
            }
        }
        let text = self.slice_current();
        let n: f64 = text.parse().map_err(|_| LexError {
            line: self.line,
            col: self.col,
            msg: format!("número inválido `{}`", text),
        })?;
        Ok(TokenKind::Number(n))
    }

    fn read_string(&mut self) -> Result<TokenKind, LexError> {
        // consume la comilla de apertura
        self.advance(); // "
        let mut s = String::new();
        loop {
            match self.peek_char() {
                None => {
                    return Err(LexError {
                        line: self.line,
                        col: self.col,
                        msg: "string sin cerrar".to_string(),
                    });
                }
                Some('"') => {
                    self.advance();
                    break;
                }
                Some('\\') => {
                    self.advance();
                    match self.peek_char() {
                        Some('n') => { self.advance(); s.push('\n'); }
                        Some('t') => { self.advance(); s.push('\t'); }
                        Some('r') => { self.advance(); s.push('\r'); }
                        Some('\\') => { self.advance(); s.push('\\'); }
                        Some('"') => { self.advance(); s.push('"'); }
                        Some(c) => {
                            return Err(LexError {
                                line: self.line,
                                col: self.col,
                                msg: format!("escape inválido `\\{}`", c),
                            });
                        }
                        None => {
                            return Err(LexError {
                                line: self.line,
                                col: self.col,
                                msg: "string sin cerrar".to_string(),
                            });
                        }
                    }
                }
                Some(c) => {
                    self.advance();
                    s.push(c);
                }
            }
        }
        Ok(TokenKind::String(s))
    }

    fn read_punct_or_op(&mut self) -> Result<TokenKind, LexError> {
        let c = self.advance().unwrap();
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
                if self.peek_char() == Some(':') {
                    self.advance();
                    TokenKind::ColonColon
                } else {
                    TokenKind::Colon
                }
            }
            '.' => TokenKind::Dot,
            '=' => {
                if self.peek_char() == Some('=') {
                    self.advance();
                    TokenKind::EqEq
                } else {
                    TokenKind::Eq
                }
            }
            '!' => {
                if self.peek_char() == Some('=') {
                    self.advance();
                    TokenKind::NotEq
                } else {
                    TokenKind::Bang
                }
            }
            '<' => {
                if self.peek_char() == Some('=') {
                    self.advance();
                    TokenKind::LtEq
                } else {
                    TokenKind::Lt
                }
            }
            '>' => {
                if self.peek_char() == Some('=') {
                    self.advance();
                    TokenKind::GtEq
                } else {
                    TokenKind::Gt
                }
            }
            '&' if self.peek_char() == Some('&') => {
                self.advance();
                TokenKind::And
            }
            '|' if self.peek_char() == Some('|') => {
                self.advance();
                TokenKind::Or
            }
            other => {
                return Err(LexError {
                    line: self.line,
                    col: self.col,
                    msg: format!("carácter inesperado `{}`", other),
                });
            }
        };
        Ok(kind)
    }
}

fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_' || c == '$'
}

fn is_ident_continue(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '$'
}

#[derive(Debug)]
pub struct LexError {
    pub line: usize,
    pub col: usize,
    pub msg: String,
}

impl std::fmt::Display for LexError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "error léxico en {}:{}: {}", self.line, self.col, self.msg)
    }
}

impl std::error::Error for LexError {}