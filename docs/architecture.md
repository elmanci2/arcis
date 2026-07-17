# Architecture

This document explains how the Arcis compiler is organised and why.

## Goals

- **Production-grade structure**: one Cargo crate per compilation phase,
  mirroring how `rustc`, `swc`, and `biome` are organised.
- **Parallel compilation**: each crate can build in isolation.
- **Clean public surfaces**: every phase exposes a small API; internal
  helpers are kept private to that crate.
- **Forward-compatible**: future additions (LSP, formatter, type checker)
  slot in as new crates that depend on the existing analysis crates only.

## Pipeline

```
                ┌──────────┐
                │   User   │
                └────┬─────┘
                     │  .tsr source
                     ▼
            ┌────────────────┐
            │   arcis CLI    │   (binary)
            └────────┬───────┘
                     ▼
        ┌────────────────────────┐
        │     arcis-driver       │
        │  build / run / init    │
        └─┬──────────┬───────────┘
          │          │
          ▼          ▼
   ┌────────────┐  ┌──────────────────┐
   │arcis-linker│  │ arcis-validation │
   └─────┬──────┘  └──────────────────┘
         │
         ▼
   ┌────────────┐
   │arcis-lexer │   (source → tokens)
   └─────┬──────┘
         │
         ▼
   ┌─────────────┐
   │arcis-parser │   (tokens → AST)
   └─────┬───────┘
         │
         ▼
    ┌─────────┐
    │arcis-ast│   (shared types)
    └─────────┘

   ┌─────────────────┐
   │  arcis-codegen  │   (AST → Rust source)
   └────┬────────────┘
        ▼
   ┌─────────────────┐
   │  rustc / cargo  │   (Rust source → binary)
   └─────────────────┘
```

## Crate dependency graph

The arrows below mean "depends on":

```
                  ┌─────────┐
                  │ arcis   │   (CLI binary)
                  └────┬────┘
                       │
                       ▼
              ┌────────────────┐
              │ arcis-driver   │
              └──┬───────┬─────┘
                 │       │
                 │       ▼
                 │  ┌──────────────┐
                 │  │arcis-codegen │
                 │  └──┬───────┬───┘
                 │     │       │
                 ▼     ▼       │
        ┌───────────────────┐  │
        │   arcis-linker    │◀─┘
        └──┬─────────────┬──┘
           │             │
           ▼             ▼
   ┌──────────────┐ ┌──────────────┐
   │ arcis-parser │ │ arcis-lexer  │
   └──────┬───────┘ └──────────────┘
          │
          ▼
       ┌──────────┐
       │ arcis-ast│   (no deps)
       └──────────┘

           ┌────────────┐
           │ arcis-std  │   (.tsr source files only)
           └────────────┘
```

## Per-crate responsibilities

| Crate                | Responsibility                                                | Inputs                          | Outputs                                 |
|----------------------|---------------------------------------------------------------|---------------------------------|-----------------------------------------|
| `arcis-ast`          | Plain data types: statements, expressions, types, modules     | (none)                          | `Program`, `Stmt`, `Expr`, `Type`       |
| `arcis-lexer`        | Tokenize source text                                         | `&str`                          | `Vec<Token>`, `LexError`                |
| `arcis-parser`       | Recursive-descent parser with precedence climbing            | `Vec<Token>`                    | `Program`, `ParseError`                 |
| `arcis-validation`   | Unused / duplicate / loop-context checks                      | `&Program`                      | `Vec<ValidationIssue>`                  |
| `arcis-linker`       | DFS module resolution + import/export validation              | `&Path` (entry)                 | `Vec<Module>`                           |
| `arcis-codegen`      | AST → Rust source emission (built-ins, methods, sys calls)    | `&[Module]`                     | `Vec<(id, String)>` (Rust source)       |
| `arcis-driver`       | Build pipeline: link, validate, codegen, write, invoke rustc  | `&Path`                         | `BuildOutput` (paths + sources)         |
| `arcis`              | CLI binary: clap subcommands → driver                         | CLI args                        | exit code                                |
| `arcis-std`          | Standard library as `.tsr` source files                       | (none)                          | embeddable via `include_str!`            |

## Why a multi-crate workspace?

A single monolithic crate works for the first hundred lines of a compiler,
but breaks down once you cross ~1000 lines:

1. **Build times**: every edit triggers a full rebuild. A workspace only
   rebuilds the changed crate and its downstream consumers.
2. **Public API contracts**: the boundary between phases becomes a hard
   line — you cannot accidentally reach into the parser from the lexer.
3. **Reusable pieces**: someone can write an LSP that depends on
   `arcis-lexer`, `arcis-parser`, `arcis-ast`, and stop there — no need
   to pull in the whole driver and CLI.
4. **Forces one-way dependencies**: the dependency graph
   (`ast → lexer → parser → linker/codegen → driver → cli`) makes the
   direction of data flow explicit, which is exactly the kind of clarity
   we want from a "real language".

## Phase 2 (planned)

The current code is moved verbatim into the right crates, but each crate
still has its original monolithic file. The next phase splits them per
AST-node category so no single file is over a few hundred lines:

| File today                | Split target (phase 2)                                  |
|---------------------------|----------------------------------------------------------|
| `arcis-parser/src/lib.rs` | `state.rs` · `stmt.rs` · `expr.rs` · `types.rs` · `modules.rs` · `error.rs` |
| `arcis-codegen/src/lib.rs`| `context.rs` · `collect.rs` · `types.rs` · `module.rs` · `stmt.rs` · `expr.rs` · `builtin.rs` · `method.rs` · `function.rs` |
| `arcis-validation/src/lib.rs` | `unused.rs` · `duplicate.rs` · `loop_ctx.rs` · `format.rs` |
| `arcis-lexer/src/lib.rs`  | `state.rs` · `scanner.rs` · `error.rs`                   |

Mirrors how `rustc_parse` (`expr.rs`, `item.rs`, `pat.rs`, ...) and
`swc_ecma_codegen` (`expr.rs`, `stmt.rs`, `decl.rs`, ...) split their
files by AST-node category.

The plan file at `/home/nuxa/.claude/plans/wondrous-honking-sutherland.md`
captures the original restructure intent.