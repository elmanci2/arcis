# `arcis-fmt` — the Arcis source formatter

`arcis-fmt` normalizes `.tsr` source to a canonical Prettier-like style:
**2-space indentation, semicolons after statements, normalised spacing
around operators/keywords/punctuation, and comments preserved**.

It is used by `arcis fmt` (CLI) and `arcis-lsp` (textDocument/formatting
handler).

## Architecture: token-stream formatter

`arcis-fmt` operates on the **token stream** (not the AST). The lexer
(`arcis-lexer`) now exposes `lex_with_comments(source)` which returns
both real tokens and comments, each with 1-based source positions. The
formatter merges them into one position-ordered stream of `Item`s and
walks it with a small state machine.

**Why not AST-based?** Preserving comments with an AST formatter needs
source spans on every node (today only `let`/`const`/`param` carry
positions). Token-stream avoids that: comments ARE tokens, so they
survive the formatter natively.

The trade-off: token-stream can't re-flow long lines across breaks.
A future AST-based re-flow layer is compatible with the current pipeline
(the lexer + comment merge can feed either engine).

## Crate layout

```
crates/arcis-fmt/
├── Cargo.toml
├── src/
│   ├── lib.rs        # pub fn format(src) -> Result<String, LexError>
│   ├── stream.rs     # merge tokens + comments into Vec<Item>
│   └── emit.rs       # state machine that walks items and emits
└── tests/fmt.rs      # integration tests
```

## Lexer change: `lex_with_comments`

`crates/arcis-lexer/` was extended with a zero-breakage change:

- `lex(source)` continues to work identically (discards comments).
- `lex_with_comments(source)` returns `(Vec<Token>, Vec<CommentToken>)`.
  `CommentToken { line, col, end_line, text, is_block }` carries the raw
  comment text (including delimiters) and its 1-based position.
- `lex(src)` is now implemented as `lex_with_comments(src).map(|(t,_)| t)`.

The scanner's `skip_whitespace_and_comments` now takes a `&mut
Vec<CommentToken>` collector. Nothing else changed — the parser and
codegen continue to receive only real tokens.

## Formatting rules (state machine)

The emitter in `emit.rs` walks `Item`s in order and applies decisions:

- **Separator before each item**: `None`, `Space`, `Newline`, or
  `BlankThenIndent` (original source had a blank line → preserve it,
  capped at one).
- **Indentation**: derived from bracket depth. When a `{` opens a block
  (detected as "preceded by `)`, `else`, or a type keyword"), indent
  increments by 1 (2 spaces). On the matching `}`, indent decrements
  before emitting the brace's line.
- **Spacing table**: a large `match (prev_token, current_token)` that
  returns the separator. Specific no-space rules (`.`, `::`, `(` after
  call, `[` after type/ident) come first; keyword→space and binary-op
  rules follow; a default `Space` catches the rest.

See `fn separator(...)` in `emit.rs` for the full table.

## Tuning a formatting rule

1. Open `crates/arcis-fmt/src/emit.rs`.
2. Find the `let sep = match (prev_tok.clone(), cur.clone())` block in
   `fn separator`.
3. Add or modify a match arm. Arms are grouped: **no-space** first,
   **newline** second, **space-after** third, **default** last.
4. Run `cargo test -p arcis-fmt` to confirm existing tests still pass.
5. Add or update a test in `crates/arcis-fmt/tests/fmt.rs`.

## Limitations (v1)

- Long lines are NOT broken — a 200-character `if` condition stays on
  one line. Re-flow is planned for v2 (AST-based).
- Object/array literals are kept inline. Multi-line expansion for long
  literals is deferred.
- C-style `for (init; cond; update)` headers are not tested (Arcis apps
  typically use `for ... of`). The `;` in for-headers is detected via
  the bracket stack, so it should work; just untested.
- Import order is not sorted.
- No configuration file (`.arcisfmt`); defaults are 2-space indent +
  semicolons.
