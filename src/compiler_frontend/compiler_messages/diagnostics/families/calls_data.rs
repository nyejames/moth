//! Family rows for call, data and multi-bind diagnostics.
//!
//! WHAT: compact family rows and legacy adapters for calls, data, and multi-bind diagnostics.
//! WHY: typed facts retain message inputs while stable reason identity follows their reason codes.

use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::facts::{
    CountFact, OptCountFact, OptStringFact, OptTypeFact, OptUnitFact, TypeFact, UnitFact, unit_code,
};
use crate::compiler_frontend::compiler_messages::diagnostics::schema::{
    FamilyRow, diagnostic_family,
};
use crate::compiler_frontend::compiler_messages::render::{
    render_invalid_builtin_call, render_invalid_call_shape, render_invalid_cast,
    render_invalid_collection_type, render_invalid_copy_target, render_invalid_field_access,
    render_invalid_map_literal, render_invalid_map_type, render_invalid_multi_bind,
    render_invalid_range_operand, render_invalid_receiver_call, render_invalid_return_shape,
    render_invalid_value_receiver,
};
use crate::compiler_frontend::compiler_messages::{
    DiagnosticCategory, DiagnosticKind, DiagnosticPayload, DiagnosticSeverity,
    InvalidBuiltinCallReason, InvalidCallShapeReason, InvalidCastReason,
    InvalidCollectionTypeReason, InvalidCopyTargetReason, InvalidFieldAccessReason,
    InvalidMapLiteralReason, InvalidMapTypeReason, InvalidMultiBindReason,
    InvalidRangeOperandReason, InvalidReceiverCallReason, InvalidReturnShapeReason,
    RangeOperandKind, ReceiverCallKind, RuleDiagnosticKind, SyntaxDiagnosticKind,
    ValueReceiverContext,
};
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::string_interning::StringId;

unit_code! {
    impl InvalidMapLiteralReason {
        MixedCollectionMapEntries = 1,
        DuplicateKnownKey = 2,
        MissingKeyExpression = 3,
        MissingValueExpression = 4,
    }
}

unit_code! {
    impl InvalidBuiltinCallReason {
        MissingParentheses = 1,
        TakesNoArguments = 2,
        NamedArgumentsNotSupported = 3,
        UnhandledFallibleCall = 4,
        DoesNotAcceptMutableAccess = 5,
        CastMissingParentheses = 6,
        CastMissingArgument = 7,
        CastTooManyArguments = 8,
        CastMissingClosingParenthesis = 9,
        MissingArgument = 10,
        TooManyArguments = 11,
        ExpressionPositionNotAllowed = 12,
        MapLengthIsProperty = 13,
        ScalarConstructorRemoved = 14,
    }
}

unit_code! {
    impl InvalidCopyTargetReason {
        FunctionName = 1,
        FunctionCall = 2,
        NonPlace = 3,
        MutableMarkerNotAllowed = 4,
    }
}

unit_code! {
    impl InvalidRangeOperandReason {
        NotNumeric = 1,
        IncompatibleDomain = 2,
    }
}

unit_code! {
    impl RangeOperandKind {
        Start = 1,
        End = 2,
        Step = 3,
    }
}

unit_code! {
    impl InvalidCastReason {
        MissingExplicitTarget = 1,
        TargetNotBuiltin = 2,
        TargetIsGenericParameter = 3,
        SameSourceAndTarget = 4,
        SourceIsOptional = 5,
        OperandArityMismatch = 6,
        TargetArityMismatch = 7,
        CastPropagationRemoved = 8,
        ScalarConstructorRemoved = 9,
        NoEvidence = 10,
        BuiltinEvidenceNotConstFoldable = 11,
        UserDefinedEvidenceNotConstFoldable = 12,
        GenericBoundEvidenceNotConstFoldable = 13,
        BuiltinCastFailedInConst = 14,
    }
}

unit_code! {
    impl InvalidReceiverCallReason {
        CalledAsFreeFunction = 1,
        MustUseParentheses = 2,
        ConstRecordNoRuntimeCalls = 3,
        MutableReceiverMissingMarker = 4,
        ImmutableReceiverMutableMethod = 5,
        NonPlaceReceiverMutableMethod = 6,
        MutableMarkerOnImmutableReceiver = 7,
        MutableMarkerOnNonPlaceReceiver = 8,
        UnneededMutableAccessMarker = 9,
        MutableMarkerOnNonReceiverCall = 10,
        AmbiguousGenericBoundMethod = 11,
    }
}

unit_code! {
    impl ReceiverCallKind {
        SourceMethod = 1,
        CollectionBuiltin = 2,
        MapBuiltin = 3,
    }
}

unit_code! {
    impl InvalidFieldAccessReason {
        ExpectedNameAfterDot = 1,
        FieldNotMethod = 2,
        ChoicePayloadMutation = 3,
        ChoicePayloadDeferred = 4,
        UnknownExternalMember = 5,
        UnknownMember = 6,
    }
}

unit_code! {
    impl ValueReceiverContext {
        Declaration = 1,
        Assignment = 2,
        Return = 3,
        Then = 4,
        FunctionArgument = 5,
        ConstructorArgument = 6,
        OperatorOperand = 7,
        Condition = 8,
        ReceiverBase = 9,
        CollectionElement = 10,
        MapKey = 11,
        MapValue = 12,
        TemplateInterpolation = 13,
        CastSource = 14,
        MatchScrutinee = 15,
        AssertionArgument = 16,
        General = 17,
    }
}

