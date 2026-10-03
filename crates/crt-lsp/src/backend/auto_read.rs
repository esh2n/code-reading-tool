//! Auto-read: reading the functions the user can see without being asked,
//! and reading an edited function again when it is saved (or when typing
//! stops, with `readOn: idle`).

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;

use crt_domain::Function;
use crt_wire::protocol::{ReadOn, VisibleRangeParams};
use tower_lsp_server::ls_types::request::WorkDoneProgressCreate;
use tower_lsp_server::ls_types::{NumberOrString, WorkDoneProgressCreateParams};
use tower_lsp_server::{NotCancellable, OngoingProgress, Unbounded};

use super::{Backend, FAILURE_COOLDOWN, InFlight, lock};

impl Backend {
    /// `codeReading/visibleRange`: read uncached functions the user can see.
    pub async fn visible_range(&self, params: VisibleRangeParams) {
        lock(&self.inner.views).insert(params.uri.clone(), (params.start_line, params.end_line));
        self.read_in_view(&params.uri).await;
    }

    /// Auto-read: reads the uncached functions in the document's last
    /// reported view. With `readOn: save`, only functions as they were last
    /// saved.
    pub(super) async fn read_in_view(&self, uri: &str) {
        let Some((start_line, end_line)) = lock(&self.inner.views).get(uri).copied() else {
            return;
        };
        let Some(explainer) = self.inner.services.explainer.clone() else {
            return;
        };
        if !self.options().auto_read {
            return;
        }
        let authors = tokio::task::spawn_blocking(move || explainer.authors())
            .await
            .unwrap_or_default();
        {
            let mut seen = lock(&self.inner.authors_seen);
            if *seen != authors {
                *seen = authors;
                lock(&self.inner.failed).clear();
                lock(&self.inner.shown).clear();
            }
        }
        let Some(doc) = self.doc(uri) else {
            return;
        };
        let Some((analysis, readings)) = self.cached(&doc).await else {
            return;
        };
        let (first, last) = (start_line as usize + 1, end_line as usize + 1);
        let saved = match self.options().read_on {
            ReadOn::Save => lock(&self.inner.saved).get(uri).cloned(),
            ReadOn::Idle => None,
        };
        let wanted: Vec<Function> = analysis
            .functions
            .into_iter()
            .zip(readings)
            .filter(|(f, r)| r.is_none() && f.span.start_line <= last && first <= f.span.end_line)
            .filter(|(f, _)| {
                saved
                    .as_ref()
                    .is_none_or(|s| s.contains(&f.hash.to_string()))
            })
            .map(|(f, _)| f)
            .collect();
        for function in wanted {
            let key = (uri.to_string(), function.hash.to_string());
            let recently_failed = lock(&self.inner.failed)
                .get(&key)
                .is_some_and(|at| at.elapsed() < FAILURE_COOLDOWN);
            if recently_failed || !lock(&self.inner.in_flight).insert(key.clone()) {
                continue;
            }
            let guard = InFlight {
                inner: Arc::clone(&self.inner),
                key,
            };
            let this = self.clone();
            tokio::spawn(async move { this.auto_read(guard, function.name).await });
        }
    }

    /// Records the document's functions as saved: with `readOn: save`,
    /// they may now be auto-read.
    pub(super) async fn mark_saved(&self, uri: &str) {
        let Some(doc) = self.doc(uri) else { return };
        let services = self.inner.services.clone();
        let (path, text) = (doc.path.clone(), Arc::clone(&doc.text));
        let hashes = tokio::task::spawn_blocking(move || {
            crt_app::analyze_file(services.structure.as_ref(), &path, &text)
                .map(|a| {
                    a.functions
                        .iter()
                        .map(|f| f.hash.to_string())
                        .collect::<HashSet<_>>()
                })
                .unwrap_or_default()
        })
        .await
        .unwrap_or_default();
        lock(&self.inner.saved).insert(uri.to_string(), hashes);
    }

    pub(super) async fn auto_read(&self, guard: InFlight, name: String) {
        let Some(permits) = self.inner.permits.get().cloned() else {
            return;
        };
        let Ok(_permit) = permits.acquire_owned().await else {
            return;
        };
        let key = guard.key.clone();
        // The document may have changed while waiting: read the function
        // with this hash if it is still there and still unread.
        let Some(doc) = self.doc(&key.0) else { return };
        let Some((analysis, readings)) = self.cached(&doc).await else {
            return;
        };
        let target = analysis
            .functions
            .iter()
            .zip(&readings)
            .find(|(f, _)| f.hash.to_string() == key.1);
        let Some((function, None)) = target else {
            return;
        };
        let function = function.clone();
        self.publish(&key.0).await;
        let progress = self.begin_progress(&key, &name).await;
        let outcome = self.read_function(&doc, &function, false).await;
        drop(guard);
        if let Some(p) = progress {
            p.finish().await;
        }
        match outcome {
            Ok(r) => {
                for w in r.warnings {
                    self.show_once(w).await;
                }
            }
            Err(e) => {
                lock(&self.inner.failed).insert(key.clone(), Instant::now());
                self.show_once(e).await;
            }
        }
        self.publish(&key.0).await;
    }

    /// Shows "reading <function>" in the editor's progress area, when the
    /// client supports server-initiated progress.
    pub(super) async fn begin_progress(
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
