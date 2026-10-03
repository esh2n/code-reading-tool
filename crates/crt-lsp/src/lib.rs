//! The use cases served over LSP. A driving adapter: it calls `crt-app`,
//! translates results into LSP and the custom methods in
//! `crt_wire::protocol`, and never decides anything about readings itself.

mod backend;
mod render;

use std::sync::Arc;

use crt_app::{Explainer, ReadingStore, StructureSource};
use tokio::io::{AsyncRead, AsyncWrite};
use tower_lsp_server::{LspService, Server};

pub use backend::Backend;

/// The ports the server works through, chosen by the composition root.
#[derive(Clone)]
pub struct Services {
    pub structure: Arc<dyn StructureSource + Send + Sync>,
    /// `None` when no model is configured; the server then serves
    /// structural facts only and explains why reads fail.
    pub explainer: Option<Arc<dyn Explainer + Send + Sync>>,
    /// Why `explainer` is `None`, shown to the user on a read.
    pub explainer_unavailable: Option<String>,
    pub store: Arc<dyn ReadingStore + Send + Sync>,
}

/// Serves LSP on the given streams until the client exits.
pub async fn serve<I, O>(services: Services, input: I, output: O)
where
    I: AsyncRead + Unpin,
    O: AsyncWrite,
{
    let (service, socket) = LspService::build(|client| Backend::new(client, services))
        .custom_method(crt_wire::protocol::READ, Backend::read)
        .custom_method(crt_wire::protocol::VISIBLE_RANGE, Backend::visible_range)
        .finish();
    Server::new(input, output, socket).serve(service).await;
}

/// Serves LSP on stdin/stdout.
pub async fn serve_stdio(services: Services) {
    serve(services, tokio::io::stdin(), tokio::io::stdout()).await;
}
