//! Type-annotation parsing for declaration and signature syntax.
//!
//! WHAT: converts token streams into unresolved parsed type references, including narrow
//!      fixed-capacity collection syntax using integer literals or bare constant names.
//! WHY: parsing stays separate from semantic type resolution so header and AST
//!      callers can share syntax without rebuilding type-environment policy here.

use super::*;
use crate::compiler_frontend::datatypes::parsed::ParsedCollectionCapacity;
use crate::compiler_frontend::numeric_text::parse::materialize_i32;
use crate::compiler_frontend::numeric_text::token::{NumericLiteralKind, NumericLiteralSign};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::Token;

/// Boxed diagnostic result for type-annotation parsing.
///
/// WHAT: keeps the connected parser family on one small error boundary.
/// WHY: type parsing otherwise carries the large diagnostic value through every
///      successful declaration and signature parse.
type TypeParseResult<T> = Result<T, Box<CompilerDiagnostic>>;

// -------------------------
//  Type annotation parsing
// -------------------------

/// Parse a type annotation and return the parsed type reference.
///
/// WHAT: produces `ParsedTypeRef` — unresolved parsed syntax, not semantic identity.
/// WHY: resolution into `TypeId` or `DataType` happens later when the environment is available.
pub(crate) fn parse_type_annotation(
    token_stream: &mut FileTokens,
    context: TypeAnnotationContext,
    string_table: &StringTable,
) -> TypeParseResult<ParsedTypeRef> {
    // Only ordinary declaration targets may omit a type. Build-config contracts intentionally use
    // a required context so `#Config of = value` is rejected at the authored `=` token.
    if matches!(context, TypeAnnotationContext::DeclarationTarget)
        && matches!(
            token_stream.current_token_kind(),
            TokenKind::Assign | TokenKind::Newline | TokenKind::Comma
        )
    {
        return Ok(ParsedTypeRef::Inferred);
    }

    parse_required_type(token_stream, context, string_table)
}

fn parse_required_type(
    token_stream: &mut FileTokens,
    context: TypeAnnotationContext,
    string_table: &StringTable,
) -> TypeParseResult<ParsedTypeRef> {
    parse_required_type_with_generic_application(token_stream, context, string_table, true)
}

