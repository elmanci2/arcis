//! The formatter's emission state machine.
//!
//! Walks the position-ordered [`Item`] stream and emits canonical
//! (Prettier-like) Arcis source. The machine tracks:
//!
//! - `indent`: the current indentation depth (in 2-space units).
//! - `stack`: what brackets we're nested in, so we know whether a `}`
//!   closes a block (needs newline + dedent) or an object literal
//!   (inline).
//! - `prev`: the kind of the last significant token emitted, so we can
//!   decide the separator before the next one.
//!
//! Bracket handling is done **inline in the main loop** (not in a helper)
//! so the indent mutation always happens at the right moment:
//! - Opening a block `{`: emit `{`, then bump indent and push a `Block`
//!   frame; the *next* item gets a `Newline` separator.
//! - Closing a block `}`: dedent and pop the frame *before* indenting
//!   the `}` line, so the brace lines up with the opener.

use arcis_lexer::TokenKind;

use crate::stream::{Item, ItemKind};

/// One level of indentation = this many spaces.
const INDENT_SPACES: usize = 2;

/// What kind of bracket frame we're inside.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Frame {
    /// A `{ ... }` block (if/while/for/function body). Indented.
    Block,
    /// A `{ ... }` object literal. Kept inline.
    Object,
    /// A `( ... )` group or call argument list. Kept inline.
    Paren,
    /// A `[ ... ]` index or array literal. Kept inline.
    Bracket,
}

/// The separator to emit *before* the next item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sep {
    /// Nothing (e.g. between `.` and an identifier).
    None,
    /// A single space.
    Space,
    /// A newline, then indent to the current depth.
    Newline,
    /// A blank line followed by an indented line (Prettier caps blank
    /// lines at one).
    BlankThenIndent,
}