unit_code! {
    pub(crate) enum InvalidCallShapeCode {
        MissingArgument = 1,
        ExtraPositionalArgument = 2,
        DuplicateArgument = 3,
        NamedArgumentNotFound = 4,
        PositionalAfterNamed = 5,
        NamedArgumentsNotSupported = 6,
        WireArgumentOrigin = 7,
        WireArgumentMutableMarker = 8,
        RouteArgumentNotDirect = 9,
        RouteForwardingContractMismatch = 10,
        RouteParameterRead = 11,
        RouteInputFieldMismatch = 12,
        MutableAccessRequired = 13,
        MutableAccessNotAllowed = 14,
        MutableAccessOnNonPlace = 15,
        MutableAccessOnImmutablePlace = 16,
        ImmutablePlaceMutableAccessRequired = 17,
    }
}

impl InvalidCallShapeCode {
    pub(crate) fn of(reason: &InvalidCallShapeReason) -> Self {
        match reason {
            InvalidCallShapeReason::MissingArgument { .. } => Self::MissingArgument,
            InvalidCallShapeReason::ExtraPositionalArgument { .. } => Self::ExtraPositionalArgument,
            InvalidCallShapeReason::DuplicateArgument { .. } => Self::DuplicateArgument,
            InvalidCallShapeReason::NamedArgumentNotFound { .. } => Self::NamedArgumentNotFound,
            InvalidCallShapeReason::PositionalAfterNamed => Self::PositionalAfterNamed,
            InvalidCallShapeReason::NamedArgumentsNotSupported => Self::NamedArgumentsNotSupported,
            InvalidCallShapeReason::WireArgumentOrigin => Self::WireArgumentOrigin,
            InvalidCallShapeReason::WireArgumentMutableMarker => Self::WireArgumentMutableMarker,
            InvalidCallShapeReason::RouteArgumentNotDirect => Self::RouteArgumentNotDirect,
            InvalidCallShapeReason::RouteForwardingContractMismatch => {
                Self::RouteForwardingContractMismatch
            }
            InvalidCallShapeReason::RouteParameterRead => Self::RouteParameterRead,
            InvalidCallShapeReason::RouteInputFieldMismatch => Self::RouteInputFieldMismatch,
            InvalidCallShapeReason::MutableAccessRequired { .. } => Self::MutableAccessRequired,
            InvalidCallShapeReason::MutableAccessNotAllowed { .. } => Self::MutableAccessNotAllowed,
            InvalidCallShapeReason::MutableAccessOnNonPlace { .. } => Self::MutableAccessOnNonPlace,
            InvalidCallShapeReason::MutableAccessOnImmutablePlace { .. } => {
                Self::MutableAccessOnImmutablePlace
            }
            InvalidCallShapeReason::ImmutablePlaceMutableAccessRequired { .. } => {
                Self::ImmutablePlaceMutableAccessRequired
            }
        }
    }
}
unit_code! {
    pub(crate) enum InvalidCollectionTypeCode {
        NegativeCapacity = 1,
        ShorthandCapacityNotAllowed = 2,
        ZeroCapacity = 3,
        CapacityNotInt = 4,
        CapacityNotConstant = 5,
        CapacityOverflow = 6,
        InitializerExceedsFixedCapacity = 7,
        EmptyImmutableFixedCollection = 8,
        ShorthandEmptyLiteralAmbiguous = 9,
        ShorthandNonLiteralRhs = 10,
    }
}

impl InvalidCollectionTypeCode {
    pub(crate) fn of(reason: &InvalidCollectionTypeReason) -> Self {
        match reason {
            InvalidCollectionTypeReason::NegativeCapacity => Self::NegativeCapacity,
            InvalidCollectionTypeReason::ShorthandCapacityNotAllowed => {
                Self::ShorthandCapacityNotAllowed
            }
            InvalidCollectionTypeReason::ZeroCapacity => Self::ZeroCapacity,
            InvalidCollectionTypeReason::CapacityNotInt => Self::CapacityNotInt,
            InvalidCollectionTypeReason::CapacityNotConstant => Self::CapacityNotConstant,
            InvalidCollectionTypeReason::CapacityOverflow => Self::CapacityOverflow,
            InvalidCollectionTypeReason::InitializerExceedsFixedCapacity { .. } => {
                Self::InitializerExceedsFixedCapacity
            }
            InvalidCollectionTypeReason::EmptyImmutableFixedCollection => {
                Self::EmptyImmutableFixedCollection
            }
            InvalidCollectionTypeReason::ShorthandEmptyLiteralAmbiguous => {
                Self::ShorthandEmptyLiteralAmbiguous
            }
            InvalidCollectionTypeReason::ShorthandNonLiteralRhs => Self::ShorthandNonLiteralRhs,
        }
    }

    pub(crate) fn to_reason(
        self,
        capacity: Option<usize>,
        length: Option<usize>,
    ) -> InvalidCollectionTypeReason {
        match self {
            Self::NegativeCapacity => InvalidCollectionTypeReason::NegativeCapacity,
            Self::ShorthandCapacityNotAllowed => {
                InvalidCollectionTypeReason::ShorthandCapacityNotAllowed
            }
            Self::ZeroCapacity => InvalidCollectionTypeReason::ZeroCapacity,
            Self::CapacityNotInt => InvalidCollectionTypeReason::CapacityNotInt,
            Self::CapacityNotConstant => InvalidCollectionTypeReason::CapacityNotConstant,
            Self::CapacityOverflow => InvalidCollectionTypeReason::CapacityOverflow,
            Self::InitializerExceedsFixedCapacity => {
                InvalidCollectionTypeReason::InitializerExceedsFixedCapacity {
                    capacity: capacity.expect("collection capacity code stores its capacity"),
                    length: length.expect("collection capacity code stores its length"),
                }
            }
            Self::EmptyImmutableFixedCollection => {
                InvalidCollectionTypeReason::EmptyImmutableFixedCollection
            }
            Self::ShorthandEmptyLiteralAmbiguous => {
                InvalidCollectionTypeReason::ShorthandEmptyLiteralAmbiguous
            }
            Self::ShorthandNonLiteralRhs => InvalidCollectionTypeReason::ShorthandNonLiteralRhs,
        }
    }
}

