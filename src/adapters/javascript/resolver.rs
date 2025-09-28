use crate::core::fs::FileSystemProvider;
use crate::core::language::ResolvedImport;
use anyhow::Result;
use path_absolutize::Absolutize;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy)]
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

        if self.is_asset_import(specifier) {
            let from_dir = from_file.parent().unwrap_or_else(|| Path::new("/"));
            let asset_path = from_dir.join(specifier);

            if fs.exists(&asset_path).await {
                return Ok(Some(ResolvedImport {
                    path: asset_path.absolutize()?.to_path_buf(),
                    is_local: true,
                    is_asset: true,
                }));
            }

            log::debug!(
                "Asset import '{}' from {} not found",
                specifier,
                from_file.display()
            );
            return Ok(None);
        }

        let from_dir = from_file.parent().unwrap_or_else(|| Path::new("/"));
        let base_path = from_dir.join(specifier);

        let resolved_path = self.resolve_file_with_extensions(&base_path, fs).await?;

        if let Some(path) = resolved_path {
            Ok(Some(ResolvedImport {
                path: path.absolutize()?.to_path_buf(),
                is_local: true,
                is_asset: false,
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
        // Relative paths are not external
        if specifier.starts_with('.') || specifier.starts_with('/') {
            return false;
        }

        // Path aliases from tsconfig/bundler config are not external
        if self.is_configured_alias(specifier) {
            return false;
        }

        // Everything else is treated as an external package
        true
    }

    fn is_asset_import(&self, specifier: &str) -> bool {
        const ASSET_EXTENSIONS: &[&str] = &[
            "css", "scss", "sass", "less", "styl", "stylus", // Stylesheets
            "png", "jpg", "jpeg", "gif", "svg", "webp", "ico", // Images
            "woff", "woff2", "ttf", "otf", "eot", // Fonts
            "mp4", "webm", "ogg", "mp3", "wav", // Media
            "pdf", "doc", "docx", "xls", "xlsx", // Documents
            "json", "xml", "yaml", "yml", "toml", // Data files
            "md", "mdx", // Markdown
            "txt", "csv", // Text files
        ];

        // Get the file extension from the specifier
        if let Some(extension) = Path::new(specifier)
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_lowercase())
        {
            return ASSET_EXTENSIONS.contains(&extension.as_str());
        }

        // Check for special webpack/vite style imports
        // e.g., import styles from './App.module.css'
        if specifier.contains(".module.") {
            return true;
        }

        // Check for query parameters that indicate asset handling
        // e.g., import logo from './logo.svg?react'
        if specifier.contains('?') {
            let base = specifier.split('?').next().unwrap_or(specifier);
            return self.is_asset_import(base);
        }

        false
    }

    fn is_configured_alias(&self, specifier: &str) -> bool {
        const COMMON_ALIASES: &[&str] = &[
            "@/", // Common Vite/Next.js alias
            "~/", // Common webpack alias
            "@components/",
            "@utils/",
            "@assets/",
            "@hooks/",
            "@services/",
            "@store/",
            "@styles/",
        ];

        COMMON_ALIASES
            .iter()
            .any(|alias| specifier.starts_with(alias))
    }

    async fn resolve_file_with_extensions(
        &self,
        path: &Path,
        fs: &dyn FileSystemProvider,
    ) -> Result<Option<PathBuf>> {
        let extensions = ["tsx", "ts", "jsx", "js", "mjs", "cjs", "json"];

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
