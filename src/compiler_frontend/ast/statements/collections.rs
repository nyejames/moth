//! Collection and map literal parsing helpers.
//!
//! WHAT: parses `{...}` literals into `ExpressionOp::Collection` or
//!       `ExpressionOp::Map` during AST construction. A literal whose entries are still
//!       pending numeric fragments is returned as a deferred literal for body-root completion.
//! WHY: curly-brace syntax introduces both homogeneous collections and ordered maps; the parser
//!      must share the normal expression parser for item/key/value type-checking, and a literal
//!      that cannot be typed yet must keep its numeric entries retained instead of guessing a type.

use std::collections::HashMap;

use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::const_eval::{ConstStringRequirement, require_concrete_text};
use crate::compiler_frontend::ast::const_values::store::{ConstValuePayload, ConstValueStore};
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::expressions::error::{ExpressionParseError, NumericDeferral};
use crate::compiler_frontend::ast::expressions::expression::{
    CollectionExpressionType, MapLiteralExpressionType,
};
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExprId, ExpressionOperationView, ExpressionView, MapEntry, OwnedExpr,
};
use crate::compiler_frontend::ast::expressions::parse_expression::{
    create_expression_until, create_expression_with_trailing_newline_policy,
};
use crate::compiler_frontend::ast::expressions::parse_expression_input::{
    ExpressionParseInput, ExpressionParseResources,
};
use crate::compiler_frontend::ast::numeric_construction::{
    DeferredCollectionLiteral, DeferredExpression, DeferredExpressionCompletion,
    DeferredLiteralValue, DeferredMapEntry, DeferredMapLiteral, DeferredNumericExpression,
    deferred_numeric_expression,
};
use crate::compiler_frontend::ast::numeric_known_shape::{
    KnownShape, KnownShapeId, fragment_shape, materialise_known_shape, register_shape_witness,
    shape_has_forward_leaf, shapes_join_refused, union_shape_classes,
};
use crate::compiler_frontend::ast::statements::fallible_handling::apply_completed_handling_suffix;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::ast::type_resolution::validate_map_key_type;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidCollectionTypeReason, InvalidMapLiteralReason, TypeMismatchContext,
    ValueReceiverContext,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::syntax_errors::expression_position::check_expression_common_mistake;
use crate::compiler_frontend::tokenizer::tokens::TokenTag;
use crate::compiler_frontend::type_coercion::contextual::coerce_expression_to_explicit_type_boundary;
use crate::compiler_frontend::type_coercion::parse_context::{
    CastTargetContext, ExpectedCollectionContext, ExpectedCurlyLiteralContext, ExpectedMapContext,
    ExpectedType, cast_target_context_for_type_id, parse_expectation_for_type_id,
};
use crate::compiler_frontend::value_mode::ValueMode;
use moth_lexical::numeric::fixed_scalar::FixedScalarValue;

/// Stage-local result for collection and map literal parsing.
///
/// WHY: collection entries delegate to the ordinary expression parser, whose retained-data
///      failures must stay in the infrastructure lane instead of being rebuilt as source
///      diagnostics at this syntax boundary.
type CollectionParseResult<T> = Result<T, ExpressionParseError>;

/// A parsed `{...}` literal at the expression boundary.
///
/// WHAT: `Ready` is a fully typed `OwnedExpr`. `Retained` is a literal whose entries still contain
///       pending numeric fragments and must be finished by body-root completion.
pub(crate) enum CurlyLiteral {
    Ready(OwnedExpr),
    Retained(DeferredExpression),
}

/// Entry point for parsing a `{...}` literal that may be a collection or a map.
///
/// WHAT: `retain_pending_numeric_fragment` lets literal-only numeric recipes stay deferred for the
///       caller to finish instead of being resolved against the entry's default type. Every pending
///       entry is captured as a fragment; a captured fragment gives its side a shape, so the literal
///       is retained, because a named binding's type comes from later body context.
#[allow(
    clippy::too_many_arguments,
    reason = "curly parsing keeps cursor, expected curly context, scope, mutable type/expression/constant/string/path owners, value mode and numeric-retention policy separate"
)]
pub(crate) fn new_curly_literal(
    token_stream: &mut AstCursor,
    curly_context: ExpectedCurlyLiteralContext,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    value_mode: &ValueMode,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
    retain_pending_numeric_fragment: bool,
) -> CollectionParseResult<CurlyLiteral> {
    match curly_context {
        ExpectedCurlyLiteralContext::Collection(collection_context) => parse_collection_literal(
            token_stream,
            collection_context,
            context,
            type_interner,
            expression_builder,
            const_values,
            value_mode,
            string_table,
            path_fork,
            retain_pending_numeric_fragment,
        ),
        ExpectedCurlyLiteralContext::Map(map_context) => parse_map_literal(
            token_stream,
            map_context,
            context,
            type_interner,
            expression_builder,
            const_values,
            value_mode,
            string_table,
            path_fork,
            retain_pending_numeric_fragment,
        ),
        ExpectedCurlyLiteralContext::Infer => parse_inferred_curly_literal(
            token_stream,
            context,
            type_interner,
            expression_builder,
            const_values,
            value_mode,
            string_table,
            path_fork,
            retain_pending_numeric_fragment,
        ),
    }
}

