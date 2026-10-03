//! The LSP backend: document state, the standard methods, and the custom
//! `codeReading/*` methods. Every use-case call runs on a blocking thread,
//! because the ports are synchronous (tree-sitter, blocking HTTP).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use crt_app::{FunctionSelector, ReadOptions, Readers};
use crt_domain::{Function, Reading};
use crt_wire::protocol::{
    FILE_READINGS, FileReadingsParams, InitOptions, ReadParams, VisibleRangeParams,
};
use crt_wire::{FileAnalysisDto, FunctionReadingDto, ReadingDto};
use tokio::sync::Semaphore;
use tower_lsp_server::jsonrpc::{Error as RpcError, Result as RpcResult};
use tower_lsp_server::ls_types::notification::Notification;
use tower_lsp_server::ls_types::request::WorkDoneProgressCreate;
use tower_lsp_server::ls_types::{
    DidChangeTextDocumentParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams, Hover,
    HoverContents, HoverParams, HoverProviderCapability, InitializeParams, InitializeResult,
    InitializedParams, MarkupContent, MarkupKind, MessageType, NumberOrString, ServerCapabilities,
    ServerInfo, TextDocumentSyncCapability, TextDocumentSyncKind, Uri,
    WorkDoneProgressCreateParams,
};
use tower_lsp_server::{Client, LanguageServer};
use tower_lsp_server::{NotCancellable, OngoingProgress, Unbounded};

use crate::Services;
use crate::render;

/// The `codeReading/fileReadings` notification.
enum FileReadings {}

impl Notification for FileReadings {
    type Params = FileReadingsParams;
    const METHOD: &'static str = FILE_READINGS;
}

#[derive(Clone)]
struct Doc {
    uri: Uri,
    path: PathBuf,
    version: i32,
    text: Arc<Vec<u8>>,
}

struct Inner {
    client: Client,
    services: Services,
    docs: Mutex<HashMap<String, Doc>>,
    /// (document key, function hash) pairs being read right now.
    in_flight: Mutex<HashSet<(String, String)>>,
    options: OnceLock<InitOptions>,
    permits: OnceLock<Arc<Semaphore>>,
    /// Errors already shown, so auto-read does not repeat them.
    shown: Mutex<HashSet<String>>,
}

#[derive(Clone)]
pub struct Backend {
    inner: Arc<Inner>,
}

