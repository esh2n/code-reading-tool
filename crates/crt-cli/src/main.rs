//! The `crt` command. Composition root: picks the adapters, calls the use
//! cases, prints wire JSON.

mod config;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use crt_app::{FunctionSelector, ReadOptions, Readers};
use crt_llm::OpenAiCompatible;
use crt_store::FileStore;
use crt_treesitter::TreeSitterSource;
use crt_wire::{FileAnalysisDto, FunctionReadingDto, ReadingDto};
use serde::Serialize;

#[derive(Parser)]
#[command(name = "crt", version, about = "Read code by how it behaves")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Args)]
struct Locations {
    /// Configuration file (default: the platform config directory).
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    /// Where readings are cached (default: the platform cache directory).
    #[arg(long, global = true)]
    cache_dir: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Command {
    /// Print the structural facts of every function in a file as JSON.
    Analyze {
        /// Source file; the language is chosen from its extension.
        file: PathBuf,
        /// Only the function with this name.
        #[arg(long)]
        func: Option<String>,
    },
    /// Explain one function: per-line notes and behaviour scenarios.
    Read {
        file: PathBuf,
        /// The function with this name.
        #[arg(long, conflicts_with = "line", required_unless_present = "line")]
        func: Option<String>,
        /// The innermost function containing this 1-based line.
        #[arg(long)]
        line: Option<usize>,
        /// Ignore the cache and ask the model again.
        #[arg(long)]
        refresh: bool,
        #[command(flatten)]
        at: Locations,
    },
    /// Print every function of a file with its cached reading, if any.
    /// Never calls the model.
    Cached {
        file: PathBuf,
        #[command(flatten)]
        at: Locations,
    },
    /// Write one HTML page with the file's readings. Uses the cache; with
    /// --read-missing, explains uncached functions first.
    Render {
        file: PathBuf,
        /// Output file (default: stdout).
        #[arg(long)]
        out: Option<PathBuf>,
        /// Explain functions that have no cached reading before rendering.
        #[arg(long)]
        read_missing: bool,
        #[command(flatten)]
        at: Locations,
    },
    /// Serve the editor integration over LSP on stdin/stdout.
    Lsp {
        #[command(flatten)]
        at: Locations,
    },
    /// List the bundled languages and their file extensions.
    Languages,
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Analyze { file, func } => analyze(&file, func.as_deref()),
        Command::Read {
            file,
            func,
            line,
            refresh,
            at,
        } => {
            let selector = match (func, line) {
                (Some(name), _) => FunctionSelector::Name(name),
                (None, Some(line)) => FunctionSelector::Line(line),
                (None, None) => unreachable!("clap requires --func or --line"),
            };
            read(&file, &selector, refresh, &at)
        }
        Command::Cached { file, at } => cached(&file, &at),
        Command::Lsp { at } => lsp(&at),
        Command::Render {
            file,
            out,
            read_missing,
            at,
        } => render(&file, out.as_deref(), read_missing, &at),
        Command::Languages => {
            for l in TreeSitterSource::languages() {
                println!("{}\t{}", l.id, l.extensions.join(","));
            }
            Ok(())
        }
    }
}

fn read_source(file: &Path) -> Result<Vec<u8>> {
    std::fs::read(file).with_context(|| format!("reading {}", file.display()))
}

fn print<T: Serialize>(value: &T) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn analyze(file: &Path, func: Option<&str>) -> Result<()> {
    let source = read_source(file)?;
    let mut analysis = crt_app::analyze_file(&TreeSitterSource, file, &source)
        .with_context(|| format!("analysing {}", file.display()))?;
    if analysis.has_syntax_error {
        eprintln!(
            "warning: {} has syntax errors; facts may be incomplete",
            file.display()
        );
    }
    if let Some(name) = func {
        analysis.functions.retain(|f| f.name == name);
        anyhow::ensure!(
            !analysis.functions.is_empty(),
            "no function named {name} in {}",
            file.display()
        );
    }
    print(&FileAnalysisDto::from(&analysis))
}

fn explainer(at: &Locations) -> Result<OpenAiCompatible> {
    let path = config::config_path(at.config.as_deref())?;
    let llm = config::load_llm(&path)?;
    OpenAiCompatible::new(llm).context("setting up the LLM client")
}

