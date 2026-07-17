#!/usr/bin/env bash
# Instala la extensión arcis en VS Code copiándola bajo ~/.vscode/extensions/.
# No requiere empaquetar con vsce.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SRC="$SCRIPT_DIR/vscode-arcis"
DEST="${HOME}/.vscode/extensions/arcis-0.2.0"

if [ ! -d "$SRC" ]; then
    echo "ERROR: no se encontró $SRC" >&2
    exit 1
fi

if [ -e "$DEST" ]; then
    echo "Ya existe $DEST. Reemplazando..."
    rm -rf "$DEST"
fi

cp -r "$SRC" "$DEST"
echo "✓ Extensión instalada en $DEST"
echo "Reinicia VS Code para que surta efecto."