unit_code! {
    pub(crate) enum InvalidMapTypeCode {
        UnsupportedKeyType = 1,
        ExcessiveInlineNesting = 2,
        EmptyMapKeyType = 3,
        EmptyMapValueType = 4,
        MultipleMapSeparators = 5,
        FixedCapacityNotAllowed = 6,
    }
}

impl InvalidMapTypeCode {
    pub(crate) fn of(reason: &InvalidMapTypeReason) -> Self {
        match reason {
            InvalidMapTypeReason::UnsupportedKeyType { .. } => Self::UnsupportedKeyType,
            InvalidMapTypeReason::ExcessiveInlineNesting { .. } => Self::ExcessiveInlineNesting,
            InvalidMapTypeReason::EmptyMapKeyType => Self::EmptyMapKeyType,
            InvalidMapTypeReason::EmptyMapValueType => Self::EmptyMapValueType,
            InvalidMapTypeReason::MultipleMapSeparators => Self::MultipleMapSeparators,
            InvalidMapTypeReason::FixedCapacityNotAllowed => Self::FixedCapacityNotAllowed,
        }
    }
}

unit_code! {
    pub(crate) enum InvalidReturnShapeCode {
        BareReturnWithExpectedValues = 1,
        ReturnValuesWithBareSignature = 2,
        TooManyReturnValues = 3,
        TooFewReturnValues = 4,
        MissingReturnBangValue = 5,
        FunctionMayFallThrough = 6,
    }
}

impl InvalidReturnShapeCode {
    pub(crate) fn of(reason: &InvalidReturnShapeReason) -> Self {
        match reason {
            InvalidReturnShapeReason::BareReturnWithExpectedValues { .. } => {
                Self::BareReturnWithExpectedValues
            }
            InvalidReturnShapeReason::ReturnValuesWithBareSignature => {
                Self::ReturnValuesWithBareSignature
            }
            InvalidReturnShapeReason::TooManyReturnValues { .. } => Self::TooManyReturnValues,
            InvalidReturnShapeReason::TooFewReturnValues { .. } => Self::TooFewReturnValues,
            InvalidReturnShapeReason::MissingReturnBangValue => Self::MissingReturnBangValue,
            InvalidReturnShapeReason::FunctionMayFallThrough => Self::FunctionMayFallThrough,
        }
    }

    pub(crate) fn to_reason(
        self,
        expected_count: Option<usize>,
        provided_count: Option<usize>,
    ) -> InvalidReturnShapeReason {
        match self {
            Self::BareReturnWithExpectedValues => {
                InvalidReturnShapeReason::BareReturnWithExpectedValues {
                    expected_count: expected_count
                        .expect("bare return code stores its expected value count"),
                }
            }
            Self::ReturnValuesWithBareSignature => {
                InvalidReturnShapeReason::ReturnValuesWithBareSignature
            }
            Self::TooManyReturnValues => InvalidReturnShapeReason::TooManyReturnValues {
                expected_count: expected_count.expect("too-many code stores expected value count"),
            },
            Self::TooFewReturnValues => InvalidReturnShapeReason::TooFewReturnValues {
                expected_count: expected_count.expect("too-few code stores expected value count"),
                provided_count: provided_count.expect("too-few code stores provided value count"),
            },
            Self::MissingReturnBangValue => InvalidReturnShapeReason::MissingReturnBangValue,
            Self::FunctionMayFallThrough => InvalidReturnShapeReason::FunctionMayFallThrough,
        }
    }
}

unit_code! {
    pub(crate) enum InvalidMultiBindCode {
        ThisTargetReserved = 1,
        ExpectedTargetName = 2,
        MissingTargetAfterComma = 3,
        MissingAssignmentOperator = 4,
        InvalidTokenAfterTarget = 5,
        MissingRightHandExpression = 6,
        MultipleRightHandExpressions = 7,
        MutableTargetNeedsExplicitType = 8,
        CompileTimeMarkerOnTarget = 9,
        DuplicateTarget = 10,
        UnsupportedRhs = 11,
        ExistingTargetMutableMarker = 12,
        ExistingTargetImmutable = 13,
        ArityMismatch = 14,
        RhsNotMultiValue = 15,
    }
}