/// Finishes a retained collection literal once its element type is known.
///
/// WHAT: `destination` is the collection type a consumer chose for this literal, when one was chosen. It may add
///       optional wraps to the element at each raw nesting level (`{T?}`, `{{T?}}`), see `optional_collection_element`.
/// WHY: the optional template slot is the only record of the element's `?`; the literal's own origin keeps the
///      raw numeric element, so without the destination the entries materialise bare and fail the field.
pub(crate) fn complete_deferred_collection<'env>(
    literal: DeferredCollectionLiteral,
    destination: Option<TypeId>,
    session: &mut impl DeferredExpressionCompletion<'env>,
) -> Result<OwnedExpr, ExpressionParseError> {
    let DeferredCollectionLiteral {
        shape: _,
        element_shape,
        elements,
        collection_context,
        span,
        value_mode,
    } = literal;

    let explicit_element = match &collection_context {
        Some(ExpectedCollectionContext::Explicit {
            element_type_id, ..
        }) => Some(*element_type_id),
        _ => None,
    };
    let elements = elements
        .into_iter()
        .map(|element| {
            complete_forward_value(
                session,
                element,
                explicit_element,
                ValueReceiverContext::CollectionElement,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;

    let materialised_element = materialise_side(session, element_shape)?;
    let element_type_id = optional_collection_element(session, destination, materialised_element);
    let mut items = Vec::with_capacity(elements.len());
    for element in elements {
        items.push(
            complete_entry_value(
                session,
                element,
                element_type_id,
                ValueReceiverContext::CollectionElement,
            )?
            .id(),
        );
    }

    let services = session.session_services();
    build_collection_expression(
        items,
        element_type_id,
        collection_context,
        span,
        value_mode,
        services.type_interner,
        services.expression_builder,
    )
}

/// Finishes a retained map literal once its key and value types are known.
///
/// WHAT: `destination` is the map type a consumer chose for this literal, when one was chosen. It may add
///       optional wraps to the value at each raw nesting level (`{K = T?}`, `{K = {T?}}`), see `optional_map_sides`.
/// WHY: the optional template slot is the only record of the value's `?`; the literal's own origin keeps the
///      raw numeric value, so without the destination the entry would fail the instantiated field.
pub(crate) fn complete_deferred_map<'env>(
    literal: DeferredMapLiteral,
    destination: Option<TypeId>,
    session: &mut impl DeferredExpressionCompletion<'env>,
) -> Result<OwnedExpr, ExpressionParseError> {
    let DeferredMapLiteral {
        shape: _,
        key_shape,
        value_shape,
        entries,
        map_context,
        span,
        value_mode,
    } = literal;

    let (explicit_key, explicit_value) = match &map_context {
        Some(context) => (Some(context.key_type_id), Some(context.value_type_id)),
        None => (None, None),
    };
    let entries = entries
        .into_iter()
        .map(|entry| -> Result<DeferredMapEntry, ExpressionParseError> {
            Ok(DeferredMapEntry {
                key: complete_forward_value(
                    session,
                    entry.key,
                    explicit_key,
                    ValueReceiverContext::MapKey,
                )?,
                value: complete_forward_value(
                    session,
                    entry.value,
                    explicit_value,
                    ValueReceiverContext::MapValue,
                )?,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    let materialised_key = materialise_side(session, key_shape)?;
    let materialised_value = materialise_side(session, value_shape)?;
    let (key_type_id, value_type_id) =
        optional_map_sides(session, destination, materialised_key, materialised_value);

    let mut known_keys: KnownMapKeys = HashMap::new();
    let mut built_entries = Vec::with_capacity(entries.len());
    for entry in entries {
        let key = complete_entry_value(
            session,
            entry.key,
            key_type_id,
            ValueReceiverContext::MapKey,
        )?;
        let value = complete_entry_value(
            session,
            entry.value,
            value_type_id,
            ValueReceiverContext::MapValue,
        )?;

        let services = session.session_services();
        validate_map_key_entry(
            services.expression_builder.view_owned(key)?,
            key_type_id,
            services.context,
            services.type_interner,
            services.const_values,
            services.string_table,
            &mut known_keys,
        )?;

        built_entries.push(MapEntry {
            key: key.id(),
            value: value.id(),
        });
    }

    let services = session.session_services();
    build_map_expression(
        built_entries,
        key_type_id,
        value_type_id,
        map_context,
        span,
        value_mode,
        services.type_interner,
        services.expression_builder,
    )
}

/// Returns the collection element a consumer destination asks a raw entry to take.
///
/// WHAT: `destination` is the collection type the consumer chose, optionally inside an optional envelope.
///       Its element replaces `element` only when it is `element` with an optional wrap at each raw nesting level.
/// WHY: a constructor's optional template slot (`{T?}` or `{{T?}}`) is the only place the `?` lives. A raw
///      literal entry takes those wraps, so nested entries materialise at the authored field type. Typed
///      entries never match here, because they coerce to the element and an aggregate does not gain a `?`.
fn optional_collection_element<'env>(
    session: &mut impl DeferredExpressionCompletion<'env>,
    destination: Option<TypeId>,
    element: TypeId,
) -> TypeId {
    let Some(destination) = destination else {
        return element;
    };
    let services = session.session_services();
    let environment = services.type_interner.environment();
    let destination = environment
        .option_inner_type(destination)
        .unwrap_or(destination);
    let Some(destination_element) = environment
        .collection_shape(destination)
        .map(|shape| shape.element_type)
    else {
        return element;
    };
    optional_entry_destination(environment, destination_element, element)
}

/// Returns the map key and value a consumer destination asks a raw literal to take.
///
/// WHAT: `destination` is the map type the consumer chose, optionally inside an optional envelope. The value is
///       replaced only when the destination key is the materialised key and its value is the materialised value
///       with an optional wrap at each raw nesting level.
/// WHY: as for collections, a key mismatch means the destination does not describe this literal, so neither
///      side changes.
fn optional_map_sides<'env>(
    session: &mut impl DeferredExpressionCompletion<'env>,
    destination: Option<TypeId>,
    key: TypeId,
    value: TypeId,
) -> (TypeId, TypeId) {
    let Some(destination) = destination else {
        return (key, value);
    };
    let services = session.session_services();
    let environment = services.type_interner.environment();
    let destination = environment
        .option_inner_type(destination)
        .unwrap_or(destination);
    let Some(shape) = environment.map_shape(destination) else {
        return (key, value);
    };
    if shape.key_type != key {
        return (key, value);
    }
    (
        key,
        optional_entry_destination(environment, shape.value_type, value),
    )
}

/// Returns `destination_entry` when it describes `entry` with optional wraps at its raw nesting levels.
///
/// WHAT: `destination_entry` is the element or value type the consumer chose. It is returned only when
///       `optional_entry_matches` accepts it, otherwise `entry` stays.
/// WHY: returning the destination type itself, rather than rebuilding it, lets the nested literal's own
///      destination check see the same type the outer entry was completed against.
fn optional_entry_destination(
    environment: &TypeEnvironment,
    destination_entry: TypeId,
    entry: TypeId,
) -> TypeId {
    if optional_entry_matches(environment, destination_entry, entry) {
        destination_entry
    } else {
        entry
    }
}

/// Reports whether `destination_entry` is `entry` with one optional wrap at each raw nesting level.
///
/// WHAT: a destination matches when it equals `entry`, when its payload (after one envelope) equals `entry`,
///       or when both are collections or both are maps whose entries match recursively. Collections must share
///       a fixed capacity and maps must share a key type.
/// WHY: only raw literal structure recurses. A typed `{U64}` entry does not match `{U64?}` here, so a typed
///      aggregate never becomes an optional-entry aggregate and still fails the field at its own boundary.
fn optional_entry_matches(
    environment: &TypeEnvironment,
    destination_entry: TypeId,
    entry: TypeId,
) -> bool {
    if destination_entry == entry {
        return true;
    }
    let payload = environment
        .option_inner_type(destination_entry)
        .unwrap_or(destination_entry);
    if payload == entry {
        return true;
    }
    if let (Some(destination_shape), Some(entry_shape)) = (
        environment.collection_shape(payload),
        environment.collection_shape(entry),
    ) {
        return destination_shape.fixed_capacity == entry_shape.fixed_capacity
            && optional_entry_matches(
                environment,
                destination_shape.element_type,
                entry_shape.element_type,
            );
    }
    if let (Some(destination_shape), Some(entry_shape)) =
        (environment.map_shape(payload), environment.map_shape(entry))
    {
        return destination_shape.key_type == entry_shape.key_type
            && optional_entry_matches(
                environment,
                destination_shape.value_type,
                entry_shape.value_type,
            );
    }
    false
}

// ------------------------
//  Shared literal entries
// ------------------------

/// Which expression boundary a literal entry stops at.
#[derive(Clone, Copy)]
enum CurlyEntryEnd {
    /// A collection item or a map key: stops before `=`, `,`, or `}`.
    ItemOrKey,
    /// A map value: uses the ordinary expression boundary.
    Value,
}

/// Expected-type state for one literal entry.
///
/// WHY: a side whose type is still unknown is parsed with `Infer`, while a known side passes its
///      type down so nested literals and numeric recipes resolve against it.
struct EntryExpectation {
    expected_type: ExpectedType,
    cast_target_context: CastTargetContext,
    expected_types: Vec<TypeId>,
}

impl EntryExpectation {
    fn infer() -> Self {
        Self {
            expected_type: ExpectedType::Infer,
            cast_target_context: CastTargetContext::None,
            expected_types: Vec::new(),
        }
    }
}

/// Builds the expectation for one entry from its side's known type, if any.
fn side_expectation(
    known_type: Option<TypeId>,
    type_interner: &AstTypeInterner<'_>,
    string_table: &mut StringTable,
    path_fork: &PathInternerFork,
) -> EntryExpectation {
    match known_type {
        Some(type_id) => EntryExpectation {
            expected_type: parse_expectation_for_type_id(type_id, type_interner.environment()),
            cast_target_context: cast_target_context_for_type_id(
                type_id,
                type_interner.environment(),
                string_table,
                path_fork,
            ),
            expected_types: vec![type_id],
        },
        None => EntryExpectation::infer(),
    }
}

/// Parses one literal entry, retaining a pending numeric fragment instead of failing.
///
/// WHAT: a parsed entry is `Ready`. A child that defers numeric completion (`NumericDeferral`) is
///       always captured as a `Fragment` owned by the literal, whether or not the literal retains
///       literal-only recipes; `retain` controls only that literal-only retention.
#[allow(
    clippy::too_many_arguments,
    reason = "entry parsing keeps cursor and scope, mutable type/expression/constant/string/path owners and receiving expectation, entry terminator and numeric-retention policy separate"
)]
fn parse_curly_entry(
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    expectation: &mut EntryExpectation,
    end: CurlyEntryEnd,
    retain: bool,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> CollectionParseResult<DeferredLiteralValue> {
    let mut entry_context = context.new_child_expression(expectation.expected_types.clone());
    // Propagate the parent context kind so that constant-context restrictions apply to entries.
    entry_context.kind = context.kind.clone();

    // Entries are read as immutable owned values: a literal copies each element from its source, so
    // an entry's access follows its source rather than the container's mutability.
    let entry_value_mode = ValueMode::ImmutableOwned;
    let resources = ExpressionParseResources {
        token_stream,
        scope_context: &entry_context,
        type_interner,
        expression_builder,
        const_values,
        expected_type: &mut expectation.expected_type,
        cast_target_context: &mut expectation.cast_target_context,
        value_mode: &entry_value_mode,
        string_table,
        path_fork,
    };
    let input = match end {
        CurlyEntryEnd::ItemOrKey => ExpressionParseInput::without_boundary_catch(resources, false),
        CurlyEntryEnd::Value => ExpressionParseInput::ordinary(resources, false),
    };
    // The literal owns each complete entry. Nested producers must retain their own syntax before
    // the entry can retain the whole value, so a non-owning child deferral still propagates.
    let input = input.with_numeric_deferral_owner();
    let input = if retain {
        input.with_retained_pending_numeric_fragment()
    } else {
        input
    };

    let parsed = match end {
        CurlyEntryEnd::ItemOrKey => create_expression_until(
            input,
            &[TokenTag::ASSIGN, TokenTag::COMMA, TokenTag::CLOSE_CURLY],
        ),
        CurlyEntryEnd::Value => create_expression_with_trailing_newline_policy(input),
    };

    match parsed {
        Ok(expression) => Ok(DeferredLiteralValue::Ready(expression.id())),
        // Retain only the complete entry, never a nested argument that lost its enclosing producer.
        Err(ExpressionParseError::NumericDeferral(deferral)) if deferral.complete_fragment => {
            let NumericDeferral {
                items,
                expected,
                value_mode,
                span,
                suffix,
                complete_fragment: _,
            } = *deferral;
            Ok(DeferredLiteralValue::Fragment(deferred_numeric_expression(
                items, expected, value_mode, span, suffix,
            )))
        }
        Err(error) => Err(error),
    }
}

/// Type and shape state for one side of a literal (a collection element, or a map key or value).
///
/// WHAT: `known_type` is the side's concrete type once fixed, from explicit context or the first
///       entry. `shape` exists only once a pending numeric fragment joins this side.
/// WHY: plain entries stay on the fast path with no known-shape graph. Only literals that actually
///      defer build shapes, and an unknown side never stores a sentinel type.
///
/// Invariant: when `known_type` is `Some` and `shape` is `Some`, the shape's concrete leaf is the
/// known type and fragments contribute receiver witnesses only.
struct LiteralSide {
    known_type: Option<TypeId>,
    shape: Option<KnownShapeId>,
    receiver: ValueReceiverContext,
}

impl LiteralSide {
    fn new(known_type: Option<TypeId>, receiver: ValueReceiverContext) -> Self {
        Self {
            known_type,
            shape: None,
            receiver,
        }
    }
}

struct CollectionBuild {
    element: LiteralSide,
    entries: CollectionEntries,
}

struct MapBuild {
    key: LiteralSide,
    value: LiteralSide,
    known_keys: KnownMapKeys,
    entries: MapEntries,
}

/// Element entries of one collection literal while it is parsed.
///
/// WHAT: `Ready` holds fully typed child `ExprId`s. `Deferred` holds entries once a numeric fragment
///       has joined the element side; the earlier ready entries move into it, in order, at that point.
/// WHY: a literal without pending numeric entries never allocates the deferred record, and a literal
///      that defers pays for exactly one transition.
enum CollectionEntries {
    Ready(Vec<ExprId>),
    Deferred(Vec<DeferredLiteralValue>),
}

impl CollectionEntries {
    fn len(&self) -> usize {
        match self {
            Self::Ready(items) => items.len(),
            Self::Deferred(items) => items.len(),
        }
    }

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn push(&mut self, value: DeferredLiteralValue) {
        match self {
            Self::Deferred(items) => items.push(value),
            Self::Ready(items) => match value {
                DeferredLiteralValue::Ready(expression) => items.push(expression),
                value => {
                    let mut deferred: Vec<DeferredLiteralValue> = std::mem::take(items)
                        .into_iter()
                        .map(DeferredLiteralValue::Ready)
                        .collect();
                    deferred.push(value);
                    *self = Self::Deferred(deferred);
                }
            },
        }
    }

    fn into_ready(self) -> CollectionParseResult<Vec<ExprId>> {
        match self {
            Self::Ready(items) => Ok(items),
            Self::Deferred(_) => Err(internal_literal_error(
                "pending numeric literal entry reached the ready-literal path",
            )),
        }
    }

    fn into_deferred(self) -> Vec<DeferredLiteralValue> {
        match self {
            Self::Ready(items) => items.into_iter().map(DeferredLiteralValue::Ready).collect(),
            Self::Deferred(items) => items,
        }
    }
}

/// Entries of one map literal while it is parsed, with the same transition as `CollectionEntries`.
enum MapEntries {
    Ready(Vec<MapEntry>),
    Deferred(Vec<DeferredMapEntry>),
}

impl MapEntries {
    fn is_empty(&self) -> bool {
        match self {
            Self::Ready(entries) => entries.is_empty(),
            Self::Deferred(entries) => entries.is_empty(),
        }
    }

    fn push(&mut self, key: DeferredLiteralValue, value: DeferredLiteralValue) {
        match self {
            Self::Deferred(entries) => entries.push(DeferredMapEntry { key, value }),
            Self::Ready(entries) => match (key, value) {
                (DeferredLiteralValue::Ready(key), DeferredLiteralValue::Ready(value)) => {
                    entries.push(MapEntry { key, value });
                }
                (key, value) => {
                    let mut deferred: Vec<DeferredMapEntry> = std::mem::take(entries)
                        .into_iter()
                        .map(|entry| DeferredMapEntry {
                            key: DeferredLiteralValue::Ready(entry.key),
                            value: DeferredLiteralValue::Ready(entry.value),
                        })
                        .collect();
                    deferred.push(DeferredMapEntry { key, value });
                    *self = Self::Deferred(deferred);
                }
            },
        }
    }

    fn into_ready(self) -> CollectionParseResult<Vec<MapEntry>> {
        match self {
            Self::Ready(entries) => Ok(entries),
            Self::Deferred(_) => Err(internal_literal_error(
                "pending numeric map entry reached the ready-literal path",
            )),
        }
    }

    fn into_deferred(self) -> Vec<DeferredMapEntry> {
        match self {
            Self::Ready(entries) => entries
                .into_iter()
                .map(|entry| DeferredMapEntry {
                    key: DeferredLiteralValue::Ready(entry.key),
                    value: DeferredLiteralValue::Ready(entry.value),
                })
                .collect(),
            Self::Deferred(entries) => entries,
        }
    }
}

fn intern_concrete_shape(
    context: &ScopeContext,
    type_id: TypeId,
    span: Option<SourceSpan>,
) -> CollectionParseResult<KnownShapeId> {
    context
        .numeric_state_mut()
        .intern_shape(KnownShape::Concrete { type_id, span })
}

fn internal_literal_error(message: &'static str) -> ExpressionParseError {
    ExpressionParseError::Infrastructure(CompilerError::compiler_error(message).into())
}

/// Rejects a literal whose closing `}` was never reached, before any constructor is built.
fn require_closing_curly(
    token_stream: &mut AstCursor,
    consumed_close_curly: bool,
    string_table: &mut StringTable,
) -> CollectionParseResult<()> {
    if consumed_close_curly {
        return Ok(());
    }

    Err(CompilerDiagnostic::missing_closing_delimiter(
        string_table.get_or_intern("}".to_owned()),
        Some(token_stream.current_span()),
    )
    .into())
}

/// Joins one incoming shape into a side, creating the side's shape on first use.
fn join_side_shape(
    context: &ScopeContext,
    type_interner: &AstTypeInterner<'_>,
    side: &mut LiteralSide,
    incoming: KnownShapeId,
) -> CollectionParseResult<()> {
    match side.shape {
        Some(existing) => {
            union_shape_classes(
                &mut context.numeric_state_mut(),
                type_interner.environment(),
                existing,
                incoming,
            );
        }
        None => {
            let shape = match side.known_type {
                Some(type_id) => {
                    let concrete = intern_concrete_shape(context, type_id, None)?;
                    union_shape_classes(
                        &mut context.numeric_state_mut(),
                        type_interner.environment(),
                        concrete,
                        incoming,
                    );
                    concrete
                }
                None => incoming,
            };
            side.shape = Some(shape);
        }
    }

    Ok(())
}

/// Joins one fragment's shape into a side, refusing a join that would retype a pinned whole origin.
///
/// WHAT: a pinned bound whole leaf and a decimal-family peer do not share one class. A refused
///       fragment is not unioned: it stays its own entry and completes against the side's
///       materialised type, which reports any real mismatch at its own span.
/// WHY: the class union merges families unconditionally, so joining a bound whole with a decimal
///      peer would silently turn the integer origin into a decimal.
fn join_fragment_shape(
    context: &ScopeContext,
    type_interner: &AstTypeInterner<'_>,
    side: &mut LiteralSide,
    incoming: KnownShapeId,
) -> CollectionParseResult<()> {
    let refused = match side.shape {
        Some(existing) => shapes_join_refused(
            &mut context.numeric_state_mut(),
            type_interner.environment(),
            existing,
            incoming,
        ),
        None => false,
    };
    if refused {
        return Ok(());
    }

    join_side_shape(context, type_interner, side, incoming)
}

/// Records one settled entry on its side, coercing ready values to a known side type.
///
/// WHAT: the first ready entry fixes an unknown side's type, including when fragments joined the
///       side before it. Those fragments then complete against that type, so the side's shape
///       becomes its concrete leaf. A fragment never fixes the type; it joins the shape and is
///       resolved at completion against the materialised type.
fn settle_side_value(
    side: &mut LiteralSide,
    value: DeferredLiteralValue,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
) -> CollectionParseResult<DeferredLiteralValue> {
    match value {
        DeferredLiteralValue::Ready(expression) => {
            let view = expression_builder.view(expression)?;
            view.require_single(side.receiver)?;
            let view_type = view.value_type_id()?;
            let view_span = view.span();
            let expression = match side.known_type {
                Some(type_id) => coerce_expression_to_explicit_type_boundary(
                    expression_builder.own(expression),
                    type_id,
                    type_interner.environment(),
                    TypeMismatchContext::CollectionElement,
                    expression_builder,
                )?
                .id(),
                None => {
                    side.known_type = Some(view_type);
                    if let Some(previous_shape) = side.shape {
                        // A ready peer types raw origins but never retypes a pinned whole leaf.
                        register_shape_witness(
                            &mut context.numeric_state_mut(),
                            type_interner.environment(),
                            previous_shape,
                            view_type,
                            view_span,
                        );
                        side.shape = Some(intern_concrete_shape(context, view_type, view_span)?);
                    }
                    return Ok(DeferredLiteralValue::Ready(expression));
                }
            };

            if side.shape.is_some() {
                let view = expression_builder.view(expression)?;
                let incoming = intern_concrete_shape(context, view.value_type_id()?, view.span())?;
                join_side_shape(context, type_interner, side, incoming)?;
            }

            Ok(DeferredLiteralValue::Ready(expression))
        }
        DeferredLiteralValue::Fragment(fragment) => {
            let incoming = fragment_shape(
                context,
                type_interner.environment_mut_for_derived_types(),
                &fragment.items,
                expression_builder,
            )?;
            // Forward facts are read before the join: joining ORs a forward peer's flag into the
            // shared root, which would make a raw literal read as a forward producer.
            let forward_class = context.numeric_state_mut().forward_numeric_class(incoming);
            let forward = shape_has_forward_leaf(&context.numeric_state_mut(), incoming);
            join_fragment_shape(context, type_interner, side, incoming)?;
            // A forward class registers its producer now, so the fragment completes from its
            // natural result before any literal default chooses the class's type.
            match forward_class {
                Some(class) => {
                    let producer = context
                        .numeric_state_mut()
                        .register_forward_producer(class, incoming, fragment)?;
                    Ok(DeferredLiteralValue::Forward(producer))
                }
                None => Ok(DeferredLiteralValue::Joined {
                    fragment,
                    shape: incoming,
                    forward,
                }),
            }
        }
        DeferredLiteralValue::Joined { .. } | DeferredLiteralValue::Forward(_) => Err(
            internal_literal_error("numeric literal entry reached settlement already joined"),
        ),
    }
}

fn known_side_type(side: &LiteralSide) -> CollectionParseResult<TypeId> {
    side.known_type.ok_or_else(|| {
        internal_literal_error("literal side reached the ready-literal path without a known type")
    })
}

/// Returns the shape a retained literal side contributes to its deferred record.
fn side_shape_or_concrete(
    side: &LiteralSide,
    context: &ScopeContext,
) -> CollectionParseResult<KnownShapeId> {
    match (side.shape, side.known_type) {
        (Some(shape), _) => Ok(shape),
        (None, Some(type_id)) => intern_concrete_shape(context, type_id, None),
        (None, None) => Err(internal_literal_error(
            "retained literal side has neither a shape nor a known type",
        )),
    }
}

/// Materialises one side's shape into a concrete type at completion.
fn materialise_side<'env>(
    session: &mut impl DeferredExpressionCompletion<'env>,
    shape: KnownShapeId,
) -> Result<TypeId, ExpressionParseError> {
    // A pending forward producer connected to this shape completes before the shape materialises.
    session.settle_forward_producers(shape)?;
    let services = session.session_services();
    let mut state = services.context.numeric_state_mut();
    materialise_known_shape(
        &mut state,
        services.type_interner.environment_mut_for_derived_types(),
        shape,
    )
}

/// Completes one retained entry against its side's materialised type.
fn complete_entry_value<'env>(
    session: &mut impl DeferredExpressionCompletion<'env>,
    value: DeferredLiteralValue,
    side_type: TypeId,
    receiver: ValueReceiverContext,
) -> Result<OwnedExpr, ExpressionParseError> {
    let expression = match value {
        DeferredLiteralValue::Ready(expression) => session
            .session_services()
            .expression_builder
            .own(expression),
        DeferredLiteralValue::Forward(producer) => session.take_forward_producer(producer, None)?,
        DeferredLiteralValue::Fragment(fragment)
        | DeferredLiteralValue::Joined { fragment, .. } => {
            let expected = {
                let services = session.session_services();
                parse_expectation_for_type_id(side_type, services.type_interner.environment())
            };
            complete_fragment_expression(session, fragment, expected)?
        }
    };

    let services = session.session_services();
    services
        .expression_builder
        .view_owned(expression)?
        .require_single(receiver)?;
    coerce_expression_to_explicit_type_boundary(
        expression,
        side_type,
        services.type_interner.environment(),
        TypeMismatchContext::CollectionElement,
        services.expression_builder,
    )
    .map_err(ExpressionParseError::from)
}

/// Resolves one retained numeric fragment against `expected`, then applies its handling suffix.
fn complete_fragment_expression<'env>(
    session: &mut impl DeferredExpressionCompletion<'env>,
    fragment: DeferredNumericExpression,
    expected: ExpectedType,
) -> Result<OwnedExpr, ExpressionParseError> {
    let items = fragment.items;
    let value_mode = fragment.value_mode;
    let suffix = fragment.suffix;

    let resolved = session.resolve_fragment(items, expected, &value_mode)?;

    let services = session.session_services();
    apply_completed_handling_suffix(
        resolved,
        suffix,
        services.context,
        services.type_interner,
        services.expression_builder,
    )
}

