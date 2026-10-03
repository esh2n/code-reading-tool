//! The `crt` command. Composition root: picks the adapters, calls the use
//! cases, prints wire JSON.

mod config;
mod reloading;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use crt_app::{FunctionSelector, ReadOptions, Readers};
use crt_domain::Note;
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

/// Which function to read, and whether to skip the cache.
#[derive(clap::Args)]
struct Target {
    file: PathBuf,
    /// The function with this name.
    #[arg(long, conflicts_with = "line", required_unless_present = "line")]
    func: Option<String>,
    /// The innermost function containing this 1-based line.
    #[arg(long)]
    line: Option<usize>,
    /// Ignore the cache and ask the model again (for scenarios: rewrite
    /// the scenarios only).
    #[arg(long)]
    refresh: bool,
}

impl Target {
    fn selector(&self) -> FunctionSelector {
        match (&self.func, self.line) {
            (Some(name), _) => FunctionSelector::Name(name.clone()),
            (None, Some(line)) => FunctionSelector::Line(line),
            (None, None) => unreachable!("clap requires --func or --line"),
        }
    }
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
    /// Explain one function's lines: per-line notes.
    Read {
        #[command(flatten)]
        target: Target,
        /// Print each note to stderr as soon as it arrives.
        #[arg(long)]
        progress: bool,
        #[command(flatten)]
        at: Locations,
    },
    /// Walk one function through normal, boundary and concurrent inputs.
    /// Reads the notes first if they are not cached.
    Scenarios {
        #[command(flatten)]
        target: Target,
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
    /// --read-missing, writes missing notes and scenarios first.
    Render {
        file: PathBuf,
        /// Output file (default: stdout).
        #[arg(long)]
        out: Option<PathBuf>,
        /// Write notes and scenarios for functions missing them before
        /// rendering.
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
    /// Print the configuration file's path, writing a commented example
    /// there first when it does not exist.
    Config {
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
            target,
            progress,
            at,
        } => read(&target, Part::Notes { progress }, &at),
        Command::Scenarios { target, at } => read(&target, Part::Scenarios, &at),
        Command::Cached { file, at } => cached(&file, &at),
        Command::Lsp { at } => lsp(&at),
        Command::Render {
            file,
            out,
            read_missing,
            at,
        } => render(&file, out.as_deref(), read_missing, &at),
        Command::Config { at } => {
            let path = config::config_path(at.config.as_deref())?;
            if config::write_example_if_missing(&path)? {
                eprintln!("wrote an example configuration; edit it before reading");
            }
            println!("{}", path.display());
            Ok(())
        }
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

/// What `read` asks for.
enum Part {
    Notes { progress: bool },
    Scenarios,
}

fn read(target: &Target, part: Part, at: &Locations) -> Result<()> {
    let file = target.file.as_path();
    let source = read_source(file)?;
    let explainer = explainer(at)?;
    let store = FileStore::new(config::cache_dir(at.cache_dir.as_deref())?);
    let readers = Readers {
        structure: &TreeSitterSource,
        explainer: &explainer,
        store: &store,
    };
    let options = ReadOptions {
        refresh: target.refresh,
        ..ReadOptions::default()
    };
    let selector = target.selector();
    let result = match part {
        Part::Notes { progress } => {
            let started = std::time::Instant::now();
            let mut shown = 0;
            let mut on_progress = |notes: &[Note]| {
                if !progress {
                    return;
                }
                if notes.len() < shown {
                    eprintln!("({:.1}s) starting over", started.elapsed().as_secs_f32());
                }
                for n in notes.iter().skip(shown) {
                    eprintln!(
                        "({:.1}s) line {}: {}",
                        started.elapsed().as_secs_f32(),
                        n.line,
                        n.text
                    );
                }
                shown = notes.len();
            };
            crt_app::read_function(
                &readers,
                file,
                &source,
                &selector,
                options,
                &mut on_progress,
            )
        }
        Part::Scenarios => crt_app::read_scenarios(&readers, file, &source, &selector, options),
    }
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
    // The configuration is read on each model call and again whenever the
    // file changes. A missing or broken file never stops the editor from
    // showing structural facts; reads say what is wrong with it.
    let path = config::config_path(at.config.as_deref())?;
    let explainer = reloading::Reloading::new(path.clone());
    if let Err(why) = explainer.current() {
        eprintln!("crt: explanations are unavailable until the configuration is fixed: {why}");
    }
    let services = crt_lsp::Services {
        structure: Arc::new(TreeSitterSource),
        explainer: Some(Arc::new(explainer)),
        explainer_unavailable: None,
        store,
        config_file: Some(crt_lsp::ConfigFile {
            ensure_exists: {
                let path = path.clone();
                Arc::new(move || {
                    config::write_example_if_missing(&path).map_err(|e| format!("{e:#}"))
                })
            },
            path,
        }),
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
            if reading.as_ref().is_some_and(|r| r.scenarios.is_some()) {
                continue;
            }
            let selector = FunctionSelector::Line(function.span.start_line);
            match crt_app::read_scenarios(
                &readers,
                file,
                &source,
                &selector,
                ReadOptions::default(),
            ) {
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
