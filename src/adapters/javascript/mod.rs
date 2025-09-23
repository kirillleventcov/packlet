mod parser;
mod resolver;

use crate::core::language::{AnalysisContext, ImportStatement, LanguageAdapter, ResolvedImport};
use anyhow::Result;
use async_trait::async_trait;
use parser::JsParser;
use resolver::JsResolver;
use std::path::Path;

#[derive(Clone, Copy)]
pub struct JsAdapter {
    parser: JsParser,
    resolver: JsResolver,
}

impl JsAdapter {
    pub fn new() -> Self {
        Self {
            parser: JsParser::new(),
            resolver: JsResolver::new(),
        }
    }
}

#[async_trait]
impl LanguageAdapter for JsAdapter {
    fn name(&self) -> &str {
        "JavaScript/TypeScript"
    }

    fn supported_extensions(&self) -> &[&str] {
        &[
            "js", "mjs", "cjs", "ts", "tsx", "jsx", "d.ts", "vue", "svelte",
        ]
    }

    async fn parse_imports(
        &self,
        file_path: &Path,
        content: &str,
        _context: &AnalysisContext,
    ) -> Result<Vec<ImportStatement>> {
        let file_path = file_path.to_path_buf();
        let content = content.to_string();
        let parser = self.parser;

        tokio::task::spawn_blocking(move || parser.parse(&file_path, &content)).await?
    }

    async fn resolve_import(
        &self,
        import: &ImportStatement,
        from_file: &Path,
        context: &AnalysisContext,
    ) -> Result<Option<ResolvedImport>> {
        self.resolver
            .resolve(&import.specifier, from_file, &*context.fs)
            .await
    }
}
