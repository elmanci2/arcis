# Arcis

A minimal **TypeScript-like** language (`.tsr`) written in Rust that compiles to
**native binaries** by transpiling to Rust and delegating the final step to
`rustc` (or `cargo`, when external crates are involved).

The syntax is identical to TypeScript — same keywords, same `let`/`const`,
same `function` declarations. The difference is that `.tsr` files do not run
through Node or Deno; they are translated to Rust and compiled to a single
binary you can ship.

## Quick start

```bash
# Build a .tsr file and leave the binary in ./bin/
cargo run -- build examples/hola.tsr
./bin/hola

# Build and run directly
cargo run -- run examples/hola.tsr

# Just see the generated Rust code (without invoking rustc)
cargo run -- check examples/hola.tsr

# Multi-file projects: by convention the entry point is `main.tsr`
# (no argument uses `./main.tsr`; a directory searches `<dir>/main.tsr`)
cargo run -- run
cargo run -- run examples/mods
```

Once installed (see below), you can invoke all of this as `arcis` directly:

```bash
arcis run examples/mods
arcis run                   # uses ./main.tsr
arcis init my-project       # scaffolds a new project in ./my-project
```

## Create a new project from scratch

```bash
arcis init my-project
cd my-project
arcis run
```

`arcis init [<dir>]` creates a minimal `main.tsr` with a "Hello" ready to run.
Without arguments it uses the current directory. If a `main.tsr` already exists
it aborts without overwriting.

## Install as a system command

```bash
cargo install --path .        # installs the binary to ~/.cargo/bin/arcis
which arcis                   # confirm it is on the PATH
```

After that you can run `arcis run`, `arcis build`, `arcis check` from any
directory, without having to be inside the repo.

```bash
# Example in an arbitrary directory:
mkdir my-app && cd my-app
# … write main.tsr and modules …
arcis run
```

To uninstall: `cargo uninstall arcis` or `rm ~/.cargo/bin/arcis`.
To update after changes: `cargo install --path . --force`.

## Backends

`arcis` ships with **two** compiler backends, selectable per command via the
`--backend` flag. The default is the original Rust backend; passing
`--backend cranelift` switches to a Cranelift-based native code generator
that does **not** depend on a Rust toolchain at runtime.

| Backend    | Toolchain needed at runtime                | Library dependencies at runtime          | Default |
|------------|--------------------------------------------|-----------------------------------------|---------|
| `rust`     | `rustc` / `cargo`                          | Rust `std`                              | ✓       |
| `cranelift`| only `cc` (gcc, clang, …)                  | libc                                    |         |

```bash
arcis build --backend rust      examples/01-hello/hello.tsr
arcis build --backend cranelift examples/01-hello/hello.tsr

# After install, an Arcis user without Rust installed can run:
arcis build --backend cranelift my-project/
./bin/my-project
```

The Cranelift backend is feature-parallel to the Rust one in scope: it
covers everything in "Supported subset" for Phase 1 except arrays,
methods, objects, modules and `sys.*`. See
[`crates/arcis-codegen-cranelift/`](crates/arcis-codegen-cranelift/) for
the implementation and the upstream plan at
`~/.claude/plans/imperative-seeking-cerf.md` (Phases 2–6) for the rest.

## Supported subset

