//! The one owned, backend-neutral folded-value vocabulary and converter for the public
//! interface draft.
//!
//! WHAT: owns [`PublicFoldedValue`], [`PublicFoldedField`], [`FiniteFloat`] and the single
//! recursive [`convert_expression_to_folded_value`] converter that translates a finalized,
//! normalized compile-time expression into an owned stable value with no donor-local identity.
//! The converter is shared by the constant folded-value join (R2b) and the parameter/field
//! default projection (R2c) so there is exactly one recursive value vocabulary and one
//! conversion path.
//!
//! WHY: the public interface must own its folded values so downstream provider binding and
//! cross-module consumers read one backend-neutral value shape instead of donor-local AST
//! expression identity. Keeping the vocabulary and converter in one narrow module prevents a
//! second parallel value enum or duplicate conversion implementation.

use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalTypeIdentity, CanonicalTypeProjectionContext, ExportedGenericParameterIdentity,
    GenericParameterOriginResolver, project_type_id_to_canonical_identity,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::{GenericParameterId, TypeId};
use crate::compiler_frontend::symbols::string_interning::StringTable;

// ===========================================================================
//  Owned folded-value vocabulary
// ===========================================================================

/// One owned field inside a const record or choice variant payload.
///
/// WHAT: preserves the authored field name as an owned stable string and the recursively
/// owned folded value. The name derives from the declaration path's last component while
/// the donor-local string table is available, so the field survives after donor-local
/// `StringId` and `InternedPath` identities are unavailable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PublicFoldedField {
    pub(crate) name: String,
    pub(crate) value: PublicFoldedValue,
}

/// A finite `f64` folded value with an equivalence relation consistent with Moth
/// semantics.
///
/// WHAT: a narrow validated wrapper that rejects non-finite input (`NaN`, `+inf`, `-inf`) and
/// normalizes negative zero to positive zero at construction. Finiteness makes `PartialEq` a
/// total equivalence relation, so the draft hierarchy can derive `Eq`.
/// WHY: `f64` itself does not implement `Eq` because `NaN != NaN`. Moth formatting renders
/// negative zero as ordinary zero, so `-0.0` is unobservable as text; normalizing it keeps the
/// folded value canonical and equality consistent with the accepted formatting contract.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FiniteFloat(f64);

impl FiniteFloat {
    /// Construct a finite float, rejecting non-finite input and normalizing negative zero.
    pub(crate) fn new(value: f64) -> Result<Self, CompilerError> {
        if !value.is_finite() {
            return Err(CompilerError::compiler_error(format!(
                "public-interface draft folded-value projection: a non-finite Float value ({}) reached \
             conversion; the AST must not materialize non-finite constants, so this is an \
             internal invariant violation",
                value
            )));
        }
        // `-0.0 == 0.0` under ordinary float equality, so this branch normalizes both signs to
        // positive zero. The accepted formatting contract renders negative zero as zero.
        let normalized = if value == 0.0 { 0.0 } else { value };
        Ok(Self(normalized))
    }

    /// Return the exact normalized IEEE-754 bits used by the canonical interface encoder.
    ///
    /// The encoder consumes this read-only value rather than formatting a float, so distinct
    /// finite semantic values remain distinct while `-0.0` and `0.0` share one canonical form.
    #[allow(dead_code)]
    pub(crate) fn normalized_bits(&self) -> u64 {
        self.0.to_bits()
    }
}

impl Eq for FiniteFloat {}

/// The owned, backend-neutral, recursive folded value for one directly exported constant or
/// retained default.
///
/// WHAT: one explicit public-interface value vocabulary for the complete normalized
/// compile-time shapes that can legally reach the draft boundary after AST normalization:
/// directly exported constants (R2b) and function-parameter, receiver-parameter or
/// struct-field defaults (R2c). Every leaf is an owned stable value: no `TypeId`,
/// `NominalTypeId`, `StringId`,
/// `InternedPath`, source location, AST/TIR identity, HIR ID, local choice tag/index or
/// absolute path crosses this boundary. Choice variants carry a stable variant name
/// derived from the donor-local type environment while it is available, not a local tag
/// index. Option presence is modeled by the recursive `OptionSome`/`OptionNone` variants, not
/// by a residual coercion operation: the interface contains values, not conversion
/// instructions.
///
/// WHY: the public interface must own its folded values so downstream provider binding and
/// cross-module consumers read one backend-neutral value shape instead of donor-local AST
/// expression identity. The vocabulary is recursive so nested const-record fields, choice
/// payloads, collection elements and option payloads all project through the same conversion.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PublicFoldedValue {
    Int(i32),
    Float(FiniteFloat),
    Bool(bool),
    Char(char),
    /// A folded template string or a plain string literal, resolved to an owned `String`.
    String(String),
    /// An ordered homogeneous collection of folded values.
    Collection(Vec<PublicFoldedValue>),
    /// A const record: ordered owned field names with recursively owned field values.
    Record(Vec<PublicFoldedField>),
    /// A choice variant with a stable variant name, the boxed choice type identity and
    /// ordered owned payload fields. The type identity is boxed to keep the recursive value
    /// enum small.
    Choice {
        type_identity: Box<CanonicalTypeIdentity>,
        variant_name: String,
        fields: Vec<PublicFoldedField>,
    },
    /// An inclusive range with folded start and end values.
    Range {
        start: Box<PublicFoldedValue>,
        end: Box<PublicFoldedValue>,
    },
    /// A present option value wrapping a recursively folded inner value. Nested options
    /// recurse through the same conversion, so `Option<Option<T>>` produces
    /// `OptionSome(OptionSome(...))`.
    OptionSome(Box<PublicFoldedValue>),
    /// An absent option value.
    OptionNone,
}

