//! Type-annotation parsing for declaration and signature syntax.
//!
//! WHAT: converts token streams into unresolved parsed type references, including narrow
//!      fixed-capacity collection syntax using integer literals or bare constant names.
//! WHY: parsing stays separate from semantic type resolution so header and AST
//!      callers can share syntax without rebuilding type-environment policy here.

use super::*;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{DiagnosticToken, InvalidTypeAnnotationReason};
use crate::compiler_frontend::datatypes::parsed::ParsedCollectionCapacity;
use crate::compiler_frontend::declaration_syntax::DeclarationCursor;
use crate::compiler_frontend::numeric_text::parse::materialize_i32;
use crate::compiler_frontend::numeric_text::token::{NumericLiteralKind, NumericLiteralSign};
use crate::compiler_frontend::source::{LocalSpan, SourceId, SourceSpan};
use crate::compiler_frontend::symbols::string_interning::StringTable;
#[cfg(test)]
use crate::compiler_frontend::tokenizer::tokens::FileTokens;
use crate::compiler_frontend::tokenizer::tokens::{
    SourceTokens, Token, TokenCursor, TokenIndex, TokenKind, TokenRange, TokenRangeError, TokenRef,
    TokenTag,
};

/// Two-lane result for type-annotation parsing.
///
/// WHAT: carries authored-source diagnostics separately from internal compiler-state failures.
/// WHY: type parsing otherwise carries the large diagnostic value through every
///      successful declaration and signature parse, and infrastructure failures must abort
///      through the typed lane instead of becoming source diagnostics.
type TypeParseResult<T> = Result<T, HeaderParseFailure>;
/// A borrowed, checked canonical window used for collection/map speculation.
///
/// The type parser never projects a braced body into a second `Vec<Token>` stream. Every
/// speculative side and nested parse is a bounded range over the same source-owned arrays.
#[derive(Clone, Copy)]
struct TypeTokenWindow<'a> {
    source: &'a SourceTokens,
    range: TokenRange,
    /// Optional remapped parser lane corresponding to `range`.
    ///
    /// The canonical source remains authoritative for spans and range validation. The transient
    /// lane supplies destination-domain payload IDs when a generic body crosses a package boundary.
    compatibility_tokens: Option<&'a [Token]>,
    compatibility_start: usize,
}

impl<'a> TypeTokenWindow<'a> {
    fn new_with_compatibility(
        source: &'a SourceTokens,
        range: TokenRange,
        compatibility: Option<(&'a [Token], usize)>,
    ) -> Result<Self, TokenRangeError> {
        source
            .full_range()
            .and_then(|_| TokenRange::try_new_for(source, range.start(), range.end()))
            .and_then(|range| {
                if let Some((tokens, start)) = compatibility {
                    let Some(end) = start.checked_add(range.len() as usize) else {
                        return Err(TokenRangeError::OutOfBounds {
                            start: u32::MAX,
                            end: u32::MAX,
                            len: tokens.len(),
                        });
                    };
                    if end > tokens.len() {
                        return Err(TokenRangeError::OutOfBounds {
                            start: u32::try_from(start).unwrap_or(u32::MAX),
                            end: u32::try_from(end).unwrap_or(u32::MAX),
                            len: tokens.len(),
                        });
                    }
                }
                Ok(Self {
                    source,
                    range,
                    compatibility_tokens: compatibility.map(|(tokens, _)| tokens),
                    compatibility_start: compatibility.map_or(0, |(_, start)| start),
                })
            })
    }

    fn len(self) -> usize {
        self.range.len() as usize
    }

    fn is_empty(self) -> bool {
        self.range.is_empty()
    }

    fn get(self, index: usize) -> Option<TokenRef<'a>> {
        let absolute = self.range.start().index().checked_add(index)?;
        if absolute >= self.range.end().index() {
            return None;
        }
        self.source
            .token(TokenIndex::try_from_index(absolute)?)
            .ok()
    }

    fn compatibility_view(self) -> Option<(&'a [Token], usize)> {
        self.compatibility_tokens
            .map(|tokens| (tokens, self.compatibility_start))
    }

    fn token_kind_at(self, index: usize) -> Option<TokenKind> {
        if let Some((tokens, start)) = self.compatibility_view() {
            return tokens
                .get(start.checked_add(index)?)
                .map(|token| token.kind.clone());
        }
        self.get(index).and_then(|token| token.to_token_kind().ok())
    }

    fn token_tag_at(self, index: usize) -> Option<TokenTag> {
        // The tag is stable across the remapped compatibility lane, so read the
        // canonical view first and only fall back to the cloned lane.
        self.get(index)
            .map(TokenRef::tag)
            .or_else(|| self.token_kind_at(index).map(|kind| kind.token_tag()))
    }

    fn token_string_id_at(self, index: usize) -> Option<StringId> {
        if self.token_tag_at(index)? != TokenTag::SYMBOL {
            return None;
        }
        if self.compatibility_view().is_some() {
            return match self.token_kind_at(index)? {
                TokenKind::Symbol(name) => Some(name),
                _ => None,
            };
        }
        self.get(index).and_then(TokenRef::string_id)
    }

    fn token_span_at(self, index: usize) -> Option<SourceSpan> {
        self.get(index).map(TokenRef::source_span)
    }

