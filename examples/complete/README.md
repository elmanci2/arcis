# `completo.tsr` — kitchen-sink: everything in one file

A single-file program that exercises **most** of Arcis's features at
once: variables, constants, functions, recursion, conditionals,
loops, arithmetic, comparison and logical operators, strings,
booleans, and comments.

## Run it

```bash
arcis run examples/completo/completo.tsr
```

## What it covers

Almost everything — see the section dividers in `completo.tsr`:

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

## Expected output (excerpt)

```
Hola Lenguaje
Versión: 2026
Activo: true
PI: 3.14
Límite: 100
Duplicar 21 = 42
-5 es: negativo
0 es: cero
42 es: positivo
5! = 120
10! = 3628800
¿25 es mayor de 18? true
¿15 es mayor de 18? false
¿Aprueba con 85? true
¿150 es inválido? true
¿8 es par? true
¿7 es par? false
Contador final: 10
Doble: 42 / Fact5: 120
```

## Notes

- This is a single-file program. For multi-module programs with
  `import`/`export`, see [`examples/mods/`](../mods/).
- The output is long (21+ lines). It's a deliberate exercise of
  every feature, not a minimal example.

## Related examples

All of them — every other example in `examples/` covers a specific
feature in isolation.