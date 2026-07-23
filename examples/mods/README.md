# `mods/` — multi-module project with `import` / `export`

A minimal multi-file Arcis project. Demonstrates:

- Python-style `import` (namespace) and `from ... import ...` (named);
  ES-style `export` (same as TypeScript).
- Named exports (`export function ...`, `export const ...`).
- Default exports (`export default function ...`).
- Named imports (`from mate import sumar, PI;`).
- Default imports (`from mate import default as calcula;`).
- Aliased imports (`from texto import gritarFuerte as GRITAR;`).
- Path resolution: imports are relative to the **importing** file's
  directory, without a `./` prefix (e.g. `from mate import ...` resolves
  to `<dir>/mate.tsr`).

## Run it

```bash
arcis run examples/mods
```

`arcis run` accepts a directory as the entry point; the linker
looks for `main.tsr` inside it. (You can also pass
`examples/mods/main.tsr` directly — both work.)

## Files

```
examples/mods/
├── main.tsr   # entry point — imports from mate and texto
├── mate.tsr   # math helpers: PI, sumar, calcula (default export)
└── texto.tsr  # string helper: gritar (re-exported as gritarFuerte)
```

### `mate.tsr`

```ts
export const PI: number = 3.14;

export function sumar(a: number, b: number): number {
    return a + b;
}

export default function calcula(n: number): number {
    return n * PI;
}
```

Exports two named items (`PI`, `sumar`) and one default export
(`calcula`).

### `texto.tsr`

```ts
export function gritar(s: string): string {
    return s + "!!!";
}

export { gritar as gritarFuerte };
```

A named export (`gritar`), re-exported under an alias (`gritarFuerte`).

### `main.tsr`

```ts
from mate import sumar, PI;
from mate import default as calcula;
from texto import gritarFuerte as GRITAR;

print("PI = " + PI);
print("2 + 3 = " + sumar(2, 3));
print("calcula(10) = " + calcula(10)); // default export
print(GRITAR("hello"));
```

Mixed import styles: named (`sumar, PI`), default (`default as calcula`),
and an aliased re-export from another module (`gritarFuerte as GRITAR`
from `texto`).

## How it works

### Path resolution

The linker resolves import specifiers relative to the **importing**
file's directory. `from mate import ...` inside `main.tsr` resolves to
`<dir>/mate.tsr`. There is no `./` prefix required (Node-style
convenience).

### Module name validation

The file stem must be a valid Rust identifier
(`[A-Za-z_][A-Za-z0-9_]*`) — `mate.tsr` works, `my-utils.tsr`
would not. The linker **sanitises** invalid stems automatically
(e.g. `01-utils.tsr` becomes `_01_utils`), so the actual filename
can be anything.

### Multi-module codegen

For multi-file projects, the codegen emits:

1. **Root module** (`main.rs`):
   - `mod <other>;` for every other module.
   - `pub struct __ObjXXX` declarations for inline object types.
   - `fn main() { ... }` with the root's statements.
2. **Per-module file** (`<id>.rs`):
   - `pub fn` / `pub const` for exports.
   - `pub use self::...;` for re-exports.

The driver writes these to `bin/` (default layout) or
`bin/<pkg>/src/` if there's a `Cargo.toml` next to `main.tsr`.

## Expected output

```
PI = 3.14
2 + 3 = 5
calcula(10) = 31.400000000000002
hello!!!
```

## Notes

- For a single-file program, you don't need this folder layout.
  Use this only when you have multiple `.tsr` files that import
  each other.
- External Rust crates (HTTP, JSON, regex, …) are meant to work via a
  `Cargo.toml` next to `main.tsr` plus `from crate:reqwest import Client;`,
  but that import form is currently a parse error (not yet implemented) —
  don't rely on it.

## Related examples

- [`examples/complete/`](../complete/) — the kitchen-sink single-file
  program.
- [`examples/05-files/`](../05-files/) — `sys.*` builtins for
  filesystem access from inside a module.