    fn subrange(self, start: usize, end: usize) -> Option<Self> {
        if start > end || end > self.len() {
            return None;
        }
        let absolute_start = self.range.start().index().checked_add(start)?;
        let absolute_end = self.range.start().index().checked_add(end)?;
        let token_start = TokenIndex::try_from_index(absolute_start)?;
        let token_end = TokenIndex::try_from_index(absolute_end)?;
        let range = TokenRange::try_new_for(self.source, token_start, token_end).ok()?;
        let compatibility = self.compatibility_tokens.and_then(|tokens| {
            self.compatibility_start
                .checked_add(start)
                .map(|compatibility_start| (tokens, compatibility_start))
        });
        Self::new_with_compatibility(self.source, range, compatibility).ok()
    }

    fn cursor(self) -> Result<TokenCursor<'a>, TokenRangeError> {
        TokenCursor::new(self.source, self.range)
    }

    fn source_id(self) -> SourceId {
        self.range.source()
    }
}

// -------------------------
//  Type annotation parsing
// -------------------------

/// Parse a type annotation and return the parsed type reference.
///
/// WHAT: produces `ParsedTypeRef` — unresolved parsed syntax, not semantic identity.
/// Compatibility adapter for legacy test/AST streams.
///
/// This boundary borrows the existing canonical owner when one exists and only mirrors the
/// consumed position back to `FileTokens`; the parser itself remains the cursor-only core above.
#[cfg(test)]
pub(crate) fn parse_type_annotation(
    token_stream: &mut FileTokens,
    context: TypeAnnotationContext,
    string_table: &StringTable,
) -> TypeParseResult<ParsedTypeRef> {
    let (result, canonical_cursor) = {
        let mut declaration_cursor = DeclarationCursor::from_file_tokens(token_stream)
            .map_err(HeaderParseFailure::Infrastructure)?;
        let result = parse_type_annotation_cursor(&mut declaration_cursor, context, string_table);
        (result, declaration_cursor.canonical_cursor())
    };
    let next_index = token_stream
        .compatibility_index_for_cursor(canonical_cursor)
        .map_err(HeaderParseFailure::Infrastructure)?;
    token_stream.index = next_index;
    result
}

/// Canonical declaration/type parser core. Callers must hand off a short-lived bounded cursor.
pub(crate) fn parse_type_annotation_cursor(
    token_stream: &mut DeclarationCursor<'_>,
    context: TypeAnnotationContext,
    string_table: &StringTable,
) -> TypeParseResult<ParsedTypeRef> {
    // Only ordinary declaration targets may omit a type. Build-config contracts intentionally use
    // a required context so `#Config of = value` is rejected at the authored `=` token.
    if matches!(context, TypeAnnotationContext::DeclarationTarget)
        && matches!(
            token_stream.current_tag(),
            TokenTag::ASSIGN | TokenTag::NEWLINE | TokenTag::COMMA
        )
    {
        return Ok(ParsedTypeRef::Inferred);
    }

    parse_required_type(token_stream, context, string_table)
}

fn parse_required_type(
    token_stream: &mut DeclarationCursor<'_>,
    context: TypeAnnotationContext,
    string_table: &StringTable,
) -> TypeParseResult<ParsedTypeRef> {
    parse_required_type_with_generic_application(token_stream, context, string_table, true)
}

fn parse_required_type_with_generic_application(
    token_stream: &mut DeclarationCursor<'_>,
    context: TypeAnnotationContext,
    string_table: &StringTable,
    allow_generic_application: bool,
) -> TypeParseResult<ParsedTypeRef> {
    let parsed_atom = parse_type_atom(token_stream, context, string_table)?;

    parse_type_postfixes(
        token_stream,
        parsed_atom,
        context,
        string_table,
        allow_generic_application,
    )
}

