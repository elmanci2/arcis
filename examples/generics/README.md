# `generics.tsr` — generic functions, interfaces, and type aliases

**Rust backend only.** Generics emit real Rust generics (`fn f<T>(...)`,
`struct Name<T>`) and let `rustc` do all the composition work —
monomorphization, nested generics, everything. There is no Arcis-side
generics runtime. Building this file with `--backend cranelift` fails with
a clear error instead of silently miscompiling:

```
$ arcis build --backend cranelift examples/generics/generics.tsr
examples/generics/generics.tsr: generic function `identity` is not yet
supported by the Cranelift backend — use --backend rust
```

This is the one example in the suite that doesn't build on both backends —
every other example does.

## Run it

```bash
arcis run examples/generics/generics.tsr
```

## What it covers

| Section | Topic |
|---------|-------|
| 1 | `function identity<T>(x: T): T` — the common case: `T` inferred from the argument, no annotation needed at the call site |
| 2 | `function makeEmpty<T>(): T[]` — `T` only appears in the return type, so it **can't** be inferred from arguments (there are none); called as `makeEmpty<number>()` with an explicit turbofish type arg. This is the shape a future `JSON.parse<T>(text): T` needs |
| 3 | `interface Holder<T> { value: T }` — a generic interface, used as `Holder<number>` |
| 4 | `type Pair<A, B> = { first: A, second: B }` — a generic, object-shaped type alias |
| 5 | `Holder<Holder<number>>` — nested generics, composed for free by Rust's own type system |

## Expected output

```
identity(5) = 5
identity("hi") = hi
makeEmpty<number>().length = 0
held.value = 42
pair = (answer, 42)
nested.value.value = 7
```

## Related examples

- [`examples/types/`](../types/) — the rest of the "everyday types" subset (unions, literal types, non-generic type aliases and interfaces).