fn parse_required_type_with_generic_application(
    token_stream: &mut FileTokens,
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
    token_stream: &mut FileTokens,
    context: TypeAnnotationContext,
    string_table: &StringTable,
) -> TypeParseResult<ParsedTypeRef> {
    let location = token_stream.current_location();

    match token_stream.current_token_kind() {
        TokenKind::DatatypeInt => {
            token_stream.advance();
            Ok(ParsedTypeRef::BuiltinInt { location })
        }

        TokenKind::DatatypeFloat => {
            token_stream.advance();
            Ok(ParsedTypeRef::BuiltinFloat { location })
        }

        TokenKind::DatatypeBool => {
            token_stream.advance();
            Ok(ParsedTypeRef::BuiltinBool { location })
        }

        TokenKind::DatatypeString => {
            token_stream.advance();
            Ok(ParsedTypeRef::BuiltinString { location })
        }

        TokenKind::DatatypeChar => {
            token_stream.advance();
            Ok(ParsedTypeRef::BuiltinChar { location })
        }

        TokenKind::DatatypeNone => Err(Box::new(CompilerDiagnostic::invalid_type_annotation(
            context,
            InvalidTypeAnnotationReason::NoneNotAllowed,
            token_stream.current_location(),
        ))),

        TokenKind::Must | TokenKind::TraitThis => {
            if matches!(context, TypeAnnotationContext::TraitRequirement)
                && token_stream.current_token_kind() == &TokenKind::TraitThis
            {
                let location = token_stream.current_location();
                token_stream.advance();
                return Ok(ParsedTypeRef::This { location });
            }

            let _keyword = reserved_trait_keyword_or_dispatch_mismatch(
                token_stream.current_token_kind(),
                token_stream.current_location(),
                compilation_stage(context),
                "type annotation parsing",
            )
            .map_err(|error| Box::new(compiler_error_to_diagnostic(&error)))?;

            Err(Box::new(CompilerDiagnostic::invalid_type_annotation(
                context,
                InvalidTypeAnnotationReason::ReservedTraitKeyword,
                token_stream.current_location(),
            )))
        }

        TokenKind::OpenCurly => parse_collection_type(token_stream, context, string_table),

        TokenKind::Reactive => Err(Box::new(CompilerDiagnostic::invalid_type_annotation(
            context,
            InvalidTypeAnnotationReason::ReactiveAccessNotAllowed,
            token_stream.current_location(),
        ))),

        TokenKind::As => Err(Box::new(CompilerDiagnostic::invalid_type_annotation(
            context,
            InvalidTypeAnnotationReason::AsNotValidHere,
            token_stream.current_location(),
        ))),
        TokenKind::Type => Err(Box::new(type_keyword_deferred_error(token_stream, context))),
        TokenKind::Of => Err(Box::new(CompilerDiagnostic::unexpected_token(
            token_stream.current_token_kind().to_owned(),
            token_stream.current_location(),
        ))),
        TokenKind::Symbol(type_name) => {
            let type_name = *type_name;
            token_stream.advance();

            // Check for namespace-qualified type syntax: `Namespace.Type` or
            // `Namespace.Child.Type`. Collect all `Symbol . Symbol` segments into a
            // single qualified path while preserving bare single-symbol types.
            if token_stream.current_token_kind() == &TokenKind::Dot {
                let mut path = vec![type_name];
                let path_location = location.clone();

                while token_stream.current_token_kind() == &TokenKind::Dot {
                    token_stream.advance(); // consume '.'

                    match token_stream.current_token_kind().to_owned() {
                        TokenKind::Symbol(member_name) => {
                            path.push(member_name);
                            token_stream.advance();
                        }
                        other => {
                            return Err(Box::new(CompilerDiagnostic::invalid_type_annotation(
                                context,
                                InvalidTypeAnnotationReason::ExpectedTypeAnnotation {
                                    found: other,
                                },
                                token_stream.current_location(),
                            )));
                        }
                    }
                }

                return Ok(ParsedTypeRef::Qualified {
                    path,
                    location: path_location,
                });
            }

            Ok(ParsedTypeRef::Named {
                name: type_name,
                location,
            })
        }
        TokenKind::Colon if matches!(context, TypeAnnotationContext::DeclarationTarget) => {
            Err(Box::new(CompilerDiagnostic::invalid_type_annotation(
                context,
                InvalidTypeAnnotationReason::UnexpectedColon,
                token_stream.current_location(),
            )))
        }
        other
            if matches!(context, TypeAnnotationContext::DeclarationTarget)
                && matches!(
                    other,
                    TokenKind::Dot
                        | TokenKind::AddAssign
                        | TokenKind::SubtractAssign
                        | TokenKind::DivideAssign
                        | TokenKind::IntDivideAssign
                        | TokenKind::MultiplyAssign
                ) =>
        {
            Err(Box::new(CompilerDiagnostic::invalid_type_annotation(
                context,
                InvalidTypeAnnotationReason::InvalidTokenAfterName {
                    token: other.to_owned(),
                },
                token_stream.current_location(),
            )))
        }
        _ => Err(Box::new(CompilerDiagnostic::invalid_type_annotation(
            context,
            InvalidTypeAnnotationReason::ExpectedTypeAnnotation {
                found: token_stream.current_token_kind().to_owned(),
            },
            token_stream.current_location(),
        ))),
    }
}