/// Completes a forward entry from its natural result before its side's type is chosen.
///
/// WHAT: a forward-class entry was registered as a producer at parse time. It completes here
///       against the side's explicit type when one was authored, otherwise with the destination
///       its immediate result gave it, or its natural result. A joined entry that held a forward
///       leaf when it joined resolves against the explicit type or `Infer` and registers its
///       natural result as a peer witness. Every other entry is returned unchanged.
/// WHY: a forward result is fixed by its operation, so it must not wait for a literal default chosen
///      from raw peers.
fn complete_forward_value<'env>(
    session: &mut impl DeferredExpressionCompletion<'env>,
    value: DeferredLiteralValue,
    explicit_type: Option<TypeId>,
    receiver: ValueReceiverContext,
) -> Result<DeferredLiteralValue, ExpressionParseError> {
    match value {
        DeferredLiteralValue::Forward(producer) => {
            let expected = forward_entry_expectation(session, explicit_type);
            let expression = session.take_forward_producer(producer, expected)?;
            session
                .session_services()
                .expression_builder
                .view_owned(expression)?
                .require_single(receiver)?;
            Ok(DeferredLiteralValue::Ready(expression.id()))
        }
        DeferredLiteralValue::Joined {
            fragment,
            shape,
            forward,
        } => {
            if !forward {
                return Ok(DeferredLiteralValue::Joined {
                    fragment,
                    shape,
                    forward,
                });
            }

            let expected =
                forward_entry_expectation(session, explicit_type).unwrap_or(ExpectedType::Infer);
            let expression = complete_fragment_expression(session, fragment, expected)?;
            let (view_type, view_span) = {
                let services = session.session_services();
                let view = services.expression_builder.view_owned(expression)?;
                view.require_single(receiver)?;
                (view.value_type_id()?, view.span())
            };

            let services = session.session_services();
            let mut state = services.context.numeric_state_mut();
            register_shape_witness(
                &mut state,
                services.type_interner.environment(),
                shape,
                view_type,
                view_span,
            );
            Ok(DeferredLiteralValue::Ready(expression.id()))
        }
        other => Ok(other),
    }
}

