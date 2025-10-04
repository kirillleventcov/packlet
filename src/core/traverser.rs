use crate::core::language::{AnalysisContext, ImportStatement, LanguageAdapter};
use anyhow::Result;
use dashmap::DashSet;
use futures::future::try_join_all;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::{Mutex, Semaphore};

#[derive(Debug)]
pub struct DependencyGraph {
    pub entry_point: PathBuf,
    pub adj_list: HashMap<PathBuf, Vec<(PathBuf, ImportStatement)>>,
    pub circular_deps: DashSet<PathBuf>,
    pub assets: DashSet<PathBuf>,
}

impl serde::Serialize for DependencyGraph {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("DependencyGraph", 4)?;
        state.serialize_field("entry_point", &self.entry_point)?;
        state.serialize_field("adj_list", &self.adj_list)?;

        let circular_deps: Vec<PathBuf> = self.circular_deps.iter().map(|p| p.clone()).collect();
        let assets: Vec<PathBuf> = self.assets.iter().map(|p| p.clone()).collect();

        state.serialize_field("circular_deps", &circular_deps)?;
        state.serialize_field("assets", &assets)?;
        state.end()
    }
}

impl DependencyGraph {
    pub fn new(entry_point: PathBuf) -> Self {
        Self {
            entry_point,
            adj_list: HashMap::new(),
            circular_deps: DashSet::new(),
            assets: DashSet::new(),
        }
    }

    pub fn add_edge(&mut self, from: &Path, to: &Path, import: ImportStatement) {
        self.adj_list
            .entry(from.to_path_buf())
            .or_default()
            .push((to.to_path_buf(), import));
    }

    pub fn mark_circular(&self, path: &Path) {
        self.circular_deps.insert(path.to_path_buf());
    }

    pub fn add_asset(&self, path: &Path) {
        self.assets.insert(path.to_path_buf());
    }
}

#[derive(Clone)]
pub struct DependencyTraverser {
    visited: Arc<DashSet<PathBuf>>,
    in_progress: Arc<DashSet<PathBuf>>,
    max_depth: usize,
    max_files: usize,
    file_count: Arc<AtomicUsize>,
    semaphore: Arc<Semaphore>,
    include_assets: bool,
}

impl DependencyTraverser {
    pub fn new() -> Self {
        Self {
            visited: Arc::new(DashSet::new()),
            in_progress: Arc::new(DashSet::new()),
            max_depth: 50,
            max_files: 10_000,
            file_count: Arc::new(AtomicUsize::new(0)),
            semaphore: Arc::new(Semaphore::new(32)),
            include_assets: false,
        }
    }

    pub fn with_max_depth(mut self, max_depth: Option<usize>) -> Self {
        self.max_depth = max_depth.unwrap_or(50);
        self
    }

    pub fn with_max_files(mut self, max_files: usize) -> Self {
        self.max_files = max_files;
        self
    }

    pub fn with_concurrency_limit(mut self, limit: usize) -> Self {
        self.semaphore = Arc::new(Semaphore::new(limit));
        self
    }

    pub fn with_assets(mut self, include: bool) -> Self {
        self.include_assets = include;
        self
    }

    pub async fn traverse(
        &self,
        entry: &Path,
        adapter: Arc<dyn LanguageAdapter>,
        context: Arc<AnalysisContext>,
    ) -> Result<DependencyGraph> {
        let graph = Arc::new(Mutex::new(DependencyGraph::new(entry.to_path_buf())));

        self.traverse_recursive(entry.to_path_buf(), adapter, context, graph.clone(), 0)
            .await?;

        Ok(Arc::try_unwrap(graph)
            .map_err(|_| {
                anyhow::anyhow!("Failed to unwrap Arc, graph is still referenced elsewhere")
            })?
            .into_inner())
    }

    async fn traverse_recursive(
        &self,
        file: PathBuf,
        adapter: Arc<dyn LanguageAdapter>,
        context: Arc<AnalysisContext>,
        graph: Arc<Mutex<DependencyGraph>>,
        depth: usize,
    ) -> Result<()> {
        if depth >= self.max_depth {
            log::debug!("Max depth {} reached at {}", self.max_depth, file.display());
            return Ok(());
        }

        // File count limit
        let current_count = self.file_count.fetch_add(1, Ordering::Relaxed);
        if current_count >= self.max_files {
            log::warn!(
                "Max file limit ({}) reached. Stopping traversal. Consider increasing --max-files or --max-depth.",
                self.max_files
            );
            return Err(anyhow::anyhow!(
                "File limit exceeded: {} files processed (limit: {})",
                current_count + 1,
                self.max_files
            ));
        }

        if current_count > 0 && current_count % 1000 == 0 {
            log::info!("Progress: {} files processed...", current_count);
        }

        let _permit = self.semaphore.acquire().await?;

        let canonical = context.fs.canonicalize(&file).await?;
        if self.visited.contains(&canonical) {
            return Ok(());
        }

        if self.in_progress.contains(&canonical) {
            graph.lock().await.mark_circular(&canonical);
            return Ok(());
        }

        self.visited.insert(canonical.clone());
        self.in_progress.insert(canonical.clone());

        if !adapter.can_parse_file(&canonical) {
            log::debug!(
                "Skipping non-parseable file: {} (not a JS/TS file)",
                canonical.display()
            );
            self.in_progress.remove(&canonical);
            return Ok(());
        }

        let content = match context.fs.read_file(&canonical).await {
            Ok(content) => content,
            Err(e) => {
                log::warn!("Could not read file {}: {}", canonical.display(), e);
                self.in_progress.remove(&canonical);
                return Ok(());
            }
        };

        let imports = adapter
            .parse_imports(&canonical, &content, &context)
            .await?;

        let mut tasks = Vec::new();
        for import in imports {
            if let Some(resolved) = adapter
                .resolve_import(&import, &canonical, &context)
                .await?
            {
                if resolved.is_local {
                    graph
                        .lock()
                        .await
                        .add_edge(&canonical, &resolved.path, import.clone());

                    if resolved.is_asset {
                        graph.lock().await.add_asset(&resolved.path);
                        log::debug!(
                            "Found asset dependency: {} -> {}",
                            canonical.display(),
                            resolved.path.display()
                        );
                        continue;
                    }

                    let adapter = adapter.clone();
                    let context = context.clone();
                    let graph = graph.clone();

                    let task =
                        self.traverse_recursive(resolved.path, adapter, context, graph, depth + 1);
                    tasks.push(task);
                }
            }
        }

        match try_join_all(tasks).await {
            Ok(_) => {}
            Err(e) => {
                log::error!("Error traversing dependencies: {}", e);
            }
        }

        self.in_progress.remove(&canonical);

        Ok(())
    }
}