fn parse_type_atom(
    token_stream: &mut DeclarationCursor<'_>,
    context: TypeAnnotationContext,
    string_table: &StringTable,
) -> TypeParseResult<ParsedTypeRef> {
    let span = current_source_span(token_stream);

    match token_stream.current_tag() {
        TokenTag::DATATYPE_INT => {
            token_stream.advance();
            Ok(ParsedTypeRef::BuiltinInt { span })
        }

        TokenTag::DATATYPE_FLOAT => {
            token_stream.advance();
            Ok(ParsedTypeRef::BuiltinFloat { span })
        }

        TokenTag::DATATYPE_BOOL => {
            token_stream.advance();
            Ok(ParsedTypeRef::BuiltinBool { span })
        }

        TokenTag::DATATYPE_STRING => {
            token_stream.advance();
            Ok(ParsedTypeRef::BuiltinString { span })
        }

        TokenTag::DATATYPE_CHAR => {
            token_stream.advance();
            Ok(ParsedTypeRef::BuiltinChar { span })
        }

        TokenTag::DATATYPE_NONE => Err(HeaderParseFailure::Diagnostic(
            CompilerDiagnostic::invalid_type_annotation(
                context,
                InvalidTypeAnnotationReason::NoneNotAllowed,
                current_source_span(token_stream),
            ),
        )),

        TokenTag::MUST | TokenTag::TRAIT_THIS => {
            if matches!(context, TypeAnnotationContext::TraitRequirement)
                && token_stream.current_tag() == TokenTag::TRAIT_THIS
            {
                token_stream.advance();
                return Ok(ParsedTypeRef::This { span });
            }
            let _keyword = reserved_trait_keyword_or_dispatch_mismatch_for_tag(
                token_stream.current_tag(),
                current_source_span(token_stream),
                compilation_stage(context),
                "type annotation parsing",
            )?;

            Err(HeaderParseFailure::Diagnostic(
                CompilerDiagnostic::invalid_type_annotation(
                    context,
                    InvalidTypeAnnotationReason::ReservedTraitKeyword,
                    current_source_span(token_stream),
                ),
            ))
        }

        TokenTag::OPEN_CURLY => parse_collection_type(token_stream, context, string_table),

        TokenTag::REACTIVE => Err(HeaderParseFailure::Diagnostic(
            CompilerDiagnostic::invalid_type_annotation(
                context,
                InvalidTypeAnnotationReason::ReactiveAccessNotAllowed,
                current_source_span(token_stream),
            ),
        )),

        TokenTag::AS => Err(HeaderParseFailure::Diagnostic(
            CompilerDiagnostic::invalid_type_annotation(
                context,
                InvalidTypeAnnotationReason::AsNotValidHere,
                current_source_span(token_stream),
            ),
        )),
        TokenTag::TYPE => Err(HeaderParseFailure::Diagnostic(type_keyword_deferred_error(
            token_stream,
            context,
        ))),
        TokenTag::OF => {
            let Some(found) = token_stream.canonical_cursor().current() else {
                return Err(HeaderParseFailure::Diagnostic(
                    CompilerDiagnostic::unexpected_end_of_file(
                        None,
                        current_source_span(token_stream),
                    ),
                ));
            };
            Err(HeaderParseFailure::Diagnostic(
                CompilerDiagnostic::unexpected_token_from_ref(
                    found,
                    current_source_span(token_stream),
                ),
            ))
        }
        TokenTag::SYMBOL => {
            let Some(type_name) = token_stream.current_string_id() else {
                return Err(HeaderParseFailure::Infrastructure(
                    CompilerError::compiler_error("symbol token is missing its string payload"),
                ));
            };
            token_stream.advance();

            // Check for namespace-qualified type syntax: `Namespace.Type` or
            // `Namespace.Child.Type`. Collect all `Symbol . Symbol` segments into a
            // single qualified path while preserving bare single-symbol types.
            if token_stream.current_tag() == TokenTag::DOT {
                let mut path = vec![type_name];

                while token_stream.current_tag() == TokenTag::DOT {
                    token_stream.advance(); // consume '.'

                    if token_stream.current_tag() == TokenTag::SYMBOL {
                        let Some(member_name) = token_stream.current_string_id() else {
                            return Err(HeaderParseFailure::Infrastructure(
                                CompilerError::compiler_error(
                                    "symbol token is missing its string payload",
                                ),
                            ));
                        };
                        path.push(member_name);
                        token_stream.advance();
                    } else {
                        let span = current_source_span(token_stream);
                        let found = DiagnosticToken::from_token_ref(
                            token_stream
                                .canonical_cursor()
                                .current()
                                .expect("validated declaration cursor token"),
                        );
                        return Err(HeaderParseFailure::Diagnostic(
                            CompilerDiagnostic::invalid_type_annotation(
                                context,
                                InvalidTypeAnnotationReason::ExpectedTypeAnnotation { found },
                                span,
                            ),
                        ));
                    }
                }

                return Ok(ParsedTypeRef::Qualified { path, span });
            }

            Ok(ParsedTypeRef::Named {
                name: type_name,
                span,
            })
        }
        TokenTag::COLON if matches!(context, TypeAnnotationContext::DeclarationTarget) => Err(
            HeaderParseFailure::Diagnostic(CompilerDiagnostic::invalid_type_annotation(
                context,
                InvalidTypeAnnotationReason::UnexpectedColon,
                current_source_span(token_stream),
            )),
        ),
        other
            if matches!(context, TypeAnnotationContext::DeclarationTarget)
                && matches!(
                    other,
                    TokenTag::DOT
                        | TokenTag::ADD_ASSIGN
                        | TokenTag::SUBTRACT_ASSIGN
                        | TokenTag::DIVIDE_ASSIGN
                        | TokenTag::INT_DIVIDE_ASSIGN
                        | TokenTag::MULTIPLY_ASSIGN
                ) =>
        {
            let found = DiagnosticToken::from_token_ref(
                token_stream
                    .canonical_cursor()
                    .current()
                    .expect("validated declaration cursor token"),
            );
            Err(HeaderParseFailure::Diagnostic(
                CompilerDiagnostic::invalid_type_annotation(
                    context,
                    InvalidTypeAnnotationReason::InvalidTokenAfterName { token: found },
                    current_source_span(token_stream),
                ),
            ))
        }
        _ => {
            let span = current_source_span(token_stream);
            let Some(token) = token_stream.canonical_cursor().current() else {
                return Err(HeaderParseFailure::Diagnostic(
                    CompilerDiagnostic::unexpected_end_of_file(None, span),
                ));
            };
            let found = DiagnosticToken::from_token_ref(token);
            Err(HeaderParseFailure::Diagnostic(
                CompilerDiagnostic::invalid_type_annotation(
                    context,
                    InvalidTypeAnnotationReason::ExpectedTypeAnnotation { found },
                    span,
                ),
            ))
        }
    }
}

fn parse_type_postfixes(
    token_stream: &mut DeclarationCursor<'_>,
    parsed_type: ParsedTypeRef,
    context: TypeAnnotationContext,
    string_table: &StringTable,
    allow_generic_application: bool,
) -> TypeParseResult<ParsedTypeRef> {
    let with_generic_arguments = parse_generic_arguments(
        token_stream,
        parsed_type,
        context,
        string_table,
        allow_generic_application,
    )?;
    parse_optional_type_suffix(token_stream, with_generic_arguments, context)
}

