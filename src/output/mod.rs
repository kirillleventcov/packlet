use crate::core::traverser::DependencyGraph;
use anyhow::Result;
use std::collections::HashMap;
use std::fmt::Write;
use std::path::{Path, PathBuf};

/// Convert absolute path to relative path from git root, or return display string if not in git repo
fn format_path(path: &Path, git_root: Option<&Path>) -> String {
    if let Some(root) = git_root {
        if let Ok(relative) = path.strip_prefix(root) {
            return relative.display().to_string();
        }
    }
    path.display().to_string()
}

pub trait OutputFormatter: Send + Sync {
    fn format(&self, graph: &DependencyGraph, files: &HashMap<PathBuf, String>) -> Result<String>;
    fn format_with_git_root(
        &self,
        graph: &DependencyGraph,
        files: &HashMap<PathBuf, String>,
        git_root: Option<&Path>,
    ) -> Result<String>;
}

pub struct MarkdownFormatter;

impl MarkdownFormatter {
    fn render_tree(&self, graph: &DependencyGraph, git_root: Option<&Path>) -> Result<String> {
        let mut output = String::new();
        let mut visited = HashMap::new();
        self.write_tree_recursive(
            &mut output,
            graph,
            &mut visited,
            &graph.entry_point,
            "",
            git_root,
        )?;
        Ok(output)
    }

    pub fn format_tree_only(&self, graph: &DependencyGraph) -> Result<String> {
        self.render_tree(graph, None)
    }

    pub fn format_tree_only_with_git_root(
        &self,
        graph: &DependencyGraph,
        git_root: Option<&Path>,
    ) -> Result<String> {
        self.render_tree(graph, git_root)
    }

    fn write_tree_recursive(
        &self,
        output: &mut String,
        graph: &DependencyGraph,
        visited: &mut HashMap<PathBuf, bool>,
        path: &Path,
        prefix: &str,
        git_root: Option<&Path>,
    ) -> Result<()> {
        if visited.get(path).is_some() {
            writeln!(
                output,
                "{} {} (circular)",
                prefix,
                format_path(path, git_root)
            )?;
            return Ok(());
        }
        writeln!(output, "{} {}", prefix, format_path(path, git_root))?;
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
                    git_root,
                )?;
            }
        }
        Ok(())
    }
}

impl OutputFormatter for MarkdownFormatter {
    fn format(&self, graph: &DependencyGraph, files: &HashMap<PathBuf, String>) -> Result<String> {
        self.format_with_git_root(graph, files, None)
    }

    fn format_with_git_root(
        &self,
        graph: &DependencyGraph,
        files: &HashMap<PathBuf, String>,
        git_root: Option<&Path>,
    ) -> Result<String> {
        let mut output = String::new();

        writeln!(output, "# Packlet Dependency Bundle\n")?;
        writeln!(output, "**Generated:** {}", chrono::Utc::now().to_rfc2822())?;
        writeln!(
            output,
            "**Entry:** `{}`\n",
            format_path(&graph.entry_point, git_root)
        )?;

        writeln!(output, "## Dependency Tree\n")?;
        writeln!(output, "```")?;
        write!(output, "{}", self.render_tree(graph, git_root)?)?;
        writeln!(output, "```\n")?;

        writeln!(output, "## File Contents\n")?;
        let mut sorted_files: Vec<_> = files.iter().collect();
        sorted_files.sort_by(|a, b| a.0.cmp(b.0));

        for (path, content) in sorted_files {
            let lang = path.extension().and_then(|s| s.to_str()).unwrap_or("");
            writeln!(output, "### `{}`\n", format_path(path, git_root))?;
            writeln!(output, "```{}", lang)?;
            writeln!(output, "{}", content)?;
            writeln!(output, "```\n")?;
        }

        Ok(output)
    }
}
