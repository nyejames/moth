//! Collection and map literal parsing helpers.
//!
//! WHAT: parses `{...}` literals into `ExpressionKind::Collection` or
//!       `ExpressionKind::MapLiteral` during AST construction.
//! WHY: curly-brace syntax introduces both homogeneous collections and ordered maps;
//!      the parser must share the normal expression parser for item/key/value type-checking.

use std::collections::HashMap;

use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::const_eval::{ConstStringRequirement, require_concrete_text};
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression::{
    CollectionExpressionType, Expression, MapLiteralEntry, MapLiteralExpressionType,
};
use crate::compiler_frontend::ast::expressions::expression_kind::ExpressionKind;
use crate::compiler_frontend::ast::expressions::parse_expression::{
    create_expression_until, create_expression_with_trailing_newline_policy,
};
use crate::compiler_frontend::ast::expressions::parse_expression_input::{
    ExpressionParseInput, ExpressionParseResources,
};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::ast::type_resolution::validate_map_key_type;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidCollectionTypeReason, InvalidMapLiteralReason, TypeMismatchContext,
};
use crate::compiler_frontend::datatypes::diagnostic_type_spelling;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::syntax_errors::expression_position::check_expression_common_mistake;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, SourceLocation, TokenKind};
use crate::compiler_frontend::type_coercion::contextual::coerce_expression_to_explicit_type_boundary;
use crate::compiler_frontend::type_coercion::parse_context::{
    CastTargetContext, ExpectedCollectionContext, ExpectedCurlyLiteralContext, ExpectedMapContext,
    ExpectedType, cast_target_context_for_type_id, parse_expectation_for_type_id,
};
use crate::compiler_frontend::value_mode::{ValueMode, ValueMode::MutableOwned};

/// Stage-local result for collection and map literal parsing.
///
/// WHY: collection entries delegate to the ordinary expression parser, whose retained-data
///      failures must stay in the infrastructure lane instead of being rebuilt as source
///      diagnostics at this syntax boundary.
type CollectionParseResult<T> = Result<T, ExpressionParseError>;

/// Entry point for parsing a `{...}` collection literal with homogeneous items.
pub fn new_collection(
    token_stream: &mut FileTokens,
    collection_context: ExpectedCollectionContext,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    value_mode: &ValueMode,
    string_table: &mut StringTable,
) -> CollectionParseResult<Expression> {
    parse_collection_literal(
        token_stream,
        collection_context,
        context,
        type_interner,
        value_mode,
        string_table,
    )
}

/// Entry point for parsing a `{...}` literal that may be a collection or a map.
pub fn new_curly_literal(
    token_stream: &mut FileTokens,
    curly_context: ExpectedCurlyLiteralContext,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    value_mode: &ValueMode,
    string_table: &mut StringTable,
) -> CollectionParseResult<Expression> {
    match curly_context {
        ExpectedCurlyLiteralContext::Collection(collection_context) => parse_collection_literal(
            token_stream,
            collection_context,
            context,
            type_interner,
            value_mode,
            string_table,
        ),
        ExpectedCurlyLiteralContext::Map(map_context) => parse_map_literal(
            token_stream,
            map_context,
            context,
            type_interner,
            value_mode,
            string_table,
        ),
        ExpectedCurlyLiteralContext::Infer => parse_inferred_curly_literal(
            token_stream,
            context,
            type_interner,
            value_mode,
            string_table,
        ),
    }
}

