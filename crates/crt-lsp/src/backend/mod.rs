//! The LSP backend: document state, the standard methods, and the custom
//! `codeReading/*` methods. Every use-case call runs on a blocking thread,
//! because the ports are synchronous (tree-sitter, blocking HTTP).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use crt_app::{FunctionSelector, ReadOptions, Readers};
use crt_domain::{Function, Note, Reading};
use crt_wire::protocol::{
    ConfigPathResult, FILE_READINGS, FileReadingsParams, InitOptions, PartialNotesDto, ReadOn,
    ReadParams, StaleReadingDto,
};
use crt_wire::{FileAnalysisDto, FunctionReadingDto, NoteDto, ReadingDto};
use tokio::sync::Semaphore;
use tower_lsp_server::jsonrpc::{Error as RpcError, ErrorCode, Result as RpcResult};
use tower_lsp_server::ls_types::notification::Notification;
use tower_lsp_server::ls_types::{
    DidChangeTextDocumentParams, DidCloseTextDocumentParams, DidOpenTextDocumentParams,
    DidSaveTextDocumentParams, Hover, HoverContents, HoverParams, HoverProviderCapability,
    InitializeParams, InitializeResult, InitializedParams, MarkupContent, MarkupKind, MessageType,
    ServerCapabilities, ServerInfo, TextDocumentSyncCapability, TextDocumentSyncKind,
    TextDocumentSyncOptions, TextDocumentSyncSaveOptions, Uri,
};
use tower_lsp_server::{Client, LanguageServer};

mod auto_read;

use crate::Services;
use crate::render;
use crate::stale;

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
    /// Notes received so far for readings being written, by (document
    /// key, function hash), with the function's first line when the read
    /// started so they can follow the function if lines move above it.
    partial: Mutex<HashMap<ReadKey, Partial>>,
    /// Numbers reads, so a read only ever removes its own partial notes.
    next_read: AtomicU64,
    /// Held for a whole publish: publishes compute and send one at a time,
    /// so one that started earlier (say, from a progress update, before the
    /// reading was stored) can never be sent after one that started later.
    publishing: tokio::sync::Mutex<()>,
    /// Auto-reads that failed, and when. They are not retried on every
    /// scroll; an explicit read or the cooldown clears them.
    failed: Mutex<HashMap<(String, String), Instant>>,
    /// The explainer's authors when auto-read last looked. A change means
    /// the configuration changed, and earlier failures may not hold.
    authors_seen: Mutex<Vec<crt_domain::Author>>,
    /// One lock per function being read: an auto-read, an explicit read
    /// and a scenarios request for the same function run one after
    /// another, so the later ones find the notes in the cache instead of
    /// asking the model again.
    function_locks: Mutex<HashMap<ReadKey, Arc<tokio::sync::Mutex<()>>>>,
    /// Per document: the last reading of each function, shown as old once
    /// the function is edited (see `stale`).
    memory: Mutex<HashMap<String, stale::Memory>>,
    /// Per document: the hashes of its functions as last saved (or
    /// opened). With `readOn: save`, auto-read only reads these, so a
    /// function being edited is read when it is saved, not while typed.
    saved: Mutex<HashMap<String, HashSet<String>>>,
    /// Per document: the lines last reported visible, 0-based inclusive.
    views: Mutex<HashMap<String, (u32, u32)>>,
}

#[derive(Clone)]
pub struct Backend {
    inner: Arc<Inner>,
}

type Cached = (crt_app::FileAnalysis, Vec<Option<Reading>>);

/// How long a failed auto-read is left alone before scrolling past the
/// function tries again.
const FAILURE_COOLDOWN: Duration = Duration::from_secs(300);

/// With `readOn: idle`, how long after the last change auto-read runs.
const IDLE_DELAY: Duration = Duration::from_secs(2);

/// How often notes still arriving are sent to the editor, at most.
const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);

/// Which reading a call is about: (document key, function hash).
type ReadKey = (String, String);

/// Notes received so far by one read.
struct Partial {
    /// Which read wrote them.
    read: u64,
    /// The function's first line when the read started.
    started_at: usize,
    notes: Vec<Note>,
}