/// The explicit type a forward entry completes against, when the side authored one.
///
/// WHAT: `None` when no explicit type exists, so the producer completes with its own destination.
/// WHY: an immediate result may already have given the producer a literal destination; an `Infer`
///      expectation here would overwrite it and default the forward leaf before its class is solved.
fn forward_entry_expectation<'env>(
    session: &mut impl DeferredExpressionCompletion<'env>,
    explicit_type: Option<TypeId>,
) -> Option<ExpectedType> {
    explicit_type.map(|type_id| {
        let services = session.session_services();
        parse_expectation_for_type_id(type_id, services.type_interner.environment())
    })
}

// ------------------------
//  Collection literal parsing
// ------------------------

#[allow(
    clippy::too_many_arguments,
    reason = "collection parsing keeps cursor, expected collection context, scope, mutable type/expression/constant/string/path owners, value mode and numeric-retention policy separate"
)]
fn parse_collection_literal(
    token_stream: &mut AstCursor,
    collection_context: ExpectedCollectionContext,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    value_mode: &ValueMode,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
    retain: bool,
) -> CollectionParseResult<CurlyLiteral> {
    let collection_span = Some(token_stream.current_span());

    let (known_element, fixed_capacity) = match collection_context {
        ExpectedCollectionContext::Explicit {
            element_type_id,
            fixed_capacity: capacity,
            ..
        } => (Some(element_type_id), capacity),
        ExpectedCollectionContext::CapacityOnlyShorthand {
            fixed_capacity: capacity,
        } => (None, Some(capacity)),
    };

    // The current token is an open curly brace; skip to the first value.
    token_stream.advance();

    let mut build = CollectionBuild {
        element: LiteralSide::new(known_element, ValueReceiverContext::CollectionElement),
        entries: CollectionEntries::Ready(Vec::new()),
    };
    let consumed_close_curly = parse_collection_entries(
        token_stream,
        context,
        type_interner,
        expression_builder,
        const_values,
        string_table,
        path_fork,
        &mut build,
        retain,
    )?;

    require_closing_curly(token_stream, consumed_close_curly, string_table)?;

    if matches!(
        collection_context,
        ExpectedCollectionContext::CapacityOnlyShorthand { .. }
    ) && build.entries.is_empty()
    {
        return Err(CompilerDiagnostic::invalid_collection_type(
            InvalidCollectionTypeReason::ShorthandEmptyLiteralAmbiguous,
            collection_span,
        )
        .into());
    }

    if let Some(capacity) = fixed_capacity
        && build.entries.len() > capacity
    {
        return Err(CompilerDiagnostic::invalid_collection_type(
            InvalidCollectionTypeReason::InitializerExceedsFixedCapacity {
                capacity,
                length: build.entries.len(),
            },
            collection_span,
        )
        .into());
    }

    if let Some(element_shape) = build.element.shape {
        let shape = context
            .numeric_state_mut()
            .intern_shape(KnownShape::Collection {
                element: element_shape,
                fixed_capacity,
            })?;
        return Ok(CurlyLiteral::Retained(DeferredExpression::Collection(
            Box::new(DeferredCollectionLiteral {
                shape,
                element_shape,
                elements: build.entries.into_deferred(),
                collection_context: Some(collection_context),
                span: collection_span,
                value_mode: *value_mode,
            }),
        )));
    }

    let element_type_id = known_side_type(&build.element)?;
    let items = build.entries.into_ready()?;

    Ok(CurlyLiteral::Ready(build_collection_expression(
        items,
        element_type_id,
        Some(collection_context),
        collection_span,
        *value_mode,
        type_interner,
        expression_builder,
    )?))
}