type Cached = (crt_app::FileAnalysis, Vec<Option<Reading>>);

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Backend {
    pub fn new(client: Client, services: Services) -> Self {
        Self {
            inner: Arc::new(Inner {
                client,
                services,
                docs: Mutex::new(HashMap::new()),
                in_flight: Mutex::new(HashSet::new()),
                options: OnceLock::new(),
                permits: OnceLock::new(),
                shown: Mutex::new(HashSet::new()),
            }),
        }
    }

    fn options(&self) -> InitOptions {
        self.inner.options.get().cloned().unwrap_or_default()
    }

    fn doc(&self, key: &str) -> Option<Doc> {
        lock(&self.inner.docs).get(key).cloned()
    }

    /// Structural facts plus cached readings for a document. Never calls
    /// the model.
    async fn cached(&self, doc: &Doc) -> Option<Cached> {
        let services = self.inner.services.clone();
        let (path, text) = (doc.path.clone(), Arc::clone(&doc.text));
        tokio::task::spawn_blocking(move || match &services.explainer {
            Some(explainer) => crt_app::cached_readings(
                services.structure.as_ref(),
                explainer.as_ref(),
                services.store.as_ref(),
                &path,
                &text,
            )
            .ok(),
            None => crt_app::analyze_file(services.structure.as_ref(), &path, &text)
                .ok()
                .map(|a| {
                    let n = a.functions.len();
                    (a, vec![None; n])
                }),
        })
        .await
        .ok()
        .flatten()
    }

    /// Sends the document's facts and readings, and its diagnostics.
    async fn publish(&self, key: &str) {
        let Some(doc) = self.doc(key) else { return };
        let Some((analysis, readings)) = self.cached(&doc).await else {
            // Not a language we know: clear anything shown before.
            self.inner
                .client
                .publish_diagnostics(doc.uri.clone(), vec![], Some(doc.version))
                .await;
            return;
        };
        let pending: Vec<String> = lock(&self.inner.in_flight)
            .iter()
            .filter(|(k, _)| k == key)
            .map(|(_, h)| h.clone())
            .collect();
        let diagnostics = readings
            .iter()
            .flatten()
            .flat_map(|r| render::diagnostics(&doc.uri, r))
            .collect();
        let params = FileReadingsParams {
            uri: key.to_string(),
            version: doc.version,
            analysis: FileAnalysisDto::from(&analysis),
            readings: readings
                .iter()
                .map(|r| r.as_ref().map(ReadingDto::from))
                .collect(),
            pending,
        };
        self.inner
            .client
            .send_notification::<FileReadings>(params)
            .await;
        self.inner
            .client
            .publish_diagnostics(doc.uri, diagnostics, Some(doc.version))
            .await;
    }

    /// Reads one function, asking the model if needed.
    async fn read_function(
        &self,
        doc: &Doc,
        selector: FunctionSelector,
        refresh: bool,
    ) -> Result<crt_app::FunctionReading, String> {
        let services = self.inner.services.clone();
        let Some(explainer) = services.explainer.clone() else {
            return Err(services
                .explainer_unavailable
                .unwrap_or_else(|| "no LLM is configured".to_string()));
        };
        let (path, text) = (doc.path.clone(), Arc::clone(&doc.text));
        tokio::task::spawn_blocking(move || {
            let readers = Readers {
                structure: services.structure.as_ref(),
                explainer: explainer.as_ref(),
                store: services.store.as_ref(),
            };
            let options = ReadOptions {
                refresh,
                ..ReadOptions::default()
            };
            crt_app::read_function(&readers, &path, &text, &selector, options)
                .map_err(|e| chain(&e))
        })
        .await
        .map_err(|e| e.to_string())?
    }

    async fn show_once(&self, message: String) {
        if lock(&self.inner.shown).insert(message.clone()) {
            self.inner
                .client
                .show_message(MessageType::WARNING, format!("crt: {message}"))
                .await;
        }
    }

    /// `codeReading/read`.
    pub async fn read(&self, params: ReadParams) -> RpcResult<FunctionReadingDto> {
        let doc = self
            .doc(&params.uri)
            .ok_or_else(|| invalid(format!("{} is not open", params.uri)))?;
        let line = params.line as usize + 1;
        let result = self
            .read_function(&doc, FunctionSelector::Line(line), params.refresh)
            .await
            .map_err(invalid)?;
        for w in &result.warnings {
            self.show_once(w.clone()).await;
        }
        self.publish(&params.uri).await;
        Ok(FunctionReadingDto::from(&result))
    }

    /// `codeReading/visibleRange`: read uncached functions the user can see.
    pub async fn visible_range(&self, params: VisibleRangeParams) {
        if !self.options().auto_read || self.inner.services.explainer.is_none() {
            return;
        }
        let Some(doc) = self.doc(&params.uri) else {
            return;
        };
        let Some((analysis, readings)) = self.cached(&doc).await else {
            return;
        };
        let (first, last) = (params.start_line as usize + 1, params.end_line as usize + 1);
        let wanted: Vec<Function> = analysis
            .functions
            .into_iter()
            .zip(readings)
            .filter(|(f, r)| r.is_none() && f.span.start_line <= last && first <= f.span.end_line)
            .map(|(f, _)| f)
            .collect();
        for function in wanted {
            let key = (params.uri.clone(), function.hash.to_string());
            if !lock(&self.inner.in_flight).insert(key.clone()) {
                continue;
            }
            let this = self.clone();
            tokio::spawn(async move { this.auto_read(key, function).await });
        }
    }

    async fn auto_read(&self, key: (String, String), function: Function) {
        let permits = Arc::clone(
            self.inner
                .permits
                .get_or_init(|| Arc::new(Semaphore::new(2))),
        );
        let Ok(_permit) = permits.acquire_owned().await else {
            return;
        };
        self.publish(&key.0).await;
        let progress = self.begin_progress(&key, &function.name).await;
        // Read the current text: if the function changed while waiting, its
        // hash no longer matches and the read below targets the new version.
        let outcome = match self.doc(&key.0) {
            Some(doc) => self
                .read_function(
                    &doc,
                    FunctionSelector::Line(function.span.start_line),
                    false,
                )
                .await
                .map(|r| r.warnings),
            None => Ok(vec![]),
        };
        lock(&self.inner.in_flight).remove(&key);
        if let Some(p) = progress {
            p.finish().await;
        }
        match outcome {
            Ok(warnings) => {
                for w in warnings {
                    self.show_once(w).await;
                }
            }
            Err(e) => self.show_once(e).await,
        }
        self.publish(&key.0).await;
    }

    /// Shows "reading <function>" in the editor's progress area, when the
    /// client supports server-initiated progress.
    async fn begin_progress(
        &self,
        key: &(String, String),
        name: &str,
    ) -> Option<OngoingProgress<Unbounded, NotCancellable>> {
        let token = NumberOrString::String(format!("crt/{}/{}", key.0, key.1));
        let created = self
            .inner
            .client
            .send_request::<WorkDoneProgressCreate>(WorkDoneProgressCreateParams {
                token: token.clone(),
            })
            .await;
        if created.is_err() {
            return None;
        }
        Some(
            self.inner
                .client
                .progress(token, "crt")
                .with_message(format!("reading {name}"))
                .begin()
                .await,
        )
    }
}

