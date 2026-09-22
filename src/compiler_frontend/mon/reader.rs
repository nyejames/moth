//! Literal-only MON document reader.
//!
//! This module deliberately owns a small caller-borrowed cursor rather than using any of the
//! compiler's token, source, AST or HIR machinery.  Parsing first records the literal shape and
//! byte spans; validation then consumes that shape against an immutable prepared schema and
//! produces the public owned [`Value`] tree.

use crate::compiler_frontend::numeric_text::parse::{
    parse_numeric_literal, parse_numeric_text_to_f64, parse_numeric_text_to_i32,
};
use crate::compiler_frontend::numeric_text::token::NumericLiteralKind;

use super::schema::{PreparedField, PreparedType, PreparedVariant};
use super::{BudgetState, MonError, MonErrorCode, PathSegment, PreparedSchema, Span, Value};

/// Decode one complete MON document against a prepared root record schema.
///
/// The input is already valid UTF-8 by virtue of this API's `&str` boundary.  No intermediate
/// value is returned on failure: the parser's temporary literal tree and the validator's partially
/// built owned tree are both local to this call and are dropped by normal error propagation.
pub fn decode_document(input: &str, schema: &PreparedSchema) -> Result<Value, MonError> {
    let input_end = span_end(input.len());
    if input.len() > schema.limits.max_input_bytes || input.len() > u32::MAX as usize {
        return Err(MonError::new(
            MonErrorCode::InputBudget,
            Some(Span::new(0, input_end)),
            &[],
            format!(
                "MON input exceeds byte budget (limit {})",
                schema.limits.max_input_bytes
            ),
        ));
    }

    let mut parser = Parser::new(input, &schema.limits);
    let raw_root = match parser.parse_document() {
        Ok(root) => root,
        Err(mut error) => {
            error.path = parser.path;
            return Err(error);
        }
    };
    parser.skip_space_and_comments();
    if !parser.at_end() {
        let span = parser.remaining_span();
        return Err(MonError::at(
            MonErrorCode::TrailingInput,
            span,
            "MON document has trailing input after its root record",
        ));
    }

    let mut budget = parser.budget;
    let mut path = Vec::new();
    validate_root(raw_root, &schema.root, &mut budget, &mut path)
}

/// Decode one complete MON document from raw bytes against a prepared root record schema.
///
/// The byte budget is enforced before UTF-8 conversion and parsing. Invalid UTF-8
/// fails with [`MonErrorCode::InvalidUtf8`] and a byte span covering the first
/// invalid sequence; no intermediate value is returned on failure.
pub fn decode_document_bytes(input: &[u8], schema: &PreparedSchema) -> Result<Value, MonError> {
    let input_end = span_end(input.len());
    if input.len() > schema.limits.max_input_bytes || input.len() > u32::MAX as usize {
        return Err(MonError::new(
            MonErrorCode::InputBudget,
            Some(Span::new(0, input_end)),
            &[],
            format!(
                "MON input exceeds byte budget (limit {})",
                schema.limits.max_input_bytes
            ),
        ));
    }
    let text = std::str::from_utf8(input).map_err(|error| {
        let start = error.valid_up_to();
        let end = match error.error_len() {
            Some(len) => start.saturating_add(len),
            None => input.len(),
        };
        MonError::new(
            MonErrorCode::InvalidUtf8,
            Some(Span::new(start, span_end(end))),
            &[],
            "MON input is not valid UTF-8",
        )
    })?;
    decode_document(text, schema)
}

fn span_end(end: usize) -> usize {
    // The input budget is checked before this helper is used for a user-visible range.  MON's
    // documented limits are well below u32::MAX, but retaining a checked conversion keeps the
    // cursor's byte-range contract explicit even for a caller with a custom limit.
    end.min(u32::MAX as usize)
}

fn with_path(mut error: MonError, path: &[PathSegment]) -> MonError {
    error.path = path.to_owned();
    error
}

fn check_depth(
    budget: &BudgetState<'_>,
    depth: usize,
    span: Option<Span>,
    path: &[PathSegment],
) -> Result<(), MonError> {
    budget
        .check_depth(depth, span)
        .map_err(|error| with_path(error, path))
}

fn charge_nodes(
    budget: &mut BudgetState<'_>,
    count: usize,
    span: Option<Span>,
    path: &[PathSegment],
) -> Result<(), MonError> {
    budget
        .charge_nodes(count, span)
        .map_err(|error| with_path(error, path))
}

fn charge_decoded_bytes(
    budget: &mut BudgetState<'_>,
    count: usize,
    span: Option<Span>,
    path: &[PathSegment],
) -> Result<(), MonError> {
    budget
        .charge_decoded_bytes(count, span)
        .map_err(|error| with_path(error, path))
}

fn charge_default(
    budget: &mut BudgetState<'_>,
    span: Option<Span>,
    path: &[PathSegment],
) -> Result<(), MonError> {
    budget
        .charge_default(span)
        .map_err(|error| with_path(error, path))
}

#[derive(Debug)]
struct Parser<'a> {
    input: &'a str,
    pos: usize,
    budget: BudgetState<'a>,
    path: Vec<PathSegment>,
}

impl<'a> Parser<'a> {
    fn new(input: &'a str, limits: &'a super::Limits) -> Self {
        Self {
            input,
            pos: 0,
            budget: BudgetState::new(limits),
            path: Vec::new(),
        }
    }

    fn at_end(&self) -> bool {
        self.pos >= self.input.len()
    }

    fn remaining_span(&self) -> Span {
        Span::new(self.pos, span_end(self.input.len()))
    }

    fn peek(&self) -> Option<char> {
        self.input.get(self.pos..)?.chars().next()
    }

    fn peek_next(&self) -> Option<char> {
        let first = self.peek()?;
        self.input
            .get(self.pos + first.len_utf8()..)?
            .chars()
            .next()
    }

    fn starts_with(&self, text: &str) -> bool {
        self.input
            .get(self.pos..)
            .is_some_and(|remaining| remaining.starts_with(text))
    }

    fn bump(&mut self) -> Option<char> {
        let character = self.peek()?;
        self.pos += character.len_utf8();
        Some(character)
    }

    fn skip_space_and_comments(&mut self) {
        loop {
            while self.peek().is_some_and(char::is_whitespace) {
                self.bump();
            }
            if !self.starts_with("--") {
                return;
            }
            self.bump();
            self.bump();
            while let Some(character) = self.peek() {
                if character == '\r' || character == '\n' {
                    break;
                }
                self.bump();
            }
        }
    }

    fn consume(&mut self, expected: char) -> bool {
        if self.peek() == Some(expected) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn parse_document(&mut self) -> Result<RawValue<'a>, MonError> {
        self.skip_space_and_comments();
        if self.consume('(') {
            let start = self.pos - 1;
            self.budget
                .charge_nodes(1, Some(Span::new(start, start + 1)))?;
            let args = self.parse_argument_list_after_open(')', 1)?;
            let span = Span::new(start, self.pos);
            return Ok(RawValue {
                span,
                kind: RawKind::Record {
                    qualifier: None,
                    args,
                },
            });
        }

        let start = self.pos;
        self.budget.check_depth(0, Some(Span::new(start, start)))?;
        self.budget
            .charge_nodes(1, Some(Span::new(start, span_end(self.input.len()))))?;
        let args = self.parse_root_arguments()?;
        Ok(RawValue {
            span: Span::new(0, span_end(self.input.len())),
            kind: RawKind::Record {
                qualifier: None,
                args,
            },
        })
    }

