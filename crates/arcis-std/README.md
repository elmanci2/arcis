# arcis-std

The Arcis standard library, shipped as `.tsr` source files (no Rust code).

This crate is intentionally source-only. It exists so that every `arcis`
distribution carries a coherent, versioned standard library. The
`arcis-driver` crate wires it into the build pipeline by `include_str!`-ing
the files at compile time, so `import { trim } from "std";` becomes a
synthesised module.

In phase 2 the driver learns to:

1. Detect `from "std"` specifiers as references to this crate.
2. Inject the corresponding `.tsr` source as a virtual module.
3. Run the standard library through the normal pipeline along with the
   user's program.

## Layout

| File          | Exports                                              |
|---------------|------------------------------------------------------|
| `std.tsr`     | Re-exports everything below.                         |
| `strings.tsr` | `trim`, `upper`, `lower`                             |
| `arrays.tsr`  | `sum`, `average`                                     |
| `math.tsr`    | `PI`, `E`                                            |

## Why a separate crate?

Real language projects ship a `std` alongside the compiler — Rust's
[`std`](https://doc.rust-lang.org/std/), Go's standard library, Node's
core modules. Keeping Arcis's std in its own crate gives us:

- A single source of truth, versioned together with the compiler.
- Reproducible builds: every Arcis release carries an exact std snapshot.
- A clear boundary: third-party "stdlib replacements" can ship in their
  own crates without forking the compiler.