pub fn run(items: &[Item]) -> String {
    let mut out = String::new();
    let mut indent: usize = 0;
    let mut stack: Vec<Frame> = Vec::new();
    let mut prev: Option<TokenKind> = None;
    // True if the last emitted thing was a line comment (forces a
    // newline after it).
    let mut last_was_line_comment = false;
    // Whether we're at column 0 of the output (indentation not yet
    // emitted for this line).
    let mut at_line_start = true;
    // 1-based source end_line of the last emitted item, for blank-line
    // gap detection.
    let mut last_end_line: usize = 0;

    for item in items {
        // The Eof sentinel just finalizes the output.
        if matches!(item.kind, ItemKind::Tok(TokenKind::Eof)) {
            break;
        }

        // ── Compute the separator before this item, using the
        //    CURRENT state (before any bracket mutations). This is
        //    critical: the separator for a closing `}` must see the
        //    Block frame still on the stack so it knows to force a
        //    newline + dedent.
        let is_block_close = matches!(item.kind, ItemKind::Tok(TokenKind::RBrace))
            && matches!(stack.last(), Some(Frame::Block));

        let sep = separator(
            prev.clone(),
            last_was_line_comment,
            &item.kind,
            &stack,
            is_block_close,
            item.line,
            last_end_line,
            at_line_start,
        );

        // ── For block closes, dedent and pop the frame BEFORE
        //    applying indentation for the `}` line.
        if is_block_close {
            indent = indent.saturating_sub(1);
            stack.pop();
        }

        // Apply the separator.
        match sep {
            Sep::None => {}
            Sep::Space => {
                if !at_line_start {
                    out.push(' ');
                }
            }
            Sep::Newline => {
                if !out.is_empty() && !out.ends_with('\n') {
                    out.push('\n');
                }
                at_line_start = true;
            }
            Sep::BlankThenIndent => {
                if !out.is_empty() {
                    if !out.ends_with('\n') {
                        out.push('\n');
                    }
                    // Ensure exactly one blank line (collapse runs).
                    if !out.ends_with("\n\n") {
                        out.push('\n');
                    }
                }
                at_line_start = true;
            }
        }

        // Emit indentation if we're starting a fresh line.
        if at_line_start {
            push_indent(&mut out, indent);
            at_line_start = false;
        }

        // Emit the item itself, updating bracket state for openers.
        match &item.kind {
            ItemKind::Tok(kind) => {
                match kind {
                    TokenKind::LBrace => {
                        let is_block = matches!(
                            prev,
                            Some(TokenKind::RParen)
                                | Some(TokenKind::Else)
                                | Some(TokenKind::TypeString)
                                | Some(TokenKind::TypeNumber)
                                | Some(TokenKind::TypeBoolean)
                                | Some(TokenKind::TypeVoid)
                        );
                        out.push('{');
                        if is_block {
                            stack.push(Frame::Block);
                            indent += 1;
                        } else {
                            stack.push(Frame::Object);
                        }
                    }
                    TokenKind::RBrace => {
                        // Frame was already popped above if it was a
                        // block; pop object frames here.
                        if matches!(stack.last(), Some(Frame::Object)) {
                            stack.pop();
                        }
                        out.push('}');
                    }
                    TokenKind::LParen => {
                        out.push('(');
                        stack.push(Frame::Paren);
                    }
                    TokenKind::RParen => {
                        out.push(')');
                        if matches!(stack.last(), Some(Frame::Paren)) {
                            stack.pop();
                        }
                    }
                    TokenKind::LBracket => {
                        out.push('[');
                        stack.push(Frame::Bracket);
                    }
                    TokenKind::RBracket => {
                        out.push(']');
                        if matches!(stack.last(), Some(Frame::Bracket)) {
                            stack.pop();
                        }
                    }
                    _ => out.push_str(&token_text(kind)),
                }
                prev = Some(kind.clone());
                last_was_line_comment = false;
            }
            ItemKind::LineComment(text) => {
                out.push_str(text);
                last_was_line_comment = true;
            }
            ItemKind::BlockComment(text) => {
                out.push_str(text);
                last_was_line_comment = false;
            }
        }

        last_end_line = item.end_line;
    }

    // Ensure exactly one trailing newline (even for empty input).
    if out.is_empty() {
        out.push('\n');
    } else if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

fn push_indent(out: &mut String, indent: usize) {
    for _ in 0..indent * INDENT_SPACES {
        out.push(' ');
    }
}

/// Decide the separator before `current`.
#[allow(clippy::too_many_arguments)]
fn separator(
    prev: Option<TokenKind>,
    last_was_line_comment: bool,
    current: &ItemKind,
    stack: &[Frame],
    is_block_close: bool,
    current_line: usize,
    last_end_line: usize,
    at_line_start: bool,
) -> Sep {
    use TokenKind::*;

    // A line comment always ends its line.
    if last_was_line_comment {
        return Sep::Newline;
    }

    // ── Comments as the current item ──────────────────────────────
    if let ItemKind::LineComment(_) = current {
        if at_line_start {
            return Sep::None;
        }
        return Sep::Space;
    }
    if let ItemKind::BlockComment(_) = current {
        if at_line_start {
            return Sep::None;
        }
        return Sep::Space;
    }

    let cur = match current {
        ItemKind::Tok(k) => k,
        _ => unreachable!(),
    };

    // Closing `}` of a block: always on its own line. The caller
    // passes `is_block_close` based on the stack state at separator-
    // computation time (BEFORE dedenting).
    if is_block_close {
        return Sep::Newline;
    }

    let prev_tok = match prev {
        Some(k) => k,
        None => return Sep::None,
    };

    let sep = match (prev_tok.clone(), cur.clone()) {
        // ── No-space rules (most specific first) ──────────────────
        (_, Semi) => Sep::None,
        (_, Comma) => Sep::None,
        (_, Colon) => Sep::None,
        (_, Dot) => Sep::None,
        (_, ColonColon) => Sep::None,
        (Dot, _) => Sep::None,
        (ColonColon, _) => Sep::None,
        (LParen, _) => Sep::None,
        (LBracket, _) => Sep::None,
        (_, RParen) => Sep::None,
        (_, RBracket) => Sep::None,
        (Bang, _) => Sep::None,
        // No space before `[` in `number[]`, `x[0]`, etc.
        (Ident(_) | Number(_) | String(_) | Bool(_), LParen) => Sep::None,
        (Ident(_) | Number(_) | String(_) | Bool(_) | TypeString | TypeNumber | TypeBoolean
        | TypeVoid | RParen | RBracket, LBracket) => Sep::None,

        // ── Newline rules ──────────────────────────────────────
        (Semi, _) => {
            if in_for_header(stack) {
                Sep::Space
            } else {
                Sep::Newline
            }
        }

        // ── Space-after rules ──────────────────────────────────
        (Comma, _) => Sep::Space,
        (Colon, _) => Sep::Space,
        // Binary operators: space both sides.
        (Plus | Minus | Star | Slash | Percent | EqEq | NotEq | Lt | Gt | LtEq | GtEq | And | Or, _) => Sep::Space,
        (_, Plus | Minus | Star | Slash | Percent | EqEq | NotEq | Lt | Gt | LtEq | GtEq | And | Or) => Sep::Space,
        // Assignment.
        (Eq, _) => Sep::Space,
        (_, Eq) => Sep::Space,
        // `)` / `else` then `{`: space.
        (RParen | Else, LBrace) => Sep::Space,
        // `}` then `else`: keep on same line (`} else`).
        (RBrace, Else) => Sep::Space,
        // Keywords / types: space before what follows.
        (Let | Const | Function | Return | If | Else | While | For | Of | Break | Continue
        | Import | Export | From | Default | As | TypeString | TypeNumber | TypeBoolean
        | TypeVoid, _) => Sep::Space,
        // After `)` / `]`: space before the next token.
        (RParen | RBracket, _) => Sep::Space,

        // ── Default ─────────────────────────────────────────────
        _ => Sep::Space,
    };

    // After a block-opening `{`, start the body on a new line.
    if let LBrace = prev_tok {
        if matches!(stack.last(), Some(Frame::Block)) {
            return Sep::Newline;
        }
        // Object literal: space after `{`.
        return Sep::Space;
    }

    // Respect original blank lines between statements: if the source had
    // a >=2 line gap and we're starting a new statement, emit one blank.
    if sep == Sep::Newline && last_end_line != 0 {
        let gap = current_line.saturating_sub(last_end_line);
        if gap >= 2 {
            return Sep::BlankThenIndent;
        }
    }

    // Suppress a leading space at the start of a fresh line.
    if at_line_start && sep == Sep::Space {
        return Sep::None;
    }

    sep
}

/// Are we inside a `for (init; cond; update)` header? Approximated by
/// the top of the stack being a `Paren` frame (calls don't use `;`).
fn in_for_header(stack: &[Frame]) -> bool {
    matches!(stack.last(), Some(Frame::Paren))
}

/// Render a token's source text.
fn token_text(kind: &TokenKind) -> String {
    use TokenKind::*;
    match kind {
        Let => "let".into(),
        Const => "const".into(),
        Function => "function".into(),
        Return => "return".into(),
        If => "if".into(),
        Else => "else".into(),
        While => "while".into(),
        For => "for".into(),
        Of => "of".into(),
        Break => "break".into(),
        Continue => "continue".into(),
        Import => "import".into(),
        Export => "export".into(),
        From => "from".into(),
        Default => "default".into(),
        As => "as".into(),
        Typeof => "typeof".into(),
        TypeString => "string".into(),
        TypeNumber => "number".into(),
        TypeBoolean => "boolean".into(),
        TypeVoid => "void".into(),
        LParen => "(".into(),
        RParen => ")".into(),
        LBrace => "{".into(),
        RBrace => "}".into(),
        LBracket => "[".into(),
        RBracket => "]".into(),
        Semi => ";".into(),
        Comma => ",".into(),
        Colon => ":".into(),
        ColonColon => "::".into(),
        Dot => ".".into(),
        Plus => "+".into(),
        Minus => "-".into(),
        Star => "*".into(),
        Slash => "/".into(),
        Percent => "%".into(),
        Eq => "=".into(),
        EqEq => "==".into(),
        NotEq => "!=".into(),
        Lt => "<".into(),
        Gt => ">".into(),
        LtEq => "<=".into(),
        GtEq => ">=".into(),
        And => "&&".into(),
        Or => "||".into(),
        Bang => "!".into(),
        Ident(s) => s.clone(),
        Number(n) => format_number(*n),
        String(s) => format!("\"{}\"", escape_string(s)),
        Bool(b) => if *b { "true" } else { "false" }.into(),
        Eof => "".to_string(),
    }
}

fn format_number(n: f64) -> String {
    if n.fract() == 0.0 && n.is_finite() {
        format!("{}", n as i64)
    } else {
        format!("{}", n)
    }
}

fn escape_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out
}