    fn parse_root_arguments(&mut self) -> Result<Vec<RawArg<'a>>, MonError> {
        let mut args = Vec::new();
        self.skip_space_and_comments();
        if self.at_end() {
            return Ok(args);
        }
        loop {
            let argument = self.parse_argument(1)?;
            if !matches!(&argument, RawArg::Named { .. }) {
                let span = match &argument {
                    RawArg::Positional { span, .. } | RawArg::Named { span, .. } => *span,
                };
                return Err(MonError::at(
                    MonErrorCode::RootNotRecord,
                    span,
                    "MON document root entries must be named fields",
                ));
            }
            args.push(argument);
            self.skip_space_and_comments();
            if self.at_end() {
                return Ok(args);
            }
            if self.consume(',') {
                self.skip_space_and_comments();
                if self.at_end() {
                    return Ok(args);
                }
                continue;
            }
            return Err(MonError::at(
                MonErrorCode::MissingComma,
                self.current_token_span(),
                "MON record entries require commas",
            ));
        }
    }

    fn parse_argument_list_after_open(
        &mut self,
        closing: char,
        depth: usize,
    ) -> Result<Vec<RawArg<'a>>, MonError> {
        let mut args = Vec::new();
        self.skip_space_and_comments();
        if self.consume(closing) {
            return Ok(args);
        }
        loop {
            let argument = self.parse_argument(depth)?;
            args.push(argument);
            self.skip_space_and_comments();
            if self.consume(closing) {
                return Ok(args);
            }
            if self.consume(',') {
                self.skip_space_and_comments();
                if self.consume(closing) {
                    return Ok(args);
                }
                if self.at_end() {
                    return Err(MonError::at(
                        MonErrorCode::UnexpectedEnd,
                        Span::new(self.pos, self.pos),
                        "MON argument list ended after a comma",
                    ));
                }
                continue;
            }
            if self.at_end() {
                return Err(MonError::new(
                    MonErrorCode::UnexpectedEnd,
                    Some(Span::new(self.pos, self.pos)),
                    &[],
                    format!("MON argument list is missing closing '{closing}'"),
                ));
            }
            return Err(MonError::at(
                MonErrorCode::MissingComma,
                self.current_token_span(),
                "MON entries require commas",
            ));
        }
    }

    fn parse_argument(&mut self, depth: usize) -> Result<RawArg<'a>, MonError> {
        self.skip_space_and_comments();
        let start = self.pos;
        if let Some((name, name_span)) = self.try_named_label() {
            self.skip_space_and_comments();
            if !self.consume('=') {
                return Err(MonError::at(
                    MonErrorCode::UnexpectedToken,
                    self.current_token_span(),
                    "MON named entry requires '='",
                ));
            }
            self.budget
                .charge_decoded_bytes(name.len(), Some(name_span))?;
            let base_len = self.path.len();
            self.path.push(PathSegment::Field(name.to_owned()));
            self.skip_space_and_comments();
            if self.at_end() {
                return Err(MonError::new(
                    MonErrorCode::UnexpectedEnd,
                    Some(Span::new(self.pos, self.pos)),
                    &[],
                    "MON named entry is missing its value",
                ));
            }
            let value = {
                let value = self.parse_value(depth)?;
                self.path.truncate(base_len);
                value
            };
            let span = Span::new(start, value.span.end as usize);
            return Ok(RawArg::Named {
                name,
                name_span,
                value,
                span,
            });
        }
        let value = self.parse_value(depth)?;
        let span = value.span;
        Ok(RawArg::Positional { value, span })
    }

    fn try_named_label(&mut self) -> Option<(&'a str, Span)> {
        let checkpoint = self.pos;
        let (name, span) = self.parse_identifier().ok()?;
        self.skip_space_and_comments();
        if self.peek() == Some('=') {
            Some((name, span))
        } else {
            self.pos = checkpoint;
            None
        }
    }

    fn parse_value(&mut self, depth: usize) -> Result<RawValue<'a>, MonError> {
        self.skip_space_and_comments();
        let start = self.pos;
        if self.at_end() {
            return Err(MonError::new(
                MonErrorCode::UnexpectedEnd,
                Some(Span::new(start, start)),
                &[],
                "MON value is missing",
            ));
        }
        self.budget
            .check_depth(depth, Some(Span::new(start, start)))?;
        self.budget.charge_nodes(1, Some(Span::new(start, start)))?;

        match self.peek() {
            Some('(') => self.parse_record_value(start, depth),
            Some('{') => self.parse_container_value(start, depth),
            Some('"') => self.parse_string_value(start),
            Some('\'') => self.parse_char_value(start),
            Some('-')
                if self
                    .peek_next()
                    .is_some_and(|character| character.is_ascii_digit()) =>
            {
                self.parse_number_value(start)
            }
            Some(character) if character.is_ascii_digit() => self.parse_number_value(start),
            Some(':') if self.starts_with("::") => self.parse_choice_value(start, None, depth),
            Some(character) if is_identifier_start(character) => {
                self.parse_identifier_value(start, depth)
            }
            _ => Err(MonError::at(
                MonErrorCode::UnexpectedToken,
                self.current_token_span(),
                "MON value is not a supported literal",
            )),
        }
    }

    fn parse_record_value(&mut self, start: usize, depth: usize) -> Result<RawValue<'a>, MonError> {
        self.bump();
        let args = self.parse_argument_list_after_open(')', depth + 1)?;
        Ok(RawValue {
            span: Span::new(start, self.pos),
            kind: RawKind::Record {
                qualifier: None,
                args,
            },
        })
    }

    fn parse_identifier_value(
        &mut self,
        start: usize,
        depth: usize,
    ) -> Result<RawValue<'a>, MonError> {
        let (name, name_span) = self.parse_identifier()?;
        if self.starts_with("::") {
            self.bump();
            self.bump();
            let (variant, variant_span) = self.parse_identifier()?;
            let args = if self.peek() == Some('(') {
                self.bump();
                self.budget
                    .charge_decoded_bytes(variant.len(), Some(variant_span))?;
                let base_len = self.path.len();
                self.path.push(PathSegment::Variant(variant.to_owned()));
                let parsed = {
                    let parsed = self.parse_argument_list_after_open(')', depth + 1)?;
                    self.path.truncate(base_len);
                    parsed
                };
                Some(parsed)
            } else {
                None
            };
            let end = args
                .as_ref()
                .and_then(|_| self.last_consumed_end())
                .unwrap_or(variant_span.end as usize);
            return Ok(RawValue {
                span: Span::new(start, end),
                kind: RawKind::Choice {
                    qualifier: Some(RawIdentifier {
                        name,
                        span: name_span,
                    }),
                    variant: RawIdentifier {
                        name: variant,
                        span: variant_span,
                    },
                    args,
                },
            });
        }

        if self.peek() == Some('(') {
            self.bump();
            let args = self.parse_argument_list_after_open(')', depth + 1)?;
            return Ok(RawValue {
                span: Span::new(start, self.pos),
                kind: RawKind::Record {
                    qualifier: Some(RawIdentifier {
                        name,
                        span: name_span,
                    }),
                    args,
                },
            });
        }

        if name == "none" {
            return Ok(RawValue {
                span: name_span,
                kind: RawKind::None,
            });
        }
        if name == "true" {
            return Ok(RawValue {
                span: name_span,
                kind: RawKind::Bool(true),
            });
        }
        if name == "false" {
            return Ok(RawValue {
                span: name_span,
                kind: RawKind::Bool(false),
            });
        }

        Err(MonError::at(
            MonErrorCode::UnexpectedToken,
            name_span,
            "bare names are not MON literals",
        ))
    }

    fn parse_choice_value(
        &mut self,
        start: usize,
        qualifier: Option<RawIdentifier<'a>>,
        depth: usize,
    ) -> Result<RawValue<'a>, MonError> {
        self.bump();
        self.bump();
        let (variant, variant_span) = self.parse_identifier()?;
        let args = if self.peek() == Some('(') {
            self.bump();
            self.budget
                .charge_decoded_bytes(variant.len(), Some(variant_span))?;
            let base_len = self.path.len();
            self.path.push(PathSegment::Variant(variant.to_owned()));
            let parsed = {
                let parsed = self.parse_argument_list_after_open(')', depth + 1)?;
                self.path.truncate(base_len);
                parsed
            };
            Some(parsed)
        } else {
            None
        };
        let end = args
            .as_ref()
            .and_then(|_| self.last_consumed_end())
            .unwrap_or(variant_span.end as usize);
        Ok(RawValue {
            span: Span::new(start, end),
            kind: RawKind::Choice {
                qualifier,
                variant: RawIdentifier {
                    name: variant,
                    span: variant_span,
                },
                args,
            },
        })
    }

    fn last_consumed_end(&self) -> Option<usize> {
        if self.pos > 0 { Some(self.pos) } else { None }
    }

    fn parse_container_value(
        &mut self,
        start: usize,
        depth: usize,
    ) -> Result<RawValue<'a>, MonError> {
        self.bump();
        self.skip_space_and_comments();
        if self.consume('}') {
            return Ok(RawValue {
                span: Span::new(start, self.pos),
                kind: RawKind::Collection(Vec::new()),
            });
        }
        if self.consume('=') {
            self.skip_space_and_comments();
            if !self.consume('}') {
                return Err(if self.at_end() {
                    MonError::new(
                        MonErrorCode::UnexpectedEnd,
                        Some(Span::new(self.pos, self.pos)),
                        &[],
                        "MON empty map is missing '}'",
                    )
                } else {
                    MonError::at(
                        MonErrorCode::UnexpectedToken,
                        self.current_token_span(),
                        "MON empty map must be written as {=}",
                    )
                });
            }
            return Ok(RawValue {
                span: Span::new(start, self.pos),
                kind: RawKind::Map(Vec::new()),
            });
        }

        let base_len = self.path.len();
        self.path.push(PathSegment::Index(0));
        let first = {
            let first = self.parse_map_key(depth + 1)?;
            self.path.truncate(base_len);
            first
        };
        self.skip_space_and_comments();
        if self.consume('=') {
            let mut entries = Vec::new();
            self.path.push(PathSegment::Index(0));
            self.skip_space_and_comments();
            if self.at_end() {
                return Err(MonError::new(
                    MonErrorCode::UnexpectedEnd,
                    Some(Span::new(self.pos, self.pos)),
                    &[],
                    "MON map entry is missing its value",
                ));
            }
            let value = {
                let value = self.parse_value(depth + 1)?;
                self.path.truncate(base_len);
                value
            };
            entries.push((first, value));
            self.parse_map_tail(&mut entries, depth)?;
            return Ok(RawValue {
                span: Span::new(start, self.pos),
                kind: RawKind::Map(entries),
            });
        }
        let mut items = vec![first];

        self.parse_collection_tail(&mut items, depth)?;
        Ok(RawValue {
            span: Span::new(start, self.pos),
            kind: RawKind::Collection(items),
        })
    }

    fn parse_collection_tail(
        &mut self,
        items: &mut Vec<RawValue<'a>>,
        depth: usize,
    ) -> Result<(), MonError> {
        loop {
            self.skip_space_and_comments();
            if self.consume('}') {
                return Ok(());
            }
            if !self.consume(',') {
                if self.at_end() {
                    return Err(MonError::new(
                        MonErrorCode::UnexpectedEnd,
                        Some(Span::new(self.pos, self.pos)),
                        &[],
                        "MON collection is missing '}'",
                    ));
                }
                return Err(MonError::at(
                    MonErrorCode::MissingComma,
                    self.current_token_span(),
                    "MON collection items require commas",
                ));
            }
            self.skip_space_and_comments();
            if self.consume('}') {
                return Ok(());
            }
            let index = items.len();
            let base_len = self.path.len();
            self.path.push(PathSegment::Index(index));
            let value = {
                let value = self.parse_value(depth + 1)?;
                self.path.truncate(base_len);
                value
            };
            items.push(value);
        }
    }

    fn parse_map_key(&mut self, depth: usize) -> Result<RawValue<'a>, MonError> {
        let checkpoint = self.pos;
        if let Ok((name, span)) = self.parse_identifier() {
            self.skip_space_and_comments();
            let keyword = matches!(name, "true" | "false" | "none");
            let constructor = matches!(self.peek(), Some('(') | Some(':'));
            if !keyword && !constructor {
                return Err(MonError::at(
                    MonErrorCode::InvalidMapKey,
                    span,
                    "bare names are not valid MON map keys",
                ));
            }
            self.pos = checkpoint;
        }
        self.parse_value(depth)
    }

    fn parse_map_tail(
        &mut self,
        entries: &mut Vec<(RawValue<'a>, RawValue<'a>)>,
        depth: usize,
    ) -> Result<(), MonError> {
        loop {
            self.skip_space_and_comments();
            if self.consume('}') {
                return Ok(());
            }
            if !self.consume(',') {
                if self.at_end() {
                    return Err(MonError::new(
                        MonErrorCode::UnexpectedEnd,
                        Some(Span::new(self.pos, self.pos)),
                        &[],
                        "MON map is missing '}'",
                    ));
                }
                return Err(MonError::at(
                    MonErrorCode::MissingComma,
                    self.current_token_span(),
                    "MON map entries require commas",
                ));
            }
            self.skip_space_and_comments();
            if self.consume('}') {
                return Ok(());
            }
            let index = entries.len();
            let base_len = self.path.len();
            self.path.push(PathSegment::Index(index));
            let key = self.parse_map_key(depth + 1)?;
            self.skip_space_and_comments();
            if !self.consume('=') {
                let error = if self.at_end() {
                    MonError::new(
                        MonErrorCode::UnexpectedEnd,
                        Some(Span::new(self.pos, self.pos)),
                        &[],
                        "MON map entry is missing '='",
                    )
                } else {
                    MonError::at(
                        MonErrorCode::MapKind,
                        self.current_token_span(),
                        "MON map entry requires '=' between key and value",
                    )
                };
                return Err(error);
            }
            self.path.truncate(base_len);
            self.skip_space_and_comments();
            self.path.push(PathSegment::Index(index));
            let value = {
                let value = self.parse_value(depth + 1)?;
                self.path.truncate(base_len);
                value
            };
            entries.push((key, value));
        }
    }

    fn parse_string_value(&mut self, start: usize) -> Result<RawValue<'a>, MonError> {
        let quote = self.bump();
        debug_assert_eq!(quote, Some('"'));
        let mut output = String::new();
        loop {
            let character_start = self.pos;
            let Some(character) = self.bump() else {
                return Err(MonError::new(
                    MonErrorCode::UnexpectedEnd,
                    Some(Span::new(self.pos, self.pos)),
                    &[],
                    "MON string is missing its closing quote",
                ));
            };
            match character {
                '"' => {
                    return Ok(RawValue {
                        span: Span::new(start, self.pos),
                        kind: RawKind::String(output),
                    });
                }
                '\\' => {
                    let escape_start = character_start;
                    let decoded = self.parse_escape('"', escape_start)?;
                    self.budget.charge_decoded_bytes(
                        decoded.len_utf8(),
                        Some(Span::new(escape_start, self.pos)),
                    )?;
                    output.push(decoded);
                }
                _ => {
                    self.budget.charge_decoded_bytes(
                        character.len_utf8(),
                        Some(Span::new(character_start, self.pos)),
                    )?;
                    output.push(character);
                }
            }
        }
    }

    fn parse_char_value(&mut self, start: usize) -> Result<RawValue<'a>, MonError> {
        let quote = self.bump();
        debug_assert_eq!(quote, Some('\''));
        let mut decoded_value = None;
        loop {
            let character_start = self.pos;
            let Some(character) = self.bump() else {
                return Err(MonError::new(
                    MonErrorCode::UnexpectedEnd,
                    Some(Span::new(self.pos, self.pos)),
                    &[],
                    "MON character is missing its closing quote",
                ));
            };
            if character == '\'' {
                let Some(value) = decoded_value else {
                    return Err(MonError::at(
                        MonErrorCode::InvalidCharacter,
                        Span::new(start, self.pos),
                        "MON character must contain exactly one Unicode scalar",
                    ));
                };
                return Ok(RawValue {
                    span: Span::new(start, self.pos),
                    kind: RawKind::Char(value),
                });
            }
            let decoded = if character == '\\' {
                self.parse_escape('\'', character_start)?
            } else {
                character
            };
            self.budget.charge_decoded_bytes(
                decoded.len_utf8(),
                Some(Span::new(character_start, self.pos)),
            )?;
            if decoded_value.replace(decoded).is_some() {
                return Err(MonError::at(
                    MonErrorCode::InvalidCharacter,
                    Span::new(character_start, self.pos),
                    "MON character must contain exactly one Unicode scalar",
                ));
            }
        }
    }

    fn parse_escape(&mut self, quote: char, start: usize) -> Result<char, MonError> {
        let Some(character) = self.bump() else {
            return Err(MonError::at(
                MonErrorCode::InvalidEscape,
                Span::new(start, self.pos),
                "MON escape is missing its character",
            ));
        };
        let simple = match character {
            '\\' => Some('\\'),
            'n' => Some('\n'),
            'r' => Some('\r'),
            't' => Some('\t'),
            c if c == quote => Some(quote),
            _ => None,
        };
        if let Some(decoded) = simple {
            return Ok(decoded);
        }
        if character != 'u' {
            return Err(MonError::at(
                MonErrorCode::InvalidEscape,
                Span::new(start, self.pos),
                "MON escape is not supported",
            ));
        }
        if !self.consume('{') {
            return Err(MonError::at(
                MonErrorCode::InvalidEscape,
                Span::new(start, self.pos),
                "MON Unicode escapes require braces",
            ));
        }
        let digits_start = self.pos;
        let mut value = 0u32;
        let mut count = 0usize;
        loop {
            let Some(next) = self.peek() else {
                return Err(MonError::at(
                    MonErrorCode::InvalidEscape,
                    Span::new(start, self.pos),
                    "MON Unicode escape is missing '}'",
                ));
            };
            if next == '}' {
                if count == 0 {
                    return Err(MonError::at(
                        MonErrorCode::InvalidEscape,
                        Span::new(start, self.pos + 1),
                        "MON Unicode escape requires one to six hex digits",
                    ));
                }
                self.bump();
                break;
            }
            let Some(digit) = next.to_digit(16).filter(|_| next.is_ascii()) else {
                return Err(MonError::at(
                    MonErrorCode::InvalidEscape,
                    Span::new(start, self.pos + next.len_utf8()),
                    "MON Unicode escape contains a non-hex digit",
                ));
            };
            if count == 6 {
                return Err(MonError::at(
                    MonErrorCode::InvalidEscape,
                    Span::new(digits_start, self.pos + next.len_utf8()),
                    "MON Unicode escape contains more than six hex digits",
                ));
            }
            value = value * 16 + digit;
            count += 1;
            self.bump();
        }
        char::from_u32(value).ok_or_else(|| {
            MonError::at(
                MonErrorCode::InvalidCharacter,
                Span::new(start, self.pos),
                "MON Unicode escape is not a Unicode scalar value",
            )
        })
    }

    fn parse_number_value(&mut self, start: usize) -> Result<RawValue<'a>, MonError> {
        if self.peek() == Some('-') {
            self.bump();
        }
        while let Some(character) = self.peek() {
            if character == '-' && self.starts_with("--") {
                break;
            }
            if is_number_delimiter(character) {
                break;
            }
            self.bump();
        }
        let text = self.input.get(start..self.pos).ok_or_else(|| {
            MonError::new(
                MonErrorCode::InternalInvariant,
                Some(Span::new(start, self.pos)),
                &[],
                "MON numeric cursor range was invalid",
            )
        })?;
        let rough_digits = text.bytes().filter(u8::is_ascii_digit).count();
        if rough_digits > self.budget.limits.max_numeric_digits {
            return Err(MonError::at(
                MonErrorCode::NumericBudget,
                Span::new(start, self.pos),
                format!(
                    "MON numeric digit budget exceeded (limit {})",
                    self.budget.limits.max_numeric_digits
                ),
            ));
        }
        let unsigned = text.strip_prefix('-').unwrap_or(text);
        let parsed = parse_numeric_literal(unsigned).map_err(|reason| {
            MonError::at(
                MonErrorCode::NumericSyntax,
                Span::new(start, self.pos),
                format!("invalid MON numeric literal ({reason:?})"),
            )
        })?;
        let digit_count = usize::try_from(parsed.digit_count).map_err(|_| {
            MonError::at(
                MonErrorCode::NumericBudget,
                Span::new(start, self.pos),
                "MON numeric digit count does not fit the host counter",
            )
        })?;
        if digit_count > self.budget.limits.max_numeric_digits {
            return Err(MonError::at(
                MonErrorCode::NumericBudget,
                Span::new(start, self.pos),
                format!(
                    "MON numeric digit budget exceeded (limit {})",
                    self.budget.limits.max_numeric_digits
                ),
            ));
        }
        Ok(RawValue {
            span: Span::new(start, self.pos),
            kind: RawKind::Number(NumericRaw {
                text,
                normalized: parsed.normalized_text,
                kind: parsed.kind,
                digit_count,
            }),
        })
    }

    fn parse_identifier(&mut self) -> Result<(&'a str, Span), MonError> {
        let start = self.pos;
        let Some(first) = self.peek() else {
            return Err(MonError::new(
                MonErrorCode::UnexpectedEnd,
                Some(Span::new(start, start)),
                &[],
                "MON identifier is missing",
            ));
        };
        if !is_identifier_start(first) {
            return Err(MonError::at(
                MonErrorCode::InvalidIdentifier,
                self.current_token_span(),
                "MON identifier has an invalid start",
            ));
        }
        self.bump();
        while self.peek().is_some_and(is_identifier_continue) {
            self.bump();
        }
        let text = self.input.get(start..self.pos).ok_or_else(|| {
            MonError::new(
                MonErrorCode::InternalInvariant,
                Some(Span::new(start, self.pos)),
                &[],
                "MON identifier cursor range was invalid",
            )
        })?;
        Ok((text, Span::new(start, self.pos)))
    }

    fn current_token_span(&self) -> Span {
        let end = self
            .peek()
            .map_or(self.pos, |character| self.pos + character.len_utf8());
        Span::new(self.pos, end)
    }
}