- `let` / `const` with optional type annotation
- Primitive types: `string`, `number`, `boolean`, `void`
- `function name(p: T, ...): T { ... }` with `return`
- `if (cond) { ... } else if (...) { ... } else { ... }`, `while`, `for (init; cond; upd)`, `break`, `continue`
- `print(expr);` (shorthand for `println!`)
- Literals: `"string"`, `42`, `3.14`, `true`, `false`, `[...]`, `{ key: value }`
- Inline object types: `let p: { name: string, age: number } = ...`
- Reassignment (`x = ...`), indexed assignment (`arr[i] = ...`), field assignment (`obj.x = ...`)
- Operators: `+ - * / % == != < > <= >= && || !`
- Comments `//` and `/* ... */`
- **Modules**: `import`/`export` with TypeScript syntax (see [Modules](#modules))
- Note: `for (let x of arr)`, `arr.length`, and the array methods
  (`push`/`pop`/`find`/`filter`/`map`/…) are listed above for completeness
  but are not yet supported by the Cranelift backend (Phase 2+).

## Example

`examples/hola.tsr`:

```ts
let name: string = "World";
let year: number = 2026;

function greet(who: string, age: number): string {
    return "Hello " + who + " in the year " + age;
}

let message: string = greet(name, year);
print(message);

if (year > 2000) {
    print("Welcome to the 21st century");
} else {
    print("Time traveller");
}
```

Output:

```
Hello World in the year 2026
Welcome to the 21st century
```

## Examples

Each example lives in its own folder under `examples/`, with a `.tsr`
source file and a `README.md` that explains what the program does,
why it does it that way, how to run it, and what output to expect.

Run any example with:

```bash
arcis run examples/<folder>/<file>.tsr
```

| Folder                                | Source                | What it shows                                                                  |
|---------------------------------------|-----------------------|---------------------------------------------------------------------------------|
| [`examples/01-hello/`](examples/01-hello/README.md)               | `hello.tsr`           | The smallest possible Arcis program                                             |
| [`examples/02-basics/`](examples/02-basics/README.md)             | `basics.tsr`          | `let` / `const`, primitive types, operators, comments                           |
| [`examples/03-functions/`](examples/03-functions/README.md)       | `functions.tsr`       | Function declarations, recursion (factorial, Fibonacci), helpers, return values  |
| [`examples/04-if-else/`](examples/04-if-else/README.md)           | `if-else.tsr`         | `if` / `else if` / `else` chains, nested conditions, logical combinations         |
| [`examples/05-files/`](examples/05-files/README.md)               | `files.tsr`           | `sys.*` filesystem builtins (`readFile`, `writeFile`, `mkdir`, `listDir`, etc.) |
| [`examples/arrays/`](examples/arrays/README.md)                   | `arrays.tsr`          | Arrays and the seven method builtins (`find` / `filter` / `map` / `reduce` / …) |
| [`examples/strings/`](examples/strings/README.md)                 | `strings.tsr`         | All seven string methods + `for-of` iteration over arrays                       |
| [`examples/object/`](examples/object/README.md)                 | `object.tsr`          | Object literals, inline object types, field access, arrays of objects           |
| [`examples/reassignment/`](examples/reassignment/README.md)     | `reassignment.tsr`   | Reassigning `let` bindings (variables, swap, strings)                           |
| [`examples/loops/`](examples/loops/README.md)                   | `loops.tsr`          | `while` / `for` / `break` / `continue`, nested loops, `.length` on strings        |
| [`examples/input/`](examples/input/README.md)                   | `input.tsr`          | The `input()` builtin (reads one line of stdin)                                 |
| [`examples/complete/`](examples/complete/README.md)             | `complete.tsr`       | A single-file kitchen-sink program that uses most features                       |
| [`examples/mods/`](examples/mods/README.md)                     | `main.tsr` + `mate.tsr` + `texto.tsr` | Multi-module project with `import` / `export`        |

## Modules

By convention the entry point is **`main.tsr`**. Without arguments, `arcis run`
uses `./main.tsr`; passing a directory searches `<dir>/main.tsr`. Standard
TypeScript syntax for `import`/`export` is supported:

```ts
// utils.tsr
export const PI: number = 3.14;
export function add(a: number, b: number): number { return a + b; }
export default function compute(n: number): number { return n * PI; }
```

```ts
// main.tsr (entry point)
import { add, PI } from "utils";                  // named imports
import { add as a } from "utils";                 // with alias
import compute from "utils";                      // default import
import compute, { PI } from "utils";              // default + named
export function f() { ... }                       // inline export
export const X = 1;                               // export const (const value)
export { f, X as Y };                             // re-export (with alias)
export default function () { ... }                // default export
```

Import paths are resolved relative to the directory of the importing file
(without the `./` prefix): `from "utils"` resolves to `<dir>/utils.tsr`.
Internally each `.tsr` is transpiled to a separate `.rs`, and `main.rs` declares
them with `mod <id>;`, referencing items via `use crate::<id>::...;`. This
translates to real Rust modules (`pub fn`, `pub const`, `pub use self::...`),
so visibility and names must be valid Rust identifiers (no hyphens, must not
start with a digit).

Full example: `examples/mods/`.

### Module limitations

- **Top-level `let`/`const` in non-`main` modules** are emitted as Rust `const`,
  so the value must be evaluable at compile time. For
  `export const X = function() {...}` with non-const-eval functions, rustc
  will fail with a clear error.
- **Module names**: must be valid Rust identifiers (the file stem:
  `[A-Za-z_][A-Za-z0-9_]*`). `from "my-mod"` will fail with a link error.
- **Object types** (`{ a: T, ... }`) are centralized in `main.rs` as
  `pub struct`, and non-`main` modules reference them as
  `crate::__ObjNAME`. This avoids duplication between modules.
- **`export { private as public }`** on a private item cannot be re-exported
  (Rust requires the original item to be public). To preserve encapsulation
  in this case a wrapper would have to be emitted; for now the original item
  must be public.
- `import * as ns` (namespace) is **not** supported in this step.

## Out of scope (for now)

- Classes, generics
- Closures that capture variables (arrow functions are supported, but
  non-capturing only), destructuring, template literals
- `bigint` literals
- `import * as ns` (namespace import)
- Async / await

## How it works

```
main.tsr (+ utils.tsr, ...)
   │
   ▼ linker (DFS, validates exports/imports, cycles)
┌─────────┐  tokens   ┌────────┐   AST    ┌────────────────┐  bin/*.rs
│  Lexer  │ ────────▶ │ Parser │ ───────▶ │ Codegen (xN)   │ ───────▶ rustc → binary
└─────────┘           └────────┘          └────────────────┘
                                              │
                                              └─ main.rs: mod …; use crate::…;
```

The codegen emits Rust source using:

| TS       | Rust        |
|----------|-------------|
| `string` | `String`    |
| `number` | `f64`       |
| `boolean`| `bool`      |
| `void`   | `()`        |
| `any`    | inferred on `let`/`const`; `Box<dyn Any>` on fn params/returns |
| `null` / `undefined` | `()` |
| `A \| B` / `A & B` | Rust type of the first member (see [`docs/language-reference.md`](docs/language-reference.md#type-erasure)) |
| `type X = ...` | resolved away before codegen |
| `interface X { ... }` | named `pub struct` (like inline object types, but with its own name) |
| `enum X { A, B = 5 }` | `pub enum X { A, B = 5 }`; `X.A` → `X::A` |
| `(x: number) => x * 2` | non-capturing Rust closure (coerces to `fn(f64) -> f64`) |
| `switch`/`case` | `if`/`else if` chain (see [`docs/language-reference.md`](docs/language-reference.md)) |
| `try`/`catch`/`throw` | `std::panic::catch_unwind` / `panic!` |

String concatenation with `+` is translated to `format!("{}{}", a, b)` when at
least one of the operands is a string literal; otherwise plain `+` is used. This
covers `print("Hello " + age)` without any extra runtime.

## Project layout (workspace)

Arcis is organized as a Cargo workspace with one crate per compilation phase
(mirroring how `rustc`, `swc`, and `biome` are structured):

```
crates/
├── arcis-ast/         # AST data types only (no dependencies)
├── arcis-lexer/       # source text → token stream
├── arcis-parser/      # token stream → AST
├── arcis-validation/  # semantic checks (unused, duplicate, loop context)
├── arcis-linker/      # module resolver + import/export validation
├── arcis-codegen/     # AST → Rust source
├── arcis-driver/      # orchestration: build, compile, run, init
├── arcis/             # CLI binary (clap)
└── arcis-std/         # standard library (.tsr files)
```

See [`docs/architecture.md`](docs/architecture.md) for the full dependency
graph and pipeline diagram.

## Known limitations

- Typing is **static in the parser but not verified**: writing `let x: number = "hi";`
  generates the same code and `rustc` will complain. That is acceptable for a
  first iteration. This extends to the newer "everyday types" additions:
  union/intersection types erase to their first member's Rust type, and
  `any`/`as`/`as const`/`!` have no runtime effect (matching TypeScript's own
  erasure model).
- The codegen follows TS semantics but does not implement shadowing, closures,
  or first-class functions yet.
- The Cranelift backend only covers primitives, `let`/`const`, control flow,
  and function calls so far — unions, interfaces, type aliases, and `any`
  fall back to an opaque handle or an error there (the Rust backend supports
  all of them).

## Next steps (ideas)

- Type checker (currently relies on `rustc` for type errors)
- Standard library (`arcis-std` is scaffolded but not yet wired into the driver)
- Language server (LSP) and formatter
- Splitting the larger `parser.rs` and `codegen.rs` into per-AST-node files
- Translating remaining Spanish error messages and internal comments to English