// ===========================================================================
//  Folded-value conversion
// ===========================================================================

/// A generic-parameter resolver that rejects every request.
///
/// WHAT: folded constant values are concrete, so a `GenericParameterId` reaching
/// the canonical type projection during folded-value conversion is an internal invariant
/// violation. This resolver returns a precise `CompilerError` instead of inventing an
/// identity. Default projection supplies its declaration-aware generic resolver separately.
pub(crate) struct FoldedValueGenericParameterResolver;

impl GenericParameterOriginResolver for FoldedValueGenericParameterResolver {
    fn resolve_generic_parameter_origin(
        &self,
        parameter_id: GenericParameterId,
    ) -> Result<ExportedGenericParameterIdentity, CompilerError> {
        Err(CompilerError::compiler_error(format!(
            "public-interface draft folded-value projection: GenericParameterId({}) reached \
             canonical projection inside a folded constant value; folded constants are concrete \
             so a generic parameter is an internal invariant violation",
            parameter_id.0
        )))
    }
}

/// Convert one finalized and normalized AST compile-time expression to an owned
/// [`PublicFoldedValue`].
///
/// WHAT: recursively walks the expression kind, resolving donor-local `StringId`s to owned
/// `String`s, donor-local choice tag indexes to stable variant names through the type
/// environment, and donor-local `TypeId`s to [`CanonicalTypeIdentity`] through the canonical
/// type projection. Every leaf is an owned stable value with no donor-local identity.
///
/// A shape that cannot legally reach a normalized exported constant or retained default returns
/// a deterministic `CompilerError` naming the invariant instead of silently omitting the value.
pub(crate) fn convert_expression_to_folded_value(
    expression: &Expression,
    type_environment: &TypeEnvironment,
    string_table: &StringTable,
    projection_context: &CanonicalTypeProjectionContext,
) -> Result<PublicFoldedValue, CompilerError> {
    match &expression.kind {
        ExpressionKind::Int(value) => Ok(PublicFoldedValue::Int(*value)),
        ExpressionKind::Float(value) => Ok(PublicFoldedValue::Float(FiniteFloat::new(*value)?)),
        ExpressionKind::Bool(value) => Ok(PublicFoldedValue::Bool(*value)),
        ExpressionKind::Char(value) => Ok(PublicFoldedValue::Char(*value)),

        ExpressionKind::StringSlice(string_id) => Ok(PublicFoldedValue::String(
            string_table.resolve(*string_id).to_owned(),
        )),

        ExpressionKind::Collection(items) => {
            let mut folded_items = Vec::with_capacity(items.len());
            for item in items {
                folded_items.push(convert_expression_to_folded_value(
                    item,
                    type_environment,
                    string_table,
                    projection_context,
                )?);
            }
            Ok(PublicFoldedValue::Collection(folded_items))
        }

        ExpressionKind::StructInstance(fields) => {
            let folded_fields = convert_declaration_fields_to_folded_fields(
                fields,
                type_environment,
                string_table,
                projection_context,
            )?;
            Ok(PublicFoldedValue::Record(folded_fields))
        }

        ExpressionKind::ChoiceConstruct { tag, fields, .. } => {
            let type_identity = project_type_id_to_canonical_identity(
                expression.type_id,
                type_environment,
                projection_context,
            )?;

            let variant_name = resolve_choice_variant_name(
                expression.type_id,
                *tag,
                type_environment,
                string_table,
            )?;

            let folded_fields = convert_declaration_fields_to_folded_fields(
                fields,
                type_environment,
                string_table,
                projection_context,
            )?;

            Ok(PublicFoldedValue::Choice {
                type_identity: Box::new(type_identity),
                variant_name,
                fields: folded_fields,
            })
        }

        ExpressionKind::Range(start, end) => {
            let folded_start = convert_expression_to_folded_value(
                start,
                type_environment,
                string_table,
                projection_context,
            )?;
            let folded_end = convert_expression_to_folded_value(
                end,
                type_environment,
                string_table,
                projection_context,
            )?;
            Ok(PublicFoldedValue::Range {
                start: Box::new(folded_start),
                end: Box::new(folded_end),
            })
        }

        ExpressionKind::Coerced { value, to_type } => {
            let inner_type_id = value.type_id;
            if type_environment.option_inner_type(*to_type) != Some(inner_type_id) {
                return Err(CompilerError::compiler_error(format!(
                    "public-interface draft folded-value projection: a Coerced expression with \
                     target TypeId({}) and inner TypeId({}) is not an option-present wrap of the \
                     inner type; only `T -> T?` coercion can legally reach this boundary",
                    to_type.0, inner_type_id.0
                )));
            }
            let folded_value = convert_expression_to_folded_value(
                value,
                type_environment,
                string_table,
                projection_context,
            )?;
            Ok(PublicFoldedValue::OptionSome(Box::new(folded_value)))
        }

        ExpressionKind::OptionNone => Ok(PublicFoldedValue::OptionNone),

        ExpressionKind::Template(_) => Err(CompilerError::compiler_error(
            "public-interface draft folded-value projection: a Template expression reached \
             conversion; normalization folds renderable templates to StringSlice and filters \
             slot-insert helpers, so only a loop-control signal could remain and it is not a \
             data value",
        )),

        ExpressionKind::Reference(_) => Err(CompilerError::compiler_error(
            "public-interface draft folded-value projection: a Reference expression reached \
             conversion; constant references are resolved and inlined by the established \
             function-signature and struct-default owners before finalization, so an unresolved \
             reference in an exported constant or a retained default is an internal invariant \
             violation",
        )),

        // Every remaining variant is not a folded value shape and must not reach a normalized
        // exported constant or retained default. Report the exact kind name so the invariant is
        // clear.
        kind => Err(CompilerError::compiler_error(format!(
            "public-interface draft folded-value projection: expression kind {:?} is not a \
             supported normalized constant value shape; only scalars, collections, const \
             records, choices, ranges and option-present wraps can legally reach this \
             boundary",
            kind
        ))),
    }
}