#[derive(Debug)]
struct RawIdentifier<'a> {
    name: &'a str,
    span: Span,
}

#[derive(Debug)]
struct NumericRaw<'a> {
    text: &'a str,
    normalized: String,
    kind: NumericLiteralKind,
    digit_count: usize,
}

#[derive(Debug)]
struct RawValue<'a> {
    span: Span,
    kind: RawKind<'a>,
}

#[derive(Debug)]
enum RawKind<'a> {
    None,
    Bool(bool),
    Char(char),
    String(String),
    Number(NumericRaw<'a>),
    Record {
        qualifier: Option<RawIdentifier<'a>>,
        args: Vec<RawArg<'a>>,
    },
    Collection(Vec<RawValue<'a>>),
    Map(Vec<(RawValue<'a>, RawValue<'a>)>),
    Choice {
        qualifier: Option<RawIdentifier<'a>>,
        variant: RawIdentifier<'a>,
        args: Option<Vec<RawArg<'a>>>,
    },
}
#[derive(Debug)]
enum RawArg<'a> {
    Positional {
        value: RawValue<'a>,
        span: Span,
    },
    Named {
        name: &'a str,
        name_span: Span,
        value: RawValue<'a>,
        span: Span,
    },
}

fn is_identifier_start(character: char) -> bool {
    character == '_' || character.is_alphabetic()
}