impl InvalidMultiBindCode {
    /// The code for a legacy reason. The arity counts are stored separately, so `ArityMismatch` carries no data here.
    pub(crate) fn of(reason: &InvalidMultiBindReason) -> Self {
        match reason {
            InvalidMultiBindReason::ThisTargetReserved => Self::ThisTargetReserved,
            InvalidMultiBindReason::ExpectedTargetName => Self::ExpectedTargetName,
            InvalidMultiBindReason::MissingTargetAfterComma => Self::MissingTargetAfterComma,
            InvalidMultiBindReason::MissingAssignmentOperator => Self::MissingAssignmentOperator,
            InvalidMultiBindReason::InvalidTokenAfterTarget => Self::InvalidTokenAfterTarget,
            InvalidMultiBindReason::MissingRightHandExpression => Self::MissingRightHandExpression,
            InvalidMultiBindReason::MultipleRightHandExpressions => {
                Self::MultipleRightHandExpressions
            }
            InvalidMultiBindReason::MutableTargetNeedsExplicitType => {
                Self::MutableTargetNeedsExplicitType
            }
            InvalidMultiBindReason::CompileTimeMarkerOnTarget => Self::CompileTimeMarkerOnTarget,
            InvalidMultiBindReason::DuplicateTarget => Self::DuplicateTarget,
            InvalidMultiBindReason::UnsupportedRhs => Self::UnsupportedRhs,
            InvalidMultiBindReason::ExistingTargetMutableMarker => {
                Self::ExistingTargetMutableMarker
            }
            InvalidMultiBindReason::ExistingTargetImmutable => Self::ExistingTargetImmutable,
            InvalidMultiBindReason::ArityMismatch { .. } => Self::ArityMismatch,
            InvalidMultiBindReason::RhsNotMultiValue => Self::RhsNotMultiValue,
        }
    }

    /// The legacy reason for this code. The arity counts fill `ArityMismatch`, the only reason that reads them.
    pub(crate) fn to_reason(self, expected: usize, found: usize) -> InvalidMultiBindReason {
        match self {
            Self::ThisTargetReserved => InvalidMultiBindReason::ThisTargetReserved,
            Self::ExpectedTargetName => InvalidMultiBindReason::ExpectedTargetName,
            Self::MissingTargetAfterComma => InvalidMultiBindReason::MissingTargetAfterComma,
            Self::MissingAssignmentOperator => InvalidMultiBindReason::MissingAssignmentOperator,
            Self::InvalidTokenAfterTarget => InvalidMultiBindReason::InvalidTokenAfterTarget,
            Self::MissingRightHandExpression => InvalidMultiBindReason::MissingRightHandExpression,
            Self::MultipleRightHandExpressions => {
                InvalidMultiBindReason::MultipleRightHandExpressions
            }
            Self::MutableTargetNeedsExplicitType => {
                InvalidMultiBindReason::MutableTargetNeedsExplicitType
            }
            Self::CompileTimeMarkerOnTarget => InvalidMultiBindReason::CompileTimeMarkerOnTarget,
            Self::DuplicateTarget => InvalidMultiBindReason::DuplicateTarget,
            Self::UnsupportedRhs => InvalidMultiBindReason::UnsupportedRhs,
            Self::ExistingTargetMutableMarker => {
                InvalidMultiBindReason::ExistingTargetMutableMarker
            }
            Self::ExistingTargetImmutable => InvalidMultiBindReason::ExistingTargetImmutable,
            Self::ArityMismatch => InvalidMultiBindReason::ArityMismatch { expected, found },
            Self::RhsNotMultiValue => InvalidMultiBindReason::RhsNotMultiValue,
        }
    }
}

diagnostic_family! {
    INVALID_MULTI_BIND {
        internal: 3045,
        stable: "MOTH-RULE-0045",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "Invalid multi-bind",
        facts: [
            reason: UnitFact<InvalidMultiBindCode>,
            target_name: OptStringFact,
            expected: CountFact,
            found: CountFact,
        ],
        labels: [],
        extra: None,
        ctor: manual,
        reason_key: reason,
        view: InvalidMultiBindView,
        render: render_invalid_multi_bind,
    }
}

/// The invalid-multi-bind draft for a rule-kind diagnostic. The arity counts are zero unless the reason is
/// `ArityMismatch`.
pub(crate) fn invalid_multi_bind(
    reason: InvalidMultiBindReason,
    target_name: Option<StringId>,
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    let (expected, found) = match reason {
        InvalidMultiBindReason::ArityMismatch { expected, found } => (expected, found),
        _ => (0, 0),
    };
    let code = InvalidMultiBindCode::of(&reason);
    DiagnosticDraft::with_facts(&INVALID_MULTI_BIND, span, |cursor| {
        cursor.push::<UnitFact<InvalidMultiBindCode>>(&code)?;
        cursor.push::<OptStringFact>(&target_name)?;
        cursor.push::<CountFact>(&expected)?;
        cursor.push::<CountFact>(&found)?;
        Ok(())
    })
}

/// Builds the compact map-literal diagnostic from its unit reason.
pub(crate) fn invalid_map_literal(
    reason: InvalidMapLiteralReason,
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    DiagnosticDraft::with_facts(&INVALID_MAP_LITERAL, span, |cursor| {
        cursor.push::<UnitFact<InvalidMapLiteralReason>>(&reason)?;
        Ok(())
    })
}

pub(crate) fn invalid_collection_type(
    reason: InvalidCollectionTypeReason,
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    let code = InvalidCollectionTypeCode::of(&reason);
    let (capacity, length) = match reason {
        InvalidCollectionTypeReason::InitializerExceedsFixedCapacity { capacity, length } => {
            (Some(capacity), Some(length))
        }
        _ => (None, None),
    };
    DiagnosticDraft::with_facts(&INVALID_COLLECTION_TYPE, span, |cursor| {
        cursor.push::<UnitFact<InvalidCollectionTypeCode>>(&code)?;
        cursor.push::<OptCountFact>(&capacity)?;
        cursor.push::<OptCountFact>(&length)?;
        Ok(())
    })
}

