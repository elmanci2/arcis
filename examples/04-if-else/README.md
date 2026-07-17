# `04-if-else/` — `if` / `else if` / `else` chains

Conditionals in Arcis look and behave like TypeScript. The example
walks through every shape: single `if`, `if/else`, chains of
`else if`, nested conditions, and short-circuit logic.

## Run it

```bash
arcis run examples/04-if-else/if-else.tsr
```

## What it covers

1. Plain `if` (no else branch).
2. `if` / `else`.
3. `if` / `else if` / `else` chains.
4. Nested conditions.
5. Logical combinations: `&&`, `||`, `!`.
6. Functions with multiple branches.
7. `if` as a one-line guard.

## How it works

### `if (cond) { ... }`

If `cond` is `true`, the body runs. Otherwise the body is skipped.

### `if (cond) { ... } else { ... }`

Exactly one of the two branches runs.

### `if / else if / else` chains

Each `else if` adds another condition; the first matching branch runs.
The chain is parsed into nested `Stmt::If` nodes in the AST, so
`else if (n == 2) { ... }` is exactly equivalent to writing
`else { if (n == 2) { ... } }` — but the chain form is much easier
to read and is what the codegen expects.

### Logical operators

`&&` and `||` short-circuit like Rust and JavaScript. `!` negates a
boolean. These work in any expression context, including function
arguments and `return` statements.

### Parentheses and expressions

The condition inside `if (...)` is any expression. Comparisons,
logical operators, function calls, even chained method calls like
`arr.find(isPositive) != 0` all work.

## Expected output

```
Score 85 → grade B
Welcome, administrator
can drive? true
access granted? true
negative
zero
small positive
medium
large
cannot divide by zero
```

## Notes

- There is no ternary `?:` operator in v1. Use `if` / `else` to
  assign a value into a variable instead.
- `else if` chains have no built-in limit; you can chain as many as
  you want. For deeper hierarchies, prefer `switch`-style data or a
  lookup table — Arcis doesn't have `switch` yet but you can
  emulate one with an array of `{ match, result }` objects.

## Related examples

- [`examples/loops/`](../loops/) — loops with `break` and `continue`.
- [`examples/03-functions/`](../03-functions/) — branching inside
  function bodies.