fn is_identifier_continue(character: char) -> bool {
    character == '_' || character.is_alphanumeric()
}

fn is_number_delimiter(character: char) -> bool {
    character.is_whitespace() || matches!(character, ',' | ')' | '}' | '{' | '=' | '(')
}

fn validate_root(
    raw: RawValue<'_>,
    ty: &PreparedType,
    budget: &mut BudgetState<'_>,
    path: &mut Vec<PathSegment>,
) -> Result<Value, MonError> {
    let RawKind::Record { qualifier, args } = raw.kind else {
        return Err(MonError::new(
            MonErrorCode::RootNotRecord,
            Some(raw.span),
            path,
            "MON document root must be a record",
        ));
    };
    if qualifier.is_some() {
        return Err(MonError::new(
            MonErrorCode::QualifierMismatch,
            Some(raw.span),
            path,
            "MON document root cannot carry a nominal qualifier",
        ));
    }
    let root = RawValue {
        span: raw.span,
        kind: RawKind::Record {
            qualifier: None,
            args,
        },
    };
    match ty {
        PreparedType::Record { .. } | PreparedType::Struct { .. } => {
            validate_value(root, ty, budget, path, 0)
        }
        _ => Err(MonError::new(
            MonErrorCode::RootNotRecord,
            Some(root.span),
            path,
            "prepared MON root schema is not a record",
        )),
    }
}

fn validate_value(
    raw: RawValue<'_>,
    ty: &PreparedType,
    budget: &mut BudgetState<'_>,
    path: &mut Vec<PathSegment>,
    depth: usize,
) -> Result<Value, MonError> {
    check_depth(budget, depth, Some(raw.span), path)?;
    match ty {
        PreparedType::None => match raw.kind {
            RawKind::None => Ok(Value::None),
            _ => type_mismatch(raw.span, path, "expected none"),
        },
        PreparedType::Bool => match raw.kind {
            RawKind::Bool(value) => Ok(Value::Bool(value)),
            _ => type_mismatch(raw.span, path, "expected bool"),
        },
        PreparedType::Char => match raw.kind {
            RawKind::Char(value) => Ok(Value::Char(value)),
            _ => type_mismatch(raw.span, path, "expected char"),
        },
        PreparedType::String => match raw.kind {
            RawKind::String(value) => Ok(Value::String(value)),
            _ => type_mismatch(raw.span, path, "expected string"),
        },
        PreparedType::Int => validate_int(raw, budget, path),
        PreparedType::Float => validate_float(raw, budget, path),
        PreparedType::Integer => validate_integer(raw, budget, path),
        PreparedType::Decimal { scale } => validate_decimal(raw, *scale, budget, path),
        PreparedType::Optional(inner) => {
            if matches!(raw.kind, RawKind::None) {
                Ok(Value::None)
            } else {
                validate_value(raw, inner, budget, path, depth)
            }
        }
        PreparedType::Record { fields } => validate_record(raw, None, fields, budget, path, depth),
        PreparedType::Struct { name, fields } => {
            validate_record(raw, Some(name), fields, budget, path, depth)
        }
        PreparedType::Collection { element } => {
            let RawValue { span, kind } = raw;
            let RawKind::Collection(items) = kind else {
                return if matches!(kind, RawKind::Map(_)) {
                    Err(MonError::new(
                        MonErrorCode::MapKind,
                        Some(span),
                        path,
                        "map literal cannot be used for a collection schema",
                    ))
                } else {
                    type_mismatch(span, path, "expected collection")
                };
            };
            let mut output = Vec::new();
            for (index, item) in items.into_iter().enumerate() {
                path.push(PathSegment::Index(index));
                let value = validate_value(item, element, budget, path, depth + 1)?;
                path.pop();
                output.push(value);
            }
            Ok(Value::Collection(output))
        }
        PreparedType::Map { key, value } => {
            let RawValue { span, kind } = raw;
            let RawKind::Map(entries) = kind else {
                return if matches!(kind, RawKind::Collection(_)) {
                    Err(MonError::new(
                        MonErrorCode::MapKind,
                        Some(span),
                        path,
                        "collection literal cannot be used for a map schema",
                    ))
                } else {
                    type_mismatch(span, path, "expected map")
                };
            };
            validate_map(entries, key, value, budget, path, depth)
        }
        PreparedType::Choice { name, variants } => {
            validate_choice(raw, name, variants, budget, path, depth)
        }
    }
}

fn type_mismatch<T>(span: Span, path: &[PathSegment], detail: &str) -> Result<T, MonError> {
    Err(MonError::new(
        MonErrorCode::TypeMismatch,
        Some(span),
        path,
        detail,
    ))
}

fn validate_int(
    raw: RawValue<'_>,
    _budget: &mut BudgetState<'_>,
    path: &[PathSegment],
) -> Result<Value, MonError> {
    let span = raw.span;
    let RawKind::Number(number) = raw.kind else {
        return type_mismatch(span, path, "expected Int numeric literal");
    };
    if number.kind != NumericLiteralKind::WholeNumber {
        return Err(MonError::new(
            MonErrorCode::NumericType,
            Some(span),
            path,
            "Int requires whole-number spelling",
        ));
    }
    parse_numeric_text_to_i32(number.text)
        .map(Value::Int)
        .map_err(|reason| {
            let code = match reason {
                crate::compiler_frontend::compiler_messages::NumberLiteralErrorReason::OutsideIntRange => {
                    MonErrorCode::NumericRange
                }
                _ => MonErrorCode::NumericSyntax,
            };
            MonError::new(
                code,
                Some(span),
                path,
                format!("cannot materialise Int ({reason:?})"),
            )
        })
}

fn validate_float(
    raw: RawValue<'_>,
    _budget: &mut BudgetState<'_>,
    path: &[PathSegment],
) -> Result<Value, MonError> {
    let span = raw.span;
    let RawKind::Number(number) = raw.kind else {
        return type_mismatch(span, path, "expected Float numeric literal");
    };
    parse_numeric_text_to_f64(number.text)
        .map(Value::Float)
        .map_err(|reason| {
            let code = match reason {
                crate::compiler_frontend::compiler_messages::NumberLiteralErrorReason::NonFiniteFloat => {
                    MonErrorCode::NonFiniteFloat
                }
                crate::compiler_frontend::compiler_messages::NumberLiteralErrorReason::ParseOverflow => {
                    MonErrorCode::NonFiniteFloat
                }
                _ => MonErrorCode::NumericSyntax,
            };
            MonError::new(
                code,
                Some(span),
                path,
                format!("cannot materialise Float ({reason:?})"),
            )
        })
}

fn validate_integer(
    raw: RawValue<'_>,
    budget: &mut BudgetState<'_>,
    path: &[PathSegment],
) -> Result<Value, MonError> {
    let span = raw.span;
    let RawKind::Number(number) = raw.kind else {
        return type_mismatch(span, path, "expected exact Integer numeric literal");
    };
    if number.kind != NumericLiteralKind::WholeNumber {
        return Err(MonError::new(
            MonErrorCode::NumericType,
            Some(span),
            path,
            "Integer requires whole-number spelling",
        ));
    }
    let owned_len = signed_normalized_len(&number);
    charge_decoded_bytes(budget, owned_len, Some(span), path)?;
    Ok(Value::Integer(signed_normalized(&number)))
}

fn validate_decimal(
    raw: RawValue<'_>,
    scale: u8,
    budget: &mut BudgetState<'_>,
    path: &[PathSegment],
) -> Result<Value, MonError> {
    let span = raw.span;
    let RawKind::Number(number) = raw.kind else {
        return type_mismatch(span, path, "expected exact Decimal numeric literal");
    };
    if effective_decimal_scale(&number).is_none_or(|effective| effective > scale as usize) {
        return Err(MonError::new(
            MonErrorCode::NumericScale,
            Some(span),
            path,
            format!("Decimal literal exceeds declared scale {scale}"),
        ));
    }
    let owned_len = signed_normalized_len(&number);
    charge_decoded_bytes(budget, owned_len, Some(span), path)?;
    Ok(Value::Decimal(signed_normalized(&number)))
}

fn signed_normalized_len(number: &NumericRaw<'_>) -> usize {
    number.normalized.len() + usize::from(number.text.starts_with('-'))
}

fn signed_normalized(number: &NumericRaw<'_>) -> String {
    if number.text.starts_with('-') {
        let mut text = String::with_capacity(number.normalized.len() + 1);
        text.push('-');
        text.push_str(&number.normalized);
        text
    } else {
        number.normalized.clone()
    }
}

