//! Extracts Moth annotations from JSDoc-style `/** ... */` comment blocks.
//!
//! WHAT: scans raw JS source text, locates multi-line comment blocks that start with
//!       `/**`, and extracts lines that begin with `@moth.`.
//! WHY: `@moth.opaque` and `@moth.sig` annotations live inside these comment blocks.
//!      Keeping extraction separate from signature parsing makes each module easier
//!      to test and reason about.
//!
//! Limitations:
//! - Regular `/* ... */` blocks are ignored.
//! - Inline `/** ... */` on a single line is supported.
//! - `//` comments are ignored even if they contain `@moth.`.

use super::parsed_js_module::{JsDiagnosticKind, JsParserDiagnostic, JsSourceSpan};

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
}

/// Result of scanning a JS file for comment blocks.
pub struct CommentExtractionResult {
    pub annotations: Vec<ExtractedAnnotation>,
    pub diagnostics: Vec<JsParserDiagnostic>,
}

/// Scans source text for `/** ... */` blocks and extracts `@moth.*` annotations.
pub fn extract_annotations(source: &str) -> CommentExtractionResult {
    let mut scanner = CommentScanner::new(source);
    scanner.scan()
}

struct CommentScanner<'a> {
    source: &'a str,
    bytes: &'a [u8],
    pos: usize,
    line: usize,
    column: usize,
    annotations: Vec<ExtractedAnnotation>,
    diagnostics: Vec<JsParserDiagnostic>,
}

impl<'a> CommentScanner<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            source,
            bytes: source.as_bytes(),
            pos: 0,
            line: 1,
            column: 1,
            annotations: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    fn scan(&mut self) -> CommentExtractionResult {
        while !self.is_at_end() {
            if self.peek_str("/**") {
                self.read_doc_comment_block();
            } else if self.peek_str("/*") {
                self.skip_block_comment();
            } else if self.peek_str("//") {
                self.skip_line_comment();
            } else {
                self.advance_char();
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
        let block_start_byte = self.pos;
        let block_start_line = self.line;
        let block_start_column = self.column;

        // Consume `/**`
        self.advance_chars(3);

        let mut block_text = String::new();
        let block_content_start = self.pos;

        while !self.is_at_end() {
            if self.peek_str("*/") {
                self.advance_chars(2);
                break;
            }
            block_text.push(self.current_char());
            self.advance_char();
        }

        let block_span = JsSourceSpan::range(
            block_start_byte,
            self.pos,
            block_start_line,
            block_start_column,
        );

        self.parse_block_content(&block_text, block_content_start, block_span);
    }

    fn parse_block_content(
        &mut self,
        content: &str,
        content_byte_offset: usize,
        block_span: JsSourceSpan,
    ) {
        // Normalize line breaks and strip leading `*` from each line
        let lines: Vec<&str> = content.lines().collect();

        for (line_index, raw_line) in lines.iter().enumerate() {
            let trimmed = raw_line.trim_start();
            let after_star = if let Some(stripped) = trimmed.strip_prefix('*') {
                stripped.trim_start()
            } else {
                trimmed
            };

            if after_star.starts_with("@moth.") {
                self.parse_annotation_line(
                    after_star,
                    content_byte_offset,
                    line_index,
                    block_span.clone(),
                );
            }
        }
    }

    fn parse_annotation_line(
        &mut self,
        line: &str,
        _content_byte_offset: usize,
        _line_index: usize,
        block_span: JsSourceSpan,
    ) {
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
                        "Unknown Moth JS annotation `{unknown}`. Supported annotations are `@moth.opaque` and `@moth.sig`."
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

    // ------------------------
    //  Skip non-doc comments
    // ------------------------

    fn skip_block_comment(&mut self) {
        // Consume `/*`
        self.advance_chars(2);
        while !self.is_at_end() {
            if self.peek_str("*/") {
                self.advance_chars(2);
                break;
            }
            self.advance_char();
        }
    }

    fn skip_line_comment(&mut self) {
        // Consume `//`
        self.advance_chars(2);
        while !self.is_at_end() && self.current_char() != '\n' {
            self.advance_char();
        }
    }

    // ------------------------
    //  Low-level character ops
    // ------------------------

    fn current_char(&self) -> char {
        self.source[self.pos..].chars().next().unwrap_or('\0')
    }

    fn is_at_end(&self) -> bool {
        self.pos >= self.bytes.len()
    }

    fn peek_str(&self, s: &str) -> bool {
        self.source[self.pos..].starts_with(s)
    }

    fn advance_char(&mut self) {
        if self.is_at_end() {
            return;
        }
        let ch = self.current_char();
        self.pos += ch.len_utf8();
        if ch == '\n' {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += 1;
        }
    }

    fn advance_chars(&mut self, count: usize) {
        for _ in 0..count {
            self.advance_char();
        }
    }
}
