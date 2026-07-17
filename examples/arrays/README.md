# `arrays.tsr` — arrays and the seven method builtins

This example walks through every aspect of working with arrays in Arcis:
creation, indexing, assignment, iteration, and the seven method builtins
on `Vec<T>` (`find`, `filter`, `map`, `reduce`, `pop`, `push`, `unshift`).

## Run it

```bash
arcis run examples/arrays/arrays.tsr
```

## What it covers

| Section | Topic                                       |
|---------|---------------------------------------------|
| 1       | Creation, indexing, `.length`                |
| 2       | Indexed assignment (`arr[i] = v`)            |
| 3       | Summing via C-style `for`                   |
| 4       | Finding the maximum                         |
| 5       | Arrays of `string` and `.length` semantics  |
| 6       | Linear search (function returning an index) |
| 7       | Pre-sized arrays                            |
| 8       | Counting elements that match a predicate    |
| 9       | **The seven array methods** (see below)      |

## The seven array methods — deep dive

Arcis transpiles each array method to its Rust standard-library
equivalent. The closure pattern in the generated Rust depends on the
method's signature, so understanding the mapping matters when you write
callbacks.

| Method | Arcis source                  | Rust emitted                                                  |
|--------|-------------------------------|---------------------------------------------------------------|
| `find` | `arr.find(cb)`                | `arr.iter().find(\|&&x\| cb(x)).cloned().unwrap_or_default()` |
| `filter` | `arr.filter(cb)`            | `arr.iter().filter(\|&&x\| cb(x)).cloned().collect()`         |
| `map`  | `arr.map(cb)`                 | `arr.iter().map(\|&x\| cb(x)).collect()`                       |
| `reduce` | `arr.reduce(cb, init)`      | `arr.iter().fold(init, \|acc, &x\| cb(acc, x))`                |
| `pop`  | `arr.pop()`                   | `arr.pop().unwrap_or_default()`                                |
| `push` | `arr.push(v)`                 | `arr.push(v)` (translates directly)                            |
| `unshift` | `arr.unshift(v)`           | `arr.insert(0, v)`                                            |

### Why the closure patterns differ

`Vec<T>::iter()` returns an iterator over `&T`. That ripples into what
the closure has to accept:

- `find` and `filter` are given `&&T` (a reference to the `&T` the
  iterator yields), so the closure pattern is `|&&x|` and inside it
  `x: &T`.
- `map` is given `T` by value (the iterator's `map` calls `FnMut(T)`),
  so the closure pattern is `|&x|` and `x: T`.
- `reduce` is implemented via `fold`, which takes `FnMut(B, T)` where
  `T` is the element type, so the pattern is `|acc, &x|`.
- User-defined callback functions take `T` by **value**, so the
  callback is called with `cb(x)` — no dereferencing needed.

This is why `find`/`filter` look slightly different from `map` even
though they share the same "predicate/transform" pattern: `find` and
`filter` keep the element as a reference (because they don't own it),
while `map` is free to consume it.

### Why `find` and `filter` use `unwrap_or_default()` / `.collect()`

`Vec::iter().find(...)` returns `Option<&T>`. If the element is not
found we return the **default value of `T`**:

- For `number` (which is `f64` in Rust) the default is `0.0`.
- For `string` (`String` in Rust) the default is `""`.
- For `boolean` (`bool`) the default is `false`.

This mirrors TypeScript: `arr.find(cb)` returns `undefined` when the
predicate matches nothing, and TypeScript allows `number | undefined`
implicitly. In Arcis we make the default explicit, so the result type
stays `number` and you can keep using arithmetic without unwrapping.

### Why `reduce` takes the accumulator first, then the initial value

In TypeScript, `arr.reduce(cb, initial)` puts the callback first and the
initial value second. We preserve that order in the Arcis surface, even
though the underlying Rust is `fold(initial, |acc, &x| cb(acc, x))`.

### Why `pop` defaults to `0.0` instead of `-1`

JavaScript's `Array.prototype.pop` returns `undefined` when the array
is empty. Arcis can't represent `undefined` at the value level, so it
falls back to `T::default()` — `0.0` for numbers, `""` for strings,
`false` for booleans. Use the array's `.length` first if you need to
detect emptiness.

## Expected output

The full output is long (50+ lines); here is an excerpt:

```
nums.length: 5
nums[0]: 10
nums[4]: 50
Sum: 219
Max: 99
find: 2
filter length: 4
reduce: 8
pop: 8
after unshift[0]: 99
```

See the source for the rest.

## Common pitfalls

- **Indexed assignment requires the slot to exist.** `let arr: number[] = []; arr[0] = 5;`
  does NOT auto-grow the array — it panics in Rust with an index out
  of bounds. Initialise the array with the size you need, or use
  `push` to grow it.
- **Strings in `.length` count Unicode codepoints**, not bytes.
- **Array `.length` counts elements**, not memory slots.
- **`find` returns the default value when nothing matches**, not
  `null`/`undefined`.
- **Callbacks must be top-level named functions.** The codegen
  rewrites `arr.find(cb)` as `arr.iter().find(|&&x| cb(x))`, so
  inline lambdas (`(x) => x > 0`) are not yet supported. Declare
  the callback with `function name(...)` at the top level.

## Related examples

- [`examples/strings/`](../strings/) — every `String` method (`toUpperCase`, `trim`, …).
- [`examples/object/`](../object/) — arrays of objects.
- [`examples/03-functions/`](../03-functions/) — defining the callback functions used here.
- [`examples/loops/`](../loops/) — classic C-style `for` loops over arrays.