/// Parses collection items until the closing `}`. Returns whether `}` was consumed.
#[allow(
    clippy::too_many_arguments,
    reason = "collection entries keep stream, scope, build, retain flag and mutable parse resources apart"
)]
fn parse_collection_entries(
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
    build: &mut CollectionBuild,
    retain: bool,
) -> CollectionParseResult<bool> {
    let mut consumed_close_curly = false;
    let mut awaiting_item = build.entries.is_empty();

    // ------------------------
    //  Parse collection items
    // ------------------------
    while token_stream.position() < token_stream.length() {
        match token_stream.current_tag() {
            TokenTag::CLOSE_CURLY => {
                consumed_close_curly = true;
                break;
            }

            TokenTag::NEWLINE => {
                token_stream.advance();
            }

            TokenTag::COMMA => {
                if awaiting_item {
                    return Err(CompilerDiagnostic::missing_collection_item(Some(
                        token_stream.current_span(),
                    ))
                    .into());
                }

                awaiting_item = true;
                token_stream.advance();
            }

            _ => {
                if !awaiting_item {
                    return Err(CompilerDiagnostic::missing_collection_item(Some(
                        token_stream.current_span(),
                    ))
                    .into());
                }

                let mut expectation = side_expectation(
                    build.element.known_type,
                    type_interner,
                    string_table,
                    path_fork,
                );
                let parsed_item = parse_curly_entry(
                    token_stream,
                    context,
                    type_interner,
                    expression_builder,
                    const_values,
                    &mut expectation,
                    CurlyEntryEnd::ItemOrKey,
                    retain,
                    string_table,
                    path_fork,
                )?;

                // A trailing `=` means the literal is mixing collection and map syntax.
                if let Some(error) = mixed_collection_entry_error(token_stream) {
                    return Err(error.into());
                }

                let settled = settle_side_value(
                    &mut build.element,
                    parsed_item,
                    context,
                    type_interner,
                    expression_builder,
                )?;
                build.entries.push(settled);

                awaiting_item = false;
            }
        }
    }

    Ok(consumed_close_curly)
}