fn parse_collection_literal(
    token_stream: &mut FileTokens,
    collection_context: ExpectedCollectionContext,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    value_mode: &ValueMode,
    string_table: &mut StringTable,
) -> CollectionParseResult<Expression> {
    let mut items: Vec<Expression> = Vec::new();
    let mut inner_type_spelling = None;
    let collection_location = token_stream.current_location();
    let mut consumed_close_curly = false;

    let (mut inferred_inner_type_id, explicit_collection_type_id, fixed_capacity) =
        match collection_context {
            ExpectedCollectionContext::Explicit {
                collection_type_id,
                element_type_id,
                fixed_capacity: capacity,
            } => (Some(element_type_id), Some(collection_type_id), capacity),
            ExpectedCollectionContext::CapacityOnlyShorthand {
                fixed_capacity: capacity,
            } => (None, None, Some(capacity)),
        };

    // The current token is an open curly brace; skip to the first value.
    token_stream.advance();

    let mut awaiting_item = true;

    // ------------------------
    //  Parse collection items
    // ------------------------
    while token_stream.index < token_stream.length {
        match token_stream.current_token_kind() {
            TokenKind::CloseCurly => {
                consumed_close_curly = true;
                break;
            }

            TokenKind::Newline => {
                token_stream.advance();
            }

            TokenKind::Comma => {
                if awaiting_item {
                    return Err(Box::new(CompilerDiagnostic::missing_collection_item(
                        token_stream.current_location(),
                    ))
                    .into());
                }

                awaiting_item = true;
                token_stream.advance();
            }

            _ => {
                if !awaiting_item {
                    return Err(Box::new(CompilerDiagnostic::missing_collection_item(
                        token_stream.current_location(),
                    ))
                    .into());
                }

                let mut expression_type = match inferred_inner_type_id {
                    Some(type_id) => {
                        parse_expectation_for_type_id(type_id, type_interner.environment())
                    }
                    None => ExpectedType::Infer,
                };
                let mut cast_target_context = match inferred_inner_type_id {
                    Some(type_id) => cast_target_context_for_type_id(
                        type_id,
                        type_interner.environment(),
                        string_table,
                    ),
                    None => CastTargetContext::None,
                };
                let expression_expected_types = inferred_inner_type_id
                    .map(|type_id| vec![type_id])
                    .unwrap_or_default();
                let mut expression_context =
                    context.new_child_expression(expression_expected_types);

                // Propagate the parent context kind so that constant-context
                // restrictions apply to expressions inside collection literals.
                expression_context.kind = context.kind.clone();

                let parsed_item = parse_expression_until_curly_entry_delimiter(
                    token_stream,
                    &expression_context,
                    type_interner,
                    &mut expression_type,
                    &mut cast_target_context,
                    string_table,
                )?;

                // A trailing `=` means the literal is mixing collection and map syntax.
                if let Some(error) = mixed_collection_entry_error(token_stream) {
                    return Err(error.into());
                }

                // Get the inferred element type from the first item if no explicit type was given.
                let item_type_id = parsed_item.type_id;
                let expected_item_type_id = inferred_inner_type_id.get_or_insert(item_type_id);

                let coerced_item = coerce_expression_to_explicit_type_boundary(
                    parsed_item,
                    *expected_item_type_id,
                    type_interner.environment(),
                    context,
                    TypeMismatchContext::CollectionElement,
                )?;

                // Capture the diagnostic type spelling from the first item
                // so the collection expression can report its element type.
                if inner_type_spelling.is_none() {
                    inner_type_spelling = Some(coerced_item.diagnostic_type.to_owned());
                }

                items.push(coerced_item);

                awaiting_item = false;
            }
        }
    }

    if !consumed_close_curly {
        return Err(Box::new(CompilerDiagnostic::missing_closing_delimiter(
            string_table.get_or_intern("}".to_owned()),
            token_stream.current_location(),
        ))
        .into());
    }

    let Some(inner_type_id) = inferred_inner_type_id else {
        match collection_context {
            ExpectedCollectionContext::CapacityOnlyShorthand { .. } => {
                return Err(Box::new(CompilerDiagnostic::invalid_collection_type(
                    InvalidCollectionTypeReason::ShorthandEmptyLiteralAmbiguous,
                    collection_location,
                ))
                .into());
            }
            _ => {
                return Err(
                    Box::new(CompilerDiagnostic::empty_collection_type_ambiguity(
                        collection_location,
                    ))
                    .into(),
                );
            }
        }
    };

    if let Some(capacity) = fixed_capacity
        && items.len() > capacity
    {
        return Err(Box::new(CompilerDiagnostic::invalid_collection_type(
            InvalidCollectionTypeReason::InitializerExceedsFixedCapacity {
                capacity,
                length: items.len(),
            },
            collection_location,
        ))
        .into());
    }

    let inner_type = inner_type_spelling.unwrap_or_else(|| {
        let type_environment = type_interner.environment();
        diagnostic_type_spelling(inner_type_id, type_environment)
    });

    Ok(Expression::collection_with_type_id(
        items,
        CollectionExpressionType {
            element_type_id: inner_type_id,
            element_diagnostic_type: inner_type,
            fixed_capacity,
            collection_type_id: explicit_collection_type_id,
        },
        type_interner.environment_mut_for_derived_types(),
        token_stream.current_location(),
        value_mode.to_owned(),
    ))
}

