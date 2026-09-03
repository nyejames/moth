//! The one owned, backend-neutral folded-value vocabulary and converter shared by public
//! interface projection and the runtime-template handoff.
//!
//! WHAT: owns [`PublicFoldedValue`], [`PublicFoldedField`], [`FiniteFloat`], the shared owned
//! string vocabulary and the single recursive [`convert_expression_to_folded_value`] converter
//! that translates a finalized, normalized compile-time expression into an owned stable value
//! with no donor-local identity. The converter is shared by the constant folded-value join (R2b)
//! and the parameter/field default projection (R2c) so there is exactly one recursive value
//! vocabulary and one conversion path.
//!
//! WHY: public interfaces and runtime template handoffs both outlive donor-local AST and TIR
//! storage. Keeping their portable string representation here prevents a second parallel piece
//! enum or duplicate conversion implementation at either boundary.

use std::hash::{Hash, Hasher};

use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::const_values::store::{
    ConstStringPiece, ConstStringValue, ConstValueId, ConstValueStore, ConstValueVisit,
};
use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalTypeIdentity, CanonicalTypeProjectionContext, ExportedGenericParameterIdentity,
    GenericParameterOriginResolver, project_type_id_to_canonical_identity,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::{GenericParameterId, TypeId};
use crate::compiler_frontend::instrumentation::{FrontendCounter, increment_frontend_counter};
use crate::compiler_frontend::paths::module_resources::ModuleResourceTable;
use crate::compiler_frontend::paths::resource_identity::StableResourceOriginId;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::synthetic_interface_provenance::SyntheticInterfaceProvenance;

// ===========================================================================
//  Owned folded-value vocabulary
// ===========================================================================

/// One owned field inside a const record or choice variant payload.
///
/// WHAT: preserves the authored field name as an owned stable string, the field value's
/// canonical [`CanonicalTypeIdentity`], and the recursively owned folded value. The name
/// derives from the declaration path's last component while the donor-local string table is
/// available, so the field survives after donor-local `StringId` and `InternedPath`
/// identities are unavailable. The type identity lets import materialize nested values
/// without donor field declarations: a nested anonymous const record projects to
/// `AnonymousConstRecord` and a nested named struct to its source nominal identity.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct PublicFoldedField {
    pub(crate) name: String,
    pub(crate) type_identity: CanonicalTypeIdentity,
    pub(crate) value: PublicFoldedValue,
}

/// One owned structural piece of a folded string.
///
/// WHAT: keeps literal text, resource origins and the site root distinct while owned values cross
/// module boundaries. Resource pieces use [`StableResourceOriginId`] rather than the donor-local
/// [`crate::compiler_frontend::paths::module_resources::ResourceId`].
/// WHY: URL context is assigned by the consuming builder, so a resource-bearing string must not be
/// flattened to rendered text during any projection or runtime handoff.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum OwnedFoldedStringPiece {
    Text(String),
    Resource(StableResourceOriginId),
    SiteRoot,
}

/// One owned folded string.
///
/// WHAT: preserves the compact `Text` fast path for plain strings and stores ordered structural
/// pieces when a value contains a resource origin or site root.
/// WHY: every owned string consumer must confront unresolved resource structure instead of
/// accidentally treating only a separate structural variant as a complete vocabulary.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum OwnedFoldedString {
    Text(String),
    Pieces(Vec<OwnedFoldedStringPiece>),
}

impl OwnedFoldedString {
    /// Move the final text of this string when every piece of it is already known.
    ///
    /// WHAT: returns the owned text for the plain fast path and for a piece list that carries only
    /// text, concatenating those pieces in authored order. A `Resource` or `SiteRoot` piece has no
    /// text until the build assigns URL contexts, so any value containing one returns `None`.
    /// WHY: this is the same availability rule that `require_concrete_text` applies inside the
    /// compiler, so both sides of the module boundary answer one question identically.
    pub(crate) fn into_text(self) -> Option<String> {
        match self {
            Self::Text(text) => Some(text),
            Self::Pieces(pieces) => {
                let mut text = String::new();
                for piece in pieces {
                    match piece {
                        OwnedFoldedStringPiece::Text(part) => text.push_str(&part),
                        OwnedFoldedStringPiece::Resource(_) | OwnedFoldedStringPiece::SiteRoot => {
                            return None;
                        }
                    }
                }
                Some(text)
            }
        }
    }
}