fn parse_type_postfixes(
    token_stream: &mut FileTokens,
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
    token_stream: &mut FileTokens,
    context: TypeAnnotationContext,
    string_table: &StringTable,
) -> TypeParseResult<ParsedTypeRef> {
    let location = token_stream.current_location();
    token_stream.advance(); // consume '{'

    let inner_tokens = collect_collection_inner_tokens(token_stream)?;
    token_stream.advance(); // consume the outer '}'

    if inner_tokens.is_empty() {
        return Ok(ParsedTypeRef::Collection {
            element: Box::new(ParsedTypeRef::Inferred),
            location,
            fixed_capacity: None,
        });
    }

    if let Some(reactive_token) = inner_tokens
        .iter()
        .find(|token| token.kind == TokenKind::Reactive)
    {
        return Err(Box::new(CompilerDiagnostic::invalid_type_annotation(
            context,
            InvalidTypeAnnotationReason::ReactiveAccessNotAllowed,
            reactive_token.location.clone(),
        )));
    }

    // Map type syntax `{K = V}` takes precedence over collection capacity splitting.
    match scan_top_level_assigns(&inner_tokens) {
        TopLevelAssignScan::None => {}
        TopLevelAssignScan::One(assign_idx) => {
            return parse_map_type_from_inner_tokens(
                &inner_tokens,
                assign_idx,
                token_stream,
                context,
                string_table,
                &location,
            );
        }
        TopLevelAssignScan::Multiple => {
            return Err(Box::new(CompilerDiagnostic::invalid_map_type(
                InvalidMapTypeReason::MultipleMapSeparators,
                location,
            )));
        }
    }

    // Collection type parsing keeps capacity syntax narrow and unambiguous:
    //   - If the entire inner content parses as a valid element type, this is `{T}` (growable).
    //   - Otherwise, the first valid element-type suffix separates capacity from element type.
    //   - If no element type suffix is found, capacity-only shorthand is allowed only in
    //     declaration target context (with `Inferred` element type).

    // Try 1: if the contents start as an element type, that type must consume the whole
    // collection body. This keeps old post-element capacity syntax like `{Int 64}` from
    // silently becoming capacity-only shorthand.
    if collection_type_slice_can_start_type(&inner_tokens, context, string_table) {
        let parsed_slice = parse_type_slice(&inner_tokens, token_stream, context, string_table)?;
        if let Some(extra_token) = parsed_slice.next_token {
            return Err(Box::new(CompilerDiagnostic::expected_token(
                TokenKind::CloseCurly,
                Some(extra_token.kind),
                extra_token.location,
            )));
        }

        let element = parsed_slice.parsed_type;
        reject_trait_this_composition(&element, context, location.clone())?;
        return Ok(ParsedTypeRef::Collection {
            element: Box::new(element),
            location,
            fixed_capacity: None,
        });
    }

    // Try 2: find the first valid element type suffix by scanning left-to-right.
    // Tokens before the suffix become the fixed-capacity syntax.
    for split_idx in 1..inner_tokens.len() {
        let type_tokens = &inner_tokens[split_idx..];
        if !collection_type_slice_can_start_type(type_tokens, context, string_table) {
            continue;
        }

        if let Some(element) =
            parse_type_slice_exact(type_tokens, token_stream, context, string_table)
        {
            reject_trait_this_composition(&element, context, location.clone())?;
            return Ok(ParsedTypeRef::Collection {
                element: Box::new(element),
                location,
                fixed_capacity: parsed_capacity(&inner_tokens[..split_idx], string_table)?,
            });
        }
    }

    // Try 3: capacity-only shorthand (declaration target only).
    if matches!(context, TypeAnnotationContext::DeclarationTarget) {
        return Ok(ParsedTypeRef::Collection {
            element: Box::new(ParsedTypeRef::Inferred),
            location,
            fixed_capacity: parsed_capacity(&inner_tokens, string_table)?,
        });
    }

    // No valid element type found in a non-declaration context.
    Err(Box::new(CompilerDiagnostic::invalid_collection_type(
        InvalidCollectionTypeReason::ShorthandCapacityNotAllowed,
        location,
    )))
}

