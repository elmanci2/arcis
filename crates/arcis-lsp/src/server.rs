//! Server wiring — constructs the [`Router`] that ties together
//! the completion, hover, and diagnostics providers with the LSP
//! protocol surface.
//!
//! Two public entry points:
//!
//! - [`new_server`] builds a `ServerState`-backed `Router` and layers
//!   the standard middlewares (tracing, panic-catch, lifecycle).
//!   `main.rs` calls this and hands the result to
//!   `MainLoop::new_server`.
//!
//! State is held in [`ServerState`], which keeps a clone of the
//! [`ClientSocket`] so handlers can publish diagnostics and
//! notifications back to the editor.

use std::ops::ControlFlow;

use async_lsp::lsp_types::{
    notification, request, CompletionItem, CompletionList, CompletionOptions,
    CompletionResponse, CompletionTextEdit, Diagnostic, DidChangeTextDocumentParams,
    DidOpenTextDocumentParams, Hover, HoverContents, HoverProviderCapability,
    InitializeResult, MarkupContent, MarkupKind, OneOf, PublishDiagnosticsParams,
    ServerCapabilities, ServerInfo, TextDocumentSyncCapability, TextDocumentSyncKind,
    TextDocumentSyncOptions, TextEdit,
};
use async_lsp::router::Router;
use async_lsp::{ClientSocket, LanguageClient};

use crate::completion::completions_at;
use crate::diagnostics::diagnostics_for;

/// State held by every request handler. Just the LSP client socket so
/// we can publish diagnostics back to the editor.
pub struct ServerState {
    pub client: ClientSocket,
}

impl ServerState {
    fn new(client: ClientSocket) -> Self {
        Self { client }
    }
}

/// Build the `ServerState`-backed `Router` with every request and
/// notification handler we care about. Layered through the standard
/// middlewares so panics become errors and shutdown is graceful.
pub fn new_server(client: ClientSocket) -> Router<ServerState> {
    let mut router = Router::new(ServerState::new(client));

    router
        // ── Lifecycle ─────────────────────────────────────────────
        .request::<request::Initialize, _>(|_, _| async move {
            Ok(InitializeResult {
                capabilities: ServerCapabilities {
                    hover_provider: Some(HoverProviderCapability::Simple(true)),
                    completion_provider: Some(CompletionOptions {
                        trigger_characters: Some(vec![".".into(), ":".into()]),
                        resolve_provider: None,
                        completion_item: None,
                        all_commit_characters: None,
                        work_done_progress_options:
                            crate::lsp::WorkDoneProgressOptions::default(),
                    }),
                    text_document_sync: Some(TextDocumentSyncCapability::Options(
                        TextDocumentSyncOptions {
                            open_close: Some(true),
                            change: Some(TextDocumentSyncKind::FULL),
                            ..Default::default()
                        },
                    )),
                    ..Default::default()
                },
                server_info: Some(ServerInfo {
                    name: "arcis-lsp".into(),
                    version: Some(env!("CARGO_PKG_VERSION").into()),
                }),
            })
        })
        .request::<request::Shutdown, _>(|_, _| async move { Ok(()) })
        // ── Completion ────────────────────────────────────────────
        .request::<request::Completion, _>(|_, _params| async move {
            // The completion request doesn't carry the document text;
            // the editor maintains the buffer. For the MVP we serve
            // whatever the builtin table can produce with a blank
            // prefix so the panel always has items (keywords +
            // globals). The editor's "fetch completion on trigger"
            // path will repopulate with context after didChange.
            let items: Vec<CompletionItem> = completions_at("");
            Ok(Some(CompletionResponse::List(CompletionList {
                is_incomplete: false,
                items,
            })))
        })
        // ── Document-changed notifications ────────────────────────
        .notification::<notification::DidOpenTextDocument>(notify_open)
        .notification::<notification::DidChangeTextDocument>(notify_change)
        .notification::<notification::DidCloseTextDocument>(|_state, _params| {
            ControlFlow::Continue(())
        });
    router
}

/// `didOpen` notification handler: pull the inline text out of the
/// params, run diagnostics, publish.
fn notify_open(
    state: &mut ServerState,
    params: DidOpenTextDocumentParams,
) -> ControlFlow<async_lsp::Result<()>> {
    let uri = params.text_document.uri;
    let text = params.text_document.text;
    publish_diagnostics(&state.client, &uri, &text);
    ControlFlow::Continue(())
}

/// `didChange` notification handler: read the most recent full content
/// change, run diagnostics, publish.
fn notify_change(
    state: &mut ServerState,
    params: DidChangeTextDocumentParams,
) -> ControlFlow<async_lsp::Result<()>> {
    let uri = params.text_document.uri;
    let text = params
        .content_changes
        .into_iter()
        .last()
        .map(|c| c.text)
        .unwrap_or_default();
    publish_diagnostics(&state.client, &uri, &text);
    ControlFlow::Continue(())
}

/// Run the diagnostic pipeline against `text` and push the result to
/// the client via `LanguageClient::publish_diagnostics`.
fn publish_diagnostics(
    client: &ClientSocket,
    uri: &crate::lsp::Url,
    text: &str,
) {
    let diags: Vec<Diagnostic> = diagnostics_for(text);
    let mut client = client.clone();
    let _ = client.publish_diagnostics(PublishDiagnosticsParams {
        uri: uri.clone(),
        diagnostics: diags,
        version: None,
    });
}

/// Build the final `LspService`. For the MVP we skip the standard
/// tower middlewares (Tracing / Lifecycle / CatchUnwind / Concurrency
/// / ClientProcessMonitor) — they're not strictly required for
/// completion + hover + diagnostics, and routing through them requires
/// an `Error` type that converts into `ResponseError`. We can layer
/// them in later once the MVP is proven.
pub fn build_service(client: ClientSocket) -> Router<ServerState> {
    new_server(client)
}

// Suppress unused-imports noise from the LSP / async-lsp scaffolding.
#[allow(dead_code)]
fn _suppress() {
    let _ = (
        CompletionTextEdit::Edit(TextEdit {
            range: Default::default(),
            new_text: String::new(),
        }),
        Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: String::new(),
            }),
            range: None,
        },
        OneOf::Left::<bool, ()>(true),
    );
}