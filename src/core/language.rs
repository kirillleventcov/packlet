use crate::adapters::javascript::JsAdapter;
use anyhow::Result;
use async_trait::async_trait;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub struct AnalysisContext {
    pub fs: Arc<dyn FileSystemProvider>,
}

#[async_trait]
pub trait LanguageAdapter: Send + Sync {
    fn name(&self) -> &str;
    fn supported_extensions(&self) -> &[&str];
    async fn parse_imports(
        &self,
        file_path: &Path,
        content: &str,
        context: &AnalysisContext,
    ) -> Result<Vec<ImportStatement>>;
    async fn resolve_import(
        &self,
        import: &ImportStatement,
        from_file: &Path,
        context: &AnalysisContext,
    ) -> Result<Option<ResolvedImport>>;
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct ImportStatement {
    pub specifier: String,
    pub kind: ImportKind,
    pub line: usize,
    pub column: usize,
    pub raw: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub enum ImportKind {
    EsModule,
    CommonJs,
    Dynamic,
    TypeOnly,
    Asset,
    Custom(String),
}

#[derive(Debug, Clone)]
pub struct ResolvedImport {
    pub path: PathBuf,
    pub is_local: bool,
}

pub fn get_adapter_for_extension(extension: &str) -> Option<Box<dyn LanguageAdapter>> {
    let js_adapter = JsAdapter::new();
    if js_adapter.supported_extensions().contains(&extension) {
        Some(Box::new(js_adapter))
    } else {
        None
    }
}

pub use super::fs::FileSystemProvider;
