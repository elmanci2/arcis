# `03-functions/` — function declarations, parameters, recursion

How to declare functions, pass arguments, return values, and use
recursion. Arcis compiles top-level function declarations into real
Rust functions with the same signatures.

## Run it

```bash
arcis run examples/03-functions/functions.tsr
```

## What it covers

1. Functions with no parameters.
2. Functions with parameters and an explicit return type.
3. Recursion: factorial.
4. Recursion: Fibonacci.
5. Functions that take arrays.
6. Functions that return objects.
7. Forward references and source-order independence.

## How it works

### Function syntax

```ts
function name(param1: T1, param2: T2): TReturn {
    return ...;
}
```

This compiles to:

```rust
fn name(param1: T1, param2: T2) -> TReturn {
    ...
}
```

Functions can have any number of parameters. Type annotations are
required on parameters and on the return type.

### Returning objects

A function with an inline object type as its return type can use
`return { ... }` directly. The codegen propagates the function's
return type into the body context, so the object literal's
`__ObjHASH` struct name can be resolved without an intermediate
`let` binding.

### Recursion

Recursion is straightforward — each recursive call emits a Rust call
to the same `fn`. The Rust compiler's optimiser can then decide
whether to inline, monomorphise, etc. There is no special keyword;
the function just calls itself.

The Fibonacci example uses the naive exponential-time recursion on
purpose, so it stays readable. For real code, prefer an iterative
approach.

### Functions taking arrays

Arrays are passed **by reference** so the caller keeps ownership.
This matches TypeScript's semantics: passing an array to a function
does not invalidate the array on the caller's side. If you need to
consume the array inside the function (e.g. take ownership of a
vector), Arcis v1 always passes by reference; mutation happens
through the `&mut` reference the standard library provides.

### Source-order independence

Function bodies can call functions defined later in the file
(forward reference). The codegen runs in two passes: first collect
top-level items, then emit the body of `fn main`. As long as the
function name is visible at codegen time, order doesn't matter.

> We use the name `entry()` instead of `main()` in section 7 because
> the root module's source is automatically wrapped in `fn main()`
> by the codegen — having a function called `main` would clash.

## Expected output

```
Hello from Arcis!
Hello from Arcis!
square(5) = 25
add(3, 4) = 7
5! = 120
10! = 3628800
fib(0..10):
  fib(0) = 0
  ...
  fib(10) = 55
sum(data) = 150
avg(data) = 30
p1 = (0, 0)
p2 = (3, 4)
dist² = 25
helper() returned: ok
```

## Notes

- Top-level functions (not nested inside another function) are the
  primary way to organise logic.
- Inner `function` declarations (inside another function body) are
  not yet supported — keep functions at the top level.

## Related examples

- [`examples/04-if-else/`](../04-if-else/) — functions that branch.
- [`examples/arrays/`](../arrays/) — functions used as array
  callbacks (`find`, `filter`, `map`, `reduce`).