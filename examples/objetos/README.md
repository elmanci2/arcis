# `objetos.tsr` — inline object types and `struct` codegen

How Arcis handles object literals with declared shape: the inline
type `{ name: string, edad: number }` becomes a Rust `struct`, and
each occurrence of that shape (regardless of source file) refers to
the same struct.

## Run it

```bash
arcis run examples/objetos/objetos.tsr
```

## What it covers

1. Object literal with two fields.
2. Object literal with three heterogeneous fields.
3. Arrays of objects.
4. Iteration over an array of objects with `for-of`.
5. Field reassignment on `obj.x = v`.
6. Field reassignment on `arr[i].x = v`.

## How it works

### Inline object types

When you write:

```ts
let persona: { nombre: string, edad: number } = {
    nombre: "Ana",
    edad: 30,
};
```

the codegen declares a struct at the root of the program:

```rust
#[derive(Clone)]
pub struct __Obja8edc {
    pub nombre: String,
    pub edad: f64,
}
```

The struct name is the hash of the field shape (truncated to 24 bits
and prefixed with `__Obj`). Two inline types with the same shape —
same field names, same field types, in the same order — get the
same struct name, so they are interchangeable.

### Field assignment

`persona.edad = 31;` translates to `persona.edad = 31.0;` — a plain
field assignment. The variable `persona` was declared as a regular
`let` (no `mut` because the codegen doesn't know about field-level
mutation yet), so Rust would refuse this on the generated
non-`mut` binding.

To allow mutation, Arcis detects whether the struct is ever assigned
to a field and emits `let mut` accordingly. Look for the variable
name in the assignment chain — if `obj.field = v` appears anywhere
in the program, the binding becomes `mut`.

### `arr[i].x = v` (chained field assignment)

For `puntos[0].x = 100`, the codegen emits `puntos[0].x = 100.0`. The
chain (Index → Member → Assign) is emitted inline.

## Expected output

```
Nombre: Ana
Edad: 30
Nueva edad: 31
Label: main
Max: 100
Debug ahora: true
Punto: (1, 2)
Punto: (3, 4)
Punto: (5, 6)
Total X: 9
Total Y: 12
puntos[0].x = 100
```

## Notes

- Object types must be **declared inline at the binding site**
  (`let x: { ... } = ...`). Inferring the type from a bare `let
  x = { ... };` is not supported in v1.
- All object fields are `pub` in the generated Rust struct. Arcis
  has no visibility modifier yet.
- Field reassignment requires the whole object to be declared
  `mut`, not just the field.

## Related examples

- [`examples/03-functions/`](../03-functions/) — functions that
  take and return objects.
- [`examples/arrays/`](../arrays/) — arrays of objects and iterating
  with `for-of`.