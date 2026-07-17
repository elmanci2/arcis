# arcis — extensión para VS Code

Resaltado de sintaxis para archivos `.tsr` (compilados a Rust por `arcis`).

## Instalación rápida (sin empaquetar)

La extensión es un directorio con `package.json` válido. VS Code detecta los
archivos `.tsr` automáticamente al copiarla bajo `~/.vscode/extensions/`:

```bash
# desde la raíz del proyecto arcis
bash editor/install-vscode.sh
```

O manualmente:

```bash
cp -r editor/vscode-arcis ~/.vscode/extensions/arcis-0.2.0
```

Luego **reinicia VS Code**. Los archivos `*.tsr` aparecen con el lenguaje
"Arcis" en la barra de estado, y `import`/`export`/`from`/`as` quedan
resaltados con su scope propio.

## Instalación empaquetada (opcional)

Si tienes `vsce` (`npm install -g @vscode/vsce`):

```bash
cd editor/vscode-arcis
npm run install:dev        # instala como carpeta (sin vsix)
# o, para empaquetar un .vsx:
npm run package
code --install-extension arcis-0.2.0.vsix
```

## Qué reconoce

- **Control de flujo**: `let`, `const`, `function`, `return`, `if`, `else`, `while`, `for`, `of`, `break`, `continue`
- **Módulos** (resaltado en un scope distinto, `keyword.control.import`): `import`, `export`, `from`, `default`, `as`
- **Tipos**: `string`, `number`, `boolean`, `void`
- **Constantes**: `true`, `false`
- **Strings** con escapes
- **Numbers** enteros y decimales
- **Comentarios** `//` y `/* */`
- **Llamadas a función** (`print`, `saludar`, etc.)
- **Operadores**: `+ - * / % == != < > <= >= && || ! =`
- **Puntuación**: `() {} [] , ; :`

## Alcance

Es solo **resaltado de sintaxis** (TextMate grammar). NO provee:
- Autocompletado
- Diagnóstico de errores en vivo
- Hover / go-to-definition

Para esas funcionalidades hace falta un Language Server, que es un proyecto
aparte (no incluido).

## Desinstalar

```bash
rm -rf ~/.vscode/extensions/arcis-0.2.0
```