// ------------------------
//  Map literal parsing
// ------------------------

/// Foldable literal keys that can be checked for duplicates at parse time.
///
/// Integer and `Byte` keys arrive as exact fixed scalar values, so the scalar identity and its
/// exact payload both take part in key equality; no lossy host-number projection is involved.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum KnownMapKey {
    String(StringId),
    FixedScalar(FixedScalarValue),
    Bool(bool),
    Char(char),
}

/// Tracks scalar literal keys that have already appeared so duplicate keys
/// can be diagnosed at parse time.
type KnownMapKeys = HashMap<KnownMapKey, Option<SourceSpan>>;

/// Attempt to extract a `KnownMapKey` from a coerced key view so duplicate-key detection runs
/// for literal scalars without cloning declaration content.
fn try_extract_known_map_key(
    key: ExpressionView<'_>,
    const_values: &ConstValueStore,
    string_table: &mut StringTable,
) -> CollectionParseResult<Option<KnownMapKey>> {
    let ExpressionOperationView::Constant(id) = key.operation() else {
        return Ok(None);
    };
    let known_key = match const_values.payload(id) {
        Some(ConstValuePayload::String(_)) => {
            // A key whose final text is still unresolved is simply not knowable here, exactly like
            // a runtime operand. The constant-context check above owns the refusal, so duplicate
            // detection must not raise a second one from this collector.
            let Ok(Some(id)) = require_concrete_text(
                key,
                const_values,
                ConstStringRequirement::DuplicateKeyValidation,
                string_table,
            ) else {
                return Ok(None);
            };
            KnownMapKey::String(id)
        }
        // Every exact integer and `Byte` key keeps its scalar identity and payload, so
        // negatives and unsigned range facts stay part of what duplicate detection compares.
        Some(ConstValuePayload::FixedScalar(value)) => KnownMapKey::FixedScalar(*value),
        Some(ConstValuePayload::Bool(value)) => KnownMapKey::Bool(*value),
        Some(ConstValuePayload::Char(value)) => KnownMapKey::Char(*value),
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
    key: ExpressionView<'_>,
    const_values: &ConstValueStore,
    context: &ScopeContext,
    string_table: &mut StringTable,
) -> CollectionParseResult<()> {
    if !context.kind.is_constant_context() {
        return Ok(());
    }
    let ExpressionOperationView::Constant(_) = key.operation() else {
        return Ok(());
    };
    require_concrete_text(
        key,
        const_values,
        ConstStringRequirement::CompileTimeMapKey,
        string_table,
    )
    .map_err(ExpressionParseError::from)?;
    Ok(())
}

fn record_known_map_key(
    known_keys: &mut KnownMapKeys,
    key: ExpressionView<'_>,
    const_values: &ConstValueStore,
    string_table: &mut StringTable,
) -> CollectionParseResult<()> {
    let Some(known_key) = try_extract_known_map_key(key, const_values, string_table)? else {
        return Ok(());
    };

    if known_keys.insert(known_key, key.span()).is_some() {
        return Err(CompilerDiagnostic::invalid_map_literal(
            InvalidMapLiteralReason::DuplicateKnownKey,
            key.span(),
        )
        .into());
    }

    Ok(())
}

/// Validates one ready map key: key type policy, constant-context text, then duplicate detection.
fn validate_map_key_entry(
    key: ExpressionView<'_>,
    key_type_id: TypeId,
    context: &ScopeContext,
    type_interner: &AstTypeInterner<'_>,
    const_values: &ConstValueStore,
    string_table: &mut StringTable,
    known_keys: &mut KnownMapKeys,
) -> CollectionParseResult<()> {
    validate_map_key_type(key_type_id, type_interner.environment(), key.span())?;
    validate_compile_time_map_key(key, const_values, context, string_table)?;
    record_known_map_key(known_keys, key, const_values, string_table)
}

fn mixed_collection_entry_error(token_stream: &AstCursor) -> Option<CompilerDiagnostic> {
    if token_stream.current_tag() != TokenTag::ASSIGN {
        return None;
    }

    if token_stream.peek_next_tag() == Some(TokenTag::ASSIGN) {
        return check_expression_common_mistake(token_stream, false);
    }

    Some(CompilerDiagnostic::invalid_map_literal(
        InvalidMapLiteralReason::MixedCollectionMapEntries,
        Some(token_stream.current_span()),
    ))
}

fn consume_map_entry_separator(token_stream: &mut AstCursor) -> CollectionParseResult<()> {
    if token_stream.current_tag() != TokenTag::ASSIGN {
        return Err(CompilerDiagnostic::invalid_map_literal(
            InvalidMapLiteralReason::MixedCollectionMapEntries,
            Some(token_stream.current_span()),
        )
        .into());
    }

    if token_stream.peek_next_tag() == Some(TokenTag::ASSIGN)
        && let Some(error) = check_expression_common_mistake(token_stream, false)
    {
        return Err(error.into());
    }

    token_stream.advance();
    Ok(())
}

fn reject_missing_map_key_expression(token_stream: &AstCursor) -> CollectionParseResult<()> {
    if token_stream.current_tag() == TokenTag::ASSIGN {
        return Err(CompilerDiagnostic::invalid_map_literal(
            InvalidMapLiteralReason::MissingKeyExpression,
            Some(token_stream.current_span()),
        )
        .into());
    }

    Ok(())
}

fn reject_missing_map_value_expression(token_stream: &mut AstCursor) -> CollectionParseResult<()> {
    while token_stream.current_tag() == TokenTag::NEWLINE {
        token_stream.advance();
    }

    match token_stream.current_tag() {
        TokenTag::CLOSE_CURLY | TokenTag::COMMA => Err(CompilerDiagnostic::invalid_map_literal(
            InvalidMapLiteralReason::MissingValueExpression,
            Some(token_stream.current_span()),
        )
        .into()),
        _ => Ok(()),
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "map parsing keeps cursor, expected map context, scope, mutable type/expression/constant/string/path owners, value mode and numeric-retention policy separate"
)]
fn parse_map_literal(
    token_stream: &mut AstCursor,
    map_context: ExpectedMapContext,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    value_mode: &ValueMode,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
    retain: bool,
) -> CollectionParseResult<CurlyLiteral> {
    let map_span = Some(token_stream.current_span());
    let key_type_id = map_context.key_type_id;
    let value_type_id = map_context.value_type_id;

    // The current token is an open curly brace; skip to the first entry.
    token_stream.advance();

    let mut build = MapBuild {
        key: LiteralSide::new(Some(key_type_id), ValueReceiverContext::MapKey),
        value: LiteralSide::new(Some(value_type_id), ValueReceiverContext::MapValue),
        known_keys: HashMap::new(),
        entries: MapEntries::Ready(Vec::new()),
    };
    let consumed_close_curly = parse_map_entries(
        token_stream,
        context,
        type_interner,
        expression_builder,
        const_values,
        string_table,
        path_fork,
        &mut build,
        retain,
    )?;

    require_closing_curly(token_stream, consumed_close_curly, string_table)?;

    if build.key.shape.is_some() || build.value.shape.is_some() {
        let key_shape = side_shape_or_concrete(&build.key, context)?;
        let value_shape = side_shape_or_concrete(&build.value, context)?;
        let shape = context.numeric_state_mut().intern_shape(KnownShape::Map {
            key: key_shape,
            value: value_shape,
        })?;
        return Ok(CurlyLiteral::Retained(DeferredExpression::Map(Box::new(
            DeferredMapLiteral {
                shape,
                key_shape,
                value_shape,
                entries: build.entries.into_deferred(),
                map_context: Some(map_context),
                span: map_span,
                value_mode: *value_mode,
            },
        ))));
    }

    let entries = build.entries.into_ready()?;
    Ok(CurlyLiteral::Ready(build_map_expression(
        entries,
        key_type_id,
        value_type_id,
        Some(map_context),
        map_span,
        *value_mode,
        type_interner,
        expression_builder,
    )?))
}

/// Parses map entries until the closing `}`. Returns whether `}` was consumed.
#[allow(
    clippy::too_many_arguments,
    reason = "map entries keep stream, scope, build, retain flag and mutable parse resources apart"
)]
fn parse_map_entries(
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
    build: &mut MapBuild,
    retain: bool,
) -> CollectionParseResult<bool> {
    let mut consumed_close_curly = false;
    let mut awaiting_entry = build.entries.is_empty();

    // ------------------------
    //  Parse map entries
    // ------------------------
    while token_stream.position() < token_stream.length() {
        match token_stream.current_tag() {
            TokenTag::CLOSE_CURLY => {
                consumed_close_curly = true;
                break;
            }

            TokenTag::NEWLINE => {
                token_stream.advance();
            }

            TokenTag::COMMA => {
                if awaiting_entry {
                    return Err(CompilerDiagnostic::missing_collection_item(Some(
                        token_stream.current_span(),
                    ))
                    .into());
                }

                awaiting_entry = true;
                token_stream.advance();
            }

            _ => {
                if !awaiting_entry {
                    return Err(CompilerDiagnostic::missing_collection_item(Some(
                        token_stream.current_span(),
                    ))
                    .into());
                }
                reject_missing_map_key_expression(token_stream)?;

                let mut key_expectation =
                    side_expectation(build.key.known_type, type_interner, string_table, path_fork);
                let parsed_key = parse_curly_entry(
                    token_stream,
                    context,
                    type_interner,
                    expression_builder,
                    const_values,
                    &mut key_expectation,
                    CurlyEntryEnd::ItemOrKey,
                    retain,
                    string_table,
                    path_fork,
                )?;

                let value_expectation = side_expectation(
                    build.value.known_type,
                    type_interner,
                    string_table,
                    path_fork,
                );
                finish_map_entry(
                    token_stream,
                    context,
                    type_interner,
                    expression_builder,
                    const_values,
                    string_table,
                    path_fork,
                    build,
                    parsed_key,
                    value_expectation,
                    retain,
                )?;

                awaiting_entry = false;
            }
        }
    }

    Ok(consumed_close_curly)
}