/// Parse a type annotation enclosed in `{...}`.
///
/// WHAT: handles both growable and fixed collection syntax at the parser boundary.
///
/// Two main cases:
///  1. Growable: `{T}` — the entire inner content parses as a single type.
///  2. Fixed:    `{N T}` — tokens before the element type become the capacity syntax.
///
/// Capacity-only shorthand (`{N}`) is only valid for declaration targets, where the
/// element type is inferred.
fn parse_collection_type(
    token_stream: &mut DeclarationCursor<'_>,
    context: TypeAnnotationContext,
    string_table: &StringTable,
) -> TypeParseResult<ParsedTypeRef> {
    let span = current_source_span(token_stream);
    token_stream.advance(); // consume '{'
    let compatibility_start = token_stream.compatibility_index();

    let inner_range = collect_collection_inner_range(token_stream)?;
    let source_tokens = token_stream.canonical_cursor().source_tokens();
    let compatibility = token_stream.compatibility_tokens().zip(compatibility_start);
    let inner = TypeTokenWindow::new_with_compatibility(source_tokens, inner_range, compatibility)
        .map_err(|error| {
            HeaderParseFailure::Infrastructure(CompilerError::compiler_error(format!(
                "collection type inner range was invalid: {error:?}",
            )))
        })?;
    token_stream.advance(); // consume the outer '}'

    if inner.is_empty() {
        return Ok(ParsedTypeRef::Collection {
            element: Box::new(ParsedTypeRef::Inferred),
            span,
            fixed_capacity: None,
        });
    }

    if let Some(reactive_span) = (0..inner.len()).find_map(|index| {
        inner
            .token_tag_at(index)
            .filter(|tag| *tag == TokenTag::REACTIVE)
            .and_then(|_| inner.token_span_at(index))
    }) {
        return Err(HeaderParseFailure::Diagnostic(
            CompilerDiagnostic::invalid_type_annotation(
                context,
                InvalidTypeAnnotationReason::ReactiveAccessNotAllowed,
                Some(reactive_span),
            ),
        ));
    }

    // Map type syntax `{K = V}` takes precedence over collection capacity splitting.
    match scan_top_level_assigns(inner) {
        TopLevelAssignScan::None => {}
        TopLevelAssignScan::One(assign_idx) => {
            return parse_map_type_from_inner_window(
                inner,
                assign_idx,
                context,
                string_table,
                span,
            );
        }
        TopLevelAssignScan::Multiple => {
            return Err(HeaderParseFailure::Diagnostic(
                CompilerDiagnostic::invalid_map_type(
                    InvalidMapTypeReason::MultipleMapSeparators,
                    span,
                ),
            ));
        }
    }

    if collection_type_slice_can_start_type(inner, context, string_table) {
        let parsed_slice = parse_type_slice(inner, context, string_table)?;
        if let Some(extra_token) = parsed_slice.next_token {
            let found = DiagnosticToken::try_from_token_ref(extra_token.view).map_err(|error| {
                HeaderParseFailure::Infrastructure(CompilerDiagnostic::token_view_invariant_error(
                    error,
                    "collection element trailing token",
                ))
            })?;
            return Err(HeaderParseFailure::Diagnostic(
                CompilerDiagnostic::expected_token_from_tags(
                    TokenTag::CLOSE_CURLY,
                    Some(found),
                    Some(SourceSpan::new(inner.source_id(), extra_token.span)),
                ),
            ));
        }

        let element = parsed_slice.parsed_type;
        reject_trait_this_composition(&element, context, span)?;
        return Ok(ParsedTypeRef::Collection {
            element: Box::new(element),
            span,
            fixed_capacity: None,
        });
    }

    for split_idx in 1..inner.len() {
        let Some(type_tokens) = inner.subrange(split_idx, inner.len()) else {
            continue;
        };
        if !collection_type_slice_can_start_type(type_tokens, context, string_table) {
            continue;
        }

        if let Some(element) = parse_type_slice_exact(type_tokens, context, string_table) {
            reject_trait_this_composition(&element, context, span)?;
            return Ok(ParsedTypeRef::Collection {
                element: Box::new(element),
                span,
                fixed_capacity: parsed_capacity(
                    inner
                        .subrange(0, split_idx)
                        .expect("split range is bounded"),
                    string_table,
                )?,
            });
        }
    }

    if matches!(context, TypeAnnotationContext::DeclarationTarget) {
        return Ok(ParsedTypeRef::Collection {
            element: Box::new(ParsedTypeRef::Inferred),
            span,
            fixed_capacity: parsed_capacity(inner, string_table)?,
        });
    }

    Err(HeaderParseFailure::Diagnostic(
        CompilerDiagnostic::invalid_collection_type(
            InvalidCollectionTypeReason::ShorthandCapacityNotAllowed,
            span,
        ),
    ))
}

/// Collect the canonical range between the current opening `{` and its matching `}`.
fn collect_collection_inner_range(
    token_stream: &mut DeclarationCursor<'_>,
) -> TypeParseResult<TokenRange> {
    let start = token_stream.canonical_cursor().position();
    let mut nested_collection_depth = 0usize;

    loop {
        match token_stream.current_tag() {
            TokenTag::CLOSE_CURLY if nested_collection_depth == 0 => {
                let end = token_stream.canonical_cursor().position();
                return TokenRange::try_new_for(
                    token_stream.canonical_cursor().source_tokens(),
                    start,
                    end,
                )
                .map_err(|error| {
                    HeaderParseFailure::Infrastructure(CompilerError::compiler_error(format!(
                        "collection type inner range exceeded its source owner: {error:?}",
                    )))
                });
            }
            TokenTag::CLOSE_CURLY => {
                nested_collection_depth = nested_collection_depth.saturating_sub(1);
                token_stream.advance();
            }
            TokenTag::OPEN_CURLY => {
                nested_collection_depth = nested_collection_depth.saturating_add(1);
                token_stream.advance();
            }
            TokenTag::EOF => {
                return Err(HeaderParseFailure::Diagnostic(
                    CompilerDiagnostic::expected_token_from_tags(
                        TokenTag::CLOSE_CURLY,
                        Some(DiagnosticToken::from_static_tag(TokenTag::EOF)),
                        current_source_span(token_stream),
                    ),
                ));
            }
            _ => token_stream.advance(),
        }
    }
}