/// Convert a slice of [`Declaration`] fields to owned [`PublicFoldedField`] values.
///
/// WHAT: resolves each field name from the declaration path's last component through the
/// string table and recursively converts each field value. Preserves authored field order.
pub(crate) fn convert_declaration_fields_to_folded_fields(
    fields: &[Declaration],
    type_environment: &TypeEnvironment,
    string_table: &StringTable,
    projection_context: &CanonicalTypeProjectionContext,
) -> Result<Vec<PublicFoldedField>, CompilerError> {
    let mut folded_fields = Vec::with_capacity(fields.len());
    for field in fields {
        let name = field
            .id
            .name_str(string_table)
            .ok_or_else(|| {
                CompilerError::compiler_error(
                    "public-interface draft folded-value projection: a const-record or choice \
             payload field declaration has no resolvable field name; the interned path \
             is empty",
                )
            })?
            .to_owned();

        let value = convert_expression_to_folded_value(
            &field.value,
            type_environment,
            string_table,
            projection_context,
        )?;

        folded_fields.push(PublicFoldedField { name, value });
    }
    Ok(folded_fields)
}

/// Resolve a choice variant's stable name from the donor-local tag index.
///
/// WHAT: looks up the choice definition for the expression's `type_id` through the type
/// environment, finds the variant with the matching tag, and resolves its `StringId` name to
/// an owned `String`. This replaces the donor-local tag index with a stable variant name
/// while the local type environment and string table are available.
pub(crate) fn resolve_choice_variant_name(
    type_id: TypeId,
    tag: usize,
    type_environment: &TypeEnvironment,
    string_table: &StringTable,
) -> Result<String, CompilerError> {
    let variants = type_environment.variants_for(type_id).ok_or_else(|| {
        CompilerError::compiler_error(format!(
            "public-interface draft folded-value projection: the choice construct TypeId({}) \
             has no choice definition in the TypeEnvironment; a ChoiceConstruct must \
             resolve to a choice or generic choice instance",
            type_id.0
        ))
    })?;

    let variant = variants.iter().find(|v| v.tag == tag).ok_or_else(|| {
        CompilerError::compiler_error(format!(
            "public-interface draft folded-value projection: the choice construct TypeId({}) \
             has no variant with tag {}; the tag is out of range",
            type_id.0, tag
        ))
    })?;

    Ok(string_table.resolve(variant.name).to_owned())
}
