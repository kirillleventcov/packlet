use anyhow::Result;
use async_trait::async_trait;
use dashmap::DashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::fs;

#[async_trait]
pub trait FileSystemProvider: Send + Sync {
    async fn read_file(&self, path: &Path) -> Result<String>;
    async fn exists(&self, path: &Path) -> bool;
    async fn is_directory(&self, path: &Path) -> bool;
    async fn canonicalize(&self, path: &Path) -> Result<PathBuf>;
}

pub struct LocalFileSystem;

#[async_trait]
impl FileSystemProvider for LocalFileSystem {
    async fn read_file(&self, path: &Path) -> Result<String> {
        Ok(fs::read_to_string(path).await?)
    }

    async fn exists(&self, path: &Path) -> bool {
        fs::try_exists(path).await.unwrap_or(false)
    }

    async fn is_directory(&self, path: &Path) -> bool {
        if let Ok(metadata) = fs::metadata(path).await {
            metadata.is_dir()
        } else {
            false
        }
    }

    async fn canonicalize(&self, path: &Path) -> Result<PathBuf> {
        Ok(tokio::fs::canonicalize(path).await?)
    }
}

pub struct CachedFileSystem {
    inner: Box<dyn FileSystemProvider>,
    cache: Arc<DashMap<PathBuf, String>>,
}

impl CachedFileSystem {
    pub fn new(inner: Box<dyn FileSystemProvider>) -> Self {
        Self {
            inner,
            cache: Arc::new(DashMap::new()),
        }
    }
}

#[async_trait]
impl FileSystemProvider for CachedFileSystem {
    async fn read_file(&self, path: &Path) -> Result<String> {
        if let Some(content) = self.cache.get(path) {
            return Ok(content.clone());
        }
        let content = self.inner.read_file(path).await?;
        self.cache.insert(path.to_path_buf(), content.clone());
        Ok(content)
    }

    async fn exists(&self, path: &Path) -> bool {
        self.inner.exists(path).await
    }

    async fn is_directory(&self, path: &Path) -> bool {
        self.inner.is_directory(path).await
    }

    async fn canonicalize(&self, path: &Path) -> Result<PathBuf> {
        self.inner.canonicalize(path).await
    }
}