/// WHAT: accepts only a single integer literal or a single bare symbol token.
///       Anything else (arithmetic, calls, field access, floats, etc.) is rejected
///       with a structured diagnostic so users get the narrow-rule message at the
///       syntax site rather than a generic parse failure.
/// WHY: the language only allows literal-or-bare-const capacity in type position;
///      named constants can still hold arithmetic before they are used in type annotations.
fn parsed_capacity(
    tokens: TypeTokenWindow<'_>,
    string_table: &StringTable,
) -> TypeParseResult<Option<ParsedCollectionCapacity>> {
    if tokens.is_empty() {
        return Ok(None);
    }
    if tokens.len() == 1 {
        let token_span = tokens.token_span_at(0);
        match tokens.token_tag_at(0) {
            Some(TokenTag::NUMERIC_LITERAL) => {
                let numeric = if tokens.compatibility_view().is_some() {
                    match tokens.token_kind_at(0) {
                        Some(TokenKind::NumericLiteral(numeric)) => numeric,
                        _ => {
                            return Err(HeaderParseFailure::Infrastructure(
                                CompilerError::compiler_error("capacity token payload is invalid"),
                            ));
                        }
                    }
                } else {
                    let token = tokens.get(0).ok_or_else(|| {
                        HeaderParseFailure::Infrastructure(CompilerError::compiler_error(
                            "capacity token payload is invalid",
                        ))
                    })?;
                    match token.numeric_literal() {
                        Ok(Some(numeric)) => numeric.clone(),
                        _ => {
                            return Err(HeaderParseFailure::Infrastructure(
                                CompilerError::compiler_error("capacity token payload is invalid"),
                            ));
                        }
                    }
                };

                if numeric.kind != NumericLiteralKind::WholeNumber {
                    return Err(HeaderParseFailure::Diagnostic(
                        CompilerDiagnostic::invalid_collection_type(
                            InvalidCollectionTypeReason::CapacityNotInt,
                            token_span,
                        ),
                    ));
                }

                let value = materialize_i32(&numeric, string_table).map_err(|reason| {
                    HeaderParseFailure::Diagnostic(CompilerDiagnostic::invalid_number_literal(
                        numeric.source_text,
                        reason,
                        token_span,
                    ))
                })?;

                if numeric.sign == NumericLiteralSign::Negative {
                    return Err(HeaderParseFailure::Diagnostic(
                        CompilerDiagnostic::invalid_collection_type(
                            InvalidCollectionTypeReason::NegativeCapacity,
                            token_span,
                        ),
                    ));
                }

                return Ok(Some(ParsedCollectionCapacity::Literal {
                    value,
                    span: token_span,
                }));
            }
            Some(TokenTag::SYMBOL) => {
                let Some(name) = tokens.token_string_id_at(0) else {
                    return Err(HeaderParseFailure::Infrastructure(
                        CompilerError::compiler_error("symbol token is missing its string payload"),
                    ));
                };
                return Ok(Some(ParsedCollectionCapacity::BareConstant {
                    name,
                    span: token_span,
                }));
            }
            _ => {}
        }
    }

    let span = tokens.get(0).map(TokenRef::source_span);
    Err(HeaderParseFailure::Diagnostic(
        CompilerDiagnostic::invalid_collection_type(
            InvalidCollectionTypeReason::CapacityNotConstant,
            span,
        ),
    ))
}

// -------------------------
//  Map type parsing helpers
// -------------------------

enum TopLevelAssignScan {
    None,
    One(usize),
    Multiple,
}

/// Scan a canonical range for top-level `=` tokens while tracking nested delimiters.
fn scan_top_level_assigns(tokens: TypeTokenWindow<'_>) -> TopLevelAssignScan {
    let mut depth = 0usize;
    let mut first_assign = None;
    for idx in 0..tokens.len() {
        let Some(tag) = tokens.token_tag_at(idx) else {
            continue;
        };
        match tag {
            TokenTag::OPEN_CURLY | TokenTag::OPEN_PARENTHESIS => depth += 1,
            TokenTag::CLOSE_CURLY | TokenTag::CLOSE_PARENTHESIS => {
                depth = depth.saturating_sub(1);
            }
            TokenTag::ASSIGN if depth == 0 => {
                if first_assign.is_some() {
                    return TopLevelAssignScan::Multiple;
                }
                first_assign = Some(idx);
            }
            _ => {}
        }
    }

    first_assign.map_or(TopLevelAssignScan::None, TopLevelAssignScan::One)
}

