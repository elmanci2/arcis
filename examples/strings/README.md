# `strings.tsr` — string methods + `for-of` over arrays

All seven string methods Arcis supports, plus iteration over arrays
with `for-of`.

## Run it

```bash
arcis run examples/strings/strings.tsr
```

## What it covers

1. Case and whitespace: `toUpperCase`, `toLowerCase`, `trim`.
2. Substring search: `substring`, `indexOf`, `includes`.
3. Character access: `charAt`.
4. `for-of` iteration with `break` and `continue`.
5. Composing methods.

## The string methods — table

| Method           | Arcis source                  | Rust emitted                                        |
|------------------|-------------------------------|-----------------------------------------------------|
| `toUpperCase`    | `s.toUpperCase()`             | `s.to_uppercase()`                                  |
| `toLowerCase`    | `s.toLowerCase()`             | `s.to_lowercase()`                                  |
| `trim`           | `s.trim()`                    | `s.trim().to_string()`                              |
| `substring(a, b)`| `s.substring(a, b)`           | `s[a as usize..b as usize].to_string()`             |
| `indexOf(sub)`   | `s.indexOf(sub)`              | `s.find(&sub).map(\|i\| i as f64).unwrap_or(-1.0)` |
| `includes(sub)`  | `s.includes(sub)`             | `s.contains(&sub)`                                  |
| `charAt(i)`      | `s.charAt(i)`                 | `s.chars().nth(i as usize).unwrap_or_default().to_string()` |
| `.length`        | `s.length`                    | `s.chars().count() as f64` (for strings), `.len() as f64` (for arrays) |

### Why `trim` returns `&str` → `String` and not just `&str`

Rust's `String::trim` returns `&str` (a borrow of the original). To
make the result usable as an owned `String` (the Arcis type), the
codegen appends `.to_string()`. The Arcis surface stays a clean
`string` — there's no separate "borrowed string" type.

### Why `indexOf` returns `-1` instead of `Option<f64>`

TypeScript's `String.prototype.indexOf` returns `-1` when not found.
Arcis mirrors that, so you can keep using `indexOf` directly in
arithmetic or comparisons without unwrapping an `Option`.

### Why `charAt` returns the empty string on out-of-range

Same reasoning — TypeScript returns `""` for an out-of-range index,
not `undefined`. We use `unwrap_or_default()` which yields `""`.

### Why `.length` on a string is in characters, not bytes

Arcis exposes `String::chars().count()` so multi-byte characters
are counted correctly. `"ñ".length` is `1`, not `2`. If you need
the byte length for some reason, drop down to Rust via a `crate:`
import.

## Why iterate arrays with `for (let x of arr)`

`for (let x of arr) { ... }` translates to either:

- `for x in arr.iter().cloned()` when `arr` is a plain `Vec` local
  (we don't want to consume the variable).
- `for x in arr.into_iter()` when `arr` is the result of a function
  call or member access (typical of `Iterator`-returning Rust APIs).

The first form preserves the array for later use; the second
consumes the iterator. You don't need to think about which one to
write — the codegen picks based on whether the iterable is a plain
identifier or a more complex expression.

## Expected output

```
toUpperCase: HELLO WORLD
toLowerCase: hello world
trim: 'hello'
substring(0,4): Hello
substring(5): World
indexOf('World'): 5
indexOf('xyz'): -1
includes('World'): true
includes('xyz'): false
charAt(0): H
charAt(5): W
charAt(99): ''
for-of sum: 100
Without 20: 3
Found: 30
Normalized: 'hello'
Contains 'hello'? true
```

## Notes

- `s.length` on a string returns a `number` (Rust `f64`). The value
  is the count of Unicode characters, **not bytes**.
- `s.length` on an array returns the element count.

## Related examples

- [`examples/arrays/`](../arrays/) — the array methods that pair
  naturally with string iteration.
- [`examples/loops/`](../loops/) — C-style `for` loops as an
  alternative to `for-of`.