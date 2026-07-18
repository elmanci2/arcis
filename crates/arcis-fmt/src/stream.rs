//! Merging real tokens and captured comments into one position-ordered
//! stream of [`Item`]s that the emitter walks.
//!
//! Both tokens (`Token { line, col, .. }`) and comments
//! (`CommentToken { line, col, .. }`) carry 1-based positions, so a
//! simple merge on `(line, col)` reconstructs the original source order.

use arcis_lexer::{CommentToken, Token, TokenKind};

/// A single unit the formatter walks: either a real token or a comment.
#[derive(Debug, Clone)]
pub struct Item {
    pub kind: ItemKind,
    /// 1-based source line.
    pub line: usize,
    /// 1-based source column.
    pub col: usize,
    /// For a line comment, the source line on which it ends (same as
    /// `line`). For a block comment, the last line it spans. Used by
    /// the emitter to detect blank-line gaps between items.
    pub end_line: usize,
}

#[derive(Debug, Clone)]
pub enum ItemKind {
    /// A real lexer token (the `Eof` sentinel is included as the last
    /// item so the emitter can finalize trailing newlines).
    Tok(TokenKind),
    /// A `// ...` line comment (raw text, including the leading `//`).
    LineComment(String),
    /// A `/* ... */` block comment (raw text, including delimiters).
    BlockComment(String),
}

/// Merge `tokens` and `comments` into a single [`Vec<Item>`] ordered by
/// source position. The trailing `Eof` token is preserved as the last
/// item.
pub fn merge(tokens: Vec<Token>, comments: Vec<CommentToken>) -> Vec<Item> {
    // Both inputs are already in source order. Merge them by (line, col).
    let mut out = Vec::with_capacity(tokens.len() + comments.len());
    let mut ti = 0;
    let mut ci = 0;
    while ti < tokens.len() && ci < comments.len() {
        let t = &tokens[ti];
        let c = &comments[ci];
        if (t.line, t.col) <= (c.line, c.col) {
            out.push(Item {
                kind: ItemKind::Tok(t.kind.clone()),
                line: t.line,
                col: t.col,
                end_line: t.line,
            });
            ti += 1;
        } else {
            out.push(Item {
                kind: if c.is_block {
                    ItemKind::BlockComment(c.text.clone())
                } else {
                    ItemKind::LineComment(c.text.clone())
                },
                line: c.line,
                col: c.col,
                end_line: c.end_line,
            });
            ci += 1;
        }
    }
    while ti < tokens.len() {
        let t = &tokens[ti];
        out.push(Item {
            kind: ItemKind::Tok(t.kind.clone()),
            line: t.line,
            col: t.col,
            end_line: t.line,
        });
        ti += 1;
    }
    while ci < comments.len() {
        let c = &comments[ci];
        out.push(Item {
            kind: if c.is_block {
                ItemKind::BlockComment(c.text.clone())
            } else {
                ItemKind::LineComment(c.text.clone())
            },
            line: c.line,
            col: c.col,
            end_line: c.end_line,
        });
        ci += 1;
    }
    out
}
