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
