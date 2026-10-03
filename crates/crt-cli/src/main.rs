//! The `crt` command. Composition root: picks the adapters, calls the use
//! cases, prints wire JSON.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use crt_treesitter::TreeSitterSource;
use crt_wire::FileAnalysisDto;

#[derive(Parser)]
#[command(name = "crt", version, about = "Read code by how it behaves")]
struct Cli {
    #[command(subcommand)]
    command: Command,
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
    /// List the bundled languages and their file extensions.
    Languages,
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Analyze { file, func } => analyze(&file, func.as_deref()),
        Command::Languages => {
            for g in TreeSitterSource::languages() {
                println!("{}\t{}", g.id, g.extensions.join(","));
            }
            Ok(())
        }
    }
}

fn analyze(file: &Path, func: Option<&str>) -> Result<()> {
    let source = std::fs::read(file).with_context(|| format!("reading {}", file.display()))?;
    let mut analysis = crt_app::analyze_file(&TreeSitterSource, file, &source)?;
    if let Some(name) = func {
        analysis.functions.retain(|f| f.name == name);
        anyhow::ensure!(
            !analysis.functions.is_empty(),
            "no function named {name} in {}",
            file.display()
        );
    }
    println!(
        "{}",
        serde_json::to_string_pretty(&FileAnalysisDto::from(&analysis))?
    );
    Ok(())
}
