# arcis — extensión para VS Code

Resaltado de sintaxis + Language Server para archivos `.tsr` (compilados
a Rust por `arcis`).

## Instalación

La extensión espera que `arcis-lsp` esté instalado y que `node`/`npm`
estén disponibles para compilar el cliente TypeScript.

```bash
# 1. Instalar el binario del LSP (una sola vez por máquina).
cargo install --path crates/arcis-lsp --force

# 2. Instalar la extensión: copia + compila el cliente TS + instala
#    las dependencias JS.
bash editor/install-vscode.sh
```

Reiniciá VS Code. Los archivos `*.tsr` aparecen con el lenguaje "Arcis"
en la barra de estado.

### Ubicación del binario `arcis-lsp`

El cliente TypeScript busca `arcis-lsp` en este orden:

1. `$ARCIS_LSP_BIN` (variable de entorno, override).
2. `~/.cargo/bin/arcis-lsp` (ubicación por defecto de `cargo install`).
3. Lo que `which arcis-lsp` devuelva en `PATH`.

Si no lo encuentra, la extensión funciona en modo "solo resaltado de
sintaxis" y muestra una notificación al usuario.

## Instalación manual

Si preferís no usar el script, los pasos son:

```bash
# Copiar los archivos de la extensión.
cp -r editor/vscode-arcis ~/.vscode/extensions/arcis-0.3.0

# Compilar el cliente TS (necesita npm install primero).
cd ~/.vscode/extensions/arcis-0.3.0
npm install
npm run build
```

## Qué provee

- **Resaltado de sintaxis** (TextMate grammar) — siempre disponible.
- **Autocompletado** contextual: keywords, primitive types, los
  ~50 builtins de `sys.*`, los métodos de `sys.<ns>.*`, métodos de
  array/string, namespaces. Triggered por `.` y `:`.
- **Hover** sobre cualquier builtin mostrando la firma + documentación
  breve.
- **Diagnósticos en vivo**: errores de lexer y parser aparecen como
  subrayados rojos en el editor.

## Qué reconoce el resaltado

- **Control de flujo**: `let`, `const`, `function`, `return`, `if`,
  `else`, `while`, `for`, `of`, `break`, `continue`
- **Módulos**: `import`, `export`, `from`, `default`, `as`
- **Tipos**: `string`, `number`, `boolean`, `void`
- **Constantes**: `true`, `false`
- **Strings** con escapes
- **Numbers** enteros y decimales
- **Comentarios** `//` y `/* */`
- **Llamadas a función** (`print`, `saludar`, etc.)
- **Operadores**: `+ - * / % == != < > <= >= && || ! =`
- **Puntuación**: `() {} [] , ; :`

## Desinstalar

```bash
rm -rf ~/.vscode/extensions/arcis-0.3.0
```
