# Adding a builtin

Arcis ships a handful of built-in functions that do not require any
`import`. The two main families are:

1. `print` and `input` — top-level I/O functions.
2. `sys.*` — system calls (filesystem operations, `argv`).

This guide explains how to add another builtin in the same vein.
Future versions of this guide will cover array/string methods,
operator extensions, and type-coercion hooks.

## Where builtins live in code

The current implementation lives in **one place**, in
`crates/arcis-codegen/src/lib.rs`. The relevant functions are:

- `emit_expr` — top-level emitter for expressions. It pattern-matches
  on the expression kind to translate builtins before the generic
  `f(args)` fallback.
- `emit_sys_call` — translates `sys.<fn>(...)` calls into calls into
  `std::fs::*` and `std::env::*`.

In phase 2 these will be moved into focused modules:
`crates/arcis-codegen/src/builtin.rs` (for `print`/`input`) and the
method dispatch in `crates/arcis-codegen/src/method.rs` (for array /
string methods).

## Adding a new `sys.*` builtin

1. **Open** `crates/arcis-codegen/src/lib.rs` (or `builtin.rs` once
   that file exists in phase 2).
2. **Find** the `emit_sys_call` function.
3. **Add** a new match arm for your builtin name. Example:

   ```rust
   match property {
       // existing arms ...
       "readFile"  => { /* emit std::fs::read_to_string(...) */ }
       "writeFile" => { /* emit std::fs::write(...) */ }

       // NEW builtin: append the contents of `b` to the file at path `a`.
       "appendFile" => {
           out.push_str("std::fs::OpenOptions::new()");
           out.push_str(".append(true).create(true).open(");
           emit_arg_ref(out, &args[0], ctx);
           out.push_str(").expect(\"open\")");
           // Write `b` through the OpenOptions file by chaining .write_all.
           // ...
       }

       _ => { /* unknown sys.X — emit as-is */ }
   }
   ```

4. **Document** the builtin:
   - Add an entry to the "Supported subset" table in
     `docs/language-reference.md`.
   - Mention it in the README under the relevant section.
5. **Add a test** in `tests/codegen.rs` that lexes + parses + generates
   a snippet using the new builtin, and asserts on the emitted code.

## Adding a new `print`-like function

`print` is handled as a special case inside `emit_expr`, near the top
of the `Expr::Call` arm:

```rust
if let Expr::Ident(name) = callee.as_ref() {
    if name == "print" {
        out.push_str("println!(\"{}\", ");
        // ... emit first arg ...
        return;
    }
    if name == "input" {
        out.push_str("{ let mut __arcis_input = String::new(); \
                      std::io::stdin().read_line(&mut __arcis_input).unwrap(); \
                      __arcis_input.trim_end().to_string() }");
        return;
    }
}
```

To add a new global function, add another `if name == "your_func" { ... }`
arm in the same spot, following the same pattern.

## Adding a new array / string method

Methods are translated in the chained-method block inside `emit_expr`
right below the `sys.*` case. Each method uses a `match property.as_str()`
arm. To add, e.g., `slice`:

```rust
"slice" => {
    emit_expr(out, object, ctx);
    out.push('[');
    if let Some(a) = args.first() {
        emit_expr(out, a, ctx);
        out.push_str(" as usize");
    }
    out.push_str("..");
    if let Some(b) = args.get(1) {
        emit_expr(out, b, ctx);
        out.push_str(" as usize");
    }
    out.push_str("].to_vec()");
    return;
}
```

Document it in `docs/language-reference.md` and add a test.

## Style rules

- Always push to `out: &mut String` directly — never go through a
  temporary `String`.
- Use lower-case idiomatic Rust output. Let `rustfmt` reformat if you
  want pretty-looking strings; the emitted code ends up in a `.rs`
  file that will be `cargo fmt`-cleaned anyway.
- Prefer emitting entire expressions over emitting fragments and
  concatenating.

## Verifying the change

After adding a builtin:

```bash
cargo test --workspace     # all unit + integration tests pass
cargo run -- run examples/your-fixture.tsr   # smoke-test the new builtin
```

If the new builtin needs a fixture, add it under `examples/`.
If the smoke test is enough, skip the fixture.

## What's next?

When phase 2 lands:

- A shared `arcis-diagnostics` crate for nice error messages when a
  builtin is misused.
- A `arcis-lsp` crate that exposes `print`, `input`, `sys.*`, and
  every method as a completion item.

For now, the codegen layer is the single source of truth.