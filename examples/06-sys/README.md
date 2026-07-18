# `06-sys` — every `sys.*` builtin in one place

This example exercises the entire `sys.*` API end-to-end. Run it with:

```bash
arcis run examples/06-sys/sys.tsr
```

It groups the demos by the codegen submodule that implements each
namespace (`crates/arcis-codegen/src/sys/`):

| Section            | Implementation file | What it demos |
|--------------------|---------------------|---------------|
| `sys.fs`           | `sys/fs.rs`         | `readFile`, `writeFile`, `appendFile`, `listDir`, `createFile`, `deleteFile`, `mkdir`, `deleteDir`, `copy`, `move`, `rename`, `fileSize` |
| `sys.path`         | `sys/path.rs`       | `exists`, `isFile`, `isDir`, `fileSize`, `fileInfo`, `absolute`, `relative`, `createSymlink`, `readLink` |
| `sys.env` (top)    | `sys/proc_env.rs`   | `currentDir`, `changeDir`, `tempDir`, `homeDir`, `executablePath` |
| `sys.process`      | `sys/process.rs`    | `process` (returns the built-in `ArcisProcess` struct), `exec`, `spawn`, `kill`, `currentPid`, `parentPid`, `processes` |
| `sys.env` (vars)   | `sys/env.rs`        | `get`, `set`, `delete`, `all` (environment variables) |
| `sys.os`           | `sys/os.rs`         | `name`, `version`, `arch`, `hostname`, `username`, `uptime`, `locale`, `cpuCount` |
| `sys.memory`       | `sys/memory.rs`     | `total`, `free`, `used`, `available` |
| `sys.cpu`          | `sys/cpu.rs`        | `model`, `brand`, `frequency`, `usage`, `cores` |
| `sys.gpu`          | `sys/gpu.rs`        | `list`, `name`, `vendor`, `memory` |
| `sys.disk`         | `sys/disk.rs`       | `list`, `free`, `used`, `total` |
| `sys.net`          | `sys/net.rs`        | `hostname`, `interfaces`, `ip`, `publicIp`, `online` |

## What it does

1. Creates a temporary working directory under `sys.tempDir()`.
2. Runs every builtin in order, printing the result.
3. Cleans up the working directory at the end.

## Caveats

- The example is Linux-first (matches the rest of `sys.*`).
  On macOS / Windows, Linux-only builtins (`sys.memory.*`,
  `sys.cpu.*`, `sys.gpu.*`, several `sys.net.*`) will return `0`,
  `""`, `[]`, or `false` rather than real data.
- `sys.net.publicIp()` requires a working internet connection and
  `curl` installed; if either is missing it returns `""`.
- `sys.net.online()` requires `ping`; on networks that block ICMP
  it returns `false` even when other traffic works.