pub(crate) fn invalid_map_type(
    reason: InvalidMapTypeReason,
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    let code = InvalidMapTypeCode::of(&reason);
    let (key_type, depth) = match reason {
        InvalidMapTypeReason::UnsupportedKeyType { key_type } => (Some(key_type), None),
        InvalidMapTypeReason::ExcessiveInlineNesting { depth } => (None, Some(depth)),
        _ => (None, None),
    };
    DiagnosticDraft::with_facts(&INVALID_MAP_TYPE, span, |cursor| {
        cursor.push::<UnitFact<InvalidMapTypeCode>>(&code)?;
        cursor.push::<OptTypeFact>(&key_type)?;
        cursor.push::<OptCountFact>(&depth)?;
        Ok(())
    })
}

pub(crate) fn invalid_return_shape(
    reason: InvalidReturnShapeReason,
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    let code = InvalidReturnShapeCode::of(&reason);
    let (expected_count, provided_count) = match reason {
        InvalidReturnShapeReason::BareReturnWithExpectedValues { expected_count }
        | InvalidReturnShapeReason::TooManyReturnValues { expected_count } => {
            (Some(expected_count), None)
        }
        InvalidReturnShapeReason::TooFewReturnValues {
            expected_count,
            provided_count,
        } => (Some(expected_count), Some(provided_count)),
        _ => (None, None),
    };
    DiagnosticDraft::with_facts(&INVALID_RETURN_SHAPE, span, |cursor| {
        cursor.push::<UnitFact<InvalidReturnShapeCode>>(&code)?;
        cursor.push::<OptCountFact>(&expected_count)?;
        cursor.push::<OptCountFact>(&provided_count)?;
        Ok(())
    })
}
pub(crate) fn invalid_builtin_call(
    reason: InvalidBuiltinCallReason,
    builtin_name: Option<StringId>,
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    DiagnosticDraft::with_facts(&INVALID_BUILTIN_CALL, span, |cursor| {
        cursor.push::<UnitFact<InvalidBuiltinCallReason>>(&reason)?;
        cursor.push::<OptStringFact>(&builtin_name)?;
        Ok(())
    })
}

pub(crate) fn invalid_copy_target(
    reason: InvalidCopyTargetReason,
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    DiagnosticDraft::with_facts(&INVALID_COPY_TARGET, span, |cursor| {
        cursor.push::<UnitFact<InvalidCopyTargetReason>>(&reason)?;
        Ok(())
    })
}

pub(crate) fn invalid_range_operand(
    operand: RangeOperandKind,
    found_type: TypeId,
    reason: InvalidRangeOperandReason,
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    DiagnosticDraft::with_facts(&INVALID_RANGE_OPERAND, span, |cursor| {
        cursor.push::<UnitFact<RangeOperandKind>>(&operand)?;
        cursor.push::<TypeFact>(&found_type)?;
        cursor.push::<UnitFact<InvalidRangeOperandReason>>(&reason)?;
        Ok(())
    })
}

pub(crate) fn invalid_receiver_call(
    reason: InvalidReceiverCallReason,
    receiver_type: Option<StringId>,
    method_name: Option<StringId>,
    receiver_kind: Option<ReceiverCallKind>,
    receiver_binding_name: Option<StringId>,
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    DiagnosticDraft::with_facts(&INVALID_RECEIVER_CALL, span, |cursor| {
        cursor.push::<UnitFact<InvalidReceiverCallReason>>(&reason)?;
        cursor.push::<OptUnitFact<ReceiverCallKind>>(&receiver_kind)?;
        cursor.push::<OptStringFact>(&method_name)?;
        cursor.push::<OptStringFact>(&receiver_type)?;
        Ok(())
    })
    .with_string_ids(receiver_binding_name.into_iter().collect())
}

pub(crate) fn invalid_field_access(
    reason: InvalidFieldAccessReason,
    field_name: Option<StringId>,
    receiver_type: Option<TypeId>,
    known_fields: Vec<StringId>,
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    DiagnosticDraft::with_facts(&INVALID_FIELD_ACCESS, span, |cursor| {
        cursor.push::<UnitFact<InvalidFieldAccessReason>>(&reason)?;
        cursor.push::<OptStringFact>(&field_name)?;
        cursor.push::<OptTypeFact>(&receiver_type)?;
        Ok(())
    })
    .with_string_ids(known_fields)
}