// ------------------------
//  Map literal parsing
// ------------------------

/// Foldable literal keys that can be checked for duplicates at parse time.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum KnownMapKey {
    String(StringId),
    Int(i32),
    Bool(bool),
    Char(char),
}

/// Tracks scalar literal keys that have already appeared so duplicate keys
/// can be diagnosed at parse time.
type KnownMapKeys = HashMap<KnownMapKey, SourceLocation>;

/// Attempt to extract a `KnownMapKey` from a coerced key expression so
/// duplicate-key detection can run for literal scalars.
fn try_extract_known_map_key(
    key: &Expression,
    string_table: &mut StringTable,
) -> CollectionParseResult<Option<KnownMapKey>> {
    let known_key = match &key.kind {
        ExpressionKind::StringSlice(_) | ExpressionKind::StructuralString { .. } => {
            // A key whose final text is still unresolved is simply not knowable here, exactly like
            // a runtime operand. The constant-context check above owns the refusal, so duplicate
            // detection must not raise a second one from this collector.
            let Ok(Some(id)) = require_concrete_text(
                key,
                ConstStringRequirement::DuplicateKeyValidation,
                string_table,
            ) else {
                return Ok(None);
            };
            KnownMapKey::String(id)
        }
        ExpressionKind::Int(v) => KnownMapKey::Int(*v),
        ExpressionKind::Bool(v) => KnownMapKey::Bool(*v),
        ExpressionKind::Char(v) => KnownMapKey::Char(*v),
        _ => return Ok(None),
    };

    Ok(Some(known_key))
}

/// Require concrete text before a string enters compile-time map-key handling.
///
/// WHAT: refuses a resource-bearing or site-root key only inside a constant context.
/// WHY: a constant map must have its complete key set at compile time, so an unresolved key is a
/// real error there. An ordinary runtime map may be keyed by a structural string, whose text the
/// build resolves later, so rejecting it would remove legal expressive power; such a key is merely
/// unknowable for compile-time duplicate detection.
fn validate_compile_time_map_key(
    key: &Expression,
    context: &ScopeContext,
    string_table: &mut StringTable,
) -> CollectionParseResult<()> {
    if context.kind.is_constant_context()
        && matches!(
            &key.kind,
            ExpressionKind::StringSlice(_) | ExpressionKind::StructuralString { .. }
        )
    {
        require_concrete_text(key, ConstStringRequirement::CompileTimeMapKey, string_table)
            .map_err(ExpressionParseError::from)?;
    }

    Ok(())
}

fn record_known_map_key(
    known_keys: &mut KnownMapKeys,
    key: &Expression,
    string_table: &mut StringTable,
) -> CollectionParseResult<()> {
    let Some(known_key) = try_extract_known_map_key(key, string_table)? else {
        return Ok(());
    };

    if known_keys.insert(known_key, key.location.clone()).is_some() {
        return Err(Box::new(CompilerDiagnostic::invalid_map_literal(
            InvalidMapLiteralReason::DuplicateKnownKey,
            key.location.clone(),
        ))
        .into());
    }

    Ok(())
}

fn parse_expression_until_curly_entry_delimiter(
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expected_type: &mut ExpectedType,
    cast_target_context: &mut CastTargetContext,
    string_table: &mut StringTable,
) -> CollectionParseResult<Expression> {
    let input = ExpressionParseInput::without_boundary_catch(
        ExpressionParseResources {
            token_stream,
            scope_context: context,
            type_interner,
            expected_type,
            cast_target_context,
            value_mode: &MutableOwned,
            string_table,
        },
        false,
    );
    create_expression_until(
        input,
        &[TokenKind::Assign, TokenKind::Comma, TokenKind::CloseCurly],
    )
}

fn mixed_collection_entry_error(token_stream: &FileTokens) -> Option<CompilerDiagnostic> {
    if token_stream.current_token_kind() != &TokenKind::Assign {
        return None;
    }

    if token_stream.peek_next_token() == Some(&TokenKind::Assign) {
        return check_expression_common_mistake(token_stream, false);
    }

    Some(CompilerDiagnostic::invalid_map_literal(
        InvalidMapLiteralReason::MixedCollectionMapEntries,
        token_stream.current_location(),
    ))
}