fn effective_decimal_scale(number: &NumericRaw<'_>) -> Option<usize> {
    let (mantissa, exponent) = number.normalized.split_once('e').map_or(
        (number.normalized.as_str(), None),
        |(mantissa, exponent)| (mantissa, Some(exponent)),
    );
    let fractional = mantissa
        .find('.')
        .map_or(0, |point| mantissa.len().saturating_sub(point + 1));
    let digits = mantissa
        .bytes()
        .filter(u8::is_ascii_digit)
        .collect::<Vec<_>>();
    if digits.iter().all(|digit| *digit == b'0') {
        return Some(0);
    }
    let trailing_zeroes = digits
        .iter()
        .rev()
        .take_while(|digit| **digit == b'0')
        .count();
    let mut exponent_negative = false;
    let mut exponent_magnitude = 0usize;
    if let Some(exponent) = exponent {
        let digits = if let Some(rest) = exponent.strip_prefix('-') {
            exponent_negative = true;
            rest
        } else {
            exponent.strip_prefix('+').unwrap_or(exponent)
        };
        // Only the range around the declared scale and source coefficient matters.  Saturating
        // here avoids a second arbitrary-precision arithmetic implementation while still making
        // very large negative exponents fail the exact-scale check deterministically.
        let cap = fractional
            .saturating_add(number.digit_count)
            .saturating_add(19);
        for digit in digits.bytes() {
            let digit = usize::from(digit - b'0');
            exponent_magnitude = exponent_magnitude
                .saturating_mul(10)
                .saturating_add(digit)
                .min(cap.saturating_add(1));
        }
    }
    let base_scale = if exponent_negative {
        fractional.saturating_add(exponent_magnitude)
    } else {
        fractional.saturating_sub(exponent_magnitude)
    };
    Some(base_scale.saturating_sub(trailing_zeroes.min(base_scale)))
}

fn validate_record(
    raw: RawValue<'_>,
    expected_qualifier: Option<&str>,
    fields: &[PreparedField],
    budget: &mut BudgetState<'_>,
    path: &mut Vec<PathSegment>,
    depth: usize,
) -> Result<Value, MonError> {
    let RawValue { span, kind } = raw;
    let RawKind::Record { qualifier, args } = kind else {
        return type_mismatch(span, path, "expected record literal");
    };
    match (expected_qualifier, qualifier.as_ref()) {
        (None, Some(_)) => {
            return Err(MonError::new(
                MonErrorCode::QualifierMismatch,
                Some(span),
                path,
                "anonymous record schema does not accept a nominal qualifier",
            ));
        }
        (Some(expected), Some(actual)) if expected != actual.name => {
            return Err(MonError::new(
                MonErrorCode::QualifierMismatch,
                Some(actual.span),
                path,
                format!("record qualifier must be '{expected}'"),
            ));
        }
        _ => {}
    }

    let allow_positional = expected_qualifier.is_some() && qualifier.is_some();
    let mut supplied: Vec<Option<Value>> = (0..fields.len()).map(|_| None).collect();
    let mut positional_index = 0usize;
    let mut named_started = false;
    for argument in args {
        match argument {
            RawArg::Positional { value, span } => {
                if !allow_positional || named_started {
                    return Err(MonError::new(
                        MonErrorCode::ArgumentOrder,
                        Some(span),
                        path,
                        "record positional arguments must precede named arguments and require a qualifier",
                    ));
                }
                if positional_index >= fields.len() {
                    return Err(MonError::new(
                        MonErrorCode::Arity,
                        Some(span),
                        path,
                        "too many positional record arguments",
                    ));
                }
                let index = positional_index;
                charge_decoded_bytes(budget, fields[index].name.len(), Some(span), path)?;
                path.push(PathSegment::Field(fields[index].name.clone()));
                let converted = validate_value(value, &fields[index].ty, budget, path, depth + 1)?;
                path.pop();
                supplied[index] = Some(converted);
                positional_index += 1;
            }
            RawArg::Named {
                name,
                name_span,
                value,
                ..
            } => {
                named_started = true;
                let Some(index) = fields.iter().position(|field| field.name == name) else {
                    charge_decoded_bytes(budget, name.len(), Some(name_span), path)?;
                    let mut field_path = path.to_owned();
                    field_path.push(PathSegment::Field(name.to_owned()));
                    return Err(MonError::new(
                        MonErrorCode::UnknownField,
                        Some(name_span),
                        &field_path,
                        "MON record contains an unknown field",
                    ));
                };
                if supplied[index].is_some() {
                    charge_decoded_bytes(budget, name.len(), Some(name_span), path)?;
                    let mut field_path = path.to_owned();
                    field_path.push(PathSegment::Field(name.to_owned()));
                    return Err(MonError::new(
                        MonErrorCode::DuplicateField,
                        Some(name_span),
                        &field_path,
                        "MON record field is supplied more than once",
                    ));
                }
                charge_decoded_bytes(budget, name.len(), Some(name_span), path)?;
                path.push(PathSegment::Field(name.to_owned()));
                let converted = validate_value(value, &fields[index].ty, budget, path, depth + 1)?;
                path.pop();
                supplied[index] = Some(converted);
            }
        }
    }

    let mut output = Vec::new();
    for (index, field) in fields.iter().enumerate() {
        let value = if let Some(value) = supplied[index].take() {
            value
        } else if let Some(default) = &field.default {
            charge_default(budget, Some(span), path)?;
            charge_decoded_bytes(budget, field.name.len(), Some(span), path)?;
            path.push(PathSegment::Field(field.name.clone()));
            let value = clone_default(default, budget, path, depth + 1, Some(span))?;
            path.pop();
            value
        } else {
            charge_decoded_bytes(budget, field.name.len(), Some(span), path)?;
            let mut field_path = path.to_owned();
            field_path.push(PathSegment::Field(field.name.clone()));
            return Err(MonError::new(
                MonErrorCode::MissingField,
                Some(span),
                &field_path,
                "MON record is missing a required field",
            ));
        };
        charge_decoded_bytes(budget, field.name.len(), Some(span), path)?;
        output.push((field.name.clone(), value));
    }
    Ok(Value::Record(output))
}

fn validate_map(
    entries: Vec<(RawValue<'_>, RawValue<'_>)>,
    key_type: &PreparedType,
    value_type: &PreparedType,
    budget: &mut BudgetState<'_>,
    path: &mut Vec<PathSegment>,
    depth: usize,
) -> Result<Value, MonError> {
    let mut output: Vec<(Value, Value)> = Vec::new();
    for (index, (raw_key, raw_value)) in entries.into_iter().enumerate() {
        let key_span = raw_key.span;
        let base_path_len = path.len();
        path.push(PathSegment::Index(index));
        let key = validate_value(raw_key, key_type, budget, path, depth + 1)?;
        path.truncate(base_path_len);
        if !is_supported_map_key(&key) {
            let span = value_span(&key);
            path.push(PathSegment::Index(index));
            return Err(MonError::new(
                MonErrorCode::InvalidMapKey,
                Some(span),
                path,
                "MON map key type is not supported",
            ));
        }
        let key_name_len = map_key_name_len(&key);
        charge_decoded_bytes(budget, key_name_len, Some(key_span), path)?;
        let key_name = map_key_name(&key);
        if output.iter().any(|(existing, _)| existing == &key) {
            let duplicate_span = key_span;
            path.push(PathSegment::MapKey(key_name));
            return Err(MonError::new(
                MonErrorCode::DuplicateMapKey,
                Some(duplicate_span),
                path,
                "MON map contains a duplicate decoded key",
            ));
        }
        path.push(PathSegment::MapKey(key_name));
        let value = validate_value(raw_value, value_type, budget, path, depth + 1)?;
        path.truncate(base_path_len);
        output.push((key, value));
    }
    Ok(Value::Map(output))
}

fn is_supported_map_key(value: &Value) -> bool {
    matches!(
        value,
        Value::String(_) | Value::Int(_) | Value::Bool(_) | Value::Char(_)
    )
}

fn map_key_name(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Int(number) => number.to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Char(value) => value.to_string(),
        _ => String::new(),
    }
}

fn map_key_name_len(value: &Value) -> usize {
    match value {
        Value::String(text) => text.len(),
        Value::Int(number) => {
            let mut magnitude = number.unsigned_abs();
            let mut digits = 1;
            while magnitude >= 10 {
                magnitude /= 10;
                digits += 1;
            }
            digits + usize::from(*number < 0)
        }
        Value::Bool(value) => {
            if *value {
                4
            } else {
                5
            }
        }
        Value::Char(value) => value.len_utf8(),
        _ => 0,
    }
}

fn value_span(_value: &Value) -> Span {
    // Owned values intentionally carry no source span.  Callers only reach this path if a
    // malformed prepared map-key schema bypassed schema preparation, so use an empty local range
    // rather than inventing a source location.
    Span::new(0, 0)
}

fn owned_choice_header(
    qualifier: Option<&RawIdentifier<'_>>,
    variant: &RawIdentifier<'_>,
    budget: &mut BudgetState<'_>,
    path: &[PathSegment],
) -> Result<(Option<String>, String), MonError> {
    let qualifier = if let Some(qualifier) = qualifier {
        charge_decoded_bytes(budget, qualifier.name.len(), Some(qualifier.span), path)?;
        Some(qualifier.name.to_owned())
    } else {
        None
    };
    charge_decoded_bytes(budget, variant.name.len(), Some(variant.span), path)?;
    Ok((qualifier, variant.name.to_owned()))
}