pub(crate) fn invalid_call_shape(
    reason: InvalidCallShapeReason,
    callee_name: Option<StringId>,
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    let code = InvalidCallShapeCode::of(&reason);
    let (parameter_name, count, string_ids) = match reason {
        InvalidCallShapeReason::MissingArgument {
            parameter_name,
            parameter_index,
        }
        | InvalidCallShapeReason::DuplicateArgument {
            parameter_name,
            parameter_index,
        }
        | InvalidCallShapeReason::MutableAccessRequired {
            parameter_name,
            parameter_index,
        }
        | InvalidCallShapeReason::MutableAccessNotAllowed {
            parameter_name,
            parameter_index,
        }
        | InvalidCallShapeReason::MutableAccessOnNonPlace {
            parameter_name,
            parameter_index,
        } => (parameter_name, Some(parameter_index), Vec::new()),
        InvalidCallShapeReason::ExtraPositionalArgument { expected_count } => {
            (None, Some(expected_count), Vec::new())
        }
        InvalidCallShapeReason::NamedArgumentNotFound {
            name,
            known_parameters,
        } => {
            let mut string_ids = Vec::with_capacity(known_parameters.len() + 1);
            string_ids.push(name);
            string_ids.extend(known_parameters);
            (None, None, string_ids)
        }
        InvalidCallShapeReason::MutableAccessOnImmutablePlace {
            parameter_name,
            parameter_index,
            binding_name,
        }
        | InvalidCallShapeReason::ImmutablePlaceMutableAccessRequired {
            parameter_name,
            parameter_index,
            binding_name,
        } => (
            parameter_name,
            Some(parameter_index),
            binding_name.into_iter().collect(),
        ),
        InvalidCallShapeReason::PositionalAfterNamed
        | InvalidCallShapeReason::NamedArgumentsNotSupported
        | InvalidCallShapeReason::WireArgumentOrigin
        | InvalidCallShapeReason::WireArgumentMutableMarker
        | InvalidCallShapeReason::RouteArgumentNotDirect
        | InvalidCallShapeReason::RouteForwardingContractMismatch
        | InvalidCallShapeReason::RouteParameterRead
        | InvalidCallShapeReason::RouteInputFieldMismatch => (None, None, Vec::new()),
    };
    DiagnosticDraft::with_facts(&INVALID_CALL_SHAPE, span, |cursor| {
        cursor.push::<UnitFact<InvalidCallShapeCode>>(&code)?;
        cursor.push::<OptStringFact>(&callee_name)?;
        cursor.push::<OptStringFact>(&parameter_name)?;
        cursor.push::<OptCountFact>(&count)?;
        Ok(())
    })
    .with_string_ids(string_ids)
}

pub(crate) fn invalid_cast(
    reason: InvalidCastReason,
    source_type: Option<TypeId>,
    target_type: Option<TypeId>,
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    DiagnosticDraft::with_facts(&INVALID_CAST, span, |cursor| {
        cursor.push::<UnitFact<InvalidCastReason>>(&reason)?;
        cursor.push::<OptTypeFact>(&source_type)?;
        cursor.push::<OptTypeFact>(&target_type)?;
        Ok(())
    })
}

pub(crate) fn invalid_value_receiver(
    receiver: ValueReceiverContext,
    target_count: usize,
    produced_count: usize,
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    DiagnosticDraft::with_facts(&INVALID_VALUE_RECEIVER, span, |cursor| {
        cursor.push::<UnitFact<ValueReceiverContext>>(&receiver)?;
        cursor.push::<CountFact>(&target_count)?;
        cursor.push::<CountFact>(&produced_count)?;
        Ok(())
    })
}

/// Consumes this group's legacy kind and payload when they match a compact family.
pub(super) fn from_legacy(
    kind: DiagnosticKind,
    payload: DiagnosticPayload,
    span: Option<SourceSpan>,
) -> Result<DiagnosticDraft, DiagnosticPayload> {
    match (kind, payload) {
        (
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidMultiBind),
            DiagnosticPayload::InvalidMultiBind {
                reason,
                target_name,
            },
        ) => Ok(invalid_multi_bind(reason, target_name, span)),
        (
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::InvalidCollectionType),
            DiagnosticPayload::InvalidCollectionType { reason },
        ) => Ok(invalid_collection_type(reason, span)),
        (
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::InvalidMapType),
            DiagnosticPayload::InvalidMapType { reason },
        ) => Ok(invalid_map_type(reason, span)),
        (
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::InvalidMapLiteral),
            DiagnosticPayload::InvalidMapLiteral { reason },
        ) => Ok(invalid_map_literal(reason, span)),
        (
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidBuiltinCall),
            DiagnosticPayload::InvalidBuiltinCall {
                reason,
                builtin_name,
            },
        ) => Ok(invalid_builtin_call(reason, builtin_name, span)),
        (
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidCopyTarget),
            DiagnosticPayload::InvalidCopyTarget { reason },
        ) => Ok(invalid_copy_target(reason, span)),
        (
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidRangeOperand),
            DiagnosticPayload::InvalidRangeOperand {
                operand,
                found_type,
                reason,
            },
        ) => Ok(invalid_range_operand(operand, found_type, reason, span)),
        (
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidReceiverCall),
            DiagnosticPayload::InvalidReceiverCall {
                reason,
                receiver_type,
                method_name,
                receiver_kind,
                receiver_binding_name,
            },
        ) => Ok(invalid_receiver_call(
            reason,
            receiver_type,
            method_name,
            receiver_kind,
            receiver_binding_name,
            span,
        )),
        (
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidFieldAccess),
            DiagnosticPayload::InvalidFieldAccess {
                reason,
                field_name,
                receiver_type,
                known_fields,
            },
        ) => Ok(invalid_field_access(
            reason,
            field_name,
            receiver_type,
            known_fields,
            span,
        )),
        (
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidCallShape),
            DiagnosticPayload::InvalidCallShape {
                reason,
                callee_name,
            },
        ) => Ok(invalid_call_shape(reason, callee_name, span)),
        (
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidCast),
            DiagnosticPayload::InvalidCast {
                reason,
                source_type,
                target_type,
            },
        ) => Ok(invalid_cast(reason, source_type, target_type, span)),
        (
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidValueReceiver),
            DiagnosticPayload::InvalidValueReceiver {
                receiver,
                target_count,
                produced_count,
            },
        ) => Ok(invalid_value_receiver(
            receiver,
            target_count,
            produced_count,
            span,
        )),
        (
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidReturnShape),
            DiagnosticPayload::InvalidReturnShape { reason },
        ) => Ok(invalid_return_shape(reason, span)),
        (_, payload) => Err(payload),
    }
}

