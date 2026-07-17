# `02-basics/` — variables, primitive types, operators, comments

A walkthrough of Arcis's value-level primitives: `let` and `const`,
the four primitive types (`string`, `number`, `boolean`, `void`),
the operator set, and both comment styles.

## Run it

```bash
arcis run examples/02-basics/basics.tsr
```

## What it covers

1. `let` (rebindable) vs `const` (treated as immutable).
2. Primitive types with and without explicit annotations.
3. Arithmetic operators: `+ - * / %`.
4. Comparison operators: `== != < > <= >=`.
5. Logical operators: `&& || !` with short-circuit semantics.
6. Line comments (`//`) and block comments (`/* ... */`).

## How it works

### `let` vs `const`

In TypeScript, `let` and `const` differ only in rebindability. In
Arcis, both currently compile to `let` in Rust. The compiler emits a
warning if a `const` is rebound, but the resulting code is otherwise
identical. This mirrors TypeScript semantics: the difference is at
the language level, not the runtime level.

### Type annotations vs inference

When you write `let x: number = 1`, the parser records both the name
and the type. When you write `let x = 1`, the parser still emits the
inferred type but does not store it on the AST node — it ends up in
the codegen's type-map either way, so `let x = 1` works the same as
`let x: number = 1` for type-checking downstream (e.g. `.length` on
strings vs arrays). For inline object types you **must** annotate
the binding, because the codegen needs the shape to declare the struct.

### Operator semantics

Most operators translate directly to their Rust counterparts. The
exception is `+`, which becomes `format!("{}{}", a, b)` if **any**
operand contains a string literal anywhere in its tree (a heuristic).
Otherwise it stays as Rust's `+`. So `"x=" + 42` becomes
`format!("{}{}", "x=", 42)` while `1 + 2` stays `1 + 2`. This
mirrors JavaScript's automatic string coercion.

### `&&` / `||` short-circuit

Like Rust and JavaScript, `&&` and `||` short-circuit. The right-hand
side is only evaluated when needed. This matters when the right side
has side effects (e.g. function calls) or might panic on certain
inputs.

## Expected output

```
counter after two ++: 2
PI = 3.14159
name: Arcis
version: 0.1
isStable: true
a + b = 13
a - b = 7
a * b = 30
a / b = 3.3333333333333335
a % b = 1
x == y: false
x != y: true
x < y:  true
x >= y: false
x in (0, 100): true
x out of range: false
!isStable: false
enabled: true
```

## Notes

- `number` is an IEEE-754 `f64` under the hood. Integer literals
  without an explicit decimal (`42`) are emitted with a `.0` suffix
  so Rust treats them as floats — there's no integer type in v1.
- Comments do not nest in v1: `/* /* inner */ */` is a syntax error
  on the inner `*/`.

## Related examples

- [`examples/03-functions/`](../03-functions/) — operators in
  function bodies.
- [`examples/04-if-else/`](../04-if-else/) — comparison and logical
  operators in conditions.