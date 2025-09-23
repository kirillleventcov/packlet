use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;
use path_absolutize::Absolutize;
use std::sync::Arc;
use crate::{core, output};
use crate::core::fs::FileSystemProvider;

#[derive(Parser)]
#[command(name = "packlet")]
#[command(about = "Lightning-fast local dependency bundler")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,

    #[arg(short, long, global = true)]
    pub verbose: bool,

    #[arg(long, global = true)]
    pub config: Option<PathBuf>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Bundle dependencies from an entry file
    Bundle {
        /// Entry file path
        file: PathBuf,

        /// Output format
        #[arg(short, long, value_enum, default_value = "markdown")]
        format: OutputFormat,

        /// Output file path (stdout if not specified)
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Maximum traversal depth
        #[arg(long)]
        max_depth: Option<usize>,

        /// Include only specific file extensions
        #[arg(long, value_delimiter = ',')]
        extensions: Option<Vec<String>>,

        /// Exclude patterns (gitignore syntax)
        #[arg(long, value_delimiter = ',')]
        exclude: Option<Vec<String>>,
    },

    /// Analyze dependencies without bundling
    Analyze {
        file: PathBuf,

        #[arg(long)]
        json: bool,

        #[arg(long)]
        stats: bool,
    },

    /// Visualize dependency graph
    Graph {
        file: PathBuf,

        #[arg(long, value_enum, default_value = "dot")]
        format: GraphFormat,
    },
}

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum, Debug)]
pub enum OutputFormat {
    Markdown,
    Xml,
}

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum, Debug)]
pub enum GraphFormat {
    Dot,
    Json,
}

pub async fn run(cli: Cli) -> anyhow::Result<()> {
    match cli.command {
        Commands::Bundle { file, format, output, max_depth, .. } => {
            let entry_file = file.absolutize()?.to_path_buf();
            log::info!("Bundling {}...", entry_file.display());

            let fs_provider = Arc::new(core::fs::CachedFileSystem::new(Box::new(core::fs::LocalFileSystem)));
            
            let extension = entry_file.extension().and_then(|s| s.to_str()).unwrap_or("");
            let adapter = Arc::from(core::language::get_adapter_for_extension(extension)
                .ok_or_else(|| anyhow::anyhow!("Unsupported file type: {}", entry_file.display()))?);

            let context = Arc::new(core::language::AnalysisContext { fs: fs_provider.clone() });

            let traverser = core::traverser::DependencyTraverser::new().with_max_depth(max_depth);
            let graph = traverser.traverse(&entry_file, adapter, context).await?;

            let mut file_contents = std::collections::HashMap::new();
            let mut files_to_read = vec![graph.entry_point.clone()];
            for (from, deps) in &graph.adj_list {
                files_to_read.push(from.clone());
                for (to, _) in deps {
                    files_to_read.push(to.clone());
                }
            }
            files_to_read.sort();
            files_to_read.dedup();

            for file_path in files_to_read {
                if let Ok(content) = fs_provider.read_file(&file_path).await {
                    file_contents.insert(file_path, content);
                } else {
                    log::warn!("Could not read file: {}", file_path.display());
                }
            }

            let formatter: Box<dyn output::OutputFormatter> = match format {
                OutputFormat::Markdown => Box::new(output::MarkdownFormatter),
                OutputFormat::Xml => unimplemented!("XML output is not supported yet"),
            };

            let output_str = formatter.format(&graph, &file_contents)?;

            if let Some(output_path) = output {
                tokio::fs::write(output_path, output_str).await?;
            } else {
                println!("{}", output_str);
            }
        }
        Commands::Analyze { file, .. } => {
            println!("Analyzing {}...", file.display());
            println!("(Not implemented yet)");
        }
        Commands::Graph { file, .. } => {
            println!("Generating graph for {}...", file.display());
            println!("(Not implemented yet)");
        }
    }
    Ok(())
}
