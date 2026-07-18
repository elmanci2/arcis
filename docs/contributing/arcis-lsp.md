# `arcis-lsp` — the Arcis Language Server

`arcis-lsp` is a binary that speaks the Language Server Protocol (LSP)
on stdin/stdout and provides completion, hover, and diagnostics for
`.tsr` files. The companion VS Code extension launches it as a child
process.

This document explains how the LSP is structured and how to extend it.

## Crate layout

```
crates/arcis-lsp/
├── Cargo.toml
├── src/
│   ├── lib.rs        # re-exports; alias `lsp` for `async_lsp::lsp_types`
│   ├── main.rs       # binary: stdin/stdout + `MainLoop::new_server`
│   ├── builtins.rs   # static table of every completion / hover candidate
│   ├── completion.rs # context detection + completion items
│   ├── hover.rs      # identifier + chain label lookup
│   ├── diagnostics.rs# lexer + parser pipeline → LSP diagnostics
│   └── server.rs     # `Router` wiring initialize / completion / didOpen / didChange
└── tests/            # unit tests per provider
```

The LSP uses `async-lsp` 0.2 (not `tower-lsp`) because `tower-lsp` 0.20
has a compile error on Rust 1.93 and `tower-lsp-f` (the maintained fork)
has the same issue. `async-lsp` is the only LSP framework that compiles
cleanly today.

## Builtin table

The heart of the LSP is `src/builtins.rs`. It defines a hard-coded
table of every completion / hover candidate:

- `KEYWORDS` — every reserved keyword + the four primitive type names.
- `TOP_LEVEL_BUILTINS` — `print`, `input`, and every top-level `sys.X`
  (~30 entries).
- `SYS_NAMESPACES` — names that, after `sys.`, lead to a sub-namespace
  (`env`, `os`, `memory`, …).
- `NS_METHODS` — keyed map from namespace name to its methods.
- `ARRAY_METHODS` / `STRING_METHODS` — chained methods on `<expr>.`.

Each entry is a `Builtin { label, kind, detail, documentation }`. The
`find_by_label` and `ns_methods` helpers in the same module are the
only entry points used by the providers.

## Adding a new builtin

1. Edit `crates/arcis-codegen/src/sys/<ns>.rs` to add the Rust emit
   logic for the new `sys.X` or `sys.<ns>.<method>` call.
2. Edit `crates/arcis-lsp/src/builtins.rs` to add a `Builtin` entry.
   - For top-level `sys.X`: add to `TOP_LEVEL_BUILTINS`.
   - For a new sub-namespace: add to `SYS_NAMESPACES` and create a new
     `NS_METHODS` entry.
   - For a new method on an existing namespace: add to the relevant
     `&[...]` slice inside `NS_METHODS`.
3. Add a test in `crates/arcis-lsp/tests/completion.rs` asserting the
   new label appears in the right context.
4. If you added a top-level `sys.X` with `args` (not a member access),
   the completion provider also needs to know — but currently it
   auto-discovers them via `TOP_LEVEL_BUILTINS`, so nothing else
   changes.

## Completion provider

`src/completion.rs` exposes `completions_at(text_before_cursor)`,
which returns a `Vec<CompletionItem>`. The provider:

1. Walks back from the cursor collecting the `.<identifier>` chain.
2. Reverses the chain to left-to-right order.
3. Matches the chain against patterns:
   - `[]` → `KEYWORDS` + `print`/`input` + `TOP_LEVEL_BUILTINS`.
   - `["sys"]` → `SYS_NAMESPACES` + `TOP_LEVEL_BUILTINS`.
   - `["sys", ns]` → `NS_METHODS[ns]`.
   - `[known-ns]` (without `sys`) → `TOP_LEVEL_BUILTINS` (typo recovery).
   - `[anything-else]` → `ARRAY_METHODS` + `STRING_METHODS`.
4. Builds `CompletionItem`s for each match.

## Hover provider

`src/hover.rs` exposes `hover_at(before, after)` which returns an
`Option<Hover>`. It:

1. Extracts the bare identifier under the cursor from `before` (the text
   up to the cursor) and `after` (the text from the cursor).
2. Extracts the full chain label (e.g. `sys.readFile` when the cursor
   is between `sys.read` and `File(p)`).
3. Looks up the label in `find_by_label`; if not found, falls back to
   the bare identifier.
4. Returns a `Hover` with a `MarkupContent::Markdown` body containing
   the `detail` and `documentation` fields.

The chain label builder is finicky: the inner `cut` and the partial
identifier `partial_end` must be captured in the right order. See the
`chain_label_at` function for the details.

## Diagnostics provider

`src/diagnostics.rs` exposes `diagnostics_for(text)` which runs:

1. `arcis_lexer::lex(text)`. If the lexer errors, we surface a
   whole-document diagnostic.
2. `arcis_parser::parse(tokens)`. Parse errors carry `line` and `col`
   in 1-indexed form; we translate to LSP's 0-indexed `Range`.

`arcis-validation` is NOT wired in yet. The validator's reports don't
currently carry `line` info compatible with LSP `Range`; doing this
cleanly is a follow-up that depends on the validator's structured
output stabilising.

## Server wiring

`src/server.rs` constructs a `Router<ServerState>` and registers:

- `request::Initialize` — returns `ServerCapabilities` (hover +
  completion with `.` and `:` as trigger characters + text document
  sync FULL).
- `request::Shutdown` — returns `Ok(())`.
- `request::Completion` — returns `completions_at("")` (the document
  text isn't available in this request; the editor's "fetch on
  trigger" path repopulates with context after a `didChange`).
- `notification::Initialized` — `ControlFlow::Continue(())` to
  acknowledge the LSP lifecycle.
- `notification::DidOpenTextDocument` / `DidChangeTextDocument` /
  `DidCloseTextDocument` — `didOpen` and `didChange` call
  `diagnostics_for` and publish via
  `LanguageClient::publish_diagnostics`.
- `notification::Exit` — `ControlFlow::Break(Ok(()))` to shut the
  process down cleanly.

`build_service(client)` wraps the router via the standard
async-lsp middleware stack (Tracing / Lifecycle / CatchUnwind /
Concurrency / ClientProcessMonitor). For the MVP we just return the
router directly; the middleware stack can be added once the basic
flow is proven.

## VS Code extension

`editor/vscode-arcis/` is a minimal TypeScript extension that uses
`vscode-languageclient` to wrap the LSP. The `extension.ts` file:

1. Resolves the `arcis-lsp` binary path (env var → `~/.cargo/bin/`
   → `PATH`).
2. Spawns it as a child process via `LanguageClient` (transport:
   stdio).
3. Filters documents by `language: "arcis"`.
4. Disables itself gracefully if the binary can't be found (the
   extension falls back to syntax highlighting only).

The build chain is: `tsc -p .` compiles `src/extension.ts` into
`out/extension.js`, which is what `package.json::main` points to.

## Testing the LSP

The `tests/` directory has one test file per provider. Each test is
a unit test that calls the pure function and asserts on the result.

For end-to-end testing, the test suite in
`crates/arcis-codegen/tests/sys_codegen.rs` already exercises the
codegen layer. For the LSP itself, the recommended path is a
JSON-RPC smoke test that spawns the binary and sends an
`initialize` + `textDocument/completion` + `exit` triple. See the
project's CI for a working example.