fn read(file: &Path, selector: &FunctionSelector, refresh: bool, at: &Locations) -> Result<()> {
    let source = read_source(file)?;
    let explainer = explainer(at)?;
    let store = FileStore::new(config::cache_dir(at.cache_dir.as_deref())?);
    let readers = Readers {
        structure: &TreeSitterSource,
        explainer: &explainer,
        store: &store,
    };
    let options = ReadOptions {
        refresh,
        ..ReadOptions::default()
    };
    let result = crt_app::read_function(&readers, file, &source, selector, options)
        .with_context(|| format!("reading {}", file.display()))?;
    for w in &result.warnings {
        eprintln!("warning: {w}");
    }
    print(&FunctionReadingDto::from(&result))
}

#[derive(Serialize)]
struct CachedDto {
    analysis: FileAnalysisDto,
    /// One entry per function, in the same order; `null` when not cached.
    readings: Vec<Option<ReadingDto>>,
}

fn cached(file: &Path, at: &Locations) -> Result<()> {
    let source = read_source(file)?;
    let explainer = explainer(at)?;
    let store = FileStore::new(config::cache_dir(at.cache_dir.as_deref())?);
    let (analysis, readings) =
        crt_app::cached_readings(&TreeSitterSource, &explainer, &store, file, &source)
            .with_context(|| format!("analysing {}", file.display()))?;
    print(&CachedDto {
        analysis: FileAnalysisDto::from(&analysis),
        readings: readings
            .iter()
            .map(|r| r.as_ref().map(ReadingDto::from))
            .collect(),
    })
}

fn lsp(at: &Locations) -> Result<()> {
    use std::sync::Arc;

    let store = Arc::new(FileStore::new(config::cache_dir(at.cache_dir.as_deref())?));
    // A missing or broken configuration must not stop the editor from
    // showing structural facts; it turns explanations off and says why.
    let (explainer, explainer_unavailable) = match explainer(at) {
        Ok(e) => (
            Some(Arc::new(e) as Arc<dyn crt_app::Explainer + Send + Sync>),
            None,
        ),
        Err(e) => (None, Some(format!("{e:#}"))),
    };
    let services = crt_lsp::Services {
        structure: Arc::new(TreeSitterSource),
        explainer,
        explainer_unavailable,
        store,
    };
    // Hold one handle outside the runtime so the blocking HTTP client is
    // dropped after the runtime is gone; dropping it inside the runtime
    // panics.
    let keep = services.clone();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("starting the async runtime")?;
    runtime.block_on(crt_lsp::serve_stdio(services));
    drop(runtime);
    drop(keep);
    Ok(())
}

fn render(file: &Path, out: Option<&Path>, read_missing: bool, at: &Locations) -> Result<()> {
    let source = read_source(file)?;
    let explainer = explainer(at)?;
    let store = FileStore::new(config::cache_dir(at.cache_dir.as_deref())?);
    let (analysis, mut readings) =
        crt_app::cached_readings(&TreeSitterSource, &explainer, &store, file, &source)
            .with_context(|| format!("analysing {}", file.display()))?;
    if read_missing {
        let readers = Readers {
            structure: &TreeSitterSource,
            explainer: &explainer,
            store: &store,
        };
        for (function, reading) in analysis.functions.iter().zip(readings.iter_mut()) {
            if reading.is_some() {
                continue;
            }
            let selector = FunctionSelector::Line(function.span.start_line);
            match crt_app::read_function(&readers, file, &source, &selector, ReadOptions::default())
            {
                Ok(r) => {
                    for w in &r.warnings {
                        eprintln!("warning: {w}");
                    }
                    *reading = Some(r.reading);
                }
                Err(e) => eprintln!("warning: {}: {:#}", function.name, anyhow::Error::from(e)),
            }
        }
    }
    let dto = FileAnalysisDto::from(&analysis);
    let reading_dtos: Vec<Option<ReadingDto>> = readings
        .iter()
        .map(|r| r.as_ref().map(ReadingDto::from))
        .collect();
    let text = String::from_utf8_lossy(&source);
    let title = file.display().to_string();
    let html = crt_html::render(&crt_html::Page {
        title: &title,
        source: &text,
        analysis: &dto,
        readings: &reading_dtos,
    })
    .context("rendering HTML")?;
    match out {
        Some(path) => {
            std::fs::write(path, html).with_context(|| format!("writing {}", path.display()))
        }
        None => {
            print!("{html}");
            Ok(())
        }
    }
}
