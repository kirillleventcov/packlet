use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TsConfigJson {
    #[serde(default)]
    pub compiler_options: CompilerOptions,
    #[serde(rename = "extends")]
    pub extends: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CompilerOptions {
    pub base_url: Option<String>,
    #[serde(default)]
    pub paths: HashMap<String, Vec<String>>,
    pub root_dir: Option<String>,
    pub root_dirs: Option<Vec<String>>,
}

#[derive(Debug, Clone)]
pub struct TsConfig {
    pub base_url: Option<PathBuf>,
    pub paths: HashMap<String, Vec<String>>,
    pub extends: Option<String>,
    pub config_dir: PathBuf,
}

pub struct TsConfigParser {
    cache: Arc<Mutex<HashMap<PathBuf, Arc<TsConfig>>>>,
}

impl TsConfigParser {
    pub fn new() -> Self {
        Self {
            cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn find_and_parse_config(
        &self,
        from_file: &Path,
        fs: &dyn crate::core::fs::FileSystemProvider,
    ) -> Result<Option<Arc<TsConfig>>> {
        let config_path = self.find_config_file(from_file, fs).await?;

        if let Some(path) = config_path {
            self.parse_config(&path, fs).await.map(Some)
        } else {
            Ok(None)
        }
    }

    async fn find_config_file(
        &self,
        from_file: &Path,
        fs: &dyn crate::core::fs::FileSystemProvider,
    ) -> Result<Option<PathBuf>> {
        let mut current = if fs.is_directory(from_file).await {
            from_file.to_path_buf()
        } else {
            from_file
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| PathBuf::from("/"))
        };

        loop {
            let tsconfig_path = current.join("tsconfig.json");
            if fs.exists(&tsconfig_path).await {
                log::debug!("Found tsconfig.json at: {}", tsconfig_path.display());
                return Ok(Some(tsconfig_path));
            }

            let jsconfig_path = current.join("jsconfig.json");
            if fs.exists(&jsconfig_path).await {
                log::debug!("Found jsconfig.json at: {}", jsconfig_path.display());
                return Ok(Some(jsconfig_path));
            }

            if let Some(parent) = current.parent() {
                current = parent.to_path_buf();
            } else {
                break;
            }
        }

        Ok(None)
    }

    pub async fn parse_config(
        &self,
        config_path: &Path,
        fs: &dyn crate::core::fs::FileSystemProvider,
    ) -> Result<Arc<TsConfig>> {
        let cache = self.cache.lock().await;

        if let Some(cached) = cache.get(config_path) {
            return Ok(cached.clone());
        }

        drop(cache);

        let content = fs
            .read_file(config_path)
            .await
            .context("Failed to read tsconfig.json")?;

        let content = Self::strip_json_comments(&content);

        let tsconfig_json: TsConfigJson = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse {}", config_path.display()))?;

        let config_dir = config_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));

        let mut base_url = tsconfig_json
            .compiler_options
            .base_url
            .map(|url| config_dir.join(url));
        let mut all_paths = HashMap::new();

        let mut current_extends = tsconfig_json.extends.clone();
        let mut visited_extends = std::collections::HashSet::new();

        while let Some(extends) = current_extends {
            if visited_extends.contains(&extends) {
                log::warn!("Circular extends detected: {}", extends);
                break;
            }
            visited_extends.insert(extends.clone());

            let extended_path = if extends.starts_with("./") || extends.starts_with("../") {
                config_dir.join(&extends)
            } else {
                let node_modules_path = self.find_node_modules(&config_dir, fs).await?;
                node_modules_path.join(&extends)
            };

            let extended_path = if extended_path.extension().is_none() {
                extended_path.with_extension("json")
            } else {
                extended_path
            };

            if fs.exists(&extended_path).await {
                let extended_content = fs
                    .read_file(&extended_path)
                    .await
                    .context("Failed to read extended tsconfig")?;

                let extended_content = Self::strip_json_comments(&extended_content);
                let extended_json: TsConfigJson = serde_json::from_str(&extended_content)
                    .with_context(|| {
                        format!(
                            "Failed to parse extended config {}",
                            extended_path.display()
                        )
                    })?;

                if base_url.is_none() {
                    base_url = extended_json
                        .compiler_options
                        .base_url
                        .map(|url| extended_path.parent().unwrap_or(&config_dir).join(url));
                }

                for (key, value) in extended_json.compiler_options.paths {
                    all_paths.entry(key).or_insert(value);
                }

                current_extends = extended_json.extends;
            } else {
                log::warn!(
                    "Extended config not found: {} (from {})",
                    extended_path.display(),
                    extends
                );
                break;
            }
        }

        for (key, value) in tsconfig_json.compiler_options.paths {
            all_paths.insert(key, value);
        }

        let config = Arc::new(TsConfig {
            base_url,
            paths: all_paths,
            extends: tsconfig_json.extends,
            config_dir: config_dir.clone(),
        });

        let mut cache = self.cache.lock().await;
        cache.insert(config_path.to_path_buf(), config.clone());

        Ok(config)
    }

    async fn find_node_modules(
        &self,
        from: &Path,
        fs: &dyn crate::core::fs::FileSystemProvider,
    ) -> Result<PathBuf> {
        let mut current = from.to_path_buf();

        loop {
            let node_modules = current.join("node_modules");
            if fs.exists(&node_modules).await && fs.is_directory(&node_modules).await {
                return Ok(node_modules);
            }

            if let Some(parent) = current.parent() {
                current = parent.to_path_buf();
            } else {
                break;
            }
        }

        Ok(from.join("node_modules"))
    }

    fn strip_json_comments(content: &str) -> String {
        let mut result = String::new();
        let mut chars = content.chars().peekable();
        let mut in_string = false;
        let mut escape_next = false;

        while let Some(ch) = chars.next() {
            if escape_next {
                result.push(ch);
                escape_next = false;
                continue;
            }

            if ch == '\\' && in_string {
                result.push(ch);
                escape_next = true;
                continue;
            }

            if ch == '"' && !in_string {
                in_string = true;
                result.push(ch);
            } else if ch == '"' && in_string {
                in_string = false;
                result.push(ch);
            } else if ch == '/' && !in_string {
                if let Some(&next_ch) = chars.peek() {
                    if next_ch == '/' {
                        chars.next();
                        while let Some(ch) = chars.next() {
                            if ch == '\n' {
                                result.push('\n');
                                break;
                            }
                        }
                    } else if next_ch == '*' {
                        chars.next();
                        let mut prev_ch = ' ';
                        while let Some(ch) = chars.next() {
                            if prev_ch == '*' && ch == '/' {
                                break;
                            }
                            prev_ch = ch;
                        }
                    } else {
                        result.push(ch);
                    }
                } else {
                    result.push(ch);
                }
            } else {
                result.push(ch);
            }
        }

        result
    }
}