/// A turn to read one function; see `Inner::function_locks`. Dropping it
/// also forgets the lock when nobody else is waiting for it.
struct FunctionTurn {
    guard: Option<tokio::sync::OwnedMutexGuard<()>>,
    inner: Arc<Inner>,
    key: ReadKey,
}

impl Drop for FunctionTurn {
    fn drop(&mut self) {
        drop(self.guard.take());
        let mut locks = lock(&self.inner.function_locks);
        // Only this map holds it now: no one is reading or waiting.
        if locks
            .get(&self.key)
            .is_some_and(|l| Arc::strong_count(l) == 1)
        {
            locks.remove(&self.key);
        }
    }
}

/// Removes an in-flight key when the read ends, however it ends.
struct InFlight {
    inner: Arc<Inner>,
    key: (String, String),
}

impl Drop for InFlight {
    fn drop(&mut self) {
        lock(&self.inner.in_flight).remove(&self.key);
    }
}

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
                partial: Mutex::new(HashMap::new()),
                next_read: AtomicU64::new(0),
                publishing: tokio::sync::Mutex::new(()),
                failed: Mutex::new(HashMap::new()),
                authors_seen: Mutex::new(Vec::new()),
                function_locks: Mutex::new(HashMap::new()),
                memory: Mutex::new(HashMap::new()),
                saved: Mutex::new(HashMap::new()),
                views: Mutex::new(HashMap::new()),
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
        let _turn = self.inner.publishing.lock().await;
        let Some(doc) = self.doc(key) else { return };
        let cached = self.cached(&doc).await;
        // A newer version arrived while this one was being analysed; its own
        // publish follows, and sending this one now could land after it.
        if self.doc(key).map(|d| d.version) != Some(doc.version) {
            return;
        }
        let Some((analysis, readings)) = cached else {
            // Unknown language or unreadable: clear anything shown before.
            let params = FileReadingsParams {
                uri: key.to_string(),
                version: doc.version,
                analysis: FileAnalysisDto {
                    wire_version: crt_wire::WIRE_VERSION,
                    language: String::new(),
                    has_syntax_error: false,
                    functions: vec![],
                },
                readings: vec![],
                pending: vec![],
                partial: vec![],
                stale: vec![],
            };
            self.inner
                .client
                .send_notification::<FileReadings>(params)
                .await;
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
        let partial = self.partial_notes(key, &analysis.functions);
        let old = lock(&self.inner.memory)
            .entry(key.to_string())
            .or_default()
            .update(&analysis.functions, &readings);
        let stale = analysis
            .functions
            .iter()
            .zip(old)
            .filter_map(|(f, r)| {
                Some(StaleReadingDto {
                    function_hash: f.hash.to_string(),
                    reading: ReadingDto::from(&r?),
                })
            })
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
            partial,
            stale,
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

    /// The notes still arriving for functions of this document, placed at
    /// each function's current lines.
    fn partial_notes(&self, key: &str, functions: &[Function]) -> Vec<PartialNotesDto> {
        let partial = lock(&self.inner.partial);
        functions
            .iter()
            .filter_map(|f| {
                let hash = f.hash.to_string();
                let p = partial.get(&(key.to_string(), hash.clone()))?;
                // An empty list (a read starting over) would hide a cached
                // reading being refreshed; send nothing instead.
                if p.notes.is_empty() {
                    return None;
                }
                let shift = f.span.start_line as isize - p.started_at as isize;
                Some(PartialNotesDto {
                    function_hash: hash,
                    notes: p
                        .notes
                        .iter()
                        .map(|n| {
                            let mut dto = NoteDto::from(n);
                            dto.line = n.line.saturating_add_signed(shift);
                            dto
                        })
                        .collect(),
                })
            })
            .collect()
    }

    /// Reads one function's notes, asking the model if needed. While the
    /// model writes, the notes so far are sent to the editor (at most every
    /// `PROGRESS_INTERVAL`).
    async fn read_function(
        &self,
        doc: &Doc,
        function: &Function,
        refresh: bool,
    ) -> Result<crt_app::FunctionReading, String> {
        let explainer = self.explainer()?;
        let _turn = self.function_turn(doc, function).await;
        let services = self.inner.services.clone();
        let (path, text) = (doc.path.clone(), Arc::clone(&doc.text));
        let read_key: ReadKey = (doc.uri.as_str().to_string(), function.hash.to_string());
        let start_line = function.span.start_line;
        let this = self.clone();
        let runtime = tokio::runtime::Handle::current();
        let progress_key = read_key.clone();
        let read = self.inner.next_read.fetch_add(1, Ordering::Relaxed);
        let result = tokio::task::spawn_blocking(move || {
            let readers = Readers {
                structure: services.structure.as_ref(),
                explainer: explainer.as_ref(),
                store: services.store.as_ref(),
            };
            let options = ReadOptions {
                refresh,
                ..ReadOptions::default()
            };
            let mut last_sent: Option<Instant> = None;
            let mut on_progress = |notes: &[Note]| {
                lock(&this.inner.partial).insert(
                    progress_key.clone(),
                    Partial {
                        read,
                        started_at: start_line,
                        notes: notes.to_vec(),
                    },
                );
                if last_sent.is_some_and(|t| t.elapsed() < PROGRESS_INTERVAL) {
                    return;
                }
                last_sent = Some(Instant::now());
                let (this, uri) = (this.clone(), progress_key.0.clone());
                runtime.spawn(async move { this.publish(&uri).await });
            };
            crt_app::read_function(
                &readers,
                &path,
                &text,
                &FunctionSelector::Line(start_line),
                options,
                &mut on_progress,
            )
            .map_err(|e| chain(&e))
        })
        .await
        .map_err(|e| e.to_string());
        {
            // Another read of the same function may have started meanwhile;
            // its notes are not ours to remove.
            let mut partial = lock(&self.inner.partial);
            if partial.get(&read_key).is_some_and(|p| p.read == read) {
                partial.remove(&read_key);
            }
        }
        result?
    }

    /// Waits until no other read of `function` in `doc` is running, and
    /// holds that turn until the guard is dropped.
    async fn function_turn(&self, doc: &Doc, function: &Function) -> FunctionTurn {
        let key: ReadKey = (doc.uri.as_str().to_string(), function.hash.to_string());
        let lock = Arc::clone(
            lock(&self.inner.function_locks)
                .entry(key.clone())
                .or_default(),
        );
        FunctionTurn {
            guard: Some(lock.lock_owned().await),
            inner: Arc::clone(&self.inner),
            key,
        }
    }

    /// Reads one function's scenarios, asking the model if needed.
    async fn read_scenarios(
        &self,
        doc: &Doc,
        function: &Function,
        refresh: bool,
    ) -> Result<crt_app::FunctionReading, String> {
        let explainer = self.explainer()?;
        let _turn = self.function_turn(doc, function).await;
        let services = self.inner.services.clone();
        let (path, text) = (doc.path.clone(), Arc::clone(&doc.text));
        let start_line = function.span.start_line;
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
            crt_app::read_scenarios(
                &readers,
                &path,
                &text,
                &FunctionSelector::Line(start_line),
                options,
            )
            .map_err(|e| chain(&e))
        })
        .await
        .map_err(|e| e.to_string())?
    }

    fn explainer(&self) -> Result<Arc<dyn crt_app::Explainer + Send + Sync>, String> {
        let services = &self.inner.services;
        services.explainer.clone().ok_or_else(|| {
            services
                .explainer_unavailable
                .clone()
                .unwrap_or_else(|| "no LLM is configured".to_string())
        })
    }

    /// The innermost function of an open document containing the 0-based
    /// `line`.
    async fn function_at(&self, uri: &str, line: u32) -> RpcResult<(Doc, Function)> {
        let doc = self
            .doc(uri)
            .ok_or_else(|| rpc_error(ErrorCode::InvalidParams, format!("{uri} is not open")))?;
        let line = line as usize + 1;
        let function = self
            .cached(&doc)
            .await
            .and_then(|(analysis, _)| {
                analysis
                    .functions
                    .into_iter()
                    .filter(|f| f.span.start_line <= line && line <= f.span.end_line)
                    .min_by_key(|f| f.span.len())
            })
            .ok_or_else(|| {
                rpc_error(
                    ErrorCode::InvalidParams,
                    format!("no function at line {line}"),
                )
            })?;
        Ok((doc, function))
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
        let (doc, function) = self.function_at(&params.uri, params.line).await?;
        let key = (params.uri.clone(), function.hash.to_string());
        let progress = self.begin_progress(&key, &function.name).await;
        let result = self.read_function(&doc, &function, params.refresh).await;
        if let Some(p) = progress {
            p.finish().await;
        }
        let result = result.map_err(|e| rpc_error(ErrorCode::InternalError, e))?;
        // Asking explicitly is a fresh start for auto-read in this file.
        lock(&self.inner.failed).retain(|(uri, _), _| uri != &params.uri);
        for w in &result.warnings {
            self.show_once(w.clone()).await;
        }
        self.publish(&params.uri).await;
        Ok(FunctionReadingDto::from(&result))
    }

    /// `codeReading/scenarios`.
    pub async fn scenarios(&self, params: ReadParams) -> RpcResult<FunctionReadingDto> {
        let (doc, function) = self.function_at(&params.uri, params.line).await?;
        let key = (params.uri.clone(), format!("{}/scenarios", function.hash));
        let progress = self
            .begin_progress(&key, &format!("scenarios of {}", function.name))
            .await;
        let result = self.read_scenarios(&doc, &function, params.refresh).await;
        if let Some(p) = progress {
            p.finish().await;
        }
        let result = result.map_err(|e| rpc_error(ErrorCode::InternalError, e))?;
        for w in &result.warnings {
            self.show_once(w.clone()).await;
        }
        self.publish(&params.uri).await;
        Ok(FunctionReadingDto::from(&result))
    }

    /// `codeReading/configPath`.
    pub async fn config_path(&self) -> RpcResult<ConfigPathResult> {
        let Some(file) = self.inner.services.config_file.clone() else {
            return Err(rpc_error(
                ErrorCode::InternalError,
                "this server has no configuration file".into(),
            ));
        };
        let path = file.path.display().to_string();
        let created = tokio::task::spawn_blocking(move || (file.ensure_exists)())
            .await
            .map_err(|e| rpc_error(ErrorCode::InternalError, e.to_string()))?
            .map_err(|e| rpc_error(ErrorCode::InternalError, e))?;
        Ok(ConfigPathResult { path, created })
    }
}

