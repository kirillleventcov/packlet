use crate::core::language::{AnalysisContext, ImportStatement, LanguageAdapter};
use anyhow::Result;
use dashmap::DashSet;
use futures::future::try_join_all;
use glob::Pattern;
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
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

/// PathScore evaluates whether a path should be skipped during traversal
#[derive(Debug, Default)]
struct PathScore {
    depth: usize,
    node_modules_count: usize,
    parent_traversals: usize,
    total_components: usize,
}

impl PathScore {
    /// Analyzes a path relative to the entry point
    fn from_path(path: &Path, entry: &Path) -> Self {
        let mut score = PathScore::default();

        for component in path.components() {
            match component {
                Component::ParentDir => score.parent_traversals += 1,
                Component::Normal(name) => {
                    if name == "node_modules" {
                        score.node_modules_count += 1;
                    }
                    score.total_components += 1;
                }
                _ => {}
            }
        }

        if let Some(entry_parent) = entry.parent() {
            if let Ok(relative) = path.strip_prefix(entry_parent) {
                score.depth = relative.components().count();
            }
        }

        score
    }

    fn should_skip(&self) -> bool {
        // Skip if too many parent traversals (likely escaping project boundary)
        self.parent_traversals > 3
            // Skip if entering node_modules
            || self.node_modules_count > 0
            // Skip if path is getting suspiciously long (potential infinite loop)
            || self.total_components > 20
            // Skip if too deep from entry (backup check)
            || self.depth > 15
    }
}

/// Default exclusion patterns for React/JS projects
const DEFAULT_EXCLUDES: &[&str] = &[
    "**/node_modules/**",
    "**/.next/**",
    "**/dist/**",
    "**/build/**",
    "**/.cache/**",
    "**/coverage/**",
    "**/*.test.*",
    "**/*.spec.*",
    "**/*.stories.*",
    "**/test/**",
    "**/tests/**",
    "**/__tests__/**",
    "**/__mocks__/**",
    "**/storybook-static/**",
    "**/.storybook/**",
    "**/public/**",
    "**/.git/**",
    "**/.svn/**",
    "**/.hg/**",
    "**/vendor/**",
    "**/tmp/**",
    "**/temp/**",
];

/// Checks if a path should be excluded based on patterns
fn should_exclude_path(path: &Path, exclude_patterns: &[Pattern]) -> bool {
    let path_str = path.to_string_lossy();

    for pattern in exclude_patterns {
        if pattern.matches(&path_str) {
            return true;
        }
    }

    for pattern_str in DEFAULT_EXCLUDES {
        if let Ok(pattern) = Pattern::new(pattern_str) {
            if pattern.matches(&path_str) {
                return true;
            }
        }
    }

    let exclude_dirs = [
        "node_modules",
        ".next",
        "dist",
        "build",
        "coverage",
        "__tests__",
        "__mocks__",
        ".git",
    ];

    path.components().any(|c| {
        if let Component::Normal(name) = c {
            exclude_dirs.contains(&name.to_str().unwrap_or(""))
        } else {
            false
        }
    })
}

/// TraversalStats monitors traversal health and detects stuck scenarios
#[derive(Clone)]
struct TraversalStats {
    files_processed: Arc<AtomicUsize>,
    last_progress: Arc<Mutex<Instant>>,
    stuck_threshold: Duration,
}

impl TraversalStats {
    fn new(stuck_threshold_secs: u64) -> Self {
        Self {
            files_processed: Arc::new(AtomicUsize::new(0)),
            last_progress: Arc::new(Mutex::new(Instant::now())),
            stuck_threshold: Duration::from_secs(stuck_threshold_secs),
        }
    }

    async fn record_progress(&self) {
        self.files_processed.fetch_add(1, Ordering::Relaxed);
        *self.last_progress.lock().await = Instant::now();
    }

    async fn check_health(&self) -> Result<()> {
        let last = *self.last_progress.lock().await;
        if last.elapsed() > self.stuck_threshold {
            return Err(anyhow::anyhow!(
                "Traversal appears stuck - no progress for {:?}",
                self.stuck_threshold
            ));
        }
        Ok(())
    }
}

