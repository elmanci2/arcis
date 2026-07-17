# `mods/` — multi-module project with `import` / `export`

A minimal multi-file Arcis project. Demonstrates:

- ES-module-style `import` / `export` syntax (same as TypeScript).
- Named exports (`export function ...`, `export const ...`).
- Default exports (`export default function ...`).
- Named imports (`import { foo } from "bar"`).
- Default imports (`import foo from "bar"`).
- Path resolution: imports are relative to the **importing** file's
  directory, without a `./` prefix (e.g. `from "mate"` resolves
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
├── main.tsr   # entry point — imports from "mate" and "texto"
├── mate.tsr   # math helpers: PI, sumar, calcula (default export)
└── texto.tsr  # string helper: shout
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
export function shout(s: string): string {
    return s + "!!!";
}
```

A single named export.

### `main.tsr`

```ts
import { sumar, PI } from "mate";
import calcula from "mate";
import { shout } from "texto";

print("PI = " + PI);
print("2 + 3 = " + sumar(2, 3));
print("calcula(10) = " + calcula(10));
print(shout("hola"));
```

Mixed import styles: named (`{ sumar, PI }`), default (`calcula`),
and from another module (`shout` from `texto`).

## How it works

### Path resolution

The linker resolves import specifiers relative to the **importing**
file's directory. `from "mate"` inside `main.tsr` resolves to
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
hola!!!
```

## Notes

- For a single-file program, you don't need this folder layout.
  Use this only when you have multiple `.tsr` files that import
  each other.
- To use external Rust crates (HTTP, JSON, regex, …), place a
  `Cargo.toml` next to `main.tsr` and add the dep there. Then
  `import { Client } from "crate:reqwest";`.

## Related examples

- [`examples/completo/`](../completo/) — the kitchen-sink single-file
  program.
- [`examples/05-files/`](../05-files/) — `sys.*` builtins for
  filesystem access from inside a module.