/// Collect all tokens inside a braced type body, tracking nested braces.
///
/// WHAT: gathers tokens between `{` and `}` while counting nested `{`/`}` pairs so inner
///      collection or map types are captured as part of the body.
/// WHY: the caller needs the full inner token slice to decide whether this is a collection,
///      a map, or a capacity-only shorthand.
fn collect_collection_inner_tokens(token_stream: &mut FileTokens) -> TypeParseResult<Vec<Token>> {
    let mut inner_tokens = Vec::new();
    let mut nested_collection_depth = 0usize;

    loop {
        match token_stream.current_token_kind() {
            TokenKind::CloseCurly if nested_collection_depth == 0 => break,
            TokenKind::CloseCurly => {
                nested_collection_depth -= 1;
                inner_tokens.push(token_stream.current_token());
                token_stream.advance();
            }
            TokenKind::OpenCurly => {
                nested_collection_depth += 1;
                inner_tokens.push(token_stream.current_token());
                token_stream.advance();
            }
            TokenKind::Eof => {
                return Err(Box::new(CompilerDiagnostic::expected_token(
                    TokenKind::CloseCurly,
                    Some(TokenKind::Eof),
                    token_stream.current_location(),
                )));
            }
            _ => {
                inner_tokens.push(token_stream.current_token());
                token_stream.advance();
            }
        }
    }

    Ok(inner_tokens)
}

/// WHAT: accepts only a single integer literal or a single bare symbol token.
///       Anything else (arithmetic, calls, field access, floats, etc.) is rejected
///       with a structured diagnostic so users get the narrow-rule message at the
///       syntax site rather than a generic parse failure.
/// WHY: the language only allows literal-or-bare-const capacity in type position;
///      named constants can still hold arithmetic before they are used in type annotations.
fn parsed_capacity(
    tokens: &[Token],
    string_table: &StringTable,
) -> TypeParseResult<Option<ParsedCollectionCapacity>> {
    if tokens.is_empty() {
        return Ok(None);
    }

    if tokens.len() == 1 {
        match &tokens[0].kind {
            TokenKind::NumericLiteral(token) => {
                if token.kind != NumericLiteralKind::WholeNumber {
                    return Err(Box::new(CompilerDiagnostic::invalid_collection_type(
                        InvalidCollectionTypeReason::CapacityNotInt,
                        tokens[0].location.clone(),
                    )));
                }

                let value = materialize_i32(token, string_table).map_err(|reason| {
                    CompilerDiagnostic::invalid_number_literal(
                        token.source_text,
                        reason,
                        tokens[0].location.clone(),
                    )
                })?;

                if token.sign == NumericLiteralSign::Negative {
                    return Err(Box::new(CompilerDiagnostic::invalid_collection_type(
                        InvalidCollectionTypeReason::NegativeCapacity,
                        tokens[0].location.clone(),
                    )));
                }

                return Ok(Some(ParsedCollectionCapacity::Literal {
                    value,
                    location: tokens[0].location.clone(),
                }));
            }
            TokenKind::Symbol(name) => {
                return Ok(Some(ParsedCollectionCapacity::BareConstant {
                    name: *name,
                    location: tokens[0].location.clone(),
                }));
            }
            _ => {}
        }
    }

    let location = tokens[0].location.clone();
    Err(Box::new(CompilerDiagnostic::invalid_collection_type(
        InvalidCollectionTypeReason::CapacityNotConstant,
        location,
    )))
}

// -------------------------
//  Map type parsing helpers
// -------------------------

enum TopLevelAssignScan {
    None,
    One(usize),
    Multiple,
}