/// Convert a module-local folded string to its owned boundary representation.
///
/// WHAT: resolves text IDs through the donor string table and resource IDs through the donor's
/// resource table, preserving each resource's portable stable origin and the authored piece order.
/// WHY: `ResourceId` is valid only inside the module that issued it. Owned folded values cross
/// module boundaries and therefore must carry `StableResourceOriginId` instead of a local handle.
pub(crate) fn owned_folded_string_from_const_string(
    value: &ConstStringValue,
    resources: &ModuleResourceTable,
    string_table: &StringTable,
) -> Result<OwnedFoldedString, CompilerError> {
    match value {
        ConstStringValue::Text(text) => Ok(OwnedFoldedString::Text(
            string_table.resolve(*text).to_owned(),
        )),
        ConstStringValue::Pieces(pieces) => {
            let mut public_pieces = Vec::with_capacity(pieces.len());
            for piece in pieces {
                let public_piece = match piece {
                    ConstStringPiece::Text(text) => {
                        OwnedFoldedStringPiece::Text(string_table.resolve(*text).to_owned())
                    }
                    ConstStringPiece::Resource(resource) => OwnedFoldedStringPiece::Resource(
                        resources.try_origin(*resource)?.origin.clone(),
                    ),
                    ConstStringPiece::SiteRoot => OwnedFoldedStringPiece::SiteRoot,
                };
                public_pieces.push(public_piece);
            }
            Ok(OwnedFoldedString::Pieces(public_pieces))
        }
    }
}

/// A finite `f64` folded value with an equivalence relation consistent with Moth
/// semantics.
///
/// WHAT: a narrow validated wrapper that rejects non-finite input (`NaN`, `+inf`, `-inf`) at
/// construction and preserves the input value's exact IEEE-754 bits, including the sign of
/// zero. Finiteness makes the manual `PartialEq` a total equivalence relation, so `Eq` is sound.
/// WHY: the language authority defines `Float` as finite `f64`, supports ordinary `Float`
/// equality and ordering, and normalizes `-0.0` to `0` only at the `Float -> String` formatting
/// boundary. It does not declare both IEEE zero signs globally identical. Constant arithmetic,
/// `String -> Float` parsing and `@core/math` helpers such as `atan2` can observe signed zero,
/// so folded public values must retain exact finite bits. The numeric formatter remains the sole
/// normalizer; this wrapper never normalizes.
#[derive(Clone, Debug)]
pub(crate) struct FiniteFloat(f64);

impl FiniteFloat {
    /// Construct a finite float, rejecting non-finite input and preserving exact bits.
    pub(crate) fn new(value: f64) -> Result<Self, CompilerError> {
        if !value.is_finite() {
            return Err(CompilerError::compiler_error(format!(
                "public-interface draft folded-value projection: a non-finite Float value ({}) reached \
             conversion; the AST must not materialize non-finite constants, so this is an \
             internal invariant violation",
                value
            )));
        }
        Ok(Self(value))
    }

    pub(crate) fn value(&self) -> f64 {
        self.0
    }
}

impl PartialEq for FiniteFloat {
    /// Exact-bit equality: two finite floats are equal only when their IEEE-754 bit patterns
    /// match, so `-0.0` and `0.0` are distinct.
    fn eq(&self, other: &Self) -> bool {
        self.0.to_bits() == other.0.to_bits()
    }
}

impl Eq for FiniteFloat {}

