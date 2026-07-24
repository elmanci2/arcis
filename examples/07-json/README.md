# `json(path)` — native JSON with compile-time type inference

**Rust backend only.** `json(...)`'s runtime deserialization leans on
`serde`/`serde_json` — auto-added to `Cargo.toml` the first time a build
sees a `json(...)` call, so it feels like a real builtin (no manual
dependency wiring). Building with `--backend cranelift` fails with a clear
error instead of silently miscompiling. This is the one example in the
suite (besides `examples/generics/`) that doesn't build on both backends.

**Requires a `Cargo.toml` next to the entry file** — `json(...)` needs
external crates (`serde`/`serde_json`), which the "direct rustc, no
Cargo.toml" layout every other example uses can't pull in at all (bare
`rustc` can't fetch from crates.io). This directory has one; `arcis init`
scaffolds one for a new project too. **The entry file must be named
`main.tsr`** in this layout — Cargo's implicit binary-target detection only
looks for `src/main.rs`, so the generated file needs that exact name (the
direct-rustc layout every other example uses doesn't have this constraint,
since it invokes `rustc` directly on whatever file you name).

## Run it

```bash
cd examples/07-json
arcis run main.tsr
```

`json(...)`'s path is resolved relative to the **current working
directory**, same as `sys.readFile` already is — `cd` into this directory
first (or pass an absolute/correctly-relative path).

## What it covers

| Section | Topic |
|---------|-------|
| 1 | `json("./data.json")` — a **literal** path: the compiler reads the real file at compile time and infers its shape (nested objects, arrays, primitives) automatically. No interface written by hand, full field-access checking, real editor autocomplete after `data.` |
| 2 | `json<Item>(path)` — a **non-literal** path (`path` is a variable): the compiler can't read an unknown-at-compile-time path, so an explicit type argument naming an already-declared interface is required instead |

## Expected output

```
name: arcis
version: 1
active: true
first tag: compiler
meta.id: 7
meta.maintainer: elmanci2
item.name: arcis
item.version: 1
```

## Known limitations

- A `null` value in the sample JSON infers as `string?` — a documented
  best-guess placeholder (a null sample reveals nothing about the field's
  real type).
- An empty array in the sample JSON infers its element type as `number`,
  matching the existing empty-array-literal convention elsewhere in the
  language.
- Path resolution is CWD-relative, not relative to the `.tsr` file that
  calls `json(...)` — matches `sys.readFile`'s existing convention, but
  means the inferred-at-compile-time file and the one actually read at
  runtime can differ if the compiled binary runs from a different directory
  than it was built from.

## Related examples

- [`examples/generics/`](../generics/) — the generic functions/interfaces/
  type aliases `json<T>(path)`'s explicit-type-argument form builds on.
- [`examples/object/`](../object/) — object literals and inline object
  types, the same machinery `json(...)`'s shape inference reuses.