fn validate_choice(
    raw: RawValue<'_>,
    expected_name: &str,
    variants: &[PreparedVariant],
    budget: &mut BudgetState<'_>,
    path: &mut Vec<PathSegment>,
    depth: usize,
) -> Result<Value, MonError> {
    let RawValue { span, kind } = raw;
    let RawKind::Choice {
        qualifier,
        variant,
        args,
    } = kind
    else {
        return type_mismatch(span, path, "expected choice variant literal");
    };
    if let Some(actual) = qualifier.as_ref()
        && actual.name != expected_name
    {
        return Err(MonError::new(
            MonErrorCode::QualifierMismatch,
            Some(actual.span),
            path,
            format!("choice qualifier must be '{expected_name}'"),
        ));
    }
    let Some(expected_variant) = variants
        .iter()
        .find(|candidate| candidate.name == variant.name)
    else {
        charge_decoded_bytes(budget, variant.name.len(), Some(variant.span), path)?;
        let mut variant_path = path.to_owned();
        variant_path.push(PathSegment::Variant(variant.name.to_owned()));
        return Err(MonError::new(
            MonErrorCode::UnknownVariant,
            Some(variant.span),
            &variant_path,
            "MON choice contains an unknown variant",
        ));
    };

    if expected_variant.fields.is_empty() {
        if args.is_some() {
            return Err(MonError::new(
                MonErrorCode::Arity,
                Some(span),
                path,
                "unit MON variant must not have a payload",
            ));
        }
        let (qualifier, variant_name) =
            owned_choice_header(qualifier.as_ref(), &variant, budget, path)?;
        return Ok(Value::Choice {
            qualifier,
            variant: variant_name,
            fields: Vec::new(),
        });
    }
    let Some(args) = args else {
        return Err(MonError::new(
            MonErrorCode::Arity,
            Some(span),
            path,
            "payload MON variant requires parentheses",
        ));
    };

    let mut supplied: Vec<Option<Value>> =
        (0..expected_variant.fields.len()).map(|_| None).collect();
    let mut positional_index = 0usize;
    let mut named_started = false;
    for argument in args {
        match argument {
            RawArg::Positional { value, span } => {
                if named_started {
                    return Err(MonError::new(
                        MonErrorCode::ArgumentOrder,
                        Some(span),
                        path,
                        "choice positional arguments must precede named arguments",
                    ));
                }
                if positional_index >= expected_variant.fields.len() {
                    return Err(MonError::new(
                        MonErrorCode::Arity,
                        Some(span),
                        path,
                        "too many positional choice arguments",
                    ));
                }
                charge_decoded_bytes(budget, variant.name.len(), Some(variant.span), path)?;
                charge_decoded_bytes(
                    budget,
                    expected_variant.fields[positional_index].name.len(),
                    Some(span),
                    path,
                )?;
                path.push(PathSegment::Variant(variant.name.to_owned()));
                path.push(PathSegment::Field(
                    expected_variant.fields[positional_index].name.clone(),
                ));
                let converted = validate_value(
                    value,
                    &expected_variant.fields[positional_index].ty,
                    budget,
                    path,
                    depth + 1,
                )?;
                path.truncate(path.len().saturating_sub(2));
                supplied[positional_index] = Some(converted);
                positional_index += 1;
            }
            RawArg::Named {
                name,
                name_span,
                value,
                ..
            } => {
                named_started = true;
                let Some(index) = expected_variant
                    .fields
                    .iter()
                    .position(|field| field.name == name)
                else {
                    charge_decoded_bytes(budget, variant.name.len(), Some(variant.span), path)?;
                    charge_decoded_bytes(budget, name.len(), Some(name_span), path)?;
                    let mut argument_path = path.to_owned();
                    argument_path.push(PathSegment::Variant(variant.name.to_owned()));
                    argument_path.push(PathSegment::Field(name.to_owned()));
                    return Err(MonError::new(
                        MonErrorCode::UnknownArgument,
                        Some(name_span),
                        &argument_path,
                        "MON choice payload contains an unknown field",
                    ));
                };
                if supplied[index].is_some() {
                    charge_decoded_bytes(budget, variant.name.len(), Some(variant.span), path)?;
                    charge_decoded_bytes(budget, name.len(), Some(name_span), path)?;
                    let mut argument_path = path.to_owned();
                    argument_path.push(PathSegment::Variant(variant.name.to_owned()));
                    argument_path.push(PathSegment::Field(name.to_owned()));
                    return Err(MonError::new(
                        MonErrorCode::DuplicateArgument,
                        Some(name_span),
                        &argument_path,
                        "MON choice payload field is supplied more than once",
                    ));
                }
                charge_decoded_bytes(budget, variant.name.len(), Some(variant.span), path)?;
                charge_decoded_bytes(budget, name.len(), Some(name_span), path)?;
                path.push(PathSegment::Variant(variant.name.to_owned()));
                path.push(PathSegment::Field(name.to_owned()));
                let converted = validate_value(
                    value,
                    &expected_variant.fields[index].ty,
                    budget,
                    path,
                    depth + 1,
                )?;
                path.truncate(path.len().saturating_sub(2));
                supplied[index] = Some(converted);
            }
        }
    }

    let (qualifier, variant_name) =
        owned_choice_header(qualifier.as_ref(), &variant, budget, path)?;
    let mut fields = Vec::new();
    for (index, field) in expected_variant.fields.iter().enumerate() {
        let Some(value) = supplied[index].take() else {
            charge_decoded_bytes(budget, variant.name.len(), Some(variant.span), path)?;
            charge_decoded_bytes(budget, field.name.len(), Some(span), path)?;
            let mut missing_path = path.to_owned();
            missing_path.push(PathSegment::Variant(variant.name.to_owned()));
            missing_path.push(PathSegment::Field(field.name.clone()));
            return Err(MonError::new(
                MonErrorCode::Arity,
                Some(span),
                &missing_path,
                "MON choice payload is missing a required field",
            ));
        };
        charge_decoded_bytes(budget, field.name.len(), Some(span), path)?;
        fields.push((field.name.clone(), value));
    }
    Ok(Value::Choice {
        qualifier,
        variant: variant_name,
        fields,
    })
}