/// Scan a token slice for top-level `=` tokens while tracking nested delimiters.
///
/// WHAT: classifies whether the slice has no outer separator, one outer separator, or several.
/// WHY: map type syntax `{K = V}` splits only at the outermost `=`, and nested maps may contain
///      their own separators that must not affect the enclosing type body.
fn scan_top_level_assigns(tokens: &[Token]) -> TopLevelAssignScan {
    let mut depth = 0usize;
    let mut first_assign = None;
    for (idx, token) in tokens.iter().enumerate() {
        match &token.kind {
            TokenKind::OpenCurly | TokenKind::OpenParenthesis => depth += 1,
            TokenKind::CloseCurly | TokenKind::CloseParenthesis => {
                depth = depth.saturating_sub(1);
            }
            TokenKind::Assign if depth == 0 => {
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

/// Parse a map type from the inner tokens collected between `{` and `}`.
///
/// WHAT: splits tokens at the `=` separator, validates exactly one separator, rejects empty
///      sides, and parses both key and value as type annotations.
/// WHY: map syntax is detected before collection syntax so `{K = V}` is never mis-parsed as
///      a fixed collection with capacity.
fn parse_map_type_from_inner_tokens(
    inner_tokens: &[Token],
    assign_idx: usize,
    token_stream: &FileTokens,
    context: TypeAnnotationContext,
    string_table: &StringTable,
    location: &SourceLocation,
) -> TypeParseResult<ParsedTypeRef> {
    let key_tokens = &inner_tokens[..assign_idx];
    let value_tokens = &inner_tokens[assign_idx + 1..];

    if key_tokens.is_empty() {
        return Err(Box::new(CompilerDiagnostic::invalid_map_type(
            InvalidMapTypeReason::EmptyMapKeyType,
            location.clone(),
        )));
    }

    if value_tokens.is_empty() {
        return Err(Box::new(CompilerDiagnostic::invalid_map_type(
            InvalidMapTypeReason::EmptyMapValueType,
            location.clone(),
        )));
    }

    let key = try_parse_map_side(key_tokens, token_stream, context, string_table, location)?;
    let value = try_parse_map_side(value_tokens, token_stream, context, string_table, location)?;

    reject_trait_this_composition(&key, context, location.clone())?;
    reject_trait_this_composition(&value, context, location.clone())?;

    Ok(ParsedTypeRef::Map {
        key: Box::new(key),
        value: Box::new(value),
        location: location.clone(),
    })
}

/// Attempt to parse one side of a map type (`K` or `V`) from a token slice.
///
/// WHAT: tries `parse_type_slice_exact`; on failure, detects fixed-capacity syntax and
///      postfix capacity-like syntax and emits a targeted map diagnostic instead of a generic
///      parse error.
/// WHY: `{4 String = Int}`, `{String = 4 Int}`, and `{String = Int:5}` are common mistakes that
///      deserve the "fixed capacity not allowed on maps" diagnostic.
fn try_parse_map_side(
    tokens: &[Token],
    token_stream: &FileTokens,
    context: TypeAnnotationContext,
    string_table: &StringTable,
    location: &SourceLocation,
) -> TypeParseResult<ParsedTypeRef> {
    if let Some(parsed) = parse_type_slice_exact(tokens, token_stream, context, string_table) {
        return Ok(parsed);
    }

    if map_side_looks_like_fixed_capacity(tokens, token_stream, context, string_table) {
        return Err(Box::new(CompilerDiagnostic::invalid_map_type(
            InvalidMapTypeReason::FixedCapacityNotAllowed,
            location.clone(),
        )));
    }

    if map_side_looks_like_postfix_capacity(tokens, token_stream, context, string_table) {
        return Err(Box::new(CompilerDiagnostic::invalid_map_type(
            InvalidMapTypeReason::FixedCapacityNotAllowed,
            location.clone(),
        )));
    }

    // Fall back to the normal type-slice parser so the user gets the best available error.
    let parsed_slice = parse_type_slice(tokens, token_stream, context, string_table)?;
    if let Some(extra_token) = parsed_slice.next_token {
        return Err(Box::new(CompilerDiagnostic::expected_token(
            TokenKind::CloseCurly,
            Some(extra_token.kind),
            extra_token.location,
        )));
    }

    Ok(parsed_slice.parsed_type)
}

/// Detect prefix capacity-like syntax on a map-type side.
///
/// WHAT: checks whether tokens begin with valid fixed-capacity syntax followed by a valid
///      type, which would indicate the user tried to write fixed-capacity syntax inside a map.
/// WHY: `{4 String = Int}` and similar forms should receive the targeted
///      `FixedCapacityNotAllowed` diagnostic rather than a generic parse error.
fn map_side_looks_like_fixed_capacity(
    tokens: &[Token],
    token_stream: &FileTokens,
    context: TypeAnnotationContext,
    string_table: &StringTable,
) -> bool {
    for split_idx in 1..tokens.len() {
        let type_tokens = &tokens[split_idx..];
        if collection_type_slice_can_start_type(type_tokens, context, string_table)
            && parse_type_slice_exact(type_tokens, token_stream, context, string_table).is_some()
            && matches!(
                parsed_capacity(&tokens[..split_idx], string_table),
                Ok(Some(_))
            )
        {
            return true;
        }
    }

    false
}

/// Detect postfix capacity-like syntax on a map-type side.
///
/// WHAT: checks whether a valid type prefix is followed by trailing tokens that cannot continue
///      a type expression (e.g. `:5` or `5`).
/// WHY: `{String = Int:5}` is an old postfix capacity-like map syntax that should receive the
///      same targeted `FixedCapacityNotAllowed` diagnostic as prefix capacity forms.
fn map_side_looks_like_postfix_capacity(
    tokens: &[Token],
    token_stream: &FileTokens,
    context: TypeAnnotationContext,
    string_table: &StringTable,
) -> bool {
    for split_idx in 1..=tokens.len() {
        let type_tokens = &tokens[..split_idx];
        if !collection_type_slice_can_start_type(type_tokens, context, string_table) {
            continue;
        }
        if parse_type_slice_exact(type_tokens, token_stream, context, string_table).is_some() {
            let remaining = &tokens[split_idx..];
            if remaining.is_empty() {
                continue;
            }
            return matches!(
                remaining.first().map(|t| &t.kind),
                Some(TokenKind::Colon) | Some(TokenKind::NumericLiteral(_))
            );
        }
    }
    false
}

struct ParsedTypeSlice {
    parsed_type: ParsedTypeRef,
    next_token: Option<Token>,
}

/// Parse a collected token slice as a type annotation.
///
/// WHAT: reuses the normal type parser on a temporary token stream instead of maintaining a
///       parallel type parser for collection capacity splitting.
/// WHY: collection syntax needs to detect the element-type suffix while keeping optional
///      suffixes, generic applications, namespaced types, and nested collections on the same
///      parser path as ordinary annotations.
fn parse_type_slice(
    tokens: &[Token],
    outer_stream: &FileTokens,
    context: TypeAnnotationContext,
    string_table: &StringTable,
) -> TypeParseResult<ParsedTypeSlice> {
    let mut slice_tokens = tokens.to_vec();
    let eof_location = tokens
        .last()
        .map(|token| token.location.clone())
        .unwrap_or_else(|| outer_stream.current_location());
    slice_tokens.push(Token::new(TokenKind::Eof, eof_location));

    let mut stream = FileTokens::new_path_free_substream(
        outer_stream.src_path.clone(),
        outer_stream.file_id,
        outer_stream.canonical_os_path.clone(),
        slice_tokens,
    );
    let parsed_type = parse_required_type(&mut stream, context, string_table)?;
    let next_token = if stream.current_token_kind() == &TokenKind::Eof {
        None
    } else {
        Some(stream.current_token())
    };

    Ok(ParsedTypeSlice {
        parsed_type,
        next_token,
    })
}

fn parse_type_slice_exact(
    tokens: &[Token],
    outer_stream: &FileTokens,
    context: TypeAnnotationContext,
    string_table: &StringTable,
) -> Option<ParsedTypeRef> {
    let parsed_slice = parse_type_slice(tokens, outer_stream, context, string_table).ok()?;

    parsed_slice
        .next_token
        .is_none()
        .then_some(parsed_slice.parsed_type)
}

/// Heuristic check: can the leading tokens of a slice start a valid type annotation?
///
/// WHAT: examines the first token (and optionally a `Namespace.Type` prefix) to decide
///      whether the remaining tokens in a braced body could be an element type.
/// WHY: used during the left-to-right scan in `parse_collection_type` to find the boundary
///      between fixed-capacity syntax and element type.
fn collection_type_slice_can_start_type(
    tokens: &[Token],
    context: TypeAnnotationContext,
    string_table: &StringTable,
) -> bool {
    let Some(first) = tokens.first() else {
        return false;
    };

    match &first.kind {
        TokenKind::DatatypeInt
        | TokenKind::DatatypeFloat
        | TokenKind::DatatypeBool
        | TokenKind::DatatypeString
        | TokenKind::DatatypeChar
        | TokenKind::OpenCurly => true,

        TokenKind::TraitThis => matches!(context, TypeAnnotationContext::TraitRequirement),

        TokenKind::Symbol(name) => {
            if tokens.get(1).map(|token| &token.kind) == Some(&TokenKind::Dot)
                && let Some(TokenKind::Symbol(member)) = tokens.get(2).map(|token| &token.kind)
            {
                return symbol_spelling_looks_type_name(*member, string_table);
            }

            symbol_spelling_looks_type_name(*name, string_table)
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
    token_stream: &mut FileTokens,
    parsed_type: ParsedTypeRef,
    context: TypeAnnotationContext,
    string_table: &StringTable,
    allow_generic_application: bool,
) -> TypeParseResult<ParsedTypeRef> {
    let location = token_stream.current_location();
    if token_stream.current_token_kind() != &TokenKind::Of {
        return Ok(parsed_type);
    }

    if !allow_generic_application {
        return Err(Box::new(nested_generic_application_error(
            token_stream.current_location(),
            context,
        )));
    }

    match parsed_type {
        ParsedTypeRef::This { .. } => {
            return Err(Box::new(trait_this_composition_error(
                context,
                token_stream.current_location(),
            )));
        }
        ParsedTypeRef::Named { .. } => {}
        // Qualified paths such as `io.input.Input` are concrete type references,
        // not generic bases. Generic application on namespace-qualified bases is
        // deliberately deferred until there is a clear need and a resolved generic
        // base policy; for now they fall through to the OnNonNamedType error.
        _ => {
            return Err(Box::new(CompilerDiagnostic::invalid_generic_application(
                GenericApplicationErrorReason::OnNonNamedType,
                token_stream.current_location(),
            )));
        }
    };

    token_stream.advance();

    let mut arguments = Vec::new();
    loop {
        if generic_argument_list_is_finished(token_stream.current_token_kind()) {
            if arguments.is_empty() {
                return Err(Box::new(CompilerDiagnostic::invalid_generic_application(
                    GenericApplicationErrorReason::EmptyArgumentList,
                    token_stream.current_location(),
                )));
            }
            break;
        }

        let argument = parse_generic_type_argument(token_stream, context, string_table)?;
        arguments.push(argument);

        match token_stream.current_token_kind() {
            TokenKind::Comma => {
                token_stream.advance();
                if generic_argument_list_is_finished(token_stream.current_token_kind()) {
                    return Err(Box::new(CompilerDiagnostic::invalid_generic_application(
                        GenericApplicationErrorReason::MissingArgumentAfterComma,
                        token_stream.current_location(),
                    )));
                }
            }
            token if generic_argument_list_is_finished(token) => break,
            TokenKind::Of => {
                return Err(Box::new(nested_generic_application_error(
                    token_stream.current_location(),
                    context,
                )));
            }
            other => {
                return Err(Box::new(CompilerDiagnostic::unexpected_token(
                    other.to_owned(),
                    token_stream.current_location(),
                )));
            }
        }
    }

    Ok(ParsedTypeRef::Applied {
        base: Box::new(parsed_type),
        arguments,
        location,
    })
}

fn parse_generic_type_argument(
    token_stream: &mut FileTokens,
    context: TypeAnnotationContext,
    string_table: &StringTable,
) -> TypeParseResult<ParsedTypeRef> {
    let argument_location = token_stream.current_location();
    let parsed_argument = parse_type_atom(token_stream, context, string_table)?;

    reject_trait_this_composition(&parsed_argument, context, argument_location)?;

    if token_stream.current_token_kind() == &TokenKind::Of {
        return Err(Box::new(nested_generic_application_error(
            token_stream.current_location(),
            context,
        )));
    }

    Ok(parsed_argument)
}

/// Decide whether a token terminates the generic argument list.
///
/// WHAT: lists the token kinds that cannot start a generic argument and therefore signal
///      the end of the `of <...>` application.
/// WHY: shared predicate so both the comma-after-argument check and the main loop use the
///      same boundary definition.
fn generic_argument_list_is_finished(token: &TokenKind) -> bool {
    matches!(
        token,
        TokenKind::Assign
            | TokenKind::Newline
            | TokenKind::Colon
            | TokenKind::TypeParameterBracket
            | TokenKind::CloseCurly
            | TokenKind::Bang
            | TokenKind::QuestionMark
            | TokenKind::Eof
            | TokenKind::End
            | TokenKind::NumericLiteral(_)
    )
}

fn nested_generic_application_error(
    location: SourceLocation,
    _context: TypeAnnotationContext,
) -> CompilerDiagnostic {
    CompilerDiagnostic::invalid_generic_application(
        GenericApplicationErrorReason::NestedApplication,
        location,
    )
}

fn parse_optional_type_suffix(
    token_stream: &mut FileTokens,
    parsed_type: ParsedTypeRef,
    context: TypeAnnotationContext,
) -> TypeParseResult<ParsedTypeRef> {
    let location = token_stream.current_location();
    if token_stream.current_token_kind() != &TokenKind::QuestionMark {
        return Ok(parsed_type);
    }

    reject_trait_this_composition(&parsed_type, context, location.clone())?;

    if matches!(parsed_type, ParsedTypeRef::Optional { .. }) {
        return Err(Box::new(CompilerDiagnostic::invalid_type_annotation(
            context,
            InvalidTypeAnnotationReason::DuplicateOptional,
            token_stream.current_location(),
        )));
    }

    token_stream.advance();
    if token_stream.current_token_kind() == &TokenKind::QuestionMark {
        return Err(Box::new(CompilerDiagnostic::invalid_type_annotation(
            context,
            InvalidTypeAnnotationReason::DuplicateOptional,
            token_stream.current_location(),
        )));
    }

    Ok(ParsedTypeRef::Optional {
        inner: Box::new(parsed_type),
        location,
    })
}

/// Recursively check whether a parsed type contains `This` anywhere in its structure.
///
/// WHAT: walks through applied generics, collections, optionals, and results to find
///      a `ParsedTypeRef::This` node.
/// WHY: `This` is only valid as a bare trait requirement; it must not appear nested.
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

        ParsedTypeRef::Result { ok, err, .. } => {
            parsed_type_contains_trait_this(ok) || parsed_type_contains_trait_this(err)
        }

        _ => false,
    }
}

/// Fail if a parsed type contains nested `This`, emitting the appropriate diagnostic.
///
/// WHAT: delegates to `parsed_type_contains_trait_this` and turns a positive result into
///      `InvalidTypeAnnotationReason::TraitThisMustBeDirect`.
/// WHY: centralizes the composition check so callers in map, collection, optional, and
///      generic paths share the same error message.
fn reject_trait_this_composition(
    parsed_type: &ParsedTypeRef,
    context: TypeAnnotationContext,
    location: SourceLocation,
) -> TypeParseResult<()> {
    if parsed_type_contains_trait_this(parsed_type) {
        return Err(Box::new(trait_this_composition_error(context, location)));
    }
    Ok(())
}

fn trait_this_composition_error(
    context: TypeAnnotationContext,
    location: SourceLocation,
) -> CompilerDiagnostic {
    CompilerDiagnostic::invalid_type_annotation(
        context,
        InvalidTypeAnnotationReason::TraitThisMustBeDirect,
        location,
    )
}

fn type_keyword_deferred_error(
    token_stream: &FileTokens,
    context: TypeAnnotationContext,
) -> CompilerDiagnostic {
    CompilerDiagnostic::invalid_type_annotation(
        context,
        InvalidTypeAnnotationReason::ExpectedTypeAnnotation {
            found: TokenKind::Type,
        },
        token_stream.current_location(),
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
