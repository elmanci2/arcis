#!/usr/bin/env bash
# Instala la extensión arcis en VS Code copiándola bajo
# ~/.vscode/extensions/. Requiere:
#   - Que `arcis-lsp` esté instalado (cargo install --path crates/arcis-lsp).
#   - Que `node`/`npm`/`tsc` estén disponibles para compilar el cliente TS.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SRC="$SCRIPT_DIR/vscode-arcis"
DEST="${HOME}/.vscode/extensions/arcis-0.3.0"

# Verify the LSP binary is installed somewhere reachable.
if [ ! -x "${HOME}/.cargo/bin/arcis-lsp" ] && ! command -v arcis-lsp >/dev/null 2>&1; then
    echo "ERROR: arcis-lsp binary not found. Install with:" >&2
    echo "    cargo install --path crates/arcis-lsp --force" >&2
    exit 1
fi

# Copy the extension into ~/.vscode/extensions.
if [ -e "$DEST" ]; then
    echo "Reemplazando $DEST..."
    rm -rf "$DEST"
fi
cp -r "$SRC" "$DEST"

# Install JS deps (vscode-languageclient) and build the TS client.
(cd "$DEST" && npm install --no-fund --no-audit --silent && npm run build --silent)

echo "✓ Extensión + LSP client instalados en $DEST"
echo "  Asegurate de que ~/.cargo/bin/arcis-lsp esté en el PATH."
echo "  Reiniciá VS Code para que surta efecto."