/// Parse a map type from the canonical range between `{` and `}`.
fn parse_map_type_from_inner_window(
    inner: TypeTokenWindow<'_>,
    assign_idx: usize,
    context: TypeAnnotationContext,
    string_table: &StringTable,
    span: Option<SourceSpan>,
) -> TypeParseResult<ParsedTypeRef> {
    let key_tokens = inner
        .subrange(0, assign_idx)
        .expect("map key range is bounded by its separator");
    let value_tokens = inner
        .subrange(assign_idx + 1, inner.len())
        .expect("map value range is bounded by its separator");

    if key_tokens.is_empty() {
        return Err(HeaderParseFailure::Diagnostic(
            CompilerDiagnostic::invalid_map_type(InvalidMapTypeReason::EmptyMapKeyType, span),
        ));
    }

    if value_tokens.is_empty() {
        return Err(HeaderParseFailure::Diagnostic(
            CompilerDiagnostic::invalid_map_type(InvalidMapTypeReason::EmptyMapValueType, span),
        ));
    }

    let key = try_parse_map_side(key_tokens, context, string_table, span)?;
    let value = try_parse_map_side(value_tokens, context, string_table, span)?;

    reject_trait_this_composition(&key, context, span)?;
    reject_trait_this_composition(&value, context, span)?;

    Ok(ParsedTypeRef::Map {
        key: Box::new(key),
        value: Box::new(value),
        span,
    })
}

/// Attempt to parse one side of a map type (`K` or `V`) from a canonical range.
fn try_parse_map_side(
    tokens: TypeTokenWindow<'_>,
    context: TypeAnnotationContext,
    string_table: &StringTable,
    span: Option<SourceSpan>,
) -> TypeParseResult<ParsedTypeRef> {
    if let Some(parsed) = parse_type_slice_exact(tokens, context, string_table) {
        return Ok(parsed);
    }

    if map_side_looks_like_fixed_capacity(tokens, context, string_table) {
        return Err(HeaderParseFailure::Diagnostic(
            CompilerDiagnostic::invalid_map_type(
                InvalidMapTypeReason::FixedCapacityNotAllowed,
                span,
            ),
        ));
    }

    if map_side_looks_like_postfix_capacity(tokens, context, string_table) {
        return Err(HeaderParseFailure::Diagnostic(
            CompilerDiagnostic::invalid_map_type(
                InvalidMapTypeReason::FixedCapacityNotAllowed,
                span,
            ),
        ));
    }

    let parsed_slice = parse_type_slice(tokens, context, string_table)?;
    if let Some(extra_token) = parsed_slice.next_token {
        let found = DiagnosticToken::try_from_token_ref(extra_token.view).map_err(|error| {
            HeaderParseFailure::Infrastructure(CompilerDiagnostic::token_view_invariant_error(
                error,
                "map side trailing token",
            ))
        })?;
        return Err(HeaderParseFailure::Diagnostic(
            CompilerDiagnostic::expected_token_from_tags(
                TokenTag::CLOSE_CURLY,
                Some(found),
                Some(SourceSpan::new(tokens.source_id(), extra_token.span)),
            ),
        ));
    }

    Ok(parsed_slice.parsed_type)
}

/// Detect prefix capacity-like syntax on a map-type side.
fn map_side_looks_like_fixed_capacity(
    tokens: TypeTokenWindow<'_>,
    context: TypeAnnotationContext,
    string_table: &StringTable,
) -> bool {
    for split_idx in 1..tokens.len() {
        let Some(type_tokens) = tokens.subrange(split_idx, tokens.len()) else {
            continue;
        };
        if collection_type_slice_can_start_type(type_tokens, context, string_table)
            && parse_type_slice_exact(type_tokens, context, string_table).is_some()
            && matches!(
                parsed_capacity(
                    tokens
                        .subrange(0, split_idx)
                        .expect("capacity range is bounded"),
                    string_table,
                ),
                Ok(Some(_))
            )
        {
            return true;
        }
    }

    false
}

/// Detect postfix capacity-like syntax on a map-type side.
fn map_side_looks_like_postfix_capacity(
    tokens: TypeTokenWindow<'_>,
    context: TypeAnnotationContext,
    string_table: &StringTable,
) -> bool {
    for split_idx in 1..=tokens.len() {
        let Some(type_tokens) = tokens.subrange(0, split_idx) else {
            continue;
        };
        if !collection_type_slice_can_start_type(type_tokens, context, string_table) {
            continue;
        }
        if parse_type_slice_exact(type_tokens, context, string_table).is_some() {
            let Some(remaining) = tokens.subrange(split_idx, tokens.len()) else {
                continue;
            };
            if remaining.is_empty() {
                continue;
            }
            return matches!(
                remaining.token_tag_at(0),
                Some(TokenTag::COLON) | Some(TokenTag::NUMERIC_LITERAL)
            );
        }
    }
    false
}

struct ParsedTypeSlice<'a> {
    parsed_type: ParsedTypeRef,
    next_token: Option<ParsedTypeSliceToken<'a>>,
}

struct ParsedTypeSliceToken<'a> {
    view: TokenRef<'a>,
    span: LocalSpan,
}

/// Parse a bounded canonical range as a type annotation.
fn parse_type_slice<'a>(
    tokens: TypeTokenWindow<'a>,
    context: TypeAnnotationContext,
    string_table: &StringTable,
) -> TypeParseResult<ParsedTypeSlice<'a>> {
    let cursor = tokens.cursor().map_err(|error| {
        HeaderParseFailure::Infrastructure(CompilerError::compiler_error(format!(
            "type slice range was invalid: {error:?}",
        )))
    })?;
    let mut stream = if let Some((compatibility_tokens, compatibility_start)) =
        tokens.compatibility_view()
    {
        let compatibility_end = compatibility_start
            .checked_add(tokens.len())
            .ok_or_else(|| {
                HeaderParseFailure::Infrastructure(CompilerError::compiler_error(
                    "type slice compatibility range overflowed",
                ))
            })?;
        DeclarationCursor::with_compatibility_bounds(
            cursor,
            compatibility_tokens,
            compatibility_start,
            compatibility_end,
        )
        .map_err(HeaderParseFailure::Infrastructure)?
    } else {
        DeclarationCursor::new(cursor).map_err(HeaderParseFailure::Infrastructure)?
    };
    let parsed_type = parse_required_type(&mut stream, context, string_table)?;
    let next_token = match stream.canonical_cursor().current() {
        Some(view) if stream.current_tag() != TokenTag::EOF => Some(ParsedTypeSliceToken {
            view,
            span: view.span(),
        }),
        _ => None,
    };

    Ok(ParsedTypeSlice {
        parsed_type,
        next_token,
    })
}

