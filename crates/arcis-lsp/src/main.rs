//! `arcis-lsp` binary entry point.
//!
//! Reads JSON-RPC messages from stdin and writes them to stdout, using
//! [`async_lsp::MainLoop`]. The server itself is constructed in
//! [`crate::server::build_router`].

use async_lsp::stdio::PipeStdin;
use async_lsp::stdio::PipeStdout;
use async_lsp::MainLoop;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let (server, _) =
        MainLoop::new_server(|client| arcis_lsp::server::build(client));

    // On Unix, async-lsp's PipeStdin/PipeStdout are truly async. On
    // other platforms the binary panics at startup; that is fine for
    // our Linux-first focus.
    #[cfg(unix)]
    let stdin = PipeStdin::lock_tokio().expect("lock stdin");
    #[cfg(unix)]
    let stdout = PipeStdout::lock_tokio().expect("lock stdout");
    #[cfg(not(unix))]
    compile_error!("arcis-lsp currently only supports Unix targets");

    server.run_buffered(stdin, stdout).await.expect("LSP server failed");
}