fn clone_default(
    value: &Value,
    budget: &mut BudgetState<'_>,
    path: &mut Vec<PathSegment>,
    depth: usize,
    span: Option<Span>,
) -> Result<Value, MonError> {
    check_depth(budget, depth, span, path)?;
    charge_nodes(budget, 1, span, path)?;
    match value {
        Value::None => Ok(Value::None),
        Value::Bool(value) => Ok(Value::Bool(*value)),
        Value::Char(value) => {
            charge_decoded_bytes(budget, value.len_utf8(), span, path)?;
            Ok(Value::Char(*value))
        }
        Value::String(value) => {
            charge_decoded_bytes(budget, value.len(), span, path)?;
            Ok(Value::String(value.clone()))
        }
        Value::Int(value) => Ok(Value::Int(*value)),
        Value::Float(value) => Ok(Value::Float(*value)),
        Value::Integer(value) => {
            charge_decoded_bytes(budget, value.len(), span, path)?;
            Ok(Value::Integer(value.clone()))
        }
        Value::Decimal(value) => {
            charge_decoded_bytes(budget, value.len(), span, path)?;
            Ok(Value::Decimal(value.clone()))
        }
        Value::Record(fields) => {
            let mut output = Vec::new();
            for (name, value) in fields {
                charge_decoded_bytes(budget, name.len(), span, path)?;
                path.push(PathSegment::Field(name.clone()));
                let copied = clone_default(value, budget, path, depth + 1, span)?;
                path.pop();
                charge_decoded_bytes(budget, name.len(), span, path)?;
                output.push((name.clone(), copied));
            }
            Ok(Value::Record(output))
        }
        Value::Collection(values) => {
            let mut output = Vec::new();
            for (index, value) in values.iter().enumerate() {
                path.push(PathSegment::Index(index));
                let copied = clone_default(value, budget, path, depth + 1, span)?;
                path.pop();
                output.push(copied);
            }
            Ok(Value::Collection(output))
        }
        Value::Map(entries) => {
            let mut output = Vec::new();
            for (index, (key, value)) in entries.iter().enumerate() {
                path.push(PathSegment::Index(index));
                let copied_key = clone_default(key, budget, path, depth + 1, span)?;
                let copied_value = clone_default(value, budget, path, depth + 1, span)?;
                path.pop();
                output.push((copied_key, copied_value));
            }
            Ok(Value::Map(output))
        }
        Value::Choice {
            qualifier,
            variant,
            fields,
        } => {
            let qualifier = match qualifier {
                Some(value) => {
                    charge_decoded_bytes(budget, value.len(), span, path)?;
                    Some(value.clone())
                }
                None => None,
            };
            charge_decoded_bytes(budget, variant.len(), span, path)?;
            let variant = variant.clone();
            charge_decoded_bytes(budget, variant.len(), span, path)?;
            path.push(PathSegment::Variant(variant.clone()));
            let mut output = Vec::new();
            for (name, value) in fields {
                charge_decoded_bytes(budget, name.len(), span, path)?;
                path.push(PathSegment::Field(name.clone()));
                let copied = clone_default(value, budget, path, depth + 1, span)?;
                path.pop();
                charge_decoded_bytes(budget, name.len(), span, path)?;
                output.push((name.clone(), copied));
            }
            path.pop();
            Ok(Value::Choice {
                qualifier,
                variant,
                fields: output,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler_frontend::mon::{Field, Limits, Schema, SchemaType, Variant};

    fn schema(fields: Vec<Field>) -> PreparedSchema {
        Schema::record(fields)
            .with_limits(Limits::default())
            .prepare()
            .expect("test schema prepares")
    }

    #[test]
    fn decodes_implicit_and_explicit_roots() {
        let root_schema = schema(vec![
            Field::required("title", SchemaType::String),
            Field::required("count", SchemaType::Int),
        ]);
        let expected = Value::Record(vec![
            ("title".into(), Value::String("x".into())),
            ("count".into(), Value::Int(2)),
        ]);
        assert_eq!(
            decode_document("title = \"x\", count = 2", &root_schema),
            Ok(expected.clone())
        );
        assert_eq!(
            decode_document("(count = 2, title = \"x\",)", &root_schema),
            Ok(expected)
        );
        let empty_schema = schema(Vec::new());
        assert_eq!(
            decode_document("", &empty_schema),
            Ok(Value::Record(Vec::new()))
        );
        assert_eq!(
            decode_document("-- comment only\n", &empty_schema),
            Ok(Value::Record(Vec::new()))
        );
        assert_eq!(
            decode_document("title = \"x\" (count = 2)", &root_schema)
                .unwrap_err()
                .code,
            MonErrorCode::MissingComma
        );
    }
    #[test]
    fn rejects_nonliteral_names_and_missing_commas() {
        let schema = schema(vec![Field::required("value", SchemaType::Int)]);
        assert_eq!(
            decode_document("value = other", &schema).unwrap_err().code,
            MonErrorCode::UnexpectedToken
        );
        assert_eq!(
            decode_document("value = 1 value2 = 2", &schema)
                .unwrap_err()
                .code,
            MonErrorCode::MissingComma
        );
    }

    #[test]
    fn decodes_exact_integer_and_unicode_escape() {
        let schema = schema(vec![
            Field::required("exact", SchemaType::Integer),
            Field::required("text", SchemaType::String),
        ]);
        let value = decode_document(
            "exact = 90071992547409931234567890, text = \"A\\u{1f600}\"",
            &schema,
        )
        .expect("exact MON values decode");
        assert_eq!(
            value,
            Value::Record(vec![
                (
                    "exact".into(),
                    Value::Integer("90071992547409931234567890".into()),
                ),
                ("text".into(), Value::String("A😀".into())),
            ])
        );
    }

    #[test]
    fn preserves_parser_paths_and_charges_exact_numbers_before_copying() {
        let nested_schema = schema(vec![Field::required(
            "outer",
            SchemaType::Record {
                fields: vec![Field::required("text", SchemaType::String)],
            },
        )]);
        let nested_error = decode_document(r#"outer = (text = "\u{D800}")"#, &nested_schema)
            .expect_err("invalid nested Unicode must fail");
        assert_eq!(nested_error.code, MonErrorCode::InvalidCharacter);
        assert_eq!(
            nested_error.path,
            vec![
                PathSegment::Field("outer".into()),
                PathSegment::Field("text".into()),
            ]
        );
        let truncated_error = decode_document("outer = (text =", &nested_schema)
            .expect_err("truncated nested field must fail");
        assert_eq!(truncated_error.code, MonErrorCode::UnexpectedEnd);
        assert_eq!(
            truncated_error.path,
            vec![
                PathSegment::Field("outer".into()),
                PathSegment::Field("text".into()),
            ]
        );

        let collection_schema = schema(vec![Field::required(
            "values",
            SchemaType::Collection {
                element: Box::new(SchemaType::String),
            },
        )]);
        let collection_error = decode_document(r#"values = {"\u{D800}"}"#, &collection_schema)
            .expect_err("invalid collection Unicode must fail");
        assert_eq!(collection_error.code, MonErrorCode::InvalidCharacter);
        assert_eq!(
            collection_error.path,
            vec![PathSegment::Field("values".into()), PathSegment::Index(0),]
        );

        let map_schema = schema(vec![Field::required(
            "scores",
            SchemaType::Map {
                key: Box::new(SchemaType::String),
                value: Box::new(SchemaType::Int),
            },
        )]);
        let map_error = decode_document(r#"scores = {"x" ="#, &map_schema)
            .expect_err("truncated map value must fail");
        assert_eq!(map_error.code, MonErrorCode::UnexpectedEnd);
        assert_eq!(
            map_error.path,
            vec![PathSegment::Field("scores".into()), PathSegment::Index(0)]
        );
        let map_tail_error = decode_document(r#"scores = {"x" = 1, "y""#, &map_schema)
            .expect_err("truncated map entry must fail");
        assert_eq!(map_tail_error.code, MonErrorCode::UnexpectedEnd);
        assert_eq!(
            map_tail_error.path,
            vec![PathSegment::Field("scores".into()), PathSegment::Index(1)]
        );
        let choice_schema = schema(vec![Field::required(
            "choice",
            SchemaType::Choice {
                name: "Widget".into(),
                variants: vec![Variant::payload(
                    "Text",
                    vec![Field::required("value", SchemaType::String)],
                )],
            },
        )]);
        let choice_error = decode_document(r#"choice = ::Text("\u{D800}")"#, &choice_schema)
            .expect_err("invalid choice payload Unicode must fail");
        assert_eq!(choice_error.code, MonErrorCode::InvalidCharacter);
        assert_eq!(
            choice_error.path,
            vec![
                PathSegment::Field("choice".into()),
                PathSegment::Variant("Text".into()),
            ]
        );

        let exact_schema = Schema::record(vec![Field::required("x", SchemaType::Integer)])
            .with_limits(Limits {
                max_decoded_bytes: 1,
                ..Limits::default()
            })
            .prepare()
            .expect("exact-number budget schema prepares");
        let exact_error = decode_document("x = 123", &exact_schema)
            .expect_err("exact numeric output must honor decoded-byte budget");
        assert_eq!(exact_error.code, MonErrorCode::DecodedBudget);
        assert!(exact_error.path.is_empty());

        let label_schema = Schema::record(Vec::new())
            .with_limits(Limits {
                max_decoded_bytes: 0,
                ..Limits::default()
            })
            .prepare()
            .expect("label budget schema prepares");
        let label_error = decode_document("long_name = 1", &label_schema)
            .expect_err("input-derived path names must honor decoded-byte budget");
        assert_eq!(label_error.code, MonErrorCode::DecodedBudget);
    }
    #[test]
    fn preserves_float_sign_and_decimal_text_until_conversion() {
        let numeric_schema = schema(vec![
            Field::required("float", SchemaType::Float),
            Field::required("decimal", SchemaType::Decimal { scale: 2 }),
            Field::required("whole", SchemaType::Int),
        ]);
        let value = decode_document("float = -0.0, decimal = 1.2300, whole = 7", &numeric_schema)
            .expect("finite float and lossless decimal should decode");
        let Value::Record(fields) = value else {
            panic!("root schema produced a non-record");
        };
        let float = fields
            .iter()
            .find(|(name, _)| name == "float")
            .map(|(_, value)| value)
            .expect("float field");
        assert!(matches!(float, Value::Float(value) if value.to_bits() == (-0.0f64).to_bits()));
        assert_eq!(
            fields
                .iter()
                .find(|(name, _)| name == "decimal")
                .map(|(_, value)| value),
            Some(&Value::Decimal("1.2300".into()))
        );
        let int_schema = schema(vec![Field::required("whole", SchemaType::Int)]);
        assert_eq!(
            decode_document("whole = 2147483648", &int_schema)
                .unwrap_err()
                .code,
            MonErrorCode::NumericRange
        );
        assert_eq!(
            decode_document("float = 1.0, decimal = 1.231, whole = 7", &numeric_schema,)
                .unwrap_err()
                .code,
            MonErrorCode::NumericScale
        );
    }
    #[test]
    fn decodes_defaults_containers_choices_and_comments() {
        let data_schema = schema(vec![
            Field::with_default(
                "layout",
                SchemaType::Struct {
                    name: "Layout".into(),
                    fields: vec![
                        Field::required("width", SchemaType::Int),
                        Field::with_default("height", SchemaType::Int, Value::Int(720)),
                    ],
                },
                Value::Record(vec![("width".into(), Value::Int(1280))]),
            ),
            Field::required(
                "items",
                SchemaType::Collection {
                    element: Box::new(SchemaType::Int),
                },
            ),
            Field::required(
                "scores",
                SchemaType::Map {
                    key: Box::new(SchemaType::String),
                    value: Box::new(SchemaType::Int),
                },
            ),
            Field::required(
                "theme",
                SchemaType::Choice {
                    name: "Theme".into(),
                    variants: vec![
                        Variant::unit("Light"),
                        Variant::payload(
                            "Custom",
                            vec![Field::required("name", SchemaType::String)],
                        ),
                    ],
                },
            ),
            Field::required("note", SchemaType::Optional(Box::new(SchemaType::String))),
        ]);
        let value = decode_document(
            "-- save data\nitems = {1, 2,}, scores = {\"Priya\" = 10, \"Rob\" = 12},\n\
             theme = ::Custom(name = \"Gold\"), note = none,",
            &data_schema,
        )
        .expect("literal data should decode");

        assert_eq!(
            value,
            Value::Record(vec![
                (
                    "layout".into(),
                    Value::Record(vec![
                        ("width".into(), Value::Int(1280)),
                        ("height".into(), Value::Int(720)),
                    ]),
                ),
                (
                    "items".into(),
                    Value::Collection(vec![Value::Int(1), Value::Int(2)]),
                ),
                (
                    "scores".into(),
                    Value::Map(vec![
                        (Value::String("Priya".into()), Value::Int(10)),
                        (Value::String("Rob".into()), Value::Int(12)),
                    ]),
                ),
                (
                    "theme".into(),
                    Value::Choice {
                        qualifier: None,
                        variant: "Custom".into(),
                        fields: vec![("name".into(), Value::String("Gold".into()))],
                    },
                ),
                ("note".into(), Value::None),
            ])
        );

        let optional_schema = schema(vec![Field::required(
            "note",
            SchemaType::Optional(Box::new(SchemaType::String)),
        )]);
        assert_eq!(
            decode_document("", &optional_schema).unwrap_err().code,
            MonErrorCode::MissingField
        );

        let no_merge_schema = schema(vec![Field::with_default(
            "layout",
            SchemaType::Record {
                fields: vec![
                    Field::required("width", SchemaType::Int),
                    Field::required("height", SchemaType::Int),
                ],
            },
            Value::Record(vec![
                ("width".into(), Value::Int(1)),
                ("height".into(), Value::Int(2)),
            ]),
        )]);
        assert_eq!(
            decode_document("layout = (width = 9)", &no_merge_schema)
                .unwrap_err()
                .code,
            MonErrorCode::MissingField
        );
    }

    #[test]
    fn rejects_numeric_categories_and_container_kind_changes() {
        let int_schema = schema(vec![Field::required("value", SchemaType::Int)]);
        assert_eq!(
            decode_document("1", &int_schema).unwrap_err().code,
            MonErrorCode::RootNotRecord
        );
        assert_eq!(
            decode_document("value = 3.0", &int_schema)
                .unwrap_err()
                .code,
            MonErrorCode::NumericType
        );
        assert_eq!(
            decode_document("value = 1e3", &int_schema)
                .unwrap_err()
                .code,
            MonErrorCode::NumericType
        );
        assert_eq!(
            decode_document("value = 1E3", &int_schema)
                .unwrap_err()
                .code,
            MonErrorCode::NumericSyntax
        );

        let map_schema = schema(vec![Field::required(
            "value",
            SchemaType::Map {
                key: Box::new(SchemaType::String),
                value: Box::new(SchemaType::Int),
            },
        )]);
        assert_eq!(
            decode_document("value = {}", &map_schema).unwrap_err().code,
            MonErrorCode::MapKind
        );

        let collection_schema = schema(vec![Field::required(
            "value",
            SchemaType::Collection {
                element: Box::new(SchemaType::Int),
            },
        )]);
        assert_eq!(
            decode_document("value = {=}", &collection_schema)
                .unwrap_err()
                .code,
            MonErrorCode::MapKind
        );
    }
    #[test]
    fn routes_qualified_struct_arguments_and_preserves_nested_map_paths() {
        let struct_schema = schema(vec![Field::required(
            "size",
            SchemaType::Struct {
                name: "Size".into(),
                fields: vec![
                    Field::required("width", SchemaType::Int),
                    Field::required("height", SchemaType::Int),
                ],
            },
        )]);
        assert_eq!(
            decode_document("size = Size(1280, height = 720)", &struct_schema),
            Ok(Value::Record(vec![(
                "size".into(),
                Value::Record(vec![
                    ("width".into(), Value::Int(1280)),
                    ("height".into(), Value::Int(720)),
                ]),
            )]))
        );
        assert_eq!(
            decode_document("size = (1280, height = 720)", &struct_schema)
                .unwrap_err()
                .code,
            MonErrorCode::ArgumentOrder
        );

        let nested_schema = schema(vec![Field::required(
            "outer",
            SchemaType::Record {
                fields: vec![Field::required(
                    "scores",
                    SchemaType::Map {
                        key: Box::new(SchemaType::String),
                        value: Box::new(SchemaType::Int),
                    },
                )],
            },
        )]);
        let error = decode_document(r#"outer = (scores = {"x" = 1, "x" = 2})"#, &nested_schema)
            .expect_err("duplicate nested map key must fail");
        assert_eq!(error.code, MonErrorCode::DuplicateMapKey);
        assert_eq!(
            error.path,
            vec![
                PathSegment::Field("outer".into()),
                PathSegment::Field("scores".into()),
                PathSegment::MapKey("x".into()),
            ]
        );

        let value_error = decode_document(r#"outer = (scores = {"x" = "bad"})"#, &nested_schema)
            .expect_err("map value type mismatch must fail");
        assert_eq!(value_error.code, MonErrorCode::TypeMismatch);
        assert_eq!(
            value_error.path,
            vec![
                PathSegment::Field("outer".into()),
                PathSegment::Field("scores".into()),
                PathSegment::MapKey("x".into()),
            ]
        );
    }

    #[test]
    fn rejects_duplicate_keys_and_invalid_unicode_at_their_spans() {
        let map_schema = schema(vec![Field::required(
            "scores",
            SchemaType::Map {
                key: Box::new(SchemaType::String),
                value: Box::new(SchemaType::Int),
            },
        )]);
        let duplicate = decode_document(r#"scores = {"x" = 1, "x" = 2}"#, &map_schema)
            .expect_err("duplicate map keys must fail");
        assert_eq!(duplicate.code, MonErrorCode::DuplicateMapKey);
        assert_eq!(duplicate.span, Some(Span::new(19, 22)));

        let text_schema = schema(vec![
            Field::required("text", SchemaType::String),
            Field::required("letter", SchemaType::Char),
        ]);
        assert_eq!(
            decode_document(r#"text = "\u{D800}", letter = 'a'"#, &text_schema)
                .unwrap_err()
                .code,
            MonErrorCode::InvalidCharacter
        );
        assert_eq!(
            decode_document(r#"text = "\0", letter = 'a'"#, &text_schema)
                .unwrap_err()
                .code,
            MonErrorCode::InvalidEscape
        );
        let decoded = decode_document(
            r#"text = "line
break", letter = '\u{1F600}'"#,
            &text_schema,
        )
        .expect("MON preserves string newlines and decodes scalar escapes");
        assert_eq!(
            decoded,
            Value::Record(vec![
                ("text".into(), Value::String("line\nbreak".into())),
                ("letter".into(), Value::Char('😀')),
            ])
        );
    }

    #[test]
    fn enforces_decoder_budgets_and_schema_eligibility() {
        let numeric_schema = Schema::record(vec![Field::required("value", SchemaType::Int)])
            .with_limits(Limits {
                max_numeric_digits: 2,
                ..Limits::default()
            })
            .prepare()
            .expect("numeric budget schema prepares");
        assert_eq!(
            decode_document("value = 123", &numeric_schema)
                .unwrap_err()
                .code,
            MonErrorCode::NumericBudget
        );

        let depth_schema = Schema::record(vec![Field::required("value", SchemaType::Int)])
            .with_limits(Limits {
                max_depth: 1,
                ..Limits::default()
            })
            .prepare()
            .expect("depth budget schema prepares");
        assert_eq!(
            decode_document("value = (nested = 1)", &depth_schema)
                .unwrap_err()
                .code,
            MonErrorCode::DepthBudget
        );

        let default_schema = Schema::record(vec![Field::with_default(
            "value",
            SchemaType::Int,
            Value::Int(1),
        )])
        .with_limits(Limits {
            max_default_expansions: 0,
            ..Limits::default()
        })
        .prepare()
        .expect("default budget schema prepares");
        assert_eq!(
            decode_document("", &default_schema).unwrap_err().code,
            MonErrorCode::DefaultBudget
        );

        let unsupported = Schema::record(vec![Field::required(
            "resource",
            SchemaType::Unsupported {
                name: "Resource".into(),
            },
        )])
        .prepare()
        .expect_err("unsupported schema members must fail during preparation");
        assert_eq!(unsupported.code, MonErrorCode::UnsupportedSchema);

        let depth_limited_schema = Schema::value(SchemaType::Collection {
            element: Box::new(SchemaType::Int),
        })
        .with_limits(Limits {
            max_depth: 0,
            ..Limits::default()
        })
        .prepare()
        .expect_err("schema traversal depth must be bounded");
        assert_eq!(depth_limited_schema.code, MonErrorCode::DepthBudget);

        let node_limited_schema = Schema::record(Vec::new())
            .with_limits(Limits {
                max_nodes: 0,
                ..Limits::default()
            })
            .prepare()
            .expect_err("prepared schema nodes must be bounded");
        assert_eq!(node_limited_schema.code, MonErrorCode::NodeBudget);

        let name_budget_schema =
            Schema::record(vec![Field::required("long_name", SchemaType::Int)])
                .with_limits(Limits {
                    max_decoded_bytes: 4,
                    ..Limits::default()
                })
                .prepare()
                .expect_err("schema identifier copies must honor decoded-byte budget");
        assert_eq!(name_budget_schema.code, MonErrorCode::DecodedBudget);

        let nested_default_budget = Schema::record(vec![Field::with_default(
            "layout",
            SchemaType::Record {
                fields: vec![Field::with_default(
                    "height",
                    SchemaType::Int,
                    Value::Int(720),
                )],
            },
            Value::Record(Vec::new()),
        )])
        .with_limits(Limits {
            max_default_expansions: 0,
            ..Limits::default()
        })
        .prepare()
        .expect_err("nested prepared defaults must honor expansion budget");
        assert_eq!(nested_default_budget.code, MonErrorCode::DefaultBudget);

        let default_bytes_budget = Schema::record(vec![Field::with_default(
            "layout",
            SchemaType::Record {
                fields: vec![Field::with_default(
                    "text",
                    SchemaType::String,
                    Value::String("large".into()),
                )],
            },
            Value::Record(Vec::new()),
        )])
        .with_limits(Limits {
            max_decoded_bytes: 0,
            ..Limits::default()
        })
        .prepare()
        .expect_err("prepared default copies must honor decoded-byte budget");
        assert_eq!(default_bytes_budget.code, MonErrorCode::DecodedBudget);

        let numeric_default_budget = Schema::record(vec![Field::with_default(
            "value",
            SchemaType::Integer,
            Value::Integer("123".into()),
        )])
        .with_limits(Limits {
            max_numeric_digits: 2,
            ..Limits::default()
        })
        .prepare()
        .expect_err("numeric defaults must check digit budget before parsing");
        assert_eq!(numeric_default_budget.code, MonErrorCode::NumericBudget);
        let nested_optional = Schema::record(vec![Field::required(
            "value",
            SchemaType::Optional(Box::new(SchemaType::Optional(Box::new(SchemaType::Int)))),
        )])
        .prepare()
        .expect_err("nested optional schemas must be rejected");
        assert_eq!(nested_optional.code, MonErrorCode::InvalidSchema);

        let mut choice_default_schema = Schema::record(vec![Field::with_default(
            "theme",
            SchemaType::Choice {
                name: "Theme".into(),
                variants: vec![Variant::payload(
                    "V",
                    vec![Field::required("p", SchemaType::String)],
                )],
            },
            Value::Choice {
                qualifier: None,
                variant: "V".into(),
                fields: vec![("p".into(), Value::String("x".into()))],
            },
        )])
        .prepare()
        .expect("choice default schema prepares");
        choice_default_schema.limits.max_decoded_bytes = 8;
        let choice_default_error = decode_document("", &choice_default_schema)
            .expect_err("choice default payload copy must preserve its variant path");
        assert_eq!(choice_default_error.code, MonErrorCode::DecodedBudget);
        assert_eq!(
            choice_default_error.path,
            vec![
                PathSegment::Field("theme".into()),
                PathSegment::Variant("V".into()),
                PathSegment::Field("p".into()),
            ]
        );

        let optional_none = Schema::record(vec![Field::required(
            "value",
            SchemaType::Optional(Box::new(SchemaType::None)),
        )])
        .prepare()
        .expect_err("optional none schemas must be rejected");
        assert_eq!(optional_none.code, MonErrorCode::InvalidSchema);
    }
}
