# `loops.tsr` — loops: `while`, `for`, `for-of`, `break`, `continue`

The classic control-flow constructs: C-style `while` and `for`
loops, plus `for-of` over arrays.

## Run it

```bash
arcis run examples/loops/loops.tsr
```

## What it covers

1. `while (cond) { body }` with a simple counter.
2. `while (true) { ... break; }` — infinite loop with an early exit.
3. `while` + `continue` to skip even numbers.
4. `for (init; cond; update) { body }` — C-style.
5. `for` + `break` to exit on a condition.
6. Nested loops.
7. `.length` on strings.
8. `for` + `continue` to print only some values.

## How it works

### `while` translates to a Rust `while`

Direct mapping. The condition is re-evaluated at the top of each
iteration.

### `for (init; cond; update)` is desugared

A C-style `for` is **not** a built-in primitive in Rust. The codegen
emits a `loop { … }` with a flag variable so `continue` runs the
`update` clause before the next iteration:

```rust
{
    init;
    let mut __for_first = true;
    loop {
        if !__for_first { update; }
        __for_first = false;
        if !(cond) { break; }
        body;
    }
}
```

This is the same desugaring TypeScript uses internally. The flag
variable `__for_first` makes sure the `update` step is **not** run
on the very first iteration (where `init` already happened) but
**is** run after every `continue`.

### `for (let x of arr)` is also desugared

For a plain identifier iterable (`for (let x of arr)`), the codegen
emits `for x in arr.iter().cloned()` so the array is not consumed.
For an expression iterable (e.g. `for (let x of server.incoming())`),
the codegen emits `.into_iter()` because the standard library's
`Iterator` doesn't have `.iter()`.

## Expected output (excerpt)

```
counter = 1
counter = 2
counter = 3
counter = 4
counter = 5
First multiple of 7 >= 100: 105
Sum of odds from 1 to 10: 25
7! = 5040
7 * 6 = 42
3x3 coordinates:
(0,0) (0,1) (0,2)
(1,0) (1,1) (1,2)
(2,0) (2,1) (2,2)
```

## Notes

- `for (let i = 0; i < n; i = i + 1)` is idiomatic in Arcis. There's
  no `++` or `--` operator yet, so write the increment explicitly.
- `break` and `continue` work in both `while` and `for`.
- Nested loops: `break` always refers to the **innermost** enclosing
  loop. There are no labels in v1.

## Related examples

- [`examples/strings/`](../strings/) — `for-of` over arrays,
  paired with `break` / `continue`.
- [`examples/04-if-else/`](../04-if-else/) — branches inside loops.