diagnostic_family! {
    INVALID_COLLECTION_TYPE {
        internal: 1016,
        stable: "MOTH-SYNTAX-0016",
        category: DiagnosticCategory::Syntax,
        severity: DiagnosticSeverity::Error,
        title: "Invalid collection type",
        facts: [
            reason: UnitFact<InvalidCollectionTypeCode>,
            capacity: OptCountFact,
            length: OptCountFact,
        ],
        labels: [],
        extra: None,
        ctor: manual,
        reason_key: reason,
        view: InvalidCollectionTypeView,
        render: render_invalid_collection_type,
    }
}

diagnostic_family! {
    INVALID_MAP_LITERAL {
        internal: 1033,
        stable: "MOTH-SYNTAX-0033",
        category: DiagnosticCategory::Syntax,
        severity: DiagnosticSeverity::Error,
        title: "Invalid map literal",
        facts: [reason: UnitFact<InvalidMapLiteralReason>],
        labels: [],
        extra: None,
        ctor: manual,
        reason_key: reason,
        view: InvalidMapLiteralView,
        render: render_invalid_map_literal,
    }
}

diagnostic_family! {
    INVALID_MAP_TYPE {
        internal: 1039,
        stable: "MOTH-SYNTAX-0016-MAP",
        category: DiagnosticCategory::Syntax,
        severity: DiagnosticSeverity::Error,
        title: "Invalid map type",
        facts: [
            reason: UnitFact<InvalidMapTypeCode>,
            key_type: OptTypeFact,
            depth: OptCountFact,
        ],
        labels: [],
        extra: None,
        ctor: manual,
        reason_key: reason,
        view: InvalidMapTypeView,
        render: render_invalid_map_type,
    }
}

diagnostic_family! {
    INVALID_BUILTIN_CALL {
        internal: 3046,
        stable: "MOTH-RULE-0046",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "Invalid builtin call",
        facts: [
            reason: UnitFact<InvalidBuiltinCallReason>,
            builtin_name: OptStringFact,
        ],
        labels: [],
        extra: None,
        ctor: manual,
        reason_key: reason,
        view: InvalidBuiltinCallView,
        render: render_invalid_builtin_call,
    }
}

diagnostic_family! {
    INVALID_RECEIVER_CALL {
        internal: 3047,
        stable: "MOTH-RULE-0047",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "Invalid receiver call",
        facts: [
            reason: UnitFact<InvalidReceiverCallReason>,
            receiver_kind: OptUnitFact<ReceiverCallKind>,
            method_name: OptStringFact,
            receiver_type: OptStringFact,
        ],
        labels: [],
        extra: StringIds,
        ctor: manual,
        reason_key: reason,
        view: InvalidReceiverCallView,
        render: render_invalid_receiver_call,
    }
}

/// Decodes the optional receiver binding name stored in the family string list.
pub(crate) fn invalid_receiver_call_binding_name(string_ids: &[StringId]) -> Option<StringId> {
    match string_ids {
        [] => None,
        [name] => Some(*name),
        _ => unreachable!("receiver call stores at most one binding name"),
    }
}

diagnostic_family! {
    INVALID_FIELD_ACCESS {
        internal: 3048,
        stable: "MOTH-RULE-0048",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "Invalid field access",
        facts: [
            reason: UnitFact<InvalidFieldAccessReason>,
            field_name: OptStringFact,
            receiver_type: OptTypeFact,
        ],
        labels: [],
        extra: StringIds,
        ctor: manual,
        reason_key: reason,
        view: InvalidFieldAccessView,
        render: render_invalid_field_access,
    }
}

diagnostic_family! {
    INVALID_CALL_SHAPE {
        internal: 3054,
        stable: "MOTH-RULE-0054",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "Invalid call shape",
        facts: [
            reason: UnitFact<InvalidCallShapeCode>,
            callee_name: OptStringFact,
            parameter_name: OptStringFact,
            count: OptCountFact,
        ],
        labels: [],
        extra: StringIds,
        ctor: manual,
        reason_key: reason,
        view: InvalidCallShapeView,
        render: render_invalid_call_shape,
    }
}

