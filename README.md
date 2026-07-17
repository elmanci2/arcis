# Arcis

Un clon mínimo de **TypeScript** (`.tsr`) escrito en Rust que **compila a binario nativo**.

La idea: la sintaxis del lenguaje es idéntica a TS — mismas palabras clave, misma forma de declarar variables, funciones, etc. La diferencia es que en vez de ejecutarse vía Node/Deno, los archivos `.tsr` se transpilan a Rust y se compilan con `rustc` para producir un binario.

## Uso rápido

```bash
# Compilar un .tsr y dejar el binario en ./bin/
cargo run -- build examples/hola.tsr
./bin/hola

# Compilar y ejecutar directo
cargo run -- run examples/hola.tsr

# Solo ver el código Rust generado (sin invocar rustc)
cargo run -- check examples/hola.tsr

# Proyecto multi-archivo: por convención, el punto de entrada es `main.tsr`
# (sin argumento usa `./main.tsr`; con un directorio, busca `<dir>/main.tsr`)
cargo run -- run
cargo run -- run examples/mods
```

Una vez instalado (ver abajo), todo esto se invoca como `arcis` directamente:

```bash
arcis run examples/mods
arcis run                   # usa ./main.tsr
arcis init mi-proyecto      # crea un proyecto nuevo en ./mi-proyecto
```

## Inicio rápido: crear un proyecto nuevo

```bash
arcis init mi-proyecto
cd mi-proyecto
arcis run
```

`arcis init [<dir>]` crea un `main.tsr` mínimo con un "Hola" listo para
ejecutar. Sin argumento usa el directorio actual. Si ya existe un `main.tsr`
aborta sin sobreescribir.

## Instalación como comando del sistema

```bash
cargo install --path .        # deja el binario en ~/.cargo/bin/arcis
which arcis                   # confirmar que está en el PATH
```

Tras eso podés usar `arcis run`, `arcis build`, `arcis check` desde cualquier
directorio, sin tener que estar en el repo.

```bash
# Ejemplo en un directorio cualquiera:
mkdir mi-app && cd mi-app
# … escribir main.tsr y módulos …
arcis run
```

Para desinstalar: `cargo uninstall arcis` o `rm ~/.cargo/bin/arcis`.
Para actualizar tras cambios: `cargo install --path . --force`.

## Subset soportado

