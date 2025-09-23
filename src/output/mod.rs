use crate::core::traverser::DependencyGraph;
use anyhow::Result;
use std::collections::HashMap;
use std::fmt::Write;
use std::path::{Path, PathBuf};

pub trait OutputFormatter: Send + Sync {
    fn format(&self, graph: &DependencyGraph, files: &HashMap<PathBuf, String>) -> Result<String>;
}

pub struct MarkdownFormatter;

impl MarkdownFormatter {
    pub fn format_tree_only(&self, graph: &DependencyGraph) -> Result<String> {
        let mut output = String::new();
        let mut visited = HashMap::new();
        self.write_tree_recursive(&mut output, graph, &mut visited, &graph.entry_point, "")?;
        Ok(output)
    }

    fn write_tree_recursive(
        &self,
        output: &mut String,
        graph: &DependencyGraph,
        visited: &mut HashMap<PathBuf, bool>,
        path: &Path,
        prefix: &str,
    ) -> Result<()> {
        if visited.get(path).is_some() {
            writeln!(output, "{} {} (circular)", prefix, path.display())?;
            return Ok(());
        }
        writeln!(output, "{} {}", prefix, path.display())?;
        visited.insert(path.to_path_buf(), true);

        if let Some(deps) = graph.adj_list.get(path) {
            let mut sorted_deps = deps.clone();
            sorted_deps.sort_by(|a, b| a.0.cmp(&b.0));

            for (i, (dep_path, _)) in sorted_deps.iter().enumerate() {
                let (new_prefix, branch) = if i == sorted_deps.len() - 1 {
                    (format!("{}    ", prefix), "└──")
                } else {
                    (format!("{}│   ", prefix), "├──")
                };
                self.write_tree_recursive(
                    output,
                    graph,
                    visited,
                    dep_path,
                    &format!("{}{}", new_prefix, branch),
                )?;
            }
        }
        Ok(())
    }
}

impl OutputFormatter for MarkdownFormatter {
    fn format(&self, graph: &DependencyGraph, files: &HashMap<PathBuf, String>) -> Result<String> {
        let mut output = String::new();

        writeln!(output, "# Packlet Dependency Bundle\n")?;
        writeln!(output, "**Generated:** {}", chrono::Utc::now().to_rfc2822())?;
        writeln!(output, "**Entry:** `{}`\n", graph.entry_point.display())?;

        writeln!(output, "## Dependency Tree\n")?;
        writeln!(output, "```")?;
        let mut visited = HashMap::new();
        self.write_tree_recursive(&mut output, graph, &mut visited, &graph.entry_point, "")?;
        writeln!(output, "```\n")?;

        writeln!(output, "## File Contents\n")?;
        let mut sorted_files: Vec<_> = files.iter().collect();
        sorted_files.sort_by(|a, b| a.0.cmp(b.0));

        for (path, content) in sorted_files {
            let lang = path.extension().and_then(|s| s.to_str()).unwrap_or("");
            writeln!(output, "### `{}`\n", path.display())?;
            writeln!(output, "```{}", lang)?;
            writeln!(output, "{}", content)?;
            writeln!(output, "```\n")?;
        }

        Ok(output)
    }
}