/// Decodes the call-shape string list and rebuilds the legacy reason for its prose helper.
pub(crate) fn invalid_call_shape_reason(
    code: InvalidCallShapeCode,
    parameter_name: Option<StringId>,
    count: Option<usize>,
    string_ids: &[StringId],
) -> InvalidCallShapeReason {
    let parameter_index = || count.expect("call-shape code requires a parameter index");
    match code {
        InvalidCallShapeCode::MissingArgument => InvalidCallShapeReason::MissingArgument {
            parameter_name,
            parameter_index: parameter_index(),
        },
        InvalidCallShapeCode::ExtraPositionalArgument => {
            InvalidCallShapeReason::ExtraPositionalArgument {
                expected_count: parameter_index(),
            }
        }
        InvalidCallShapeCode::DuplicateArgument => InvalidCallShapeReason::DuplicateArgument {
            parameter_name,
            parameter_index: parameter_index(),
        },
        InvalidCallShapeCode::NamedArgumentNotFound => {
            let (name, known_parameters) = string_ids
                .split_first()
                .expect("named-argument code stores its name in the string list");
            InvalidCallShapeReason::NamedArgumentNotFound {
                name: *name,
                known_parameters: known_parameters.to_vec(),
            }
        }
        InvalidCallShapeCode::PositionalAfterNamed => InvalidCallShapeReason::PositionalAfterNamed,
        InvalidCallShapeCode::NamedArgumentsNotSupported => {
            InvalidCallShapeReason::NamedArgumentsNotSupported
        }
        InvalidCallShapeCode::WireArgumentOrigin => InvalidCallShapeReason::WireArgumentOrigin,
        InvalidCallShapeCode::WireArgumentMutableMarker => {
            InvalidCallShapeReason::WireArgumentMutableMarker
        }
        InvalidCallShapeCode::RouteArgumentNotDirect => {
            InvalidCallShapeReason::RouteArgumentNotDirect
        }
        InvalidCallShapeCode::RouteForwardingContractMismatch => {
            InvalidCallShapeReason::RouteForwardingContractMismatch
        }
        InvalidCallShapeCode::RouteParameterRead => InvalidCallShapeReason::RouteParameterRead,
        InvalidCallShapeCode::RouteInputFieldMismatch => {
            InvalidCallShapeReason::RouteInputFieldMismatch
        }
        InvalidCallShapeCode::MutableAccessRequired => {
            InvalidCallShapeReason::MutableAccessRequired {
                parameter_name,
                parameter_index: parameter_index(),
            }
        }
        InvalidCallShapeCode::MutableAccessNotAllowed => {
            InvalidCallShapeReason::MutableAccessNotAllowed {
                parameter_name,
                parameter_index: parameter_index(),
            }
        }
        InvalidCallShapeCode::MutableAccessOnNonPlace => {
            InvalidCallShapeReason::MutableAccessOnNonPlace {
                parameter_name,
                parameter_index: parameter_index(),
            }
        }
        InvalidCallShapeCode::MutableAccessOnImmutablePlace
        | InvalidCallShapeCode::ImmutablePlaceMutableAccessRequired => {
            let binding_name = match string_ids {
                [] => None,
                [binding_name] => Some(*binding_name),
                _ => unreachable!("call shape stores at most one binding name"),
            };
            let parameter_index = parameter_index();
            match code {
                InvalidCallShapeCode::MutableAccessOnImmutablePlace => {
                    InvalidCallShapeReason::MutableAccessOnImmutablePlace {
                        parameter_name,
                        parameter_index,
                        binding_name,
                    }
                }
                InvalidCallShapeCode::ImmutablePlaceMutableAccessRequired => {
                    InvalidCallShapeReason::ImmutablePlaceMutableAccessRequired {
                        parameter_name,
                        parameter_index,
                        binding_name,
                    }
                }
                _ => unreachable!("the call-shape code selects a binding-name variant"),
            }
        }
    }
}

diagnostic_family! {
    INVALID_RETURN_SHAPE {
        internal: 3055,
        stable: "MOTH-RULE-0055",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "Invalid return shape",
        facts: [
            reason: UnitFact<InvalidReturnShapeCode>,
            expected_count: OptCountFact,
            provided_count: OptCountFact,
        ],
        labels: [],
        extra: None,
        ctor: manual,
        reason_key: reason,
        view: InvalidReturnShapeView,
        render: render_invalid_return_shape,
    }
}

diagnostic_family! {
    INVALID_COPY_TARGET {
        internal: 3056,
        stable: "MOTH-RULE-0056",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "Invalid copy target",
        facts: [reason: UnitFact<InvalidCopyTargetReason>],
        labels: [],
        extra: None,
        ctor: manual,
        reason_key: reason,
        view: InvalidCopyTargetView,
        render: render_invalid_copy_target,
    }
}

diagnostic_family! {
    INVALID_RANGE_OPERAND {
        internal: 3059,
        stable: "MOTH-RULE-0059",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "Invalid range operand",
        facts: [
            operand: UnitFact<RangeOperandKind>,
            found_type: TypeFact,
            reason: UnitFact<InvalidRangeOperandReason>,
        ],
        labels: [],
        extra: None,
        ctor: manual,
        reason_key: reason,
        view: InvalidRangeOperandView,
        render: render_invalid_range_operand,
    }
}

diagnostic_family! {
    INVALID_CAST {
        internal: 3083,
        stable: "MOTH-RULE-0083",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "Invalid cast",
        facts: [
            reason: UnitFact<InvalidCastReason>,
            source_type: OptTypeFact,
            target_type: OptTypeFact,
        ],
        labels: [],
        extra: None,
        ctor: manual,
        reason_key: reason,
        view: InvalidCastView,
        render: render_invalid_cast,
    }
}

diagnostic_family! {
    INVALID_VALUE_RECEIVER {
        internal: 3087,
        stable: "MOTH-RULE-0087",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "Invalid value receiver",
        facts: [
            receiver: UnitFact<ValueReceiverContext>,
            target_count: CountFact,
            produced_count: CountFact,
        ],
        labels: [],
        extra: None,
        ctor: manual,
        view: InvalidValueReceiverView,
        render: render_invalid_value_receiver,
    }
}

pub(in crate::compiler_frontend::compiler_messages::diagnostics) const FAMILIES: &[FamilyRow] = &[
    INVALID_MULTI_BIND,
    INVALID_COLLECTION_TYPE,
    INVALID_MAP_LITERAL,
    INVALID_MAP_TYPE,
    INVALID_BUILTIN_CALL,
    INVALID_RECEIVER_CALL,
    INVALID_FIELD_ACCESS,
    INVALID_CALL_SHAPE,
    INVALID_RETURN_SHAPE,
    INVALID_COPY_TARGET,
    INVALID_RANGE_OPERAND,
    INVALID_CAST,
    INVALID_VALUE_RECEIVER,
];
