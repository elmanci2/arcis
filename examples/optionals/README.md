# `optionals.tsr` — null safety: `T?`, `??`, narrowing, and `!`

Arcis enforces one guarantee, at compile time, on both backends: **a value
typed `T?` (may be missing) can never reach a place that expects a
guaranteed `T` without being resolved first.** This example walks through
every way to resolve one, plus what happens when you don't.

## Run it

```bash
arcis run examples/optionals/optionals.tsr
```

## What it covers

| Section | Topic |
|---------|-------|
| 1 | `?? fallback` — the fallback must itself be a guaranteed value |
| 2 | Narrowing: `if (x != null) { ... }` proves `x` is present in that block |
| 3 | Guard clause: `if (x == null) { return; }` proves `x` for the rest of the function |
| 4 | `!` — an explicit, deliberate assertion |
| 5 | Optional interface fields (`nickname?: string`) — the same rules apply |

## The rule, precisely

A `T?` value must be resolved before it reaches:

- a `let`/`const` with an explicit non-optional type,
- a `return` inside a function with a non-optional return type,
- a function-call argument,
- the fallback side of `??` (a `T?` fallback defeats the point — rejected),
- an arithmetic/comparison operand,
- the receiver of `.field` / `[index]`,
- `print(...)` / `str(...)`.

"Resolved" means one of: `?? fallback` (fallback must be `T`, not `T?`), a
narrowing null check, or `!`.

## What gets rejected (and why)

Uncomment any one line at the bottom of `optionals.tsr` and the build
fails immediately, before ever reaching `rustc`/Cranelift:

```ts
let bad1: Product = findById(catalog, 1);
// -> a possibly-missing value is assigned to `bad1` without being
//    resolved — add `?? fallback`, guard it, or assert it with `!`.

let bad2: number = null;
// -> `null` [assigned to `bad2`], but the target type isn't optional —
//    make it `T?`, or provide `?? fallback`.

let bad3: Product = findById(catalog, 1) ?? findById(catalog, 2);
// -> the fallback on the right of `??` must be a guaranteed
//    (non-optional) value — a possibly-missing fallback defeats the
//    guarantee `??` is supposed to give.

print(findById(catalog, 1).name);
// -> `.name` is accessed on a possibly-missing value — resolve it
//    with `?? fallback`, a null check, or `!` first.
```

These are real compiler errors (`null-safety error: ...`), not warnings —
the same class of guarantee Rust's `Option<T>` gives, at the Arcis source
level, identically on the Rust and Cranelift backends.

## Expected output

```
?? -> Laptop
narrowing -> not found
guard clause -> Mouse ($25)
guard clause -> missing
! -> Laptop
Ana: An
Beto: (no nickname)
```

## Why this matters (design rationale)

Most transpiled-to-native languages either don't track "might be missing"
at all (silent garbage / wrong defaults on a miss) or track it but let you
ignore it (TypeScript's own `strictNullChecks` is opt-in and full of
escape hatches). Arcis makes it load-bearing: you cannot compile a program
that drops an optional value on the floor. The trade-off is explicit by
design — see `docs/language-reference.md`'s "Null safety" section for the
full rule list and the sentinel-based Cranelift implementation notes.

## Related examples

- [`examples/object/`](../object/) — object literals and inline object types.
- [`examples/mods/`](../mods/) — the same `findById`-style pattern across modules.
