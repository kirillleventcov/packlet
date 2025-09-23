use crate::core::fs::FileSystemProvider;
use crate::{core, output};
use clap::{Parser, Subcommand, ValueEnum};
use path_absolutize::Absolutize;
use std::path::{Path, PathBuf};
use std::sync::Arc;

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

        /// Output file path (auto-generated if not specified)
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

/// Generate a default output filename based on the input file and format
fn generate_output_filename(input_file: &Path, format: OutputFormat) -> PathBuf {
    // Get the input filename without extension
    let stem = input_file
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("output");

    // Determine the appropriate extension based on format
    let extension = match format {
        OutputFormat::Markdown => "md",
        OutputFormat::Xml => "xml",
    };

    // Create the output filename with a "packlet" suffix to avoid conflicts
    let filename = format!("{}.packlet.{}", stem, extension);

    // Return as a PathBuf in the current directory
    PathBuf::from(filename)
}

pub async fn run(cli: Cli) -> anyhow::Result<()> {
    match cli.command {
        Commands::Bundle {
            file,
            format,
            output,
            max_depth,
            ..
        } => {
            let entry_file = file.absolutize()?.to_path_buf();

            // Determine the output path - either provided or auto-generated
            let output_path = output.unwrap_or_else(|| generate_output_filename(&file, format));

            // Log what we're doing
            log::info!(
                "Bundling {} into {}...",
                entry_file.display(),
                output_path.display()
            );

            // Create a more user-friendly console message
            println!("Bundling: {}", entry_file.display());
            println!("Output format: {:?}", format);

            let fs_provider = Arc::new(core::fs::CachedFileSystem::new(Box::new(
                core::fs::LocalFileSystem,
            )));

            let extension = entry_file
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("");
            let adapter = Arc::from(
                core::language::get_adapter_for_extension(extension).ok_or_else(|| {
                    anyhow::anyhow!("Unsupported file type: {}", entry_file.display())
                })?,
            );

            let context = Arc::new(core::language::AnalysisContext {
                fs: fs_provider.clone(),
            });

            let traverser = core::traverser::DependencyTraverser::new().with_max_depth(max_depth);

            // Show progress indicator
            println!("Analyzing dependencies...");
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

            // Show how many files we found
            println!("Found {} local dependencies", files_to_read.len() - 1);

            for file_path in files_to_read {
                if let Ok(content) = fs_provider.read_file(&file_path).await {
                    file_contents.insert(file_path, content);
                } else {
                    log::warn!("Could not read file: {}", file_path.display());
                }
            }

            let formatter: Box<dyn output::OutputFormatter> = match format {
                OutputFormat::Markdown => Box::new(output::MarkdownFormatter),
                OutputFormat::Xml => {
                    return Err(anyhow::anyhow!("XML output format is not yet supported"));
                }
            };

            println!("Generating output...");
            let output_str = formatter.format(&graph, &file_contents)?;

            // Always write to a file
            tokio::fs::write(&output_path, output_str).await?;

            // Calculate file size for user feedback
            let metadata = tokio::fs::metadata(&output_path).await?;
            let size_kb = metadata.len() as f64 / 1024.0;

            // Success message with file location and size
            println!(
                "Successfully created: {} ({:.2} KB)",
                output_path.display(),
                size_kb
            );
            println!("Tip: Use --output to specify a custom output location");
        }
        Commands::Graph { file, format } => {
            let entry_file = file.absolutize()?.to_path_buf();

            println!("Generating graph for {}...", entry_file.display());

            let fs_provider = Arc::new(core::fs::CachedFileSystem::new(Box::new(
                core::fs::LocalFileSystem,
            )));

            let extension = entry_file
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("");
            let adapter = Arc::from(
                core::language::get_adapter_for_extension(extension).ok_or_else(|| {
                    anyhow::anyhow!("Unsupported file type: {}", entry_file.display())
                })?,
            );

            let context = Arc::new(core::language::AnalysisContext {
                fs: fs_provider.clone(),
            });

            let traverser = core::traverser::DependencyTraverser::new();

            println!("Analyzing dependencies...");
            let graph = traverser.traverse(&entry_file, adapter, context).await?;

            let dep_count = graph
                .adj_list
                .values()
                .map(|deps| deps.len())
                .sum::<usize>();
            println!("Found {} local dependencies", dep_count);

            match format {
                GraphFormat::Dot => {
                    // Use the existing tree rendering logic from MarkdownFormatter
                    let formatter = output::MarkdownFormatter;
                    let tree_output = formatter.format_tree_only(&graph)?;
                    println!("\n{}", tree_output);
                }
                GraphFormat::Json => {
                    // JSON format for programmatic use
                    let json_output = serde_json::to_string_pretty(&graph)?;
                    println!("{}", json_output);
                }
            }
        }
    }
    Ok(())
}
