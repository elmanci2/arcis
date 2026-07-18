//! Server wiring — constructs the [`Router`] that ties together
//! the completion, hover, and diagnostics providers with the LSP
//! protocol surface.
//!
//! Two public entry points:
//!
//! - [`build`] builds a `ServerState`-backed `Router` and hands it
//!   to `MainLoop::new_server` in `main.rs`.
//!
//! State is held in [`ServerState`], which carries a [`DocumentStore`]
//! (text per URI) and a clone of the [`ClientSocket`] for publishing
//! diagnostics. Keeping the per-document text in the state lets
//! `completion` compute the prefix *up to the cursor* — without it,
//! the LSP can't tell `sys.|` from `|sys` and completion degenerates
//! to "all top-level builtins".

use std::collections::HashMap;
use std::ops::ControlFlow;

use async_lsp::lsp_types::{
    notification, request, CompletionItem, CompletionList, CompletionOptions,
    CompletionResponse, CompletionTextEdit, CompletionParams, Diagnostic,
    DidChangeTextDocumentParams, DidOpenTextDocumentParams, Hover, HoverContents,
    HoverProviderCapability, InitializeResult, MarkupContent, MarkupKind, OneOf,
    Position, Range, PublishDiagnosticsParams, ServerCapabilities, ServerInfo,
    TextDocumentSyncCapability, TextDocumentSyncKind, TextDocumentSyncOptions,
    TextEdit, Url,
};
use async_lsp::router::Router;
use async_lsp::{ClientSocket, LanguageClient};

use crate::completion::completions_at;
use crate::diagnostics::diagnostics_for;

/// Per-URI in-memory text store. Updated by `didOpen` and `didChange`;
/// read by `completion` to compute the cursor prefix.
#[derive(Default)]
pub struct DocumentStore {
    docs: HashMap<Url, String>,
}

impl DocumentStore {
    /// Insert or replace the document text for `uri`.
    fn upsert(&mut self, uri: Url, text: String) {
        self.docs.insert(uri, text);
    }

    /// Read the document text for `uri` if known.
    fn get(&self, uri: &Url) -> Option<&str> {
        self.docs.get(uri).map(String::as_str)
    }
}

/// State held by every request handler.
pub struct ServerState {
    pub client: ClientSocket,
    pub docs: DocumentStore,
}

impl ServerState {
    fn new(client: ClientSocket) -> Self {
        Self {
            client,
            docs: DocumentStore::default(),
        }
    }
}

/// Build the `ServerState`-backed `Router` with every request and
/// notification handler we care about.
pub fn build(client: ClientSocket) -> Router<ServerState> {
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
                    document_formatting_provider: Some(OneOf::Left(true)),
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
        .request::<request::Completion, _>(|state, params| {
            let uri = params.text_document_position.text_document.uri;
            let pos = params.text_document_position.position;
            let prefix = state
                .docs
                .get(&uri)
                .map(|text| prefix_up_to(text, pos))
                .unwrap_or_default();
            let items: Vec<CompletionItem> = completions_at(&prefix);
            async move {
                Ok(Some(CompletionResponse::List(CompletionList {
                    is_incomplete: false,
                    items,
                })))
            }
        })
        // ── Hover ────────────────────────────────────────────────
        .request::<request::HoverRequest, _>(|state, params| {
            let uri = &params.text_document_position_params.text_document.uri;
            let pos = params.text_document_position_params.position;
            let text = state.docs.get(uri).unwrap_or_default();
            let (before, after) = split_at_position(text, pos);
            let hover = crate::hover::hover_at(&before, &after);
            async move { Ok(hover) }
        })
        // ── Formatting ────────────────────────────────────────────
        .request::<request::Formatting, _>(|state, params| {
            let uri = params.text_document.uri;
            let text = state.docs.get(&uri).unwrap_or_default().to_string();
            let formatted = arcis_fmt::format(&text).unwrap_or(text.clone());
            let line_count = text.lines().count();
            async move {
                Ok(Some(vec![TextEdit {
                    range: Range::new(
                        Position::new(0, 0),
                        Position::new(line_count as u32 + 1, 0),
                    ),
                    new_text: formatted,
                }]))
            }
        })
        // ── Document-changed notifications ────────────────────────
        .notification::<notification::Initialized>(|_state, _params| {
            ControlFlow::Continue(())
        })
        .notification::<notification::DidOpenTextDocument>(on_did_open)
        .notification::<notification::DidChangeTextDocument>(on_did_change)
        .notification::<notification::DidCloseTextDocument>(|_state, _params| {
            ControlFlow::Continue(())
        })
        // `exit` notification: break the main loop so the process
        // terminates cleanly.
        .notification::<notification::Exit>(|_, _| ControlFlow::Break(Ok(())));
    router
}

