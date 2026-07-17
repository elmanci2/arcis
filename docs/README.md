# Arcis documentation

This directory holds everything besides the source code itself: architecture
notes, the language reference, and contributor guides.

## Contents

| Document                                            | Purpose                                         |
|-----------------------------------------------------|-------------------------------------------------|
| [`architecture.md`](architecture.md)               | Pipeline overview, crate dependency map          |
| [`language-reference.md`](language-reference.md)   | Formal grammar, supported-subset matrix          |
| [`contributing/adding-builtins.md`](contributing/adding-builtins.md) | Step-by-step guide for adding a new builtin |

## Topical guides (planned)

These will be added in follow-up phases; see the GitHub issues for tracking.

- `porting-from-typescript.md` — translating common TS idioms to Arcis
- `module-system.md` — deep dive into `import`/`export` semantics
- `error-format.md` — the format the compiler emits for diagnostics
- `standard-library.md` — what ships in `arcis-std` and how it is wired

## Diagrams and figures

Diagrams are written in ASCII for now (mirror-friendly, no extra tooling). If
the docs grow, we can promote them to Mermaid / Graphviz in a later phase.