fn invalid(message: String) -> RpcError {
    RpcError {
        code: tower_lsp_server::jsonrpc::ErrorCode::InvalidRequest,
        message: message.into(),
        data: None,
    }
}

/// The error and its causes, one line.
fn chain(e: &dyn std::error::Error) -> String {
    let mut out = e.to_string();
    let mut cur = e.source();
    while let Some(c) = cur {
        out.push_str(": ");
        out.push_str(&c.to_string());
        cur = c.source();
    }
    out
}

fn path_of(uri: &Uri) -> PathBuf {
    uri.to_file_path()
        .map(|p| p.into_owned())
        .unwrap_or_else(|| PathBuf::from(uri.path().as_str()))
}

impl LanguageServer for Backend {
    async fn initialize(&self, params: InitializeParams) -> RpcResult<InitializeResult> {
        let options: InitOptions = params
            .initialization_options
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();
        let _ = self
            .inner
            .permits
            .set(Arc::new(Semaphore::new(options.max_parallel.max(1))));
        let _ = self.inner.options.set(options);
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                experimental: Some(
                    serde_json::json!({ "codeReading": { "version": crt_wire::WIRE_VERSION } }),
                ),
                ..ServerCapabilities::default()
            },
            server_info: Some(ServerInfo {
                name: "crt".into(),
                version: Some(env!("CARGO_PKG_VERSION").into()),
            }),
            ..InitializeResult::default()
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        if let Some(why) = self.inner.services.explainer_unavailable.clone() {
            self.inner
                .client
                .log_message(
                    MessageType::WARNING,
                    format!("crt: explanations are off: {why}"),
                )
                .await;
        }
    }

    async fn shutdown(&self) -> RpcResult<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let d = params.text_document;
        let key = d.uri.as_str().to_string();
        let doc = Doc {
            path: path_of(&d.uri),
            uri: d.uri,
            version: d.version,
            text: Arc::new(d.text.into_bytes()),
        };
        lock(&self.inner.docs).insert(key.clone(), doc);
        self.publish(&key).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let key = params.text_document.uri.as_str().to_string();
        let Some(change) = params.content_changes.into_iter().last() else {
            return;
        };
        {
            let mut docs = lock(&self.inner.docs);
            let Some(doc) = docs.get_mut(&key) else {
                return;
            };
            doc.version = params.text_document.version;
            doc.text = Arc::new(change.text.into_bytes());
        }
        self.publish(&key).await;
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri;
        lock(&self.inner.docs).remove(uri.as_str());
        self.inner
            .client
            .publish_diagnostics(uri, vec![], None)
            .await;
    }

    async fn hover(&self, params: HoverParams) -> RpcResult<Option<Hover>> {
        let pos = params.text_document_position_params;
        let Some(doc) = self.doc(pos.text_document.uri.as_str()) else {
            return Ok(None);
        };
        let Some((analysis, readings)) = self.cached(&doc).await else {
            return Ok(None);
        };
        let line = pos.position.line as usize + 1;
        let found = analysis
            .functions
            .iter()
            .zip(readings.iter())
            .filter(|(f, _)| f.span.start_line <= line && line <= f.span.end_line)
            .min_by_key(|(f, _)| f.span.len());
        let Some((function, reading)) = found else {
            return Ok(None);
        };
        Ok(
            render::hover(function, reading.as_ref(), line).map(|value| Hover {
                contents: HoverContents::Markup(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value,
                }),
                range: None,
            }),
        )
    }
}