impl TsConfig {
    pub fn resolve_alias(&self, specifier: &str) -> Option<Vec<PathBuf>> {
        for (pattern, replacements) in &self.paths {
            if let Some(resolved) = self.match_pattern(specifier, pattern, replacements) {
                return Some(resolved);
            }
        }
        None
    }

    fn match_pattern(
        &self,
        specifier: &str,
        pattern: &str,
        replacements: &[String],
    ) -> Option<Vec<PathBuf>> {
        let base_dir = self.base_url.as_ref().unwrap_or(&self.config_dir);

        if pattern.contains('*') {
            let pattern_parts: Vec<&str> = pattern.split('*').collect();
            if pattern_parts.len() != 2 {
                return None;
            }

            let prefix = pattern_parts[0];
            let suffix = pattern_parts[1];

            if specifier.starts_with(prefix) && specifier.ends_with(suffix) {
                let wildcard_match = &specifier[prefix.len()..specifier.len() - suffix.len()];

                let mut results = Vec::new();
                for replacement in replacements {
                    let resolved = if replacement.contains('*') {
                        replacement.replace('*', wildcard_match)
                    } else {
                        replacement.clone()
                    };

                    let full_path = base_dir.join(&resolved);
                    results.push(full_path);
                }
                return Some(results);
            }
        } else if specifier == pattern {
            let mut results = Vec::new();
            for replacement in replacements {
                let full_path = base_dir.join(replacement);
                results.push(full_path);
            }
            return Some(results);
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_json_comments() {
        let input = r#"{
            // This is a line comment
            "compilerOptions": {
                /* This is a block comment */
                "baseUrl": "./src", // Another comment
                "paths": {
                    "@/*": ["*"]
                }
            }
        }"#;

        let expected = r#"{
            
            "compilerOptions": {
                
                "baseUrl": "./src", 
                "paths": {
                    "@/*": ["*"]
                }
            }
        }"#;

        let result = TsConfigParser::strip_json_comments(input);
        let parsed: Result<TsConfigJson, _> = serde_json::from_str(&result);
        assert!(parsed.is_ok());
    }

    #[test]
    fn test_pattern_matching() {
        let config = TsConfig {
            base_url: Some(PathBuf::from("/project/src")),
            paths: HashMap::from([
                ("@/*".to_string(), vec!["*".to_string()]),
                (
                    "@components/*".to_string(),
                    vec!["components/*".to_string()],
                ),
                ("utils".to_string(), vec!["utils/index.ts".to_string()]),
            ]),
            extends: None,
            config_dir: PathBuf::from("/project"),
        };

        let result = config.resolve_alias("@/components/Button");
        assert!(result.is_some());
        let paths = result.unwrap();
        assert_eq!(paths[0], PathBuf::from("/project/src/components/Button"));

        let result = config.resolve_alias("@components/Button");
        assert!(result.is_some());
        let paths = result.unwrap();
        assert_eq!(paths[0], PathBuf::from("/project/src/components/Button"));

        let result = config.resolve_alias("utils");
        assert!(result.is_some());
        let paths = result.unwrap();
        assert_eq!(paths[0], PathBuf::from("/project/src/utils/index.ts"));

        let result = config.resolve_alias("random/import");
        assert!(result.is_none());
    }
}
