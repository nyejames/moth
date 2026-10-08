//! Extracts Moth annotations from JSDoc-style `/** ... */` comment blocks.
//!
//! WHAT: scans raw JS source text, locates multi-line comment blocks that start with
//!       `/**`, and extracts lines that begin with `@moth.`.
//! WHY: `@moth.opaque`, `@moth.sig` and `@moth.const` annotations live inside these blocks.
//!      Keeping extraction separate from signature parsing makes each module easier
//!      to test and reason about.
//!
//! Limitations:
//! - Regular `/* ... */` blocks are ignored.
//! - Inline `/** ... */` on a single line is supported.
//! - `//` comments are ignored even if they contain `@moth.`.

use super::export_scanner::ExportScanner;
use super::parsed_js_module::{JsDiagnosticKind, JsParserDiagnostic, JsSourceSpan};
use crate::projects::html_project::external_js::runtime_module_registry::RuntimeModuleRegistry;
use moth_lexical::identifier::is_identifier;

/// A single `@moth.*` annotation extracted from a comment block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractedAnnotation {
    pub kind: AnnotationKind,
    pub span: JsSourceSpan,
}

/// Classification of extracted annotations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnnotationKind {
    /// `@moth.opaque TypeName`
    Opaque { type_name: String },
    /// `@moth.sig moth_name signature_body`
    Sig {
        moth_name: String,
        signature_text: String,
    },
    /// `@moth.const moth_name U32`
    Const {
        moth_name: String,
        type_name: String,
    },
}

/// Result of scanning a JS file for comment blocks.
pub struct CommentExtractionResult {
    pub annotations: Vec<ExtractedAnnotation>,
    pub diagnostics: Vec<JsParserDiagnostic>,
}

/// Scans source text for `/** ... */` blocks and extracts `@moth.*` annotations.
pub fn extract_annotations(
    source: &str,
    registry: &RuntimeModuleRegistry,
) -> CommentExtractionResult {
    let mut scanner = CommentScanner::new(source, registry);
    scanner.scan()
}

struct CommentScanner<'a> {
    source: &'a str,
    cursor: ExportScanner<'a>,
    annotations: Vec<ExtractedAnnotation>,
    diagnostics: Vec<JsParserDiagnostic>,
}