impl Hash for FiniteFloat {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.to_bits().hash(state);
    }
}

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
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum PublicFoldedValue {
    Int(i32),
    Float(FiniteFloat),
    Bool(bool),
    Char(char),
    /// A folded template string or plain string literal, retaining structural pieces when
    /// resources or the site root cannot yet be rendered.
    String(OwnedFoldedString),
    /// A provider-folded template transducer that still contains unresolved composition slots.
    /// No donor-local TIR identity crosses this owned value.
    ConstTemplate(PublicConstTemplate),
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

impl PublicFoldedValue {
    /// Visit every canonical type identity retained by this folded value.
    ///
    /// Most folded leaves are intrinsically typed by their enclosing declaration. Choice values
    /// additionally retain their nominal identity, and every record/choice payload field
    /// retains the canonical identity projected from its value's store metadata, so nested
    /// anonymous const records and named structs stay visible to closure and validation
    /// walks, including when nested in records or options.
    pub(crate) fn visit_type_identities(&self, visitor: &mut impl FnMut(&CanonicalTypeIdentity)) {
        match self {
            Self::Collection(values) => {
                for value in values {
                    value.visit_type_identities(visitor);
                }
            }
            Self::Record(fields) => {
                for field in fields {
                    field.type_identity.visit(visitor);
                    field.value.visit_type_identities(visitor);
                }
            }
            Self::Choice {
                type_identity,
                fields,
                ..
            } => {
                type_identity.visit(visitor);
                for field in fields {
                    field.type_identity.visit(visitor);
                    field.value.visit_type_identities(visitor);
                }
            }
            Self::Range { start, end } => {
                start.visit_type_identities(visitor);
                end.visit_type_identities(visitor);
            }
            Self::OptionSome(value) => value.visit_type_identities(visitor),
            Self::Int(_)
            | Self::Float(_)
            | Self::Bool(_)
            | Self::Char(_)
            | Self::String(_)
            | Self::ConstTemplate(_)
            | Self::OptionNone => {}
        }
    }
}

/// Owned value for a const template whose unresolved slots remain composable.
///
/// WHAT: retains authored text and structural string runs alongside unresolved slot payloads.
/// WHY: a template's text runs use the same portable owned string vocabulary as ordinary folded
/// strings, so resource and site-root pieces cannot be flattened while slots cross a module
/// boundary.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct PublicConstTemplate {
    pub(crate) kind: PublicConstTemplateKind,
    pub(crate) pieces: Vec<PublicConstTemplatePiece>,
    pub(crate) conditional_child_wrappers: Vec<PublicConstTemplate>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum PublicConstTemplateKind {
    Wrapper,
    SlotInsert(PublicTemplateSlotKey),
}

/// One authored text or structural run in an owned const-template projection.
///
/// WHAT: carries one contiguous [`OwnedFoldedString`] run or one unresolved slot in authored order.
/// WHY: grouping non-slot pieces lets plain text keep its fast path while structural runs reuse the
/// one portable string vocabulary shared with public constants and runtime handoff.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum PublicConstTemplatePiece {
    Text(OwnedFoldedString),
    Slot(PublicConstTemplateSlot),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum PublicTemplateSlotKey {
    Default,
    Named(String),
    Positional(usize),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct PublicConstTemplateSlot {
    pub(crate) key: PublicTemplateSlotKey,
    pub(crate) applied_child_wrappers: Vec<PublicConstTemplate>,
    pub(crate) child_wrappers: Vec<PublicConstTemplate>,
    pub(crate) skip_parent_child_wrappers: bool,
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

/// Shared inputs for converting one folded value into public owned form.
///
/// WHAT: carries the type, string and canonical-projection authorities used by both direct
/// expression projection and module-store projection. `resources` is present for ordinary module
/// projection and absent for generated generic materialisation, which must reject structural
/// strings until a later phase supplies the consuming module's table.
/// WHY: resource identity is module-local while this converter emits portable public values; the
/// optional table keeps that boundary explicit rather than allowing a caller to flatten pieces.
pub(crate) struct FoldedValueProjectionContext<'a> {
    pub(crate) type_environment: &'a TypeEnvironment,
    pub(crate) string_table: &'a StringTable,
    pub(crate) projection_context: &'a CanonicalTypeProjectionContext<'a>,
    pub(crate) resources: Option<&'a ModuleResourceTable>,
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
    context: &FoldedValueProjectionContext<'_>,
) -> Result<PublicFoldedValue, CompilerError> {
    let type_environment = context.type_environment;
    let string_table = context.string_table;
    let projection_context = context.projection_context;
    increment_frontend_counter(FrontendCounter::PublicFoldedValueConversions);

    match &expression.kind {
        ExpressionKind::Int(value) => Ok(PublicFoldedValue::Int(*value)),
        ExpressionKind::Float(value) => Ok(PublicFoldedValue::Float(FiniteFloat::new(*value)?)),
        ExpressionKind::Bool(value) => Ok(PublicFoldedValue::Bool(*value)),
        ExpressionKind::Char(value) => Ok(PublicFoldedValue::Char(*value)),
        ExpressionKind::StringSlice(string_id) => Ok(PublicFoldedValue::String(
            OwnedFoldedString::Text(string_table.resolve(*string_id).to_owned()),
        )),

        ExpressionKind::StructuralString { pieces } => {
            let resources = context.resources.ok_or_else(|| {
                CompilerError::compiler_error(
                    "public-interface folded expression projection: a structural string reached \
                     a projection context without its module resource table",
                )
            })?;
            let value = ConstStringValue::Pieces(pieces.clone());
            Ok(PublicFoldedValue::String(
                owned_folded_string_from_const_string(&value, resources, string_table)?,
            ))
        }

        ExpressionKind::Collection(items) => {
            let mut folded_items = Vec::with_capacity(items.len());
            for item in items {
                folded_items.push(convert_expression_to_folded_value(item, context)?);
            }
            Ok(PublicFoldedValue::Collection(folded_items))
        }

        ExpressionKind::StructInstance(fields) => {
            let folded_fields = convert_declaration_fields_to_folded_fields(fields, context)?;
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

            let folded_fields = convert_declaration_fields_to_folded_fields(fields, context)?;

            Ok(PublicFoldedValue::Choice {
                type_identity: Box::new(type_identity),
                variant_name,
                fields: folded_fields,
            })
        }

        ExpressionKind::Range(start, end) => {
            let folded_start = convert_expression_to_folded_value(start, context)?;
            let folded_end = convert_expression_to_folded_value(end, context)?;
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
            let folded_value = convert_expression_to_folded_value(value, context)?;
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

/// Convert one module-store value to the owned public folded-value vocabulary.
///
/// This compatibility wrapper discards the aggregate provenance returned by
/// [`convert_const_value_to_folded_value_with_provenance`]. Callers that publish a declaration
/// record should use the provenance-aware variant so nested folded nodes remain represented in the
/// declaration's aggregate fact.
pub(crate) fn convert_const_value_to_folded_value(
    const_values: &ConstValueStore,
    value_id: ConstValueId,
    context: &FoldedValueProjectionContext<'_>,
) -> Result<PublicFoldedValue, CompilerError> {
    convert_const_value_to_folded_value_with_provenance(const_values, value_id, context)
        .map(|(value, _)| value)
}

/// Convert one module-store value to the owned public folded-value vocabulary and collect the
/// synthetic-interface provenance of every visited value node.
///
/// WHAT: consumes the store's postorder visitor exactly once. The returned provenance is the
/// canonical union of the root metadata and every nested folded node's metadata; the folded-value
/// payload itself remains the existing [`PublicFoldedValue`] vocabulary.
pub(crate) fn convert_const_value_to_folded_value_with_provenance(
    const_values: &ConstValueStore,
    value_id: ConstValueId,
    context: &FoldedValueProjectionContext<'_>,
) -> Result<(PublicFoldedValue, SyntheticInterfaceProvenance), CompilerError> {
    let type_environment = context.type_environment;
    let string_table = context.string_table;
    let projection_context = context.projection_context;
    let mut provenance = SyntheticInterfaceProvenance::empty();

    let value = const_values.fold_value(value_id, &mut |metadata, visit| {
        provenance.merge(&metadata.synthetic_interface_provenance);
        increment_frontend_counter(FrontendCounter::PublicFoldedValueConversions);

        match visit {
            ConstValueVisit::Int(value) => Ok(PublicFoldedValue::Int(value)),
            ConstValueVisit::Float(value) => Ok(PublicFoldedValue::Float(FiniteFloat::new(value)?)),
            ConstValueVisit::Bool(value) => Ok(PublicFoldedValue::Bool(value)),
            ConstValueVisit::Char(value) => Ok(PublicFoldedValue::Char(value)),
            ConstValueVisit::String(value) => match value {
                ConstStringValue::Text(text) => Ok(PublicFoldedValue::String(
                    OwnedFoldedString::Text(string_table.resolve(*text).to_owned()),
                )),
                ConstStringValue::Pieces(_) => {
                    let resources = context.resources.ok_or_else(|| {
                        CompilerError::compiler_error(
                            "public-interface folded store projection: a structural string reached \
                             a projection context without its module resource table",
                        )
                    })?;
                    Ok(PublicFoldedValue::String(
                        owned_folded_string_from_const_string(value, resources, string_table)?,
                    ))
                }
            },
            ConstValueVisit::Collection(values) => Ok(PublicFoldedValue::Collection(values)),
            ConstValueVisit::Record(fields) => Ok(PublicFoldedValue::Record(
                fields
                    .into_iter()
                    .map(|field| {
                        let name = field.name.name_str(string_table).ok_or_else(|| {
                            CompilerError::compiler_error(
                                "public-interface folded store record field has no resolvable name",
                            )
                        })?;
                        let type_identity = project_type_id_to_canonical_identity(
                            field.type_id,
                            type_environment,
                            projection_context,
                        )?;
                        Ok(PublicFoldedField {
                            name: name.to_owned(),
                            type_identity,
                            value: field.value,
                        })
                    })
                    .collect::<Result<Vec<_>, CompilerError>>()?,
            )),
            ConstValueVisit::Choice { tag, fields, .. } => {
                let type_identity = project_type_id_to_canonical_identity(
                    metadata.type_id,
                    type_environment,
                    projection_context,
                )?;
                let variant_name = resolve_choice_variant_name(
                    metadata.type_id,
                    tag,
                    type_environment,
                    string_table,
                )?;
                let fields = fields
                    .into_iter()
                    .map(|field| {
                        let name = field.name.name_str(string_table).ok_or_else(|| {
                            CompilerError::compiler_error(
                                "public-interface folded store choice field has no resolvable name",
                            )
                        })?;
                        let field_type_identity = project_type_id_to_canonical_identity(
                            field.type_id,
                            type_environment,
                            projection_context,
                        )?;
                        Ok(PublicFoldedField {
                            name: name.to_owned(),
                            type_identity: field_type_identity,
                            value: field.value,
                        })
                    })
                    .collect::<Result<Vec<_>, CompilerError>>()?;
                Ok(PublicFoldedValue::Choice {
                    type_identity: Box::new(type_identity),
                    variant_name,
                    fields,
                })
            }
            ConstValueVisit::Range { start, end } => Ok(PublicFoldedValue::Range {
                start: Box::new(start),
                end: Box::new(end),
            }),
            ConstValueVisit::Coerced(value) => Ok(value),
            ConstValueVisit::OptionSome(value) => {
                Ok(PublicFoldedValue::OptionSome(Box::new(value)))
            }
            ConstValueVisit::OptionNone => Ok(PublicFoldedValue::OptionNone),
            ConstValueVisit::Template { template, .. } => {
                Ok(PublicFoldedValue::ConstTemplate(template.clone()))
            }
        }
    })?;

    Ok((value, provenance))
}

/// Convert a slice of [`Declaration`] fields to owned [`PublicFoldedField`] values.
///
/// WHAT: resolves each field name from the declaration path's last component through the
/// string table and recursively converts each field value. Preserves authored field order.
pub(crate) fn convert_declaration_fields_to_folded_fields(
    fields: &[Declaration],
    context: &FoldedValueProjectionContext<'_>,
) -> Result<Vec<PublicFoldedField>, CompilerError> {
    let string_table = context.string_table;
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

        let type_identity = project_type_id_to_canonical_identity(
            field.value.type_id,
            context.type_environment,
            context.projection_context,
        )?;

        let value = convert_expression_to_folded_value(&field.value, context)?;

        folded_fields.push(PublicFoldedField {
            name,
            type_identity,
            value,
        });
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