fn parse_type_slice_exact(
    tokens: TypeTokenWindow<'_>,
    context: TypeAnnotationContext,
    string_table: &StringTable,
) -> Option<ParsedTypeRef> {
    let parsed_slice = parse_type_slice(tokens, context, string_table).ok()?;

    parsed_slice
        .next_token
        .is_none()
        .then_some(parsed_slice.parsed_type)
}

/// Heuristic check: can the leading tokens of a range start a valid type annotation?
fn collection_type_slice_can_start_type(
    tokens: TypeTokenWindow<'_>,
    context: TypeAnnotationContext,
    string_table: &StringTable,
) -> bool {
    let Some(first_tag) = tokens.token_tag_at(0) else {
        return false;
    };

    match first_tag {
        TokenTag::DATATYPE_INT
        | TokenTag::DATATYPE_FLOAT
        | TokenTag::DATATYPE_BOOL
        | TokenTag::DATATYPE_STRING
        | TokenTag::DATATYPE_CHAR
        | TokenTag::OPEN_CURLY => true,

        TokenTag::TRAIT_THIS => matches!(context, TypeAnnotationContext::TraitRequirement),

        TokenTag::SYMBOL => {
            if tokens.token_tag_at(1) == Some(TokenTag::DOT)
                && let Some(member) = tokens.token_string_id_at(2)
            {
                return symbol_spelling_looks_type_name(member, string_table);
            }

            tokens
                .token_string_id_at(0)
                .is_some_and(|name| symbol_spelling_looks_type_name(name, string_table))
        }

        _ => false,
    }
}

fn symbol_spelling_looks_type_name(name: StringId, string_table: &StringTable) -> bool {
    string_table
        .resolve(name)
        .chars()
        .next()
        .is_some_and(|first| first.is_uppercase())
}

fn parse_generic_arguments(
    token_stream: &mut DeclarationCursor<'_>,
    parsed_type: ParsedTypeRef,
    context: TypeAnnotationContext,
    string_table: &StringTable,
    allow_generic_application: bool,
) -> TypeParseResult<ParsedTypeRef> {
    let span = current_source_span(token_stream);

    if token_stream.current_tag() != TokenTag::OF {
        return Ok(parsed_type);
    }

    if !allow_generic_application {
        return Err(HeaderParseFailure::Diagnostic(
            nested_generic_application_error(span, context),
        ));
    }
    match parsed_type {
        ParsedTypeRef::This { .. } => {
            return Err(HeaderParseFailure::Diagnostic(
                trait_this_composition_error(context, current_source_span(token_stream)),
            ));
        }
        ParsedTypeRef::Named { .. } => {}
        // Qualified paths such as `io.input.Input` are concrete type references,
        // not generic bases. Generic application on namespace-qualified bases is
        // deliberately deferred until there is a clear need and a resolved generic
        // base policy; for now they fall through to the OnNonNamedType error.
        _ => {
            return Err(HeaderParseFailure::Diagnostic(
                CompilerDiagnostic::invalid_generic_application(
                    GenericApplicationErrorReason::OnNonNamedType,
                    current_source_span(token_stream),
                ),
            ));
        }
    };

    token_stream.advance();

    let mut arguments = Vec::new();
    loop {
        if generic_argument_list_is_finished(token_stream.current_tag()) {
            if arguments.is_empty() {
                return Err(HeaderParseFailure::Diagnostic(
                    CompilerDiagnostic::invalid_generic_application(
                        GenericApplicationErrorReason::EmptyArgumentList,
                        current_source_span(token_stream),
                    ),
                ));
            }
            break;
        }

        let argument = parse_generic_type_argument(token_stream, context, string_table)?;
        arguments.push(argument);

        match token_stream.current_tag() {
            TokenTag::COMMA => {
                token_stream.advance();
                if generic_argument_list_is_finished(token_stream.current_tag()) {
                    return Err(HeaderParseFailure::Diagnostic(
                        CompilerDiagnostic::invalid_generic_application(
                            GenericApplicationErrorReason::MissingArgumentAfterComma,
                            current_source_span(token_stream),
                        ),
                    ));
                }
            }
            token if generic_argument_list_is_finished(token) => break,
            TokenTag::OF => {
                return Err(HeaderParseFailure::Diagnostic(
                    nested_generic_application_error(current_source_span(token_stream), context),
                ));
            }
            _other => {
                let span = current_source_span(token_stream);
                let found = DiagnosticToken::from_token_ref(
                    token_stream
                        .canonical_cursor()
                        .current()
                        .expect("validated declaration cursor token"),
                );
                return Err(HeaderParseFailure::Diagnostic(
                    CompilerDiagnostic::unexpected_token_from_tag(found, span),
                ));
            }
        }
    }

    Ok(ParsedTypeRef::Applied {
        base: Box::new(parsed_type),
        arguments,
        span,
    })
}

