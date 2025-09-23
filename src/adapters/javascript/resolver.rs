use crate::core::fs::FileSystemProvider;
use crate::core::language::ResolvedImport;
use anyhow::Result;
use path_absolutize::Absolutize;
use std::path::{Path, PathBuf};

pub struct JsResolver;

impl JsResolver {
    pub fn new() -> Self {
        Self
    }

    pub async fn resolve(
        &self,
        specifier: &str,
        from_file: &Path,
        fs: &dyn FileSystemProvider,
    ) -> Result<Option<ResolvedImport>> {
        if self.is_external_package(specifier) {
            return Ok(None);
        }

        let from_dir = from_file.parent().unwrap_or_else(|| Path::new("/"));
        let base_path = from_dir.join(specifier);

        let resolved_path = self.resolve_file_with_extensions(&base_path, fs).await?;

        if let Some(path) = resolved_path {
            Ok(Some(ResolvedImport {
                path: path.absolutize()?.to_path_buf(),
                is_local: true,
            }))
        } else {
            log::warn!(
                "Could not resolve local import: '{}' from {}",
                specifier,
                from_file.display()
            );
            Ok(None)
        }
    }

    fn is_external_package(&self, specifier: &str) -> bool {
        !specifier.starts_with('.') && !specifier.starts_with('/')
    }

    async fn resolve_file_with_extensions(
        &self,
        path: &Path,
        fs: &dyn FileSystemProvider,
    ) -> Result<Option<PathBuf>> {
        let extensions = ["js", "ts", "jsx", "tsx", "json", "mjs", "cjs"];

        // 1. Try as a file with existing extension
        if fs.exists(path).await && !fs.is_directory(path).await {
            return Ok(Some(path.to_path_buf()));
        }

        // 2. Try adding extensions
        for ext in extensions {
            let new_path = path.with_extension(ext);
            if fs.exists(&new_path).await && !fs.is_directory(&new_path).await {
                return Ok(Some(new_path));
            }
        }

        // 3. Try as a directory with index file
        if fs.exists(path).await && fs.is_directory(path).await {
            for ext in extensions {
                let index_path = path.join(format!("index.{}", ext));
                if fs.exists(&index_path).await {
                    return Ok(Some(index_path));
                }
            }
        }

        // 4. Try parent path with extensions if original path had no extension
        if path.extension().is_none() {
            for ext in extensions {
                let new_path =
                    PathBuf::from(format!("{}.{}", path.to_str().unwrap_or_default(), ext));
                if fs.exists(&new_path).await {
                    return Ok(Some(new_path));
                }
            }
        }

        Ok(None)
    }
}