impl<'a> CommentScanner<'a> {
    fn new(source: &'a str, registry: &'a RuntimeModuleRegistry) -> Self {
        Self {
            source,
            cursor: ExportScanner::new(source, registry),
            annotations: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    fn scan(&mut self) -> CommentExtractionResult {
        while !self.cursor.is_at_end() {
            if self.cursor.peek_str("/**") {
                self.read_doc_comment_block();
            } else if self.cursor.skip_lexical_content_at_current() {
                continue;
            } else if self.cursor.current_char_opt() == Some('/')
                && self.cursor.slash_starts_regular_expression()
            {
                self.cursor.skip_regular_expression();
            } else {
                self.cursor.advance_char();
            }
        }

        CommentExtractionResult {
            annotations: std::mem::take(&mut self.annotations),
            diagnostics: std::mem::take(&mut self.diagnostics),
        }
    }

    // ------------------------
    //  Doc-comment block
    // ------------------------

    fn read_doc_comment_block(&mut self) {
        let block_start_byte = self.cursor.pos;
        self.cursor.advance_chars(3);
        let content_start = self.cursor.pos;

        while !self.cursor.is_at_end() && !self.cursor.peek_str("*/") {
            self.cursor.advance_char();
        }
        let content_end = self.cursor.pos;
        if self.cursor.peek_str("*/") {
            self.cursor.advance_chars(2);
        }

        let block_span = JsSourceSpan::range(block_start_byte, self.cursor.pos);
        self.parse_block_content(&self.source[content_start..content_end], block_span);
    }

    fn parse_block_content(&mut self, content: &str, block_span: JsSourceSpan) {
        // Normalize line breaks and strip leading `*` from each line
        for raw_line in content.lines() {
            let trimmed = raw_line.trim_start();
            let after_star = if let Some(stripped) = trimmed.strip_prefix('*') {
                stripped.trim_start()
            } else {
                trimmed
            };

            if after_star.starts_with("@moth.") {
                self.parse_annotation_line(after_star, block_span.clone());
            }
        }
    }

    fn parse_annotation_line(&mut self, line: &str, block_span: JsSourceSpan) {
        let mut tokens = line.split_whitespace();
        let directive = tokens.next().unwrap_or("");

        match directive {
            "@moth.opaque" => {
                if let Some(type_name) = tokens.next() {
                    if tokens.next() == Some("of") {
                        self.diagnostics.push(JsParserDiagnostic {
                            message: "External package types cannot be generic. Expose concrete opaque external types.".to_string(),
                            span: block_span,
                            kind: JsDiagnosticKind::GenericExternalType,
                        });
                        return;
                    }

                    self.annotations.push(ExtractedAnnotation {
                        kind: AnnotationKind::Opaque {
                            type_name: type_name.to_string(),
                        },
                        span: block_span,
                    });
                }
            }
            "@moth.sig" => {
                // The rest of the line after `@moth.sig` is the Moth name + signature body
                let remainder = line["@moth.sig".len()..].trim_start();
                if let Some((moth_name, signature_text)) = Self::split_sig_name_and_body(remainder)
                {
                    self.annotations.push(ExtractedAnnotation {
                        kind: AnnotationKind::Sig {
                            moth_name: moth_name.to_string(),
                            signature_text: signature_text.to_string(),
                        },
                        span: block_span.clone(),
                    });
                } else {
                    self.diagnostics.push(JsParserDiagnostic {
                        message:
                            "`@moth.sig` must be followed by a Moth name and a signature body."
                                .to_string(),
                        span: block_span,
                        kind: JsDiagnosticKind::UnsupportedTypeSyntax,
                    });
                }
            }
            "@moth.const" => {
                let moth_name = tokens.next();
                let type_name = tokens.next();
                if let (Some(moth_name), Some(type_name), None) =
                    (moth_name, type_name, tokens.next())
                    && is_identifier(moth_name)
                {
                    self.annotations.push(ExtractedAnnotation {
                        kind: AnnotationKind::Const {
                            moth_name: moth_name.to_string(),
                            type_name: type_name.to_string(),
                        },
                        span: block_span,
                    });
                } else {
                    self.diagnostics.push(JsParserDiagnostic {
                        message: "`@moth.const` requires one flat Moth name followed by `U32`."
                            .to_string(),
                        span: block_span,
                        kind: JsDiagnosticKind::InvalidConstant,
                    });
                }
            }
            "@moth.package" => {
                self.diagnostics.push(JsParserDiagnostic {
                    message: "`@moth.package` is not supported in Moth JS module comments."
                        .to_string(),
                    span: block_span,
                    kind: JsDiagnosticKind::UnsupportedPackageTag,
                });
            }
            unknown => {
                self.diagnostics.push(JsParserDiagnostic {
                    message: format!(
                        "Unknown Moth JS annotation `{unknown}`. Supported annotations are `@moth.opaque`, `@moth.sig` and `@moth.const`."
                    ),
                    span: block_span,
                    kind: JsDiagnosticKind::UnknownMothDirective,
                });
            }
        }
    }

    /// Splits a `@moth.sig` remainder into `(moth_name, signature_body)`.
    ///
    /// The Moth name is the first identifier token. Everything after it
    /// (starting with `|`) is the signature body.
    fn split_sig_name_and_body(remainder: &str) -> Option<(&str, &str)> {
        let trimmed = remainder.trim_start();
        let mut end_of_name = 0;

        for (index, ch) in trimmed.char_indices() {
            if ch.is_alphanumeric() || ch == '_' {
                end_of_name = index + ch.len_utf8();
            } else {
                break;
            }
        }

        if end_of_name == 0 {
            return None;
        }

        let name = &trimmed[..end_of_name];
        let body = trimmed[end_of_name..].trim_start();

        Some((name, body))
    }
}
