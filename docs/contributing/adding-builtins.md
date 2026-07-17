# Adding a builtin

Arcis ships a handful of built-in functions that do not require any
`import`. The two main families are:

1. `print` and `input` — top-level I/O functions.
2. `sys.*` — system calls (filesystem operations, `argv`).

This guide explains how to add another builtin in the same vein.
Future versions of this guide will cover array/string methods,
operator extensions, and type-coercion hooks.

## Where builtins live in code

The current implementation is split across focused modules:

- `crates/arcis-codegen/src/builtin.rs` — `print` and `input`.
- `crates/arcis-codegen/src/method.rs` — array / string method dispatch.
- `crates/arcis-codegen/src/sys/` — `sys.*` builtins, dispatched across
  three submodules:
  - `sys/mod.rs` — top-level dispatcher (`emit_call`, `emit_member`),
    shared `emit_arg_ref` helper, and the verbatim fallback for unknown
    `sys.X` calls.
  - `sys/fs.rs` — file-system operations (read, write, create, delete,
    copy, move, list).
  - `sys/path.rs` — path queries and transformations (exists, isFile,
    isDir, size, absolute, relative, symlink).
  - `sys/env.rs` — process / environment queries (currentDir, tempDir,
    homeDir, executablePath, changeDir).

The dispatch point for `sys.<X>(...)` is `expr.rs` → `emit_call`, which
detects the `sys` namespace identifier and forwards to
`crate::sys::emit_call`. Unknown `sys.X` are re-emitted verbatim so
`rustc` produces the diagnostic.

## Adding a new `sys.*` builtin

1. **Pick the right submodule.** File/dir IO goes in
   `crates/arcis-codegen/src/sys/fs.rs`. Path queries and
   transformations go in `sys/path.rs`. Process/environment lookups go
   in `sys/env.rs`. If you are unsure, look at the doc-comment table at
   the top of each submodule.
2. **Add a match arm** to the appropriate `try_emit(...)` (or
   `try_emit_call(...)`) function. Example, in `sys/fs.rs`:

   ```rust
   "appendFile" => {
       // Bind the opened file into a local so we can take `&mut`
       // and call `write_all` via the qualified trait path.
       out.push_str(
           "({ let mut __f = std::fs::OpenOptions::new()\
            .append(true).create(true).open(",
       );
       emit_arg_ref(out, args.first(), ctx);
       out.push_str(").unwrap(); std::io::Write::write_all(&mut __f, ");
       if let Some(t) = args.get(1) {
           crate::expr::emit(out, t, ctx);
           out.push_str(".as_bytes()");
       } else {
           out.push_str("b\"\"");
       }
       out.push_str(").unwrap(); })");
       true
   }
   ```

   The match arm must return `true` to claim the property; if it falls
   through to `_ => false`, the dispatcher will continue to the next
   submodule and eventually to the verbatim fallback.

3. **Document** the builtin:
   - Add a row to the table at the top of the submodule's doc-comment.
   - Update the `sys.*` row in
     `docs/language-reference.md` if you added a new category.
   - Mention it in the README under the relevant section.
4. **Add a test** in `crates/arcis-codegen/tests/sys_codegen.rs`. The
   test lexes + parses + generates Rust for a snippet invoking the
   builtin, and asserts that the emitted source contains the expected
   `std::*` call. Use the existing tests as templates.

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