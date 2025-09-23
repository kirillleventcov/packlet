use crate::core::language::{ImportKind, ImportStatement};
use anyhow::Result;
use std::path::Path;
use std::sync::Arc;
use swc_common::{SourceMap, SourceMapper, Span};
use swc_ecma_ast::{CallExpr, ExportAll, ExportDecl, ImportDecl, Lit};
use swc_ecma_parser::{lexer::Lexer, Parser, StringInput, Syntax};
use swc_ecma_visit::{VisitMut, VisitMutWith};

#[derive(Clone, Copy)]
pub struct JsParser;

impl JsParser {
    pub fn new() -> Self {
        Self
    }

    pub fn parse(&self, file_path: &Path, content: &str) -> Result<Vec<ImportStatement>> {
        let cm = Arc::<SourceMap>::default();
        let fm = cm.new_source_file(
            swc_common::FileName::Real(file_path.to_path_buf()).into(),
            content.to_string(),
        );

        let lexer = Lexer::new(
            Syntax::Typescript(Default::default()),
            Default::default(),
            StringInput::from(&*fm),
            None,
        );

        let mut parser = Parser::new_from(lexer);
        let module = parser
            .parse_module()
            .map_err(|e| anyhow::anyhow!("SWC parse error in {}: {:?}", file_path.display(), e));

        let mut module = match module {
            Ok(m) => m,
            Err(e) => return Err(e),
        };

        let mut visitor = ImportVisitor {
            imports: Vec::new(),
            source_map: cm,
        };

        module.visit_mut_with(&mut visitor);

        Ok(visitor.imports)
    }
}

struct ImportVisitor {
    imports: Vec<ImportStatement>,
    source_map: Arc<SourceMap>,
}

impl ImportVisitor {
    fn add_import(&mut self, specifier: String, kind: ImportKind, span: Span) {
        let loc = self.source_map.lookup_char_pos(span.lo);
        self.imports.push(ImportStatement {
            specifier,
            kind,
            line: loc.line,
            column: loc.col_display,
            raw: self.source_map.span_to_snippet(span).unwrap_or_default(),
        });
    }
}

impl VisitMut for ImportVisitor {
    fn visit_mut_import_decl(&mut self, n: &mut ImportDecl) {
        let specifier = n.src.value.to_string();
        self.add_import(specifier, ImportKind::EsModule, n.span);
        n.visit_mut_children_with(self);
    }

    fn visit_mut_export_decl(&mut self, n: &mut ExportDecl) {
        match &mut n.decl {
            swc_ecma_ast::Decl::Class(_) => {}
            swc_ecma_ast::Decl::Fn(_) => {}
            swc_ecma_ast::Decl::Var(v) => {
                if let Some(src) = v.decls.get_mut(0).and_then(|d| {
                    d.init
                        .as_mut()
                        .and_then(|i| i.as_mut_lit().and_then(|l| l.as_mut_str()))
                }) {
                    let specifier = src.value.to_string();
                    self.add_import(specifier, ImportKind::EsModule, n.span);
                }
            }
            swc_ecma_ast::Decl::TsInterface(_) => {}
            swc_ecma_ast::Decl::TsTypeAlias(_) => {}
            swc_ecma_ast::Decl::TsEnum(_) => {}
            swc_ecma_ast::Decl::TsModule(_) => {}
            swc_ecma_ast::Decl::Using(_) => {
                // Using declarations don't contain import specifiers
                // They're for resource management, not module imports
            }
        }
        n.visit_mut_children_with(self);
    }

    fn visit_mut_export_all(&mut self, n: &mut ExportAll) {
        let specifier = n.src.value.to_string();
        self.add_import(specifier, ImportKind::EsModule, n.span);
        n.visit_mut_children_with(self);
    }

    fn visit_mut_call_expr(&mut self, n: &mut CallExpr) {
        if let Some(ident) = n.callee.as_expr().and_then(|e| e.as_ident()) {
            if ident.sym.as_ref() == "require" && n.args.len() == 1 {
                if let Some(lit) = n.args[0].expr.as_lit() {
                    if let Lit::Str(s) = lit {
                        let specifier = s.value.to_string();
                        self.add_import(specifier, ImportKind::CommonJs, n.span);
                    }
                }
            }
        }

        if n.callee.is_import() && n.args.len() == 1 {
            if let Some(lit) = n.args[0].expr.as_lit() {
                if let Lit::Str(s) = lit {
                    let specifier = s.value.to_string();
                    self.add_import(specifier, ImportKind::Dynamic, n.span);
                }
            }
        }

        n.visit_mut_children_with(self);
    }
}
