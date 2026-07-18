/**
 * Arcis Language Client for VS Code.
 *
 * Wires up the `arcis-lsp` binary (installed via
 * `cargo install --path crates/arcis-lsp --force`) and exposes its
 * completion / hover / diagnostics features to the editor.
 *
 * The binary location is resolved in this order:
 *   1. `ARCIS_LSP_BIN` environment variable (override).
 *   2. `~/.cargo/bin/arcis-lsp` (default `cargo install` location).
 *   3. Whatever `which arcis-lsp` returns on PATH.
 *
 * If the binary cannot be found, the extension falls back to syntax
 * highlighting only (the original behaviour) and shows a notification.
 */

import * as path from "path";
import * as os from "os";
import * as fs from "fs";
import {
  ExtensionContext,
  workspace,
  window,
} from "vscode";
import {
  LanguageClient,
  LanguageClientOptions,
  ServerOptions,
  Executable,
  TransportKind,
} from "vscode-languageclient/node";

let client: LanguageClient | undefined;

/** Resolve the path to the `arcis-lsp` binary. */
function resolveLspPath(): string | undefined {
  const fromEnv = process.env.ARCIS_LSP_BIN;
  if (fromEnv && fs.existsSync(fromEnv)) {
    return fromEnv;
  }
  const defaultPath = path.join(os.homedir(), ".cargo", "bin", "arcis-lsp");
  if (fs.existsSync(defaultPath)) {
    return defaultPath;
  }
  // Last resort: hope it's on PATH and let `executable.command` find it.
  return "arcis-lsp";
}

export async function activate(ctx: ExtensionContext): Promise<void> {
  const cmd = resolveLspPath();
  if (!cmd) {
    void window.showWarningMessage(
      "arcis-lsp binary not found — completion, hover, and diagnostics are disabled. Install with `cargo install --path crates/arcis-lsp --force`.",
    );
    return;
  }

  const run: Executable = {
    command: cmd,
    transport: TransportKind.stdio,
  };
  const serverOptions: ServerOptions = { run, debug: run };

  const clientOptions: LanguageClientOptions = {
    documentSelector: [{ scheme: "file", language: "arcis" }],
    // Synchronize settings (currently none, but the hook is here so a
    // future `arcis.*` configuration can be plumbed through).
    synchronize: {},
  };

  client = new LanguageClient(
    "arcis",
    "Arcis Language Server",
    serverOptions,
    clientOptions,
  );

  // Notify the user once on startup so they know the LSP is running.
  void window.showInformationMessage(
    `arcis-lsp: connected to ${path.basename(cmd)}`,
  );

  await client.start();
}

export function deactivate(): Thenable<void> | undefined {
  if (!client) {
    return undefined;
  }
  return client.stop();
}