/// Parses the separator and value of one map entry whose key is already parsed, then records it.
#[allow(
    clippy::too_many_arguments,
    reason = "map entry completion keeps cursor and scope, mutable type/expression/constant/string/path owners and map build, owned key and value expectation and numeric-retention policy separate"
)]
fn finish_map_entry(
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
    build: &mut MapBuild,
    key: DeferredLiteralValue,
    mut value_expectation: EntryExpectation,
    retain: bool,
) -> CollectionParseResult<()> {
    // Top-level `=` separates key from value. Check `==` before consuming the separator so
    // equality common-mistake diagnostics remain specific.
    consume_map_entry_separator(token_stream)?;
    reject_missing_map_value_expression(token_stream)?;

    let parsed_value = parse_curly_entry(
        token_stream,
        context,
        type_interner,
        expression_builder,
        const_values,
        &mut value_expectation,
        CurlyEntryEnd::Value,
        retain,
        string_table,
        path_fork,
    )?;

    let key = settle_side_value(
        &mut build.key,
        key,
        context,
        type_interner,
        expression_builder,
    )?;
    let value = settle_side_value(
        &mut build.value,
        parsed_value,
        context,
        type_interner,
        expression_builder,
    )?;

    if let (DeferredLiteralValue::Ready(key_expression), Some(key_type_id)) =
        (&key, build.key.known_type)
    {
        validate_map_key_entry(
            expression_builder.view(*key_expression)?,
            key_type_id,
            context,
            type_interner,
            const_values,
            string_table,
            &mut build.known_keys,
        )?;
    }

    build.entries.push(key, value);
    Ok(())
}

// ------------------------
//  Inferred curly literal parsing
// ------------------------

