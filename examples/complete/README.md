# `complete.tsr` — kitchen-sink: everything in one file

A single-file program that exercises **most** of Arcis's features at
once: variables, constants, functions, recursion, conditionals,
loops, arithmetic, comparison and logical operators, strings,
booleans, and comments.

## Run it

```bash
arcis run examples/complete/complete.tsr
```

## What it covers

Almost everything — see the section dividers in `complete.tsr`:

1. Variables with and without type annotations.
2. Constants (UPPER_CASE by convention).
3. Function declarations.
4. `if` / `else` branches.
5. C-style `for` loops.
6. Reassignment with `let`.
7. Recursion (factorial).
8. Operators: arithmetic, comparison, logical.
9. Strings with concatenation (`+`).
10. Booleans.
11. `print` for output.
12. Comments (`//` and `/* */`).

## Why a kitchen-sink example?

Single-file examples are easier to read top-to-bottom, and they
exercise the full pipeline (lexer → parser → codegen → rustc) on
non-trivial inputs. If a change to any phase breaks something, this
file is the first thing to break.

## Expected output

```
Hello Language
Version: 2026
Active: true
PI: 3.14
Limit: 100
Double 21 = 42
-5 is: negative
0 is: zero
42 is: positive
5! = 120
10! = 3628800
Is 25 of age 18? true
Is 15 of age 18? false
Passes with 85? true
Is 150 invalid? true
Is 8 even? true
Is 7 even? false
Final counter: 10
Double: 42 / Fact5: 120
```

## Notes

- This is a single-file program. For multi-module programs with
  `import`/`export`, see [`examples/mods/`](../mods/).
- The output is long (21+ lines). It's a deliberate exercise of
  every feature, not a minimal example.

## Related examples

All of them — every other example in `examples/` covers a specific
feature in isolation.