/// Handle `textDocument/completion`. Reads the stored document text
/// for the URI, computes the prefix up to the cursor, and serves the
/// appropriate slice of the builtin table.
fn on_completion(
    state: &mut ServerState,
    params: CompletionParams,
) -> async_lsp::Result<Option<CompletionResponse>> {
    let uri = &params.text_document_position.text_document.uri;
    let pos = params.text_document_position.position;
    let prefix = state
        .docs
        .get(uri)
        .map(|text| prefix_up_to(text, pos))
        .unwrap_or_default();
    let items: Vec<CompletionItem> = completions_at(&prefix);
    Ok(Some(CompletionResponse::List(CompletionList {
        is_incomplete: false,
        items,
    })))
}

/// `didOpen`: store the document text + publish diagnostics.
fn on_did_open(
    state: &mut ServerState,
    params: DidOpenTextDocumentParams,
) -> ControlFlow<async_lsp::Result<()>> {
    let uri = params.text_document.uri.clone();
    let text = params.text_document.text.clone();
    state.docs.upsert(uri.clone(), text.clone());
    publish_diagnostics(&state.client, &uri, &text);
    ControlFlow::Continue(())
}

/// `didChange`: store the most recent full text + publish diagnostics.
fn on_did_change(
    state: &mut ServerState,
    params: DidChangeTextDocumentParams,
) -> ControlFlow<async_lsp::Result<()>> {
    let uri = params.text_document.uri.clone();
    let text = params
        .content_changes
        .into_iter()
        .last()
        .map(|c| c.text)
        .unwrap_or_default();
    state.docs.upsert(uri.clone(), text.clone());
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

/// Split the current line of `text` at `pos`, returning
/// `(before, after)`. `before` is text on the cursor line up to the
/// cursor; `after` is the rest of that line (not including `\n`).
/// Both exclude the character under the cursor.
pub fn split_at_position(text: &str, pos: Position) -> (String, String) {
    // Find the start of the cursor line.
    let mut line_start = 0;
    let mut cur_line: u32 = 0;
    for (i, c) in text.char_indices() {
        if cur_line == pos.line {
            line_start = i;
            break;
        }
        if c == '\n' {
            cur_line += 1;
        }
    }
    // Slice the cursor line (without `\n`).
    let line_end = text[line_start..]
        .find('\n')
        .map(|n| line_start + n)
        .unwrap_or(text.len());
    let cursor_line = &text[line_start..line_end];

    let char_count = pos.character.min(cursor_line.chars().count() as u32) as usize;
    let before: String = cursor_line.chars().take(char_count).collect();
    let after: String = cursor_line.chars().skip(char_count).collect();
    (before, after)
}

/// Build the prefix (text up to the cursor) given the full document
/// text and an LSP `Position` (0-indexed line + character). Exposed
/// for testing in `tests/completion.rs`.
pub fn prefix_up_to(text: &str, pos: Position) -> String {
    let mut cur_line: u32 = 0;
    for line in text.split_inclusive('\n') {
        if cur_line == pos.line {
            // `line` includes the trailing '\n'; drop it before slicing.
            let line_no_nl = line.strip_suffix('\n').unwrap_or(line);
            let chars: String = line_no_nl.chars().take(pos.character as usize).collect();
            return chars;
        }
        cur_line += 1;
    }
    // Cursor is past the end of the document — return the whole thing.
    text.to_string()
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