fn parse_generic_type_argument(
    token_stream: &mut DeclarationCursor<'_>,
    context: TypeAnnotationContext,
    string_table: &StringTable,
) -> TypeParseResult<ParsedTypeRef> {
    let argument_span = current_source_span(token_stream);
    let parsed_argument = parse_type_atom(token_stream, context, string_table)?;

    reject_trait_this_composition(&parsed_argument, context, argument_span)?;

    if token_stream.current_tag() == TokenTag::OF {
        return Err(HeaderParseFailure::Diagnostic(
            nested_generic_application_error(current_source_span(token_stream), context),
        ));
    }

    Ok(parsed_argument)
}
/// Decide whether a token terminates the generic argument list.
///
/// WHAT: lists the tags that cannot start a generic argument and therefore signal
///      the end of the `of <...>` application.
/// WHY: shared predicate so both the comma-after-argument check and the main loop use the
///      same boundary definition.
fn generic_argument_list_is_finished(tag: TokenTag) -> bool {
    matches!(
        tag,
        TokenTag::ASSIGN
            | TokenTag::NEWLINE
            | TokenTag::COLON
            | TokenTag::TYPE_PARAMETER_BRACKET
            | TokenTag::CLOSE_CURLY
            | TokenTag::BANG
            | TokenTag::QUESTION_MARK
            | TokenTag::EOF
            | TokenTag::END
            | TokenTag::NUMERIC_LITERAL
    )
}

fn nested_generic_application_error(
    span: Option<SourceSpan>,
    _context: TypeAnnotationContext,
) -> CompilerDiagnostic {
    CompilerDiagnostic::invalid_generic_application(
        GenericApplicationErrorReason::NestedApplication,
        span,
    )
}

fn parse_optional_type_suffix(
    token_stream: &mut DeclarationCursor<'_>,
    parsed_type: ParsedTypeRef,
    context: TypeAnnotationContext,
) -> TypeParseResult<ParsedTypeRef> {
    let span = current_source_span(token_stream);

    if token_stream.current_tag() != TokenTag::QUESTION_MARK {
        return Ok(parsed_type);
    }

    reject_trait_this_composition(&parsed_type, context, span)?;

    if matches!(parsed_type, ParsedTypeRef::Optional { .. }) {
        return Err(HeaderParseFailure::Diagnostic(
            CompilerDiagnostic::invalid_type_annotation(
                context,
                InvalidTypeAnnotationReason::DuplicateOptional,
                current_source_span(token_stream),
            ),
        ));
    }

    token_stream.advance();
    if token_stream.current_tag() == TokenTag::QUESTION_MARK {
        return Err(HeaderParseFailure::Diagnostic(
            CompilerDiagnostic::invalid_type_annotation(
                context,
                InvalidTypeAnnotationReason::DuplicateOptional,
                current_source_span(token_stream),
            ),
        ));
    }

    Ok(ParsedTypeRef::Optional {
        inner: Box::new(parsed_type),
        span,
    })
}

/// Recursively check whether a parsed type contains `This` anywhere in its structure.
fn parsed_type_contains_trait_this(parsed_type: &ParsedTypeRef) -> bool {
    match parsed_type {
        ParsedTypeRef::This { .. } => true,
        ParsedTypeRef::Applied {
            base, arguments, ..
        } => {
            parsed_type_contains_trait_this(base)
                || arguments.iter().any(parsed_type_contains_trait_this)
        }
        ParsedTypeRef::Collection { element, .. }
        | ParsedTypeRef::Optional { inner: element, .. } => {
            parsed_type_contains_trait_this(element)
        }
        _ => false,
    }
}

fn reject_trait_this_composition(
    parsed_type: &ParsedTypeRef,
    context: TypeAnnotationContext,
    span: Option<SourceSpan>,
) -> TypeParseResult<()> {
    if parsed_type_contains_trait_this(parsed_type) {
        return Err(HeaderParseFailure::Diagnostic(
            trait_this_composition_error(context, span),
        ));
    }
    Ok(())
}

fn trait_this_composition_error(
    context: TypeAnnotationContext,
    span: Option<SourceSpan>,
) -> CompilerDiagnostic {
    CompilerDiagnostic::invalid_type_annotation(
        context,
        InvalidTypeAnnotationReason::TraitThisMustBeDirect,
        span,
    )
}

fn type_keyword_deferred_error(
    token_stream: &DeclarationCursor<'_>,
    context: TypeAnnotationContext,
) -> CompilerDiagnostic {
    CompilerDiagnostic::invalid_type_annotation(
        context,
        InvalidTypeAnnotationReason::ExpectedTypeAnnotation {
            found: DiagnosticToken::from_static_tag(TokenTag::TYPE),
        },
        current_source_span(token_stream),
    )
}

fn compilation_stage(context: TypeAnnotationContext) -> &'static str {
    match context {
        TypeAnnotationContext::DeclarationTarget => "Variable Declaration",
        TypeAnnotationContext::BuildConfigContract => "Build-Configuration Contract",
        TypeAnnotationContext::SignatureParameter => "Parameter Type Parsing",
        TypeAnnotationContext::SignatureReturn => "Function Signature Parsing",
        TypeAnnotationContext::TypeAliasTarget => "Type Alias Parsing",
        TypeAnnotationContext::TraitRequirement => "Trait Requirement Parsing",
    }
}
fn current_source_span(token_stream: &DeclarationCursor<'_>) -> Option<SourceSpan> {
    token_stream.current_span()
}
