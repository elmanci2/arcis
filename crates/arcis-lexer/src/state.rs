//! Lexer state machine.
//!
//! Holds the source as a flat `Vec<char>` plus a cursor pair: `pos` marks the
//! start of the current token, `cur` is the next character to consume. The
//! `line` / `col` pair is updated by [`advance`] so downstream errors carry
//! accurate positions.
//!
//! The state machine is intentionally minimal — it provides peek/advance/slice
//! primitives and lets the [`scanner`](super::scanner) module decide what to
//! do with the characters it reads.

/// The lexer cursor and bookkeeping.
pub(crate) struct Lexer<'a> {
    chars: Vec<char>,
    /// Position of the start of the current token.
    pos: usize,
    /// Position of the next character to consume.
    cur: usize,
    pub line: usize,
    pub col: usize,
    #[allow(dead_code)]
    src: &'a str,
}

impl<'a> Lexer<'a> {
    pub(crate) fn new(source: &'a str) -> Self {
        Self {
            chars: source.chars().collect(),
            pos: 0,
            cur: 0,
            line: 1,
            col: 1,
            src: source,
        }
    }

    pub(crate) fn is_eof(&self) -> bool {
        self.cur >= self.chars.len()
    }

    pub(crate) fn peek_char(&self) -> Option<char> {
        self.chars.get(self.cur).copied()
    }

    pub(crate) fn peek_char_at(&self, offset: usize) -> Option<char> {
        self.chars.get(self.cur + offset).copied()
    }

    /// Consume the next character and update `line` / `col`.
    pub(crate) fn advance(&mut self) -> Option<char> {
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

    /// Return the slice of source from `pos` to `cur` as a `String`. This is
    /// the lexeme of the current token.
    pub(crate) fn slice_current(&self) -> String {
        self.chars[self.pos..self.cur].iter().collect()
    }

    /// Mark the start of a new token at the current cursor.
    pub(crate) fn start_token(&mut self) {
        self.pos = self.cur;
    }
}