fn consume_map_entry_separator(token_stream: &mut FileTokens) -> CollectionParseResult<()> {
    if token_stream.current_token_kind() != &TokenKind::Assign {
        return Err(Box::new(CompilerDiagnostic::invalid_map_literal(
            InvalidMapLiteralReason::MixedCollectionMapEntries,
            token_stream.current_location(),
        ))
        .into());
    }

    if token_stream.peek_next_token() == Some(&TokenKind::Assign)
        && let Some(error) = check_expression_common_mistake(token_stream, false)
    {
        return Err(error.into());
    }

    token_stream.advance();
    Ok(())
}

fn reject_missing_map_key_expression(token_stream: &FileTokens) -> CollectionParseResult<()> {
    if token_stream.current_token_kind() == &TokenKind::Assign {
        return Err(Box::new(CompilerDiagnostic::invalid_map_literal(
            InvalidMapLiteralReason::MissingKeyExpression,
            token_stream.current_location(),
        ))
        .into());
    }

    Ok(())
}

fn reject_missing_map_value_expression(token_stream: &mut FileTokens) -> CollectionParseResult<()> {
    while token_stream.current_token_kind() == &TokenKind::Newline {
        token_stream.advance();
    }

    match token_stream.current_token_kind() {
        TokenKind::CloseCurly | TokenKind::Comma => {
            Err(Box::new(CompilerDiagnostic::invalid_map_literal(
                InvalidMapLiteralReason::MissingValueExpression,
                token_stream.current_location(),
            ))
            .into())
        }
        _ => Ok(()),
    }
}

fn parse_map_literal(
    token_stream: &mut FileTokens,
    map_context: ExpectedMapContext,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    value_mode: &ValueMode,
    string_table: &mut StringTable,
) -> CollectionParseResult<Expression> {
    let ExpectedMapContext {
        key_type_id,
        value_type_id,
        key_diagnostic_type,
        value_diagnostic_type,
        map_type_id,
    } = map_context;
    let mut entries: Vec<MapLiteralEntry> = Vec::new();
    let mut known_keys: KnownMapKeys = HashMap::new();
    let mut consumed_close_curly = false;

    // The current token is an open curly brace; skip to the first entry.
    token_stream.advance();

    let mut awaiting_entry = true;

    // ------------------------
    //  Parse map entries
    // ------------------------
    while token_stream.index < token_stream.length {
        match token_stream.current_token_kind() {
            TokenKind::CloseCurly => {
                consumed_close_curly = true;
                break;
            }

            TokenKind::Newline => {
                token_stream.advance();
            }

            TokenKind::Comma => {
                if awaiting_entry {
                    return Err(Box::new(CompilerDiagnostic::missing_collection_item(
                        token_stream.current_location(),
                    ))
                    .into());
                }

                awaiting_entry = true;
                token_stream.advance();
            }

            _ => {
                if !awaiting_entry {
                    return Err(Box::new(CompilerDiagnostic::missing_collection_item(
                        token_stream.current_location(),
                    ))
                    .into());
                }
                reject_missing_map_key_expression(token_stream)?;

                let mut key_expression_type =
                    parse_expectation_for_type_id(key_type_id, type_interner.environment());
                let mut key_cast_target_context = cast_target_context_for_type_id(
                    key_type_id,
                    type_interner.environment(),
                    string_table,
                );
                let key_expected_types = vec![key_type_id];
                let mut key_context = context.new_child_expression(key_expected_types);
                key_context.kind = context.kind.clone();

                let parsed_key = parse_expression_until_curly_entry_delimiter(
                    token_stream,
                    &key_context,
                    type_interner,
                    &mut key_expression_type,
                    &mut key_cast_target_context,
                    string_table,
                )?;

                // Top-level `=` separates key from value. Check `==` before consuming
                // the separator so equality common-mistake diagnostics remain specific.
                consume_map_entry_separator(token_stream)?;
                reject_missing_map_value_expression(token_stream)?;

                let mut value_expression_type =
                    parse_expectation_for_type_id(value_type_id, type_interner.environment());
                let mut value_cast_target_context = cast_target_context_for_type_id(
                    value_type_id,
                    type_interner.environment(),
                    string_table,
                );
                let value_expected_types = vec![value_type_id];
                let mut value_context = context.new_child_expression(value_expected_types);
                value_context.kind = context.kind.clone();

                let input = ExpressionParseInput::ordinary(
                    ExpressionParseResources {
                        token_stream,
                        scope_context: &value_context,
                        type_interner,
                        expected_type: &mut value_expression_type,
                        cast_target_context: &mut value_cast_target_context,
                        value_mode: &MutableOwned,
                        string_table,
                    },
                    false,
                );
                let parsed_value = create_expression_with_trailing_newline_policy(input)?;

                let coerced_key = coerce_expression_to_explicit_type_boundary(
                    parsed_key,
                    key_type_id,
                    type_interner.environment(),
                    context,
                    TypeMismatchContext::CollectionElement,
                )?;

                let coerced_value = coerce_expression_to_explicit_type_boundary(
                    parsed_value,
                    value_type_id,
                    type_interner.environment(),
                    context,
                    TypeMismatchContext::CollectionElement,
                )?;

                // Enforce the scalar-key policy once the key type is known.
                validate_map_key_type(
                    key_type_id,
                    type_interner.environment(),
                    &coerced_key.location,
                )?;
                validate_compile_time_map_key(&coerced_key, context, string_table)?;

                // Detect duplicate known keys after coercion where cheaply knowable.
                record_known_map_key(&mut known_keys, &coerced_key, string_table)?;

                entries.push(MapLiteralEntry {
                    key: coerced_key,
                    value: coerced_value,
                });

                awaiting_entry = false;
            }
        }
    }

    if !consumed_close_curly {
        return Err(Box::new(CompilerDiagnostic::missing_closing_delimiter(
            string_table.get_or_intern("}".to_owned()),
            token_stream.current_location(),
        ))
        .into());
    }

    let map_type_id = map_type_id.unwrap_or_else(|| {
        type_interner
            .environment_mut_for_derived_types()
            .intern_map(key_type_id, value_type_id)
    });

    Ok(Expression::map_literal_with_type_id(
        entries,
        MapLiteralExpressionType {
            key_type_id,
            value_type_id,
            key_diagnostic_type,
            value_diagnostic_type,
            map_type_id: Some(map_type_id),
        },
        type_interner.environment_mut_for_derived_types(),
        token_stream.current_location(),
        value_mode.to_owned(),
    ))
}