- `let` / `const` con anotación de tipo opcional
- Tipos primitivos: `string`, `number`, `boolean`, `void`
- `function nombre(p: T, ...): T { ... }` con `return`
- `if (cond) { ... } else { ... }`, `while`, `for`, `for (let x of arr)`, `break`, `continue`
- `print(expr);` (atajo a `println!`)
- Literales: `"string"`, `42`, `3.14`, `true`, `false`, `[...]`, `{ clave: valor }`
- Tipos objeto inline: `let p: { nombre: string, edad: number } = ...`
- Reasignación (`x = ...`), asignación indexada (`arr[i] = ...`) y de campo (`obj.x = ...`)
- Operadores: `+ - * / % == != < > <= >= && || !`
- Comentarios `//` y `/* ... */`
- **Módulos**: `import`/`export` con sintaxis TypeScript (ver [Módulos](#módulos))

## Ejemplo

`examples/hola.tsr`:

```ts
let nombre: string = "Mundo";
let anio: number = 2026;

function saludar(quien: string, edad: number): string {
    return "Hola " + quien + " en el año " + edad;
}

let mensaje: string = saludar(nombre, anio);
print(mensaje);

if (anio > 2000) {
    print("Bienvenido al siglo XXI");
} else {
    print("Viajero del tiempo");
}
```

Salida:

```
Hola Mundo en el año 2026
Bienvenido al siglo XXI
```

## Módulos

Por convención el punto de entrada es **`main.tsr`**. Sin argumentos, `arcis run` usa `./main.tsr`; pasando un directorio busca `<dir>/main.tsr`. Se respeta la sintaxis de TypeScript para `import`/`export`:

```ts
// utils.tsr
export const PI: number = 3.14;
export function sumar(a: number, b: number): number { return a + b; }
export default function calcula(n: number): number { return n * PI; }
```

```ts
// main.tsr (punto de entrada)
import { sumar, PI } from "mate";              // import nombrado
import { sumar as s } from "mate";             // con alias
import calcula from "mate";                    // import por defecto
import calcula, { PI } from "mate";            // default + nombrados
export function f() { ... }                    // export inline
export const X = 1;                            // export const (valor const)
export { f, X as Y };                          // re-export (con alias)
export default function () { ... }             // export por defecto
```

Las rutas de import son relativas al directorio del archivo que importa (sin prefijo `./`): `from "mate"` resuelve a `<dir>/mate.tsr`. Internamente cada `.tsr` se transpila a un `.rs` separado y `main.rs` los declara con `mod <id>;`, referenciando items vía `use crate::<id>::...;`. Esto se traduce a módulos Rust reales (`pub fn`, `pub const`, `pub use self::...`), así que la visibilidad y los nombres deben ser identificadores Rust válidos (sin guiones, sin empezar por dígito).

Ejemplo completo: `examples/mods/`.

### Limitaciones de los módulos

- **Top-level `let`/`const` en módulos que no son `main`** se emiten como `const` de Rust, así que el valor debe ser evaluable en tiempo de compilación. Para `export const X = funcion() {...}` con funciones no-const-eval, rustc se quejará con un error claro.
- **Nombres de módulo**: deben ser identificadores Rust válidos (el stem del archivo: `[A-Za-z_][A-Za-z0-9_]*`). `from "my-mod"` falla con un error de linkado.
- **Tipos objeto** (`{ a: T, ... }`) se centralizan en `main.rs` como `pub struct`, y los módulos no-`main` los referencian como `crate::__ObjNAME`. Esto evita duplicados entre módulos.
- **`export { privada as publica }`** sobre un item privado no se puede re-exportar (Rust exige que el item original sea público). Para preservar encapsulamiento en este caso habría que emitir un wrapper; por ahora el item original debe ser público.
- **No** se soporta `import * as ns` (namespace) en este paso.

## Fuera de alcance (por ahora)

- Clases / interfaces
- `import * as ns` (namespace import)

## Cómo funciona

```
main.tsr (+ utils.tsr, ...)
   │
   ▼ linker (DFS, valida exports/imports, ciclos)
┌─────────┐  tokens   ┌────────┐   AST    ┌────────────────┐  bin/*.rs
│  Lexer  │ ────────▶ │ Parser │ ───────▶ │ Codegen (xN)   │ ───────▶ rustc → binario
└─────────┘           └────────┘          └────────────────┘
                                              │
                                              └─ main.rs: mod …; use crate::…;
```

El codegen emite código Rust usando:

| TS       | Rust        |
|----------|-------------|
| `string` | `String`    |
| `number` | `f64`       |
| `boolean`| `bool`      |
| `void`   | `()`        |

La concatenación con `+` se traduce a `format!("{}{}", a, b)` cuando alguno de los operandos es un literal string; en otro caso usa `+` directo. Esto cubre `print("Hola " + edad)` sin runtime adicional.

## Limitaciones conocidas

- El tipado es **estático en el parser pero no se verifica**: si escribes `let x: number = "hola";`, el código se genera igual y `rustc` se quejará. Está bien para una primera prueba.
- No hay reasignación: `let x = 1; x = 2;` no compilará porque nuestro parser no soporta la sentencia de asignación.
- `const` se emite como `let` con nombre en MAYÚSCULAS (Rust exige valores en tiempo de compilación para `const`, lo cual limita casos de uso).

## Próximos pasos (ideas)

- Reasignación (`x = expr;`)
- Bucles `while`
- Arrays / strings indexados
- Inferencia de tipos básica
- Tests unitarios por fase (lex, parse, codegen)