/// CircuitBreaker prevents cascading failures by stopping traversal after too many errors
#[derive(Clone)]
struct CircuitBreaker {
    error_count: Arc<AtomicUsize>,
    max_total_errors: usize,
    max_consecutive_errors: usize,
    consecutive_errors: Arc<AtomicUsize>,
}

impl CircuitBreaker {
    fn new(max_total: usize, max_consecutive: usize) -> Self {
        Self {
            error_count: Arc::new(AtomicUsize::new(0)),
            max_total_errors: max_total,
            max_consecutive_errors: max_consecutive,
            consecutive_errors: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn record_error(&self) -> Result<()> {
        let total = self.error_count.fetch_add(1, Ordering::Relaxed) + 1;
        let consecutive = self.consecutive_errors.fetch_add(1, Ordering::Relaxed) + 1;

        if consecutive >= self.max_consecutive_errors {
            return Err(anyhow::anyhow!(
                "Circuit breaker tripped: {} consecutive errors encountered",
                consecutive
            ));
        }

        if total >= self.max_total_errors {
            return Err(anyhow::anyhow!(
                "Circuit breaker tripped: {} total errors encountered (limit: {})",
                total,
                self.max_total_errors
            ));
        }

        Ok(())
    }

    fn record_success(&self) {
        self.consecutive_errors.store(0, Ordering::Relaxed);
    }

    fn get_error_count(&self) -> usize {
        self.error_count.load(Ordering::Relaxed)
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
    exclude_patterns: Arc<Vec<Pattern>>,
    stats: TraversalStats,
    circuit_breaker: CircuitBreaker,
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
            exclude_patterns: Arc::new(Vec::new()),
            stats: TraversalStats::new(30),
            circuit_breaker: CircuitBreaker::new(1000, 50),
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

    pub fn with_exclude_patterns(mut self, patterns: Vec<String>) -> Self {
        let compiled_patterns: Vec<Pattern> = patterns
            .iter()
            .filter_map(|p| {
                Pattern::new(p).ok().or_else(|| {
                    log::warn!("Invalid exclude pattern: {}", p);
                    None
                })
            })
            .collect();
        self.exclude_patterns = Arc::new(compiled_patterns);
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

        let error_count = self.circuit_breaker.get_error_count();
        if error_count > 0 {
            log::warn!(
                "Traversal completed with {} errors (non-fatal)",
                error_count
            );
        }

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

        if should_exclude_path(&file, &self.exclude_patterns) {
            log::debug!("Skipping excluded path: {}", file.display());
            return Ok(());
        }

        let entry_point = {
            let g = graph.lock().await;
            g.entry_point.clone()
        };
        let path_score = PathScore::from_path(&file, &entry_point);
        if path_score.should_skip() {
            log::debug!(
                "Skipping {} - PathScore: depth={}, node_modules={}, parent_traversals={}, components={}",
                file.display(),
                path_score.depth,
                path_score.node_modules_count,
                path_score.parent_traversals,
                path_score.total_components
            );
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

        if current_count > 0 && current_count % 100 == 0 {
            self.stats.check_health().await?;
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
            Ok(content) => {
                self.circuit_breaker.record_success();
                content
            }
            Err(e) => {
                log::warn!("Could not read file {}: {}", canonical.display(), e);
                self.circuit_breaker.record_error()?;
                self.in_progress.remove(&canonical);
                return Ok(());
            }
        };

        let imports = match adapter.parse_imports(&canonical, &content, &context).await {
            Ok(imports) => {
                self.circuit_breaker.record_success();
                imports
            }
            Err(e) => {
                log::warn!("Could not parse file {}: {}", canonical.display(), e);
                self.circuit_breaker.record_error()?;
                self.in_progress.remove(&canonical);
                return Ok(());
            }
        };

        // Record progress after successfully parsing a file
        self.stats.record_progress().await;

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
