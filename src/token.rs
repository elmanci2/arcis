//! Tokens del lenguaje.
//!
//! Cada token lleva su posición (line, col) en el archivo fuente original,
//! para reportar errores con ubicación precisa estilo compilador de TS.

use std::fmt;

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub lexeme: String,
    pub line: usize,
    pub col: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    // Literales
    Number(f64),
    String(String),
    Bool(bool),

    // Identificador
    Ident(String),

    // Palabras clave (idénticas a TS)
    Let,
    Const,
    Function,
    Return,
    If,
    Else,
    While,
    For,
    Of,
    Break,
    Continue,

    // Módulos (idénticos a TS: import/export/from/default/as)
    Import,
    Export,
    From,
    Default,
    As,

    // Tipos primitivos (cuando aparecen como anotación)
    TypeString,
    TypeNumber,
    TypeBoolean,
    TypeVoid,

    // Operadores
    Plus,         // +
    Minus,        // -
    Star,         // *
    Slash,        // /
    Percent,      // %
    Eq,           // =
    EqEq,         // ==
    NotEq,        // !=
    Lt,           // <
    Gt,           // >
    LtEq,         // <=
    GtEq,         // >=
    And,          // &&
    Or,           // ||
    Bang,         // !

    // Puntuación
    LParen,    // (
    RParen,    // )
    LBrace,    // {
    RBrace,    // }
    LBracket,  // [
    RBracket,  // ]
    Semi,      // ;
    Comma,     // ,
    Colon,     // :
    ColonColon,// :: (path-qualified, p.ej. `reqwest::Client::new()`)
    Dot,       // .

    Eof,
}

impl TokenKind {
    /// Devuelve true si el token es un keyword de TS reservado.
    pub fn is_keyword(&self) -> bool {
        matches!(
            self,
            Self::Let
                | Self::Const
                | Self::Function
                | Self::Return
                | Self::If
                | Self::Else
                | Self::While
                | Self::For
                | Self::Of
                | Self::Break
                | Self::Continue
                | Self::Import
                | Self::Export
                | Self::From
                | Self::Default
                | Self::As
                | Self::TypeString
                | Self::TypeNumber
                | Self::TypeBoolean
                | Self::TypeVoid
        )
    }
}

impl fmt::Display for TokenKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Number(n) => write!(f, "número `{}`", n),
            Self::String(s) => write!(f, "string `\"{}\"`", s),
            Self::Bool(b) => write!(f, "booleano `{}`", b),
            Self::Ident(s) => write!(f, "identificador `{}`", s),
            Self::Let => write!(f, "`let`"),
            Self::Const => write!(f, "`const`"),
            Self::Function => write!(f, "`function`"),
            Self::Return => write!(f, "`return`"),
            Self::If => write!(f, "`if`"),
            Self::Else => write!(f, "`else`"),
            Self::While => write!(f, "`while`"),
            Self::For => write!(f, "`for`"),
            Self::Of => write!(f, "`of`"),
            Self::Break => write!(f, "`break`"),
            Self::Continue => write!(f, "`continue`"),
            Self::Import => write!(f, "`import`"),
            Self::Export => write!(f, "`export`"),
            Self::From => write!(f, "`from`"),
            Self::Default => write!(f, "`default`"),
            Self::As => write!(f, "`as`"),
            Self::TypeString => write!(f, "`string`"),
            Self::TypeNumber => write!(f, "`number`"),
            Self::TypeBoolean => write!(f, "`boolean`"),
            Self::TypeVoid => write!(f, "`void`"),
            Self::Plus => write!(f, "`+`"),
            Self::Minus => write!(f, "`-`"),
            Self::Star => write!(f, "`*`"),
            Self::Slash => write!(f, "`/`"),
            Self::Percent => write!(f, "`%`"),
            Self::Eq => write!(f, "`=`"),
            Self::EqEq => write!(f, "`==`"),
            Self::NotEq => write!(f, "`!=`"),
            Self::Lt => write!(f, "`<`"),
            Self::Gt => write!(f, "`>`"),
            Self::LtEq => write!(f, "`<=`"),
            Self::GtEq => write!(f, "`>=`"),
            Self::And => write!(f, "`&&`"),
            Self::Or => write!(f, "`||`"),
            Self::Bang => write!(f, "`!`"),
            Self::LParen => write!(f, "`(`"),
            Self::RParen => write!(f, "`)`"),
            Self::LBrace => write!(f, "`{{`"),
            Self::RBrace => write!(f, "`}}`"),
            Self::LBracket => write!(f, "`[`"),
            Self::RBracket => write!(f, "`]`"),
            Self::Semi => write!(f, "`;`"),
            Self::Comma => write!(f, "`,`"),
            Self::Colon => write!(f, "`:`"),
            Self::ColonColon => write!(f, "`::`"),
            Self::Dot => write!(f, "`.`"),
            Self::Eof => write!(f, "fin de archivo"),
        }
    }
}