// ------------------------
//  Inferred curly literal parsing
// ------------------------

fn parse_inferred_curly_literal(
    token_stream: &mut FileTokens,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    value_mode: &ValueMode,
    string_table: &mut StringTable,
) -> CollectionParseResult<Expression> {
    let literal_location = token_stream.current_location();

    // Collection delimiters own intervening newlines; normalize them before the bounded
    // expression parser sees the first entry as a possible statement terminator.
    token_stream.advance();
    token_stream.skip_newlines();

    // Empty inferred `{}` keeps existing collection ambiguity behavior.
    if token_stream.current_token_kind() == &TokenKind::CloseCurly {
        token_stream.advance();
        return Err(
            Box::new(CompilerDiagnostic::empty_collection_type_ambiguity(
                literal_location,
            ))
            .into(),
        );
    }
    reject_missing_map_key_expression(token_stream)?;

    // Parse the first expression to classify the literal shape.
    let mut first_expression_type = ExpectedType::Infer;
    let mut first_cast_target_context = CastTargetContext::None;
    let first_expected_types: Vec<TypeId> = Vec::new();
    let mut first_context = context.new_child_expression(first_expected_types);
    first_context.kind = context.kind.clone();

    let first_expr = parse_expression_until_curly_entry_delimiter(
        token_stream,
        &first_context,
        type_interner,
        &mut first_expression_type,
        &mut first_cast_target_context,
        string_table,
    )?;

    match token_stream.current_token_kind() {
        // First entry has `=`  =>  this is a map literal.
        TokenKind::Assign => {
            consume_map_entry_separator(token_stream)?;
            reject_missing_map_value_expression(token_stream)?;

            // Parse the value side of the first map entry.
            let mut first_value_type = ExpectedType::Infer;
            let mut first_value_cast_target_context = CastTargetContext::None;
            let input = ExpressionParseInput::ordinary(
                ExpressionParseResources {
                    token_stream,
                    scope_context: &first_context,
                    type_interner,
                    expected_type: &mut first_value_type,
                    cast_target_context: &mut first_value_cast_target_context,
                    value_mode: &MutableOwned,
                    string_table,
                },
                false,
            );
            let first_value = create_expression_with_trailing_newline_policy(input)?;

            let key_type_id = first_expr.type_id;
            let value_type_id = first_value.type_id;

            let coerced_key = coerce_expression_to_explicit_type_boundary(
                first_expr,
                key_type_id,
                type_interner.environment(),
                context,
                TypeMismatchContext::CollectionElement,
            )?;

            let coerced_value = coerce_expression_to_explicit_type_boundary(
                first_value,
                value_type_id,
                type_interner.environment(),
                context,
                TypeMismatchContext::CollectionElement,
            )?;

            validate_map_key_type(
                key_type_id,
                type_interner.environment(),
                &coerced_key.location,
            )?;
            validate_compile_time_map_key(&coerced_key, context, string_table)?;

            let mut entries = vec![MapLiteralEntry {
                key: coerced_key,
                value: coerced_value,
            }];
            let mut known_keys: KnownMapKeys = HashMap::new();
            record_known_map_key(&mut known_keys, &entries[0].key, string_table)?;

            // Parse remaining map entries.
            let mut awaiting_entry = false;
            while token_stream.index < token_stream.length {
                match token_stream.current_token_kind() {
                    TokenKind::CloseCurly => {
                        break;
                    }
                    TokenKind::Newline => {
                        token_stream.advance();
                    }
                    TokenKind::Comma => {
                        if awaiting_entry {
                            return Err(Box::new(CompilerDiagnostic::missing_collection_item(
                                token_stream.current_location(),
                            ))
                            .into());
                        }
                        awaiting_entry = true;
                        token_stream.advance();
                    }
                    _ => {
                        if !awaiting_entry {
                            return Err(Box::new(CompilerDiagnostic::missing_collection_item(
                                token_stream.current_location(),
                            ))
                            .into());
                        }
                        reject_missing_map_key_expression(token_stream)?;

                        let mut key_expression_type =
                            parse_expectation_for_type_id(key_type_id, type_interner.environment());
                        let mut key_cast_target_context = cast_target_context_for_type_id(
                            key_type_id,
                            type_interner.environment(),
                            string_table,
                        );
                        let key_expected_types = vec![key_type_id];
                        let mut key_context = context.new_child_expression(key_expected_types);
                        key_context.kind = context.kind.clone();

                        let parsed_key = parse_expression_until_curly_entry_delimiter(
                            token_stream,
                            &key_context,
                            type_interner,
                            &mut key_expression_type,
                            &mut key_cast_target_context,
                            string_table,
                        )?;

                        consume_map_entry_separator(token_stream)?;
                        reject_missing_map_value_expression(token_stream)?;

                        let mut value_expression_type = parse_expectation_for_type_id(
                            value_type_id,
                            type_interner.environment(),
                        );
                        let mut value_cast_target_context = cast_target_context_for_type_id(
                            value_type_id,
                            type_interner.environment(),
                            string_table,
                        );
                        let value_expected_types = vec![value_type_id];
                        let mut value_context = context.new_child_expression(value_expected_types);
                        value_context.kind = context.kind.clone();

                        let input = ExpressionParseInput::ordinary(
                            ExpressionParseResources {
                                token_stream,
                                scope_context: &value_context,
                                type_interner,
                                expected_type: &mut value_expression_type,
                                cast_target_context: &mut value_cast_target_context,
                                value_mode: &MutableOwned,
                                string_table,
                            },
                            false,
                        );
                        let parsed_value = create_expression_with_trailing_newline_policy(input)?;

                        let coerced_key = coerce_expression_to_explicit_type_boundary(
                            parsed_key,
                            key_type_id,
                            type_interner.environment(),
                            context,
                            TypeMismatchContext::CollectionElement,
                        )?;

                        let coerced_value = coerce_expression_to_explicit_type_boundary(
                            parsed_value,
                            value_type_id,
                            type_interner.environment(),
                            context,
                            TypeMismatchContext::CollectionElement,
                        )?;

                        validate_map_key_type(
                            key_type_id,
                            type_interner.environment(),
                            &coerced_key.location,
                        )?;
                        validate_compile_time_map_key(&coerced_key, context, string_table)?;

                        record_known_map_key(&mut known_keys, &coerced_key, string_table)?;

                        entries.push(MapLiteralEntry {
                            key: coerced_key,
                            value: coerced_value,
                        });

                        awaiting_entry = false;
                    }
                }
            }

            let map_type_id = type_interner
                .environment_mut_for_derived_types()
                .intern_map(key_type_id, value_type_id);

            Ok(Expression::map_literal_with_type_id(
                entries,
                MapLiteralExpressionType {
                    key_type_id,
                    value_type_id,
                    key_diagnostic_type: diagnostic_type_spelling(
                        key_type_id,
                        type_interner.environment(),
                    ),
                    value_diagnostic_type: diagnostic_type_spelling(
                        value_type_id,
                        type_interner.environment(),
                    ),
                    map_type_id: Some(map_type_id),
                },
                type_interner.environment_mut_for_derived_types(),
                token_stream.current_location(),
                value_mode.to_owned(),
            ))
        }

        // First expression is followed by `,` or `}`  =>  collection literal.
        TokenKind::Comma | TokenKind::CloseCurly => {
            // Infer element type from the first expression and treat the rest as collection items.
            let element_type_id = first_expr.type_id;
            let coerced_first = coerce_expression_to_explicit_type_boundary(
                first_expr,
                element_type_id,
                type_interner.environment(),
                context,
                TypeMismatchContext::CollectionElement,
            )?;

            let inner_type = coerced_first.diagnostic_type.to_owned();
            let mut items = vec![coerced_first];
            let mut awaiting_item = false;

            while token_stream.index < token_stream.length {
                match token_stream.current_token_kind() {
                    TokenKind::CloseCurly => {
                        break;
                    }
                    TokenKind::Newline => {
                        token_stream.advance();
                    }
                    TokenKind::Comma => {
                        if awaiting_item {
                            return Err(Box::new(CompilerDiagnostic::missing_collection_item(
                                token_stream.current_location(),
                            ))
                            .into());
                        }
                        awaiting_item = true;
                        token_stream.advance();
                    }
                    _ => {
                        if !awaiting_item {
                            return Err(Box::new(CompilerDiagnostic::missing_collection_item(
                                token_stream.current_location(),
                            ))
                            .into());
                        }

                        let mut expression_type = parse_expectation_for_type_id(
                            element_type_id,
                            type_interner.environment(),
                        );
                        let mut cast_target_context = cast_target_context_for_type_id(
                            element_type_id,
                            type_interner.environment(),
                            string_table,
                        );
                        let expression_expected_types = vec![element_type_id];
                        let mut expression_context =
                            context.new_child_expression(expression_expected_types);
                        expression_context.kind = context.kind.clone();

                        let parsed_item = parse_expression_until_curly_entry_delimiter(
                            token_stream,
                            &expression_context,
                            type_interner,
                            &mut expression_type,
                            &mut cast_target_context,
                            string_table,
                        )?;

                        if let Some(error) = mixed_collection_entry_error(token_stream) {
                            return Err(error.into());
                        }

                        let coerced_item = coerce_expression_to_explicit_type_boundary(
                            parsed_item,
                            element_type_id,
                            type_interner.environment(),
                            context,
                            TypeMismatchContext::CollectionElement,
                        )?;

                        items.push(coerced_item);
                        awaiting_item = false;
                    }
                }
            }

            let collection_type_id = type_interner
                .environment_mut_for_derived_types()
                .intern_collection(element_type_id, None);

            Ok(Expression::collection_with_type_id(
                items,
                CollectionExpressionType {
                    element_type_id,
                    element_diagnostic_type: inner_type,
                    fixed_capacity: None,
                    collection_type_id: Some(collection_type_id),
                },
                type_interner.environment_mut_for_derived_types(),
                token_stream.current_location(),
                value_mode.to_owned(),
            ))
        }

        _ => {
            // Any other token after the first expression is invalid in both shapes.
            Err(Box::new(CompilerDiagnostic::missing_collection_item(
                token_stream.current_location(),
            ))
            .into())
        }
    }
}

#[cfg(test)]
#[path = "tests/collections_tests.rs"]
mod collections_tests;