fn rpc_error(code: ErrorCode, message: String) -> RpcError {
    RpcError {
        code,
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
            .set(Arc::new(Semaphore::new(options.max_parallel.clamp(1, 16))));
        let _ = self.inner.options.set(options);
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Options(
                    TextDocumentSyncOptions {
                        open_close: Some(true),
                        change: Some(TextDocumentSyncKind::FULL),
                        save: Some(TextDocumentSyncSaveOptions::Supported(true)),
                        ..TextDocumentSyncOptions::default()
                    },
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
        self.mark_saved(&key).await;
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
        if self.options().read_on == ReadOn::Idle {
            // Read again once the typing stops: only if no newer change
            // arrived in the meantime.
            let (this, version) = (self.clone(), params.text_document.version);
            tokio::spawn(async move {
                tokio::time::sleep(IDLE_DELAY).await;
                if this.doc(&key).is_some_and(|d| d.version == version) {
                    this.read_in_view(&key).await;
                }
            });
        }
    }

    async fn did_save(&self, params: DidSaveTextDocumentParams) {
        let key = params.text_document.uri.as_str().to_string();
        self.mark_saved(&key).await;
        if self.options().auto_read && self.options().read_on == ReadOn::Save {
            self.read_in_view(&key).await;
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri;
        lock(&self.inner.docs).remove(uri.as_str());
        lock(&self.inner.failed).retain(|(u, _), _| u != uri.as_str());
        lock(&self.inner.memory).remove(uri.as_str());
        lock(&self.inner.saved).remove(uri.as_str());
        lock(&self.inner.views).remove(uri.as_str());
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