#[allow(
    clippy::too_many_arguments,
    reason = "inferred curly keeps stream, scope, value mode, retain flag and mutable parse resources apart"
)]
fn parse_inferred_curly_literal(
    token_stream: &mut AstCursor,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    value_mode: &ValueMode,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
    retain: bool,
) -> CollectionParseResult<CurlyLiteral> {
    let literal_span = Some(token_stream.current_span());

    // Collection delimiters own intervening newlines; normalize them before the bounded
    // expression parser sees the first entry as a possible statement terminator.
    token_stream.advance();
    token_stream.skip_newlines();

    // Empty inferred `{}` keeps existing collection ambiguity behavior.
    if token_stream.current_tag() == TokenTag::CLOSE_CURLY {
        token_stream.advance();
        return Err(CompilerDiagnostic::empty_collection_type_ambiguity(literal_span).into());
    }
    reject_missing_map_key_expression(token_stream)?;

    let mut first_expectation = EntryExpectation::infer();
    let first_entry = parse_curly_entry(
        token_stream,
        context,
        type_interner,
        expression_builder,
        const_values,
        &mut first_expectation,
        CurlyEntryEnd::ItemOrKey,
        retain,
        string_table,
        path_fork,
    )?;

    match token_stream.current_tag() {
        // First entry has `=`  =>  this is a map literal.
        TokenTag::ASSIGN => {
            let mut build = MapBuild {
                key: LiteralSide::new(None, ValueReceiverContext::MapKey),
                value: LiteralSide::new(None, ValueReceiverContext::MapValue),
                known_keys: HashMap::new(),
                entries: MapEntries::Ready(Vec::new()),
            };

            // The first value shares the first key's cast context and has no value expectation.
            let value_expectation = EntryExpectation {
                expected_type: ExpectedType::Infer,
                cast_target_context: first_expectation.cast_target_context,
                expected_types: Vec::new(),
            };
            finish_map_entry(
                token_stream,
                context,
                type_interner,
                expression_builder,
                const_values,
                string_table,
                path_fork,
                &mut build,
                first_entry,
                value_expectation,
                retain,
            )?;

            let consumed_close_curly = parse_map_entries(
                token_stream,
                context,
                type_interner,
                expression_builder,
                const_values,
                string_table,
                path_fork,
                &mut build,
                retain,
            )?;
            require_closing_curly(token_stream, consumed_close_curly, string_table)?;

            if build.key.shape.is_some() || build.value.shape.is_some() {
                let key_shape = side_shape_or_concrete(&build.key, context)?;
                let value_shape = side_shape_or_concrete(&build.value, context)?;
                let shape = context.numeric_state_mut().intern_shape(KnownShape::Map {
                    key: key_shape,
                    value: value_shape,
                })?;
                return Ok(CurlyLiteral::Retained(DeferredExpression::Map(Box::new(
                    DeferredMapLiteral {
                        shape,
                        key_shape,
                        value_shape,
                        entries: build.entries.into_deferred(),
                        map_context: None,
                        span: literal_span,
                        value_mode: *value_mode,
                    },
                ))));
            }

            let key_type_id = known_side_type(&build.key)?;
            let value_type_id = known_side_type(&build.value)?;
            let entries = build.entries.into_ready()?;
            Ok(CurlyLiteral::Ready(build_map_expression(
                entries,
                key_type_id,
                value_type_id,
                None,
                literal_span,
                *value_mode,
                type_interner,
                expression_builder,
            )?))
        }

        // First expression is followed by `,` or `}`  =>  collection literal.
        TokenTag::COMMA | TokenTag::CLOSE_CURLY => {
            let mut build = CollectionBuild {
                element: LiteralSide::new(None, ValueReceiverContext::CollectionElement),
                entries: CollectionEntries::Ready(Vec::new()),
            };
            let settled_first = settle_side_value(
                &mut build.element,
                first_entry,
                context,
                type_interner,
                expression_builder,
            )?;
            build.entries.push(settled_first);

            let consumed_close_curly = parse_collection_entries(
                token_stream,
                context,
                type_interner,
                expression_builder,
                const_values,
                string_table,
                path_fork,
                &mut build,
                retain,
            )?;
            require_closing_curly(token_stream, consumed_close_curly, string_table)?;

            if let Some(element_shape) = build.element.shape {
                let shape = context
                    .numeric_state_mut()
                    .intern_shape(KnownShape::Collection {
                        element: element_shape,
                        fixed_capacity: None,
                    })?;
                return Ok(CurlyLiteral::Retained(DeferredExpression::Collection(
                    Box::new(DeferredCollectionLiteral {
                        shape,
                        element_shape,
                        elements: build.entries.into_deferred(),
                        collection_context: None,
                        span: literal_span,
                        value_mode: *value_mode,
                    }),
                )));
            }

            let element_type_id = known_side_type(&build.element)?;
            let items = build.entries.into_ready()?;

            Ok(CurlyLiteral::Ready(build_collection_expression(
                items,
                element_type_id,
                None,
                literal_span,
                *value_mode,
                type_interner,
                expression_builder,
            )?))
        }

        _ => {
            // Any other token after the first expression is invalid in both shapes.
            Err(
                CompilerDiagnostic::missing_collection_item(Some(token_stream.current_span()))
                    .into(),
            )
        }
    }
}

// ------------------------
//  Expression builders
// ------------------------

/// Builds a collection handle, interning its collection type when no explicit one exists.
fn build_collection_expression(
    items: Vec<ExprId>,
    element_type_id: TypeId,
    collection_context: Option<ExpectedCollectionContext>,
    span: Option<SourceSpan>,
    value_mode: ValueMode,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
) -> CollectionParseResult<OwnedExpr> {
    let (collection_type_id, fixed_capacity) = match collection_context {
        Some(ExpectedCollectionContext::Explicit {
            collection_type_id,
            fixed_capacity,
            ..
        }) => (Some(collection_type_id), fixed_capacity),
        Some(ExpectedCollectionContext::CapacityOnlyShorthand { fixed_capacity }) => {
            (None, Some(fixed_capacity))
        }
        None => (None, None),
    };

    Ok(expression_builder.collection_with_type_id(
        expression_builder.own_slice(&items),
        CollectionExpressionType {
            element_type_id,
            fixed_capacity,
            collection_type_id,
        },
        type_interner.environment_mut_for_derived_types(),
        span,
        value_mode,
    )?)
}

/// Builds a map handle from child IDs in the phase-owned map-entry range.
#[allow(
    clippy::too_many_arguments,
    reason = "map expression keeps entries, types, context, span, value mode and interner/builder apart"
)]
fn build_map_expression(
    entries: Vec<MapEntry>,
    key_type_id: TypeId,
    value_type_id: TypeId,
    map_context: Option<ExpectedMapContext>,
    span: Option<SourceSpan>,
    value_mode: ValueMode,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
) -> CollectionParseResult<OwnedExpr> {
    let map_type_id = map_context.and_then(|context| context.map_type_id);
    Ok(expression_builder.map_literal_with_type_id(
        expression_builder.own_slice(&entries),
        MapLiteralExpressionType {
            key_type_id,
            value_type_id,
            map_type_id,
        },
        type_interner.environment_mut_for_derived_types(),
        span,
        value_mode,
    )?)
}

#[cfg(test)]
#[path = "tests/collections_tests.rs"]
mod collections_tests;
