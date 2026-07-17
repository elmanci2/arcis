# `reasignacion.tsr` — rebinding `let` variables

Demonstrates that `let` bindings can be reassigned in Arcis. The
codegen decides whether to emit `let` or `let mut` based on whether
the variable is ever reassigned.

## Run it

```bash
arcis run examples/reasignacion/reasignacion.tsr
```

## What it covers

1. Counter pattern: increment a number.
2. Accumulator pattern: keep a running total.
3. The classic swap using a temporary variable.
4. Reassigning string bindings.

## How it works

The codegen walks the program during the pre-pass and collects every
name that appears as the LHS of `=`. For each one, the corresponding
`let` declaration becomes `let mut`. This means:

- A `let` that is **never** reassigned compiles to `let x: ...`.
- A `let` that **is** reassigned compiles to `let mut x: ...`.

You don't write `let mut` in Arcis; the language figures it out for
you. The convention matches TypeScript, where `let` is implicitly
mutable and `const` is not — but Arcis currently compiles `const`
the same way (the compiler warns if you try to rebind a `const`).

## Why no compound assignment yet

`+=`, `-=`, `*=`, `/=` are not supported in v1. To increment, write
`x = x + 1` explicitly. This is on the roadmap.

## Expected output

```
1
10
2
2
1
Hola
Adios
```

## Notes

- The codegen also flags arrays/objects whose fields are
  reassigned (`arr[i] = v`, `obj.x = v`) and marks the binding as
  `mut`. See [`examples/objetos/`](../objetos/).

## Related examples

- [`examples/02-basics/`](../02-basics/) — `let` vs `const`.
- [`examples/objetos/`](../objetos/) — field reassignment.