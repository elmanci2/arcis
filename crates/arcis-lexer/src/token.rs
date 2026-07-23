//! Token types for the Arcis lexer.
//!
//! Each token carries its source position (`line`, `col` in 1-based
//! coordinates) so downstream phases can report errors with precise
//! locations, like a TypeScript compiler error.

use std::fmt;

/// A single token emitted by the lexer.
#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub lexeme: String,
    pub line: usize,
    pub col: usize,
}

/// All token kinds recognised by the Arcis lexer.
#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    // ── Literals ────────────────────────────────────────────────────────────
    Number(f64),
    String(String),
    Bool(bool),

    // ── Identifiers ────────────────────────────────────────────────────────
    Ident(String),

    // ── Keywords (matching TypeScript) ─────────────────────────────────────
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
    Switch,
    Case,
    Try,
    Catch,
    Throw,

    // Modules (ES modules / TS).
    Import,
    Export,
    From,
    Default,
    As,

    // Type operators.
    Typeof,

    // Primitive type keywords when used as annotations.
    TypeString,
    TypeNumber,
    TypeBoolean,
    TypeVoid,
    TypeAny,

    // Type-system keywords.
    Type,
    Interface,
    Enum,
    Extends,
    Null,
    Undefined,

    // ── Operators ──────────────────────────────────────────────────────────
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Eq,
    EqEq,
    NotEq,
    Lt,
    Gt,
    LtEq,
    GtEq,
    And,
    Or,
    Bang,
    /// `|` — union type separator (not `||`).
    Bar,
    /// `&` — intersection type separator (not `&&`).
    Ampersand,
    /// `=>` — function type arrow.
    FatArrow,
    /// `?` — optional property / non-null narrowing marker.
    Question,
    /// `??` — nullish coalescing.
    QuestionQuestion,

    // ── Punctuation ────────────────────────────────────────────────────────
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Semi,
    Comma,
    Colon,
    ColonColon,
    Dot,
    /// `...` — spread in array/object literals.
    DotDotDot,

    Eof,
}

impl TokenKind {
    /// Returns `true` if this token is one of the reserved TypeScript-style
    /// keywords.
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
                | Self::Switch
                | Self::Case
                | Self::Try
                | Self::Catch
                | Self::Throw
                | Self::Import
                | Self::Export
                | Self::From
                | Self::Default
                | Self::As
                | Self::Typeof
                | Self::TypeString
                | Self::TypeNumber
                | Self::TypeBoolean
                | Self::TypeVoid
                | Self::TypeAny
                | Self::Type
                | Self::Interface
                | Self::Enum
                | Self::Extends
                | Self::Null
                | Self::Undefined
        )
    }
}

impl fmt::Display for TokenKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Number(n) => write!(f, "number `{}`", n),
            Self::String(s) => write!(f, "string `\"{}\"`", s),
            Self::Bool(b) => write!(f, "boolean `{}`", b),
            Self::Ident(s) => write!(f, "identifier `{}`", s),
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
            Self::Switch => write!(f, "`switch`"),
            Self::Case => write!(f, "`case`"),
            Self::Try => write!(f, "`try`"),
            Self::Catch => write!(f, "`catch`"),
            Self::Throw => write!(f, "`throw`"),
            Self::Import => write!(f, "`import`"),
            Self::Export => write!(f, "`export`"),
            Self::From => write!(f, "`from`"),
            Self::Default => write!(f, "`default`"),
            Self::As => write!(f, "`as`"),
            Self::Typeof => write!(f, "`typeof`"),
            Self::TypeString => write!(f, "`string`"),
            Self::TypeNumber => write!(f, "`number`"),
            Self::TypeBoolean => write!(f, "`boolean`"),
            Self::TypeVoid => write!(f, "`void`"),
            Self::TypeAny => write!(f, "`any`"),
            Self::Type => write!(f, "`type`"),
            Self::Interface => write!(f, "`interface`"),
            Self::Enum => write!(f, "`enum`"),
            Self::Extends => write!(f, "`extends`"),
            Self::Null => write!(f, "`null`"),
            Self::Undefined => write!(f, "`undefined`"),
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
            Self::Bar => write!(f, "`|`"),
            Self::Ampersand => write!(f, "`&`"),
            Self::FatArrow => write!(f, "`=>`"),
            Self::Question => write!(f, "`?`"),
            Self::QuestionQuestion => write!(f, "`??`"),
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
            Self::DotDotDot => write!(f, "`...`"),
            Self::Eof => write!(f, "end of file"),
        }
    }
}

