//! Family rows for generics, traits and API diagnostics.
//!
//! WHAT: every generics, trait and API family has a full compact row, mirror constructor and
//! legacy-adapter arm.
//! WHY: related diagnostics share one schema table and legacy adapter.

use crate::compiler_frontend::compiler_messages::diagnostic_types::TypeDisplayId;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::facts::{
    CountFact, FactCapacityError, FactCodec, IdRemap, OptStringFact, OptTokenFact, OptTypeFact,
    OptUnitFact, StringFact, TypeFact, UnitCode, UnitFact, unit_code,
};
use crate::compiler_frontend::compiler_messages::diagnostics::labels::LabelMessageCode;
use crate::compiler_frontend::compiler_messages::diagnostics::schema::{
    FamilyRow, diagnostic_family,
};
use crate::compiler_frontend::compiler_messages::render::{
    render_duplicate_public_export, render_duplicate_trait_requirement,
    render_generic_bound_private_surface_leak, render_invalid_function_signature,
    render_invalid_generic_application, render_invalid_generic_instantiation,
    render_invalid_generic_parameter, render_invalid_parameter_contract,
    render_invalid_receiver_declaration, render_invalid_this_usage,
    render_invalid_trait_conformance, render_invalid_trait_incompatibility,
    render_invalid_trait_keyword_usage, render_private_type_in_exported_api, render_title_only,
    render_trait_name_used_as_type, render_trait_private_surface_leak, render_unknown_trait,
    render_unsupported_trait_feature,
};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticCategory, DiagnosticKind, DiagnosticLabelMessage,
    DiagnosticPayload, DiagnosticSeverity, GenericApplicationErrorReason, GenericInferenceSubject,
    InvalidFunctionSignatureReason, InvalidGenericInstantiationReason,
    InvalidGenericParameterReason, InvalidParameterContractReason,
    InvalidReceiverDeclarationReason, InvalidThisUsageReason, InvalidTraitConformanceReason,
    InvalidTraitIncompatibilityReason, InvalidTraitKeywordUsageReason, ParameterContractKind,
    PublicGenericOwnerKind, RuleDiagnosticKind, SyntaxDiagnosticKind,
};
use crate::compiler_frontend::datatypes::generic_bindings::BindingConflict;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::string_interning::StringId;

unit_code! {
    impl InvalidTraitIncompatibilityReason {
        SelfIncompatible = 1,
        UnknownTrait = 2,
        DuplicateRelation = 3,
        PrivateTraitSurfaceLeak = 4,
    }
}

unit_code! {
    impl InvalidTraitKeywordUsageReason {
        MustOutsideTraitSyntax = 1,
        ThisOutsideTraitSyntax = 2,
    }
}

unit_code! {
    impl PublicGenericOwnerKind {
        Function = 1,
        ReceiverMethod = 2,
        Struct = 3,
        Choice = 4,
    }
}

unit_code! {
    impl GenericApplicationErrorReason {
        OnNonNamedType = 1,
        EmptyArgumentList = 2,
        MissingArgumentAfterComma = 3,
        NestedApplication = 4,
    }
}

unit_code! {
    impl ParameterContractKind {
        Wire = 1,
        Route = 2,
    }
}

unit_code! {
    pub(crate) enum InvalidParameterContractCode {
        MissingSpecification = 1,
        InvalidPosition = 2,
        WireArrowNotAllowed = 3,
        RouteOutputMissing = 4,
        RouteRepeatedArrow = 5,
        NestedContract = 6,
        MutableParameter = 7,
        ParameterDefault = 8,
        ReceiverContract = 9,
        MultipleSpecificationTypes = 10,
    }
}

impl InvalidParameterContractCode {
    pub(crate) fn of(reason: &InvalidParameterContractReason) -> Self {
        Self::split(*reason).0
    }

    fn split(
        reason: InvalidParameterContractReason,
    ) -> (
        Self,
        Option<ParameterContractKind>,
        Option<ParameterContractKind>,
    ) {
        use InvalidParameterContractReason as Reason;
        match reason {
            Reason::MissingSpecification { contract } => {
                (Self::MissingSpecification, Some(contract), None)
            }
            Reason::InvalidPosition { contract } => (Self::InvalidPosition, Some(contract), None),
            Reason::WireArrowNotAllowed => (Self::WireArrowNotAllowed, None, None),
            Reason::RouteOutputMissing => (Self::RouteOutputMissing, None, None),
            Reason::RouteRepeatedArrow => (Self::RouteRepeatedArrow, None, None),
            Reason::NestedContract { outer, inner } => {
                (Self::NestedContract, Some(outer), Some(inner))
            }
            Reason::MutableParameter { contract } => (Self::MutableParameter, Some(contract), None),
            Reason::ParameterDefault { contract } => (Self::ParameterDefault, Some(contract), None),
            Reason::ReceiverContract { contract } => (Self::ReceiverContract, Some(contract), None),
            Reason::MultipleSpecificationTypes { contract } => {
                (Self::MultipleSpecificationTypes, Some(contract), None)
            }
        }
    }

    pub(crate) fn to_reason(
        self,
        contract: Option<ParameterContractKind>,
        nested_contract: Option<ParameterContractKind>,
    ) -> Option<InvalidParameterContractReason> {
        use InvalidParameterContractReason as Reason;
        Some(match self {
            Self::MissingSpecification => Reason::MissingSpecification {
                contract: contract?,
            },
            Self::InvalidPosition => Reason::InvalidPosition {
                contract: contract?,
            },
            Self::WireArrowNotAllowed => Reason::WireArrowNotAllowed,
            Self::RouteOutputMissing => Reason::RouteOutputMissing,
            Self::RouteRepeatedArrow => Reason::RouteRepeatedArrow,
            Self::NestedContract => Reason::NestedContract {
                outer: contract?,
                inner: nested_contract?,
            },
            Self::MutableParameter => Reason::MutableParameter {
                contract: contract?,
            },
            Self::ParameterDefault => Reason::ParameterDefault {
                contract: contract?,
            },
            Self::ReceiverContract => Reason::ReceiverContract {
                contract: contract?,
            },
            Self::MultipleSpecificationTypes => Reason::MultipleSpecificationTypes {
                contract: contract?,
            },
        })
    }
}

unit_code! {
    pub(crate) enum InvalidGenericParameterCode {
        EmptyParameterList = 1,
        BoundsMustUseIs = 2,
        ListMustStayWithHeader = 3,
        InvalidToken = 4,
    }
}

impl InvalidGenericParameterCode {
    pub(crate) fn of(reason: &InvalidGenericParameterReason) -> Self {
        match reason {
            InvalidGenericParameterReason::EmptyParameterList => Self::EmptyParameterList,
            InvalidGenericParameterReason::BoundsMustUseIs => Self::BoundsMustUseIs,
            InvalidGenericParameterReason::ListMustStayWithHeader => Self::ListMustStayWithHeader,
            InvalidGenericParameterReason::InvalidToken { .. } => Self::InvalidToken,
        }
    }

    fn split(
        reason: &InvalidGenericParameterReason,
    ) -> (
        Self,
        Option<crate::compiler_frontend::compiler_messages::DiagnosticToken>,
    ) {
        match reason {
            InvalidGenericParameterReason::InvalidToken { found } => {
                (Self::InvalidToken, Some(*found))
            }
            _ => (Self::of(reason), None),
        }
    }

    pub(crate) fn to_reason(
        self,
        found: Option<crate::compiler_frontend::compiler_messages::DiagnosticToken>,
    ) -> Option<InvalidGenericParameterReason> {
        Some(match self {
            Self::EmptyParameterList => InvalidGenericParameterReason::EmptyParameterList,
            Self::BoundsMustUseIs => InvalidGenericParameterReason::BoundsMustUseIs,
            Self::ListMustStayWithHeader => InvalidGenericParameterReason::ListMustStayWithHeader,
            Self::InvalidToken => InvalidGenericParameterReason::InvalidToken { found: found? },
        })
    }
}

unit_code! {
    pub(crate) enum InvalidThisUsageCode {
        NotInReceiverMethod = 1,
        Reassignment = 2,
        LoopBinding = 3,
        DeclarationBinding = 4,
        DuplicateThis = 5,
        NotFirstParameter = 6,
        OutsideTraitDeclaration = 7,
    }
}

impl InvalidThisUsageCode {
    pub(crate) fn of(reason: &InvalidThisUsageReason) -> Self {
        Self::split(*reason).0
    }

    fn split(reason: InvalidThisUsageReason) -> (Self, Option<StringId>) {
        use InvalidThisUsageReason as Reason;
        match reason {
            Reason::NotInReceiverMethod => (Self::NotInReceiverMethod, None),
            Reason::Reassignment => (Self::Reassignment, None),
            Reason::LoopBinding => (Self::LoopBinding, None),
            Reason::DeclarationBinding => (Self::DeclarationBinding, None),
            Reason::DuplicateThis { function_name } => (Self::DuplicateThis, Some(function_name)),
            Reason::NotFirstParameter { function_name } => {
                (Self::NotFirstParameter, Some(function_name))
            }
            Reason::OutsideTraitDeclaration => (Self::OutsideTraitDeclaration, None),
        }
    }

    pub(crate) fn to_reason(
        self,
        function_name: Option<StringId>,
    ) -> Option<InvalidThisUsageReason> {
        use InvalidThisUsageReason as Reason;
        Some(match self {
            Self::NotInReceiverMethod => Reason::NotInReceiverMethod,
            Self::Reassignment => Reason::Reassignment,
            Self::LoopBinding => Reason::LoopBinding,
            Self::DeclarationBinding => Reason::DeclarationBinding,
            Self::DuplicateThis => Reason::DuplicateThis {
                function_name: function_name?,
            },
            Self::NotFirstParameter => Reason::NotFirstParameter {
                function_name: function_name?,
            },
            Self::OutsideTraitDeclaration => Reason::OutsideTraitDeclaration,
        })
    }
}

unit_code! {
    pub(crate) enum InvalidReceiverDeclarationCode {
        UnknownStructTarget = 1,
        NonlocalSourceType = 2,
        BuiltinScalarType = 3,
        ExternalOpaqueType = 4,
        FieldNameConflict = 5,
        DuplicateMethod = 6,
        DuplicateVisibleMethod = 7,
        GenericReceiverType = 8,
        UnsupportedType = 9,
        ReceiverMethodImportOrExportNotAllowed = 10,
    }
}

impl InvalidReceiverDeclarationCode {
    pub(crate) fn of(reason: &InvalidReceiverDeclarationReason) -> Self {
        Self::split(*reason).0
    }

    fn split(reason: InvalidReceiverDeclarationReason) -> (Self, Option<StringId>, Option<TypeId>) {
        use InvalidReceiverDeclarationReason as Reason;
        match reason {
            Reason::UnknownStructTarget => (Self::UnknownStructTarget, None, None),
            Reason::NonlocalSourceType => (Self::NonlocalSourceType, None, None),
            Reason::BuiltinScalarType => (Self::BuiltinScalarType, None, None),
            Reason::ExternalOpaqueType => (Self::ExternalOpaqueType, None, None),
            Reason::FieldNameConflict => (Self::FieldNameConflict, None, None),
            Reason::DuplicateMethod => (Self::DuplicateMethod, None, None),
            Reason::DuplicateVisibleMethod => (Self::DuplicateVisibleMethod, None, None),
            Reason::GenericReceiverType {
                function_name,
                type_id,
            } => (
                Self::GenericReceiverType,
                Some(function_name),
                Some(type_id),
            ),
            Reason::UnsupportedType {
                function_name,
                type_id,
            } => (Self::UnsupportedType, Some(function_name), Some(type_id)),
            Reason::ReceiverMethodImportOrExportNotAllowed => {
                (Self::ReceiverMethodImportOrExportNotAllowed, None, None)
            }
        }
    }
}

unit_code! {
    pub(crate) enum InvalidFunctionSignatureCode {
        MissingArrowOrColon = 1,
        UnexpectedEndAfterParameters = 2,
        MissingReturnType = 3,
        MissingTraitRequirementReturnType = 4,
        TrailingCommaInReturns = 5,
        UnexpectedEndAfterComma = 6,
        UnexpectedEndInReturns = 7,
        MissingColonAfterReturns = 8,
        UnexpectedArrowInReturns = 9,
        MissingCommaOrColon = 10,
        VoidNotAllowed = 11,
        MultipleErrorReturnSlots = 12,
        ErrorSlotNotLast = 13,
        GenericWhereConstraintsUnsupported = 14,
    }
}

impl InvalidFunctionSignatureCode {
    pub(crate) fn of(reason: &InvalidFunctionSignatureReason) -> Self {
        Self::split(reason).0
    }

    fn split(
        reason: &InvalidFunctionSignatureReason,
    ) -> (
        Self,
        Option<crate::compiler_frontend::compiler_messages::DiagnosticToken>,
    ) {
        use InvalidFunctionSignatureReason as Reason;
        match reason {
            Reason::MissingArrowOrColon { found } => (Self::MissingArrowOrColon, Some(*found)),
            Reason::UnexpectedEndAfterParameters => (Self::UnexpectedEndAfterParameters, None),
            Reason::MissingReturnType => (Self::MissingReturnType, None),
            Reason::MissingTraitRequirementReturnType => {
                (Self::MissingTraitRequirementReturnType, None)
            }
            Reason::TrailingCommaInReturns => (Self::TrailingCommaInReturns, None),
            Reason::UnexpectedEndAfterComma => (Self::UnexpectedEndAfterComma, None),
            Reason::UnexpectedEndInReturns => (Self::UnexpectedEndInReturns, None),
            Reason::MissingColonAfterReturns => (Self::MissingColonAfterReturns, None),
            Reason::UnexpectedArrowInReturns => (Self::UnexpectedArrowInReturns, None),
            Reason::MissingCommaOrColon { found } => (Self::MissingCommaOrColon, Some(*found)),
            Reason::VoidNotAllowed => (Self::VoidNotAllowed, None),
            Reason::MultipleErrorReturnSlots => (Self::MultipleErrorReturnSlots, None),
            Reason::ErrorSlotNotLast => (Self::ErrorSlotNotLast, None),
            Reason::GenericWhereConstraintsUnsupported => {
                (Self::GenericWhereConstraintsUnsupported, None)
            }
        }
    }

    pub(crate) fn to_reason(
        self,
        found: Option<crate::compiler_frontend::compiler_messages::DiagnosticToken>,
    ) -> Option<InvalidFunctionSignatureReason> {
        use InvalidFunctionSignatureReason as Reason;
        Some(match self {
            Self::MissingArrowOrColon => Reason::MissingArrowOrColon { found: found? },
            Self::UnexpectedEndAfterParameters => Reason::UnexpectedEndAfterParameters,
            Self::MissingReturnType => Reason::MissingReturnType,
            Self::MissingTraitRequirementReturnType => Reason::MissingTraitRequirementReturnType,
            Self::TrailingCommaInReturns => Reason::TrailingCommaInReturns,
            Self::UnexpectedEndAfterComma => Reason::UnexpectedEndAfterComma,
            Self::UnexpectedEndInReturns => Reason::UnexpectedEndInReturns,
            Self::MissingColonAfterReturns => Reason::MissingColonAfterReturns,
            Self::UnexpectedArrowInReturns => Reason::UnexpectedArrowInReturns,
            Self::MissingCommaOrColon => Reason::MissingCommaOrColon { found: found? },
            Self::VoidNotAllowed => Reason::VoidNotAllowed,
            Self::MultipleErrorReturnSlots => Reason::MultipleErrorReturnSlots,
            Self::ErrorSlotNotLast => Reason::ErrorSlotNotLast,
            Self::GenericWhereConstraintsUnsupported => Reason::GenericWhereConstraintsUnsupported,
        })
    }
}

unit_code! {
    impl GenericInferenceSubject {
        Function = 1,
        NominalType = 2,
    }
}

unit_code! {
    pub(crate) enum InvalidGenericInstantiationCode {
        WrongArgumentCount = 1,
        TypeDoesNotAcceptArguments = 2,
        OptionTypeSyntaxNotSupported = 3,
        ResultTypeSyntaxNotSupported = 4,
        ExternalTypeArgumentsUnsupported = 5,
        MissingTypeArguments = 6,
        CannotInferArguments = 7,
        CannotInferFunctionArguments = 8,
        ConflictingInference = 9,
        MissingTraitEvidence = 10,
        MissingNominalTraitEvidence = 11,
        RecursiveFunctionInstantiation = 12,
        ExplicitCallTypeArgumentsUnsupported = 13,
        GenericFunctionValueDeferred = 14,
    }
}

impl InvalidGenericInstantiationCode {
    pub(crate) fn of(reason: &InvalidGenericInstantiationReason) -> Self {
        use InvalidGenericInstantiationReason as Reason;
        match reason {
            Reason::WrongArgumentCount { .. } => Self::WrongArgumentCount,
            Reason::TypeDoesNotAcceptArguments => Self::TypeDoesNotAcceptArguments,
            Reason::OptionTypeSyntaxNotSupported => Self::OptionTypeSyntaxNotSupported,
            Reason::ResultTypeSyntaxNotSupported => Self::ResultTypeSyntaxNotSupported,
            Reason::ExternalTypeArgumentsUnsupported => Self::ExternalTypeArgumentsUnsupported,
            Reason::MissingTypeArguments => Self::MissingTypeArguments,
            Reason::CannotInferArguments { .. } => Self::CannotInferArguments,
            Reason::CannotInferFunctionArguments { .. } => Self::CannotInferFunctionArguments,
            Reason::ConflictingInference { .. } => Self::ConflictingInference,
            Reason::MissingTraitEvidence { .. } => Self::MissingTraitEvidence,
            Reason::MissingNominalTraitEvidence { .. } => Self::MissingNominalTraitEvidence,
            Reason::RecursiveFunctionInstantiation => Self::RecursiveFunctionInstantiation,
            Reason::ExplicitCallTypeArgumentsUnsupported => {
                Self::ExplicitCallTypeArgumentsUnsupported
            }
            Reason::GenericFunctionValueDeferred => Self::GenericFunctionValueDeferred,
        }
    }
}

/// WHAT: one generic-instantiation reason shape, parameterized by raw or captured type identity.
/// WHY: the family uses four words across several layouts, so each code determines its typed words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum GenericInstantiationShape<Type> {
    WrongArgumentCount {
        type_name: Option<StringId>,
        expected: usize,
        found: usize,
    },
    TypeDoesNotAcceptArguments {
        type_name: Option<StringId>,
    },
    OptionTypeSyntaxNotSupported {
        type_name: Option<StringId>,
    },
    ResultTypeSyntaxNotSupported {
        type_name: Option<StringId>,
    },
    ExternalTypeArgumentsUnsupported {
        type_name: Option<StringId>,
    },
    MissingTypeArguments {
        type_name: Option<StringId>,
    },
    CannotInferArguments {
        type_name: Option<StringId>,
    },
    CannotInferFunctionArguments {
        type_name: Option<StringId>,
    },
    ConflictingInference {
        subject: GenericInferenceSubject,
        existing_type: Type,
        replacement_type: Type,
    },
    MissingTraitEvidence {
        type_name: Option<StringId>,
        concrete_type: Type,
    },
    MissingNominalTraitEvidence {
        type_name: Option<StringId>,
        concrete_type: Type,
    },
    RecursiveFunctionInstantiation {
        type_name: Option<StringId>,
    },
    ExplicitCallTypeArgumentsUnsupported {
        type_name: Option<StringId>,
    },
    GenericFunctionValueDeferred {
        type_name: Option<StringId>,
    },
}

impl<Type> GenericInstantiationShape<Type> {
    pub(crate) fn code(&self) -> InvalidGenericInstantiationCode {
        match self {
            Self::WrongArgumentCount { .. } => InvalidGenericInstantiationCode::WrongArgumentCount,
            Self::TypeDoesNotAcceptArguments { .. } => {
                InvalidGenericInstantiationCode::TypeDoesNotAcceptArguments
            }
            Self::OptionTypeSyntaxNotSupported { .. } => {
                InvalidGenericInstantiationCode::OptionTypeSyntaxNotSupported
            }
            Self::ResultTypeSyntaxNotSupported { .. } => {
                InvalidGenericInstantiationCode::ResultTypeSyntaxNotSupported
            }
            Self::ExternalTypeArgumentsUnsupported { .. } => {
                InvalidGenericInstantiationCode::ExternalTypeArgumentsUnsupported
            }
            Self::MissingTypeArguments { .. } => {
                InvalidGenericInstantiationCode::MissingTypeArguments
            }
            Self::CannotInferArguments { .. } => {
                InvalidGenericInstantiationCode::CannotInferArguments
            }
            Self::CannotInferFunctionArguments { .. } => {
                InvalidGenericInstantiationCode::CannotInferFunctionArguments
            }
            Self::ConflictingInference { .. } => {
                InvalidGenericInstantiationCode::ConflictingInference
            }
            Self::MissingTraitEvidence { .. } => {
                InvalidGenericInstantiationCode::MissingTraitEvidence
            }
            Self::MissingNominalTraitEvidence { .. } => {
                InvalidGenericInstantiationCode::MissingNominalTraitEvidence
            }
            Self::RecursiveFunctionInstantiation { .. } => {
                InvalidGenericInstantiationCode::RecursiveFunctionInstantiation
            }
            Self::ExplicitCallTypeArgumentsUnsupported { .. } => {
                InvalidGenericInstantiationCode::ExplicitCallTypeArgumentsUnsupported
            }
            Self::GenericFunctionValueDeferred { .. } => {
                InvalidGenericInstantiationCode::GenericFunctionValueDeferred
            }
        }
    }

    pub(crate) fn type_name(&self) -> Option<StringId> {
        match self {
            Self::WrongArgumentCount { type_name, .. }
            | Self::TypeDoesNotAcceptArguments { type_name }
            | Self::OptionTypeSyntaxNotSupported { type_name }
            | Self::ResultTypeSyntaxNotSupported { type_name }
            | Self::ExternalTypeArgumentsUnsupported { type_name }
            | Self::MissingTypeArguments { type_name }
            | Self::CannotInferArguments { type_name }
            | Self::CannotInferFunctionArguments { type_name }
            | Self::MissingTraitEvidence { type_name, .. }
            | Self::MissingNominalTraitEvidence { type_name, .. }
            | Self::RecursiveFunctionInstantiation { type_name }
            | Self::ExplicitCallTypeArgumentsUnsupported { type_name }
            | Self::GenericFunctionValueDeferred { type_name } => *type_name,
            Self::ConflictingInference { .. } => None,
        }
    }
}

/// Stores the reason tag and variant words; dynamic type words are limited to positions 2 and 3.
pub(crate) struct GenericInstantiationFact;

impl FactCodec for GenericInstantiationFact {
    type Draft = GenericInstantiationShape<TypeId>;
    type Durable = GenericInstantiationShape<TypeDisplayId>;
    const WORDS: usize = 4;
    const PENDING_MASK: u8 = 0b1100;

    fn pending_mask(draft: &Self::Draft) -> u8 {
        match draft {
            GenericInstantiationShape::ConflictingInference { .. } => 0b1100,
            GenericInstantiationShape::MissingTraitEvidence { .. }
            | GenericInstantiationShape::MissingNominalTraitEvidence { .. } => 0b0100,
            _ => 0,
        }
    }

    fn encode(draft: &Self::Draft, words: &mut [u32]) -> Result<(), FactCapacityError> {
        words.fill(0);
        UnitFact::<InvalidGenericInstantiationCode>::encode(&draft.code(), &mut words[0..1])?;
        match draft {
            GenericInstantiationShape::WrongArgumentCount {
                type_name,
                expected,
                found,
            } => {
                CountFact::encode(expected, &mut words[1..2])?;
                CountFact::encode(found, &mut words[2..3])?;
                OptStringFact::encode(type_name, &mut words[3..4])?;
            }
            GenericInstantiationShape::ConflictingInference {
                subject,
                existing_type,
                replacement_type,
            } => {
                UnitFact::<GenericInferenceSubject>::encode(subject, &mut words[1..2])?;
                TypeFact::encode(existing_type, &mut words[2..3])?;
                TypeFact::encode(replacement_type, &mut words[3..4])?;
            }
            GenericInstantiationShape::MissingTraitEvidence {
                type_name,
                concrete_type,
            }
            | GenericInstantiationShape::MissingNominalTraitEvidence {
                type_name,
                concrete_type,
            } => {
                OptStringFact::encode(type_name, &mut words[1..2])?;
                TypeFact::encode(concrete_type, &mut words[2..3])?;
            }
            shape => {
                let type_name = shape.type_name();
                OptStringFact::encode(&type_name, &mut words[1..2])?;
            }
        }
        Ok(())
    }

    fn decode(words: &[u32]) -> Option<Self::Durable> {
        let code = InvalidGenericInstantiationCode::from_code(*words.first()?)?;
        match code {
            InvalidGenericInstantiationCode::WrongArgumentCount => {
                Some(GenericInstantiationShape::WrongArgumentCount {
                    expected: CountFact::decode(words.get(1..2)?)?,
                    found: CountFact::decode(words.get(2..3)?)?,
                    type_name: OptStringFact::decode(words.get(3..4)?)?,
                })
            }
            InvalidGenericInstantiationCode::ConflictingInference => {
                Some(GenericInstantiationShape::ConflictingInference {
                    subject: UnitFact::<GenericInferenceSubject>::decode(words.get(1..2)?)?,
                    existing_type: TypeFact::decode(words.get(2..3)?)?,
                    replacement_type: TypeFact::decode(words.get(3..4)?)?,
                })
            }
            InvalidGenericInstantiationCode::MissingTraitEvidence => {
                Some(GenericInstantiationShape::MissingTraitEvidence {
                    type_name: OptStringFact::decode(words.get(1..2)?)?,
                    concrete_type: TypeFact::decode(words.get(2..3)?)?,
                })
            }
            InvalidGenericInstantiationCode::MissingNominalTraitEvidence => {
                Some(GenericInstantiationShape::MissingNominalTraitEvidence {
                    type_name: OptStringFact::decode(words.get(1..2)?)?,
                    concrete_type: TypeFact::decode(words.get(2..3)?)?,
                })
            }
            code => {
                let type_name = OptStringFact::decode(words.get(1..2)?)?;
                Some(match code {
                    InvalidGenericInstantiationCode::TypeDoesNotAcceptArguments => {
                        GenericInstantiationShape::TypeDoesNotAcceptArguments { type_name }
                    }
                    InvalidGenericInstantiationCode::OptionTypeSyntaxNotSupported => {
                        GenericInstantiationShape::OptionTypeSyntaxNotSupported { type_name }
                    }
                    InvalidGenericInstantiationCode::ResultTypeSyntaxNotSupported => {
                        GenericInstantiationShape::ResultTypeSyntaxNotSupported { type_name }
                    }
                    InvalidGenericInstantiationCode::ExternalTypeArgumentsUnsupported => {
                        GenericInstantiationShape::ExternalTypeArgumentsUnsupported { type_name }
                    }
                    InvalidGenericInstantiationCode::MissingTypeArguments => {
                        GenericInstantiationShape::MissingTypeArguments { type_name }
                    }
                    InvalidGenericInstantiationCode::CannotInferArguments => {
                        GenericInstantiationShape::CannotInferArguments { type_name }
                    }
                    InvalidGenericInstantiationCode::CannotInferFunctionArguments => {
                        GenericInstantiationShape::CannotInferFunctionArguments { type_name }
                    }
                    InvalidGenericInstantiationCode::RecursiveFunctionInstantiation => {
                        GenericInstantiationShape::RecursiveFunctionInstantiation { type_name }
                    }
                    InvalidGenericInstantiationCode::ExplicitCallTypeArgumentsUnsupported => {
                        GenericInstantiationShape::ExplicitCallTypeArgumentsUnsupported {
                            type_name,
                        }
                    }
                    InvalidGenericInstantiationCode::GenericFunctionValueDeferred => {
                        GenericInstantiationShape::GenericFunctionValueDeferred { type_name }
                    }
                    InvalidGenericInstantiationCode::WrongArgumentCount
                    | InvalidGenericInstantiationCode::ConflictingInference
                    | InvalidGenericInstantiationCode::MissingTraitEvidence
                    | InvalidGenericInstantiationCode::MissingNominalTraitEvidence => {
                        unreachable!("specialized generic-instantiation codes were decoded above")
                    }
                })
            }
        }
    }

    fn remap_ids(words: &mut [u32], remap: IdRemap<'_>) -> Result<(), FactCapacityError> {
        let code = InvalidGenericInstantiationCode::from_code(words[0])
            .expect("a stored generic instantiation names a valid code");
        match code {
            // Two count words precede the type name in a wrong-argument-count shape.
            InvalidGenericInstantiationCode::WrongArgumentCount => {
                OptStringFact::remap_ids(&mut words[3..4], remap)
            }
            // Inference evidence holds unit and type words only.
            InvalidGenericInstantiationCode::ConflictingInference => Ok(()),
            // Every other shape keeps its optional type name in word 1.
            _ => OptStringFact::remap_ids(&mut words[1..2], remap),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum GenericInstantiationExtra<'a> {
    None,
    MissingParameters(&'a [StringId]),
    ConflictingInference {
        parameter_name: StringId,
        type_name: Option<StringId>,
    },
    MissingTraitEvidence {
        parameter_name: StringId,
        trait_name: StringId,
    },
}

/// Decodes the variant's StringIds suffix: missing parameters, inference names or no suffix.
pub(crate) fn decode_generic_instantiation_extra(
    code: InvalidGenericInstantiationCode,
    ids: &[StringId],
) -> Option<GenericInstantiationExtra<'_>> {
    use GenericInstantiationExtra as Extra;
    use InvalidGenericInstantiationCode as Code;
    match (code, ids) {
        (Code::CannotInferArguments | Code::CannotInferFunctionArguments, ids) => {
            Some(Extra::MissingParameters(ids))
        }
        (Code::ConflictingInference, [parameter_name]) => Some(Extra::ConflictingInference {
            parameter_name: *parameter_name,
            type_name: None,
        }),
        (Code::ConflictingInference, [parameter_name, type_name]) => {
            Some(Extra::ConflictingInference {
                parameter_name: *parameter_name,
                type_name: Some(*type_name),
            })
        }
        (
            Code::MissingTraitEvidence | Code::MissingNominalTraitEvidence,
            [parameter_name, trait_name],
        ) => Some(Extra::MissingTraitEvidence {
            parameter_name: *parameter_name,
            trait_name: *trait_name,
        }),
        (_, []) => Some(Extra::None),
        _ => None,
    }
}

fn split_generic_instantiation(
    type_name: Option<StringId>,
    reason: InvalidGenericInstantiationReason,
) -> (GenericInstantiationShape<TypeId>, Vec<StringId>) {
    use InvalidGenericInstantiationReason as Reason;
    let mut ids = Vec::new();
    let shape = match reason {
        Reason::WrongArgumentCount { expected, found } => {
            GenericInstantiationShape::WrongArgumentCount {
                type_name,
                expected,
                found,
            }
        }
        Reason::TypeDoesNotAcceptArguments => {
            GenericInstantiationShape::TypeDoesNotAcceptArguments { type_name }
        }
        Reason::OptionTypeSyntaxNotSupported => {
            GenericInstantiationShape::OptionTypeSyntaxNotSupported { type_name }
        }
        Reason::ResultTypeSyntaxNotSupported => {
            GenericInstantiationShape::ResultTypeSyntaxNotSupported { type_name }
        }
        Reason::ExternalTypeArgumentsUnsupported => {
            GenericInstantiationShape::ExternalTypeArgumentsUnsupported { type_name }
        }
        Reason::MissingTypeArguments => {
            GenericInstantiationShape::MissingTypeArguments { type_name }
        }
        Reason::CannotInferArguments { missing_parameters } => {
            ids = missing_parameters;
            GenericInstantiationShape::CannotInferArguments { type_name }
        }
        Reason::CannotInferFunctionArguments { missing_parameters } => {
            ids = missing_parameters;
            GenericInstantiationShape::CannotInferFunctionArguments { type_name }
        }
        Reason::ConflictingInference {
            subject,
            parameter_name,
            existing_type_id,
            replacement_type_id,
            ..
        } => {
            ids.push(parameter_name);
            if let Some(type_name) = type_name {
                ids.push(type_name);
            }
            GenericInstantiationShape::ConflictingInference {
                subject,
                existing_type: existing_type_id,
                replacement_type: replacement_type_id,
            }
        }
        Reason::MissingTraitEvidence {
            parameter_name,
            trait_name,
            concrete_type_id,
        } => {
            ids.extend([parameter_name, trait_name]);
            GenericInstantiationShape::MissingTraitEvidence {
                type_name,
                concrete_type: concrete_type_id,
            }
        }
        Reason::MissingNominalTraitEvidence {
            parameter_name,
            trait_name,
            concrete_type_id,
        } => {
            ids.extend([parameter_name, trait_name]);
            GenericInstantiationShape::MissingNominalTraitEvidence {
                type_name,
                concrete_type: concrete_type_id,
            }
        }
        Reason::RecursiveFunctionInstantiation => {
            GenericInstantiationShape::RecursiveFunctionInstantiation { type_name }
        }
        Reason::ExplicitCallTypeArgumentsUnsupported => {
            GenericInstantiationShape::ExplicitCallTypeArgumentsUnsupported { type_name }
        }
        Reason::GenericFunctionValueDeferred => {
            GenericInstantiationShape::GenericFunctionValueDeferred { type_name }
        }
    };
    (shape, ids)
}

diagnostic_family! {
    INVALID_GENERIC_INSTANTIATION {
        internal: 3057,
        stable: "MOTH-RULE-0057",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "Invalid generic instantiation",
        facts: [shape: GenericInstantiationFact],
        labels: [LabelMessageCode::GENERIC_INFERENCE_PREVIOUS_EVIDENCE],
        extra: StringIds,
        ctor: manual,
        view: InvalidGenericInstantiationView,
        render: render_invalid_generic_instantiation,
    }
}

diagnostic_family! {
    INVALID_GENERIC_APPLICATION {
        internal: 1015,
        stable: "MOTH-SYNTAX-0015",
        category: DiagnosticCategory::Syntax,
        severity: DiagnosticSeverity::Error,
        title: "Invalid generic application",
        facts: [reason: UnitFact<GenericApplicationErrorReason>],
        labels: [],
        extra: None,
        ctor: generated invalid_generic_application,
        view: InvalidGenericApplicationView,
        render: render_invalid_generic_application,
    }
}

diagnostic_family! {
    INVALID_GENERIC_PARAMETER {
        internal: 1020,
        stable: "MOTH-SYNTAX-0020",
        category: DiagnosticCategory::Syntax,
        severity: DiagnosticSeverity::Error,
        title: "Invalid generic parameter",
        facts: [code: UnitFact<InvalidGenericParameterCode>, found: OptTokenFact],
        labels: [],
        extra: None,
        ctor: manual,
        view: InvalidGenericParameterView,
        render: render_invalid_generic_parameter,
    }
}

diagnostic_family! {
    INVALID_PARAMETER_CONTRACT {
        internal: 1038,
        stable: "MOTH-SYNTAX-0038",
        category: DiagnosticCategory::Syntax,
        severity: DiagnosticSeverity::Error,
        title: "Invalid parameter contract",
        facts: [
            code: UnitFact<InvalidParameterContractCode>,
            contract: OptUnitFact<ParameterContractKind>,
            nested_contract: OptUnitFact<ParameterContractKind>,
        ],
        labels: [],
        extra: None,
        ctor: manual,
        view: InvalidParameterContractView,
        render: render_invalid_parameter_contract,
    }
}

diagnostic_family! {
    INVALID_THIS_USAGE {
        internal: 3040,
        stable: "MOTH-RULE-0040",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "Invalid this usage",
        facts: [code: UnitFact<InvalidThisUsageCode>, function_name: OptStringFact],
        labels: [],
        extra: None,
        ctor: manual,
        view: InvalidThisUsageView,
        render: render_invalid_this_usage,
    }
}

diagnostic_family! {
    INVALID_RECEIVER_DECLARATION {
        internal: 3041,
        stable: "MOTH-RULE-0041",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "Invalid receiver declaration",
        facts: [
            code: UnitFact<InvalidReceiverDeclarationCode>,
            function_name: OptStringFact,
            type_id: OptTypeFact,
        ],
        labels: [],
        extra: None,
        ctor: manual,
        view: InvalidReceiverDeclarationView,
        render: render_invalid_receiver_declaration,
    }
}

diagnostic_family! {
    INVALID_FUNCTION_SIGNATURE {
        internal: 3062,
        stable: "MOTH-RULE-0062",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "Invalid function signature",
        facts: [code: UnitFact<InvalidFunctionSignatureCode>, found: OptTokenFact],
        labels: [],
        extra: None,
        ctor: manual,
        view: InvalidFunctionSignatureView,
        render: render_invalid_function_signature,
    }
}

diagnostic_family! {
    UNKNOWN_TRAIT {
        internal: 3069,
        stable: "MOTH-RULE-0069",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "Unknown trait",
        facts: [name: StringFact],
        labels: [],
        extra: None,
        ctor: generated unknown_trait,
        view: UnknownTraitView,
        render: render_unknown_trait,
    }
}

diagnostic_family! {
    DUPLICATE_TRAIT_REQUIREMENT {
        internal: 3070,
        stable: "MOTH-RULE-0070",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "Duplicate trait requirement",
        facts: [trait_name: StringFact, requirement_name: StringFact],
        labels: [LabelMessageCode::PREVIOUS_DECLARATION],
        extra: None,
        ctor: manual,
        view: DuplicateTraitRequirementView,
        render: render_duplicate_trait_requirement,
    }
}

diagnostic_family! {
    TRAIT_PRIVATE_SURFACE_LEAK {
        internal: 3071,
        stable: "MOTH-RULE-0071",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "Private type exposed by trait",
        facts: [trait_name: StringFact, surface_type: TypeFact],
        labels: [],
        extra: None,
        ctor: generated trait_private_surface_leak,
        view: TraitPrivateSurfaceLeakView,
        render: render_trait_private_surface_leak,
    }
}

diagnostic_family! {
    UNSUPPORTED_TRAIT_FEATURE {
        internal: 3072,
        stable: "MOTH-RULE-0072",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "Unsupported trait feature",
        facts: [trait_name: StringFact, feature: StringFact],
        labels: [],
        extra: None,
        ctor: generated unsupported_trait_feature,
        view: UnsupportedTraitFeatureView,
        render: render_unsupported_trait_feature,
    }
}

unit_code! {
    pub(crate) enum InvalidTraitConformanceCode {
        ImportedModuleRoot = 1,
        AliasTarget = 2,
        NonCanonicalTarget = 3,
        NonlocalSourceTarget = 4,
        BuiltinTarget = 5,
        ExternalOpaqueTarget = 6,
        DuplicateCanonicalEvidence = 7,
        BuiltinEvidenceOverride = 8,
        IncompatibleTraitEvidence = 9,
        MissingMethod = 10,
        ReceiverMutabilityMismatch = 11,
        ParameterCountMismatch = 12,
        ParameterModeMismatch = 13,
        ParameterContractMismatch = 14,
        ParameterTypeMismatch = 15,
        ReturnCountMismatch = 16,
        ReturnTypeMismatch = 17,
        ReturnChannelMismatch = 18,
        InferredFailureInInfallibleCast = 19,
    }
}

impl InvalidTraitConformanceCode {
    pub(crate) fn of(reason: &InvalidTraitConformanceReason) -> Self {
        use InvalidTraitConformanceReason as Reason;
        match reason {
            Reason::ImportedModuleRoot => Self::ImportedModuleRoot,
            Reason::AliasTarget => Self::AliasTarget,
            Reason::NonCanonicalTarget => Self::NonCanonicalTarget,
            Reason::NonlocalSourceTarget => Self::NonlocalSourceTarget,
            Reason::BuiltinTarget => Self::BuiltinTarget,
            Reason::ExternalOpaqueTarget => Self::ExternalOpaqueTarget,
            Reason::DuplicateCanonicalEvidence => Self::DuplicateCanonicalEvidence,
            Reason::BuiltinEvidenceOverride => Self::BuiltinEvidenceOverride,
            Reason::IncompatibleTraitEvidence { .. } => Self::IncompatibleTraitEvidence,
            Reason::MissingMethod { .. } => Self::MissingMethod,
            Reason::ReceiverMutabilityMismatch { .. } => Self::ReceiverMutabilityMismatch,
            Reason::ParameterCountMismatch { .. } => Self::ParameterCountMismatch,
            Reason::ParameterModeMismatch { .. } => Self::ParameterModeMismatch,
            Reason::ParameterContractMismatch { .. } => Self::ParameterContractMismatch,
            Reason::ParameterTypeMismatch { .. } => Self::ParameterTypeMismatch,
            Reason::ReturnCountMismatch { .. } => Self::ReturnCountMismatch,
            Reason::ReturnTypeMismatch { .. } => Self::ReturnTypeMismatch,
            Reason::ReturnChannelMismatch { .. } => Self::ReturnChannelMismatch,
            Reason::InferredFailureInInfallibleCast { .. } => Self::InferredFailureInInfallibleCast,
        }
    }
}

/// Word 0 stores the reason code; words 1–2 hold counts, word 1 holds an index, and words 2–3
/// hold the two types when the reason compares types.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum InvalidTraitConformanceShape<Type> {
    ImportedModuleRoot,
    AliasTarget,
    NonCanonicalTarget,
    NonlocalSourceTarget,
    BuiltinTarget,
    ExternalOpaqueTarget,
    DuplicateCanonicalEvidence,
    BuiltinEvidenceOverride,
    IncompatibleTraitEvidence,
    MissingMethod,
    ReceiverMutabilityMismatch,
    ParameterCountMismatch {
        expected: usize,
        found: usize,
    },
    ParameterModeMismatch {
        parameter_index: usize,
    },
    ParameterContractMismatch {
        parameter_index: usize,
    },
    ParameterTypeMismatch {
        parameter_index: usize,
        expected_type: Type,
        found_type: Type,
    },
    ReturnCountMismatch {
        expected: usize,
        found: usize,
    },
    ReturnTypeMismatch {
        return_index: usize,
        expected_type: Type,
        found_type: Type,
    },
    ReturnChannelMismatch {
        return_index: usize,
    },
    InferredFailureInInfallibleCast,
}

impl<Type> InvalidTraitConformanceShape<Type> {
    pub(crate) fn code(&self) -> InvalidTraitConformanceCode {
        use InvalidTraitConformanceShape as Shape;
        match self {
            Shape::ImportedModuleRoot => InvalidTraitConformanceCode::ImportedModuleRoot,
            Shape::AliasTarget => InvalidTraitConformanceCode::AliasTarget,
            Shape::NonCanonicalTarget => InvalidTraitConformanceCode::NonCanonicalTarget,
            Shape::NonlocalSourceTarget => InvalidTraitConformanceCode::NonlocalSourceTarget,
            Shape::BuiltinTarget => InvalidTraitConformanceCode::BuiltinTarget,
            Shape::ExternalOpaqueTarget => InvalidTraitConformanceCode::ExternalOpaqueTarget,
            Shape::DuplicateCanonicalEvidence => {
                InvalidTraitConformanceCode::DuplicateCanonicalEvidence
            }
            Shape::BuiltinEvidenceOverride => InvalidTraitConformanceCode::BuiltinEvidenceOverride,
            Shape::IncompatibleTraitEvidence => {
                InvalidTraitConformanceCode::IncompatibleTraitEvidence
            }
            Shape::MissingMethod => InvalidTraitConformanceCode::MissingMethod,
            Shape::ReceiverMutabilityMismatch => {
                InvalidTraitConformanceCode::ReceiverMutabilityMismatch
            }
            Shape::ParameterCountMismatch { .. } => {
                InvalidTraitConformanceCode::ParameterCountMismatch
            }
            Shape::ParameterModeMismatch { .. } => {
                InvalidTraitConformanceCode::ParameterModeMismatch
            }
            Shape::ParameterContractMismatch { .. } => {
                InvalidTraitConformanceCode::ParameterContractMismatch
            }
            Shape::ParameterTypeMismatch { .. } => {
                InvalidTraitConformanceCode::ParameterTypeMismatch
            }
            Shape::ReturnCountMismatch { .. } => InvalidTraitConformanceCode::ReturnCountMismatch,
            Shape::ReturnTypeMismatch { .. } => InvalidTraitConformanceCode::ReturnTypeMismatch,
            Shape::ReturnChannelMismatch { .. } => {
                InvalidTraitConformanceCode::ReturnChannelMismatch
            }
            Shape::InferredFailureInInfallibleCast => {
                InvalidTraitConformanceCode::InferredFailureInInfallibleCast
            }
        }
    }
}

/// Stores the D6 reason code and the variant's index, counts or types.
pub(crate) struct InvalidTraitConformanceFact;

impl FactCodec for InvalidTraitConformanceFact {
    type Draft = InvalidTraitConformanceShape<TypeId>;
    type Durable = InvalidTraitConformanceShape<TypeDisplayId>;
    const WORDS: usize = 4;
    const PENDING_MASK: u8 = 0b1100;

    fn pending_mask(draft: &Self::Draft) -> u8 {
        match draft {
            InvalidTraitConformanceShape::ParameterTypeMismatch { .. }
            | InvalidTraitConformanceShape::ReturnTypeMismatch { .. } => 0b1100,
            _ => 0,
        }
    }

    fn encode(draft: &Self::Draft, words: &mut [u32]) -> Result<(), FactCapacityError> {
        words.fill(0);
        UnitFact::<InvalidTraitConformanceCode>::encode(&draft.code(), &mut words[0..1])?;
        match draft {
            InvalidTraitConformanceShape::ParameterCountMismatch { expected, found }
            | InvalidTraitConformanceShape::ReturnCountMismatch { expected, found } => {
                CountFact::encode(expected, &mut words[1..2])?;
                CountFact::encode(found, &mut words[2..3])?;
            }
            InvalidTraitConformanceShape::ParameterModeMismatch { parameter_index }
            | InvalidTraitConformanceShape::ParameterContractMismatch { parameter_index } => {
                CountFact::encode(parameter_index, &mut words[1..2])?;
            }
            InvalidTraitConformanceShape::ParameterTypeMismatch {
                parameter_index,
                expected_type,
                found_type,
            } => {
                CountFact::encode(parameter_index, &mut words[1..2])?;
                TypeFact::encode(expected_type, &mut words[2..3])?;
                TypeFact::encode(found_type, &mut words[3..4])?;
            }
            InvalidTraitConformanceShape::ReturnTypeMismatch {
                return_index,
                expected_type,
                found_type,
            } => {
                CountFact::encode(return_index, &mut words[1..2])?;
                TypeFact::encode(expected_type, &mut words[2..3])?;
                TypeFact::encode(found_type, &mut words[3..4])?;
            }
            InvalidTraitConformanceShape::ReturnChannelMismatch { return_index } => {
                CountFact::encode(return_index, &mut words[1..2])?;
            }
            _ => {}
        }
        Ok(())
    }

    fn decode(words: &[u32]) -> Option<Self::Durable> {
        use InvalidTraitConformanceCode as Code;
        use InvalidTraitConformanceShape as Shape;
        let code = Code::from_code(*words.first()?)?;
        Some(match code {
            Code::ImportedModuleRoot => Shape::ImportedModuleRoot,
            Code::AliasTarget => Shape::AliasTarget,
            Code::NonCanonicalTarget => Shape::NonCanonicalTarget,
            Code::NonlocalSourceTarget => Shape::NonlocalSourceTarget,
            Code::BuiltinTarget => Shape::BuiltinTarget,
            Code::ExternalOpaqueTarget => Shape::ExternalOpaqueTarget,
            Code::DuplicateCanonicalEvidence => Shape::DuplicateCanonicalEvidence,
            Code::BuiltinEvidenceOverride => Shape::BuiltinEvidenceOverride,
            Code::IncompatibleTraitEvidence => Shape::IncompatibleTraitEvidence,
            Code::MissingMethod => Shape::MissingMethod,
            Code::ReceiverMutabilityMismatch => Shape::ReceiverMutabilityMismatch,
            Code::ParameterCountMismatch => Shape::ParameterCountMismatch {
                expected: CountFact::decode(words.get(1..2)?)?,
                found: CountFact::decode(words.get(2..3)?)?,
            },
            Code::ParameterModeMismatch => Shape::ParameterModeMismatch {
                parameter_index: CountFact::decode(words.get(1..2)?)?,
            },
            Code::ParameterContractMismatch => Shape::ParameterContractMismatch {
                parameter_index: CountFact::decode(words.get(1..2)?)?,
            },
            Code::ParameterTypeMismatch => Shape::ParameterTypeMismatch {
                parameter_index: CountFact::decode(words.get(1..2)?)?,
                expected_type: TypeFact::decode(words.get(2..3)?)?,
                found_type: TypeFact::decode(words.get(3..4)?)?,
            },
            Code::ReturnCountMismatch => Shape::ReturnCountMismatch {
                expected: CountFact::decode(words.get(1..2)?)?,
                found: CountFact::decode(words.get(2..3)?)?,
            },
            Code::ReturnTypeMismatch => Shape::ReturnTypeMismatch {
                return_index: CountFact::decode(words.get(1..2)?)?,
                expected_type: TypeFact::decode(words.get(2..3)?)?,
                found_type: TypeFact::decode(words.get(3..4)?)?,
            },
            Code::ReturnChannelMismatch => Shape::ReturnChannelMismatch {
                return_index: CountFact::decode(words.get(1..2)?)?,
            },
            Code::InferredFailureInInfallibleCast => Shape::InferredFailureInInfallibleCast,
        })
    }

    fn remap_ids(_words: &mut [u32], _remap: IdRemap<'_>) -> Result<(), FactCapacityError> {
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct InvalidTraitConformanceNames {
    pub(crate) target_name: StringId,
    pub(crate) reason_name: Option<StringId>,
    pub(crate) trait_name: Option<StringId>,
}

/// Decodes `[target_name, reason_name?, trait_name?]`; the reason name is the requirement name
/// except for `IncompatibleTraitEvidence`, and the optional trait name is always last.
pub(crate) fn decode_invalid_trait_conformance_names(
    code: InvalidTraitConformanceCode,
    ids: &[StringId],
) -> Option<InvalidTraitConformanceNames> {
    let has_reason_name = matches!(
        code,
        InvalidTraitConformanceCode::IncompatibleTraitEvidence
            | InvalidTraitConformanceCode::MissingMethod
            | InvalidTraitConformanceCode::ReceiverMutabilityMismatch
            | InvalidTraitConformanceCode::ParameterCountMismatch
            | InvalidTraitConformanceCode::ParameterModeMismatch
            | InvalidTraitConformanceCode::ParameterContractMismatch
            | InvalidTraitConformanceCode::ParameterTypeMismatch
            | InvalidTraitConformanceCode::ReturnCountMismatch
            | InvalidTraitConformanceCode::ReturnTypeMismatch
            | InvalidTraitConformanceCode::ReturnChannelMismatch
            | InvalidTraitConformanceCode::InferredFailureInInfallibleCast
    );
    match (has_reason_name, ids) {
        (false, [target_name]) => Some(InvalidTraitConformanceNames {
            target_name: *target_name,
            reason_name: None,
            trait_name: None,
        }),
        (false, [target_name, trait_name]) => Some(InvalidTraitConformanceNames {
            target_name: *target_name,
            reason_name: None,
            trait_name: Some(*trait_name),
        }),
        (true, [target_name, reason_name]) => Some(InvalidTraitConformanceNames {
            target_name: *target_name,
            reason_name: Some(*reason_name),
            trait_name: None,
        }),
        (true, [target_name, reason_name, trait_name]) => Some(InvalidTraitConformanceNames {
            target_name: *target_name,
            reason_name: Some(*reason_name),
            trait_name: Some(*trait_name),
        }),
        _ => None,
    }
}

fn split_invalid_trait_conformance(
    reason: InvalidTraitConformanceReason,
) -> (InvalidTraitConformanceShape<TypeId>, Option<StringId>) {
    use InvalidTraitConformanceReason as Reason;
    use InvalidTraitConformanceShape as Shape;
    let mut reason_name = None;
    let shape = match reason {
        Reason::ImportedModuleRoot => Shape::ImportedModuleRoot,
        Reason::AliasTarget => Shape::AliasTarget,
        Reason::NonCanonicalTarget => Shape::NonCanonicalTarget,
        Reason::NonlocalSourceTarget => Shape::NonlocalSourceTarget,
        Reason::BuiltinTarget => Shape::BuiltinTarget,
        Reason::ExternalOpaqueTarget => Shape::ExternalOpaqueTarget,
        Reason::DuplicateCanonicalEvidence => Shape::DuplicateCanonicalEvidence,
        Reason::BuiltinEvidenceOverride => Shape::BuiltinEvidenceOverride,
        Reason::IncompatibleTraitEvidence {
            incompatible_trait_name,
        } => {
            reason_name = Some(incompatible_trait_name);
            Shape::IncompatibleTraitEvidence
        }
        Reason::MissingMethod { requirement_name } => {
            reason_name = Some(requirement_name);
            Shape::MissingMethod
        }
        Reason::ReceiverMutabilityMismatch { requirement_name } => {
            reason_name = Some(requirement_name);
            Shape::ReceiverMutabilityMismatch
        }
        Reason::ParameterCountMismatch {
            requirement_name,
            expected,
            found,
        } => {
            reason_name = Some(requirement_name);
            Shape::ParameterCountMismatch { expected, found }
        }
        Reason::ParameterModeMismatch {
            requirement_name,
            parameter_index,
        } => {
            reason_name = Some(requirement_name);
            Shape::ParameterModeMismatch { parameter_index }
        }
        Reason::ParameterContractMismatch {
            requirement_name,
            parameter_index,
        } => {
            reason_name = Some(requirement_name);
            Shape::ParameterContractMismatch { parameter_index }
        }
        Reason::ParameterTypeMismatch {
            requirement_name,
            parameter_index,
            expected_type,
            found_type,
        } => {
            reason_name = Some(requirement_name);
            Shape::ParameterTypeMismatch {
                parameter_index,
                expected_type,
                found_type,
            }
        }
        Reason::ReturnCountMismatch {
            requirement_name,
            expected,
            found,
        } => {
            reason_name = Some(requirement_name);
            Shape::ReturnCountMismatch { expected, found }
        }
        Reason::ReturnTypeMismatch {
            requirement_name,
            return_index,
            expected_type,
            found_type,
        } => {
            reason_name = Some(requirement_name);
            Shape::ReturnTypeMismatch {
                return_index,
                expected_type,
                found_type,
            }
        }
        Reason::ReturnChannelMismatch {
            requirement_name,
            return_index,
        } => {
            reason_name = Some(requirement_name);
            Shape::ReturnChannelMismatch { return_index }
        }
        Reason::InferredFailureInInfallibleCast { requirement_name } => {
            reason_name = Some(requirement_name);
            Shape::InferredFailureInInfallibleCast
        }
    };
    (shape, reason_name)
}

pub(in crate::compiler_frontend::compiler_messages::diagnostics) fn invalid_trait_conformance(
    target_name: StringId,
    trait_name: Option<StringId>,
    reason: InvalidTraitConformanceReason,
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    let mut string_ids = Vec::with_capacity(3);
    string_ids.push(target_name);
    let (shape, reason_name) = split_invalid_trait_conformance(reason);
    if let Some(reason_name) = reason_name {
        string_ids.push(reason_name);
    }
    if let Some(trait_name) = trait_name {
        string_ids.push(trait_name);
    }
    DiagnosticDraft::with_facts(&INVALID_TRAIT_CONFORMANCE, span, |cursor| {
        cursor.push::<InvalidTraitConformanceFact>(&shape)
    })
    .with_string_ids(string_ids)
}

diagnostic_family! {
    INVALID_TRAIT_CONFORMANCE {
        internal: 3073,
        stable: "MOTH-RULE-0073",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "Invalid trait conformance",
        facts: [shape: InvalidTraitConformanceFact],
        labels: [
            LabelMessageCode::PREVIOUS_DECLARATION,
            LabelMessageCode::RENDERED_TEXT,
            LabelMessageCode::BUILTIN_FAILURE_CALL,
            LabelMessageCode::BUILTIN_FAILURE_ORIGIN,
            LabelMessageCode::BUILTIN_WRITE_BACK_ORIGIN,
        ],
        extra: StringIds,
        ctor: manual,
        view: InvalidTraitConformanceView,
        render: render_invalid_trait_conformance,
    }
}

diagnostic_family! {
    GENERIC_BOUND_PRIVATE_SURFACE_LEAK {
        internal: 3074,
        stable: "MOTH-RULE-0074",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "Private trait exposed by generic bound",
        facts: [
            owner_kind: UnitFact<PublicGenericOwnerKind>,
            owner_name: StringFact,
            trait_name: StringFact,
        ],
        labels: [],
        extra: None,
        ctor: generated generic_bound_private_surface_leak,
        view: GenericBoundPrivateSurfaceLeakView,
        render: render_generic_bound_private_surface_leak,
    }
}

diagnostic_family! {
    TRAIT_NAME_USED_AS_TYPE {
        internal: 3075,
        stable: "MOTH-RULE-0075",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "Trait name used as value type",
        facts: [trait_name: StringFact],
        labels: [],
        extra: None,
        ctor: generated trait_name_used_as_type,
        view: TraitNameUsedAsTypeView,
        render: render_trait_name_used_as_type,
    }
}

diagnostic_family! {
    INVALID_TRAIT_KEYWORD_USAGE {
        internal: 3076,
        stable: "MOTH-RULE-0076",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "Invalid trait keyword usage",
        facts: [reason: UnitFact<InvalidTraitKeywordUsageReason>],
        labels: [],
        extra: None,
        ctor: generated invalid_trait_keyword_usage,
        view: InvalidTraitKeywordUsageView,
        render: render_invalid_trait_keyword_usage,
    }
}

diagnostic_family! {
    EXPORT_OUTSIDE_MODULE_ROOT {
        internal: 3077,
        stable: "MOTH-RULE-0077",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "`export:` is only valid in a module root file",
        facts: [],
        labels: [],
        extra: None,
        ctor: generated export_outside_module_root,
        view: ExportOutsideModuleRootView,
        render: render_title_only,
    }
}

diagnostic_family! {
    INVALID_EXPORT_TARGET {
        internal: 3080,
        stable: "MOTH-RULE-0080",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "`export:` contains an invalid module API item",
        facts: [],
        labels: [],
        extra: None,
        ctor: generated invalid_export_target,
        view: InvalidExportTargetView,
        render: render_title_only,
    }
}

diagnostic_family! {
    DUPLICATE_PUBLIC_EXPORT {
        internal: 3081,
        stable: "MOTH-RULE-0081",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "Duplicate public export",
        facts: [name: StringFact],
        labels: [],
        extra: None,
        ctor: manual,
        view: DuplicatePublicExportView,
        render: render_duplicate_public_export,
    }
}

/// Stores the duplicate name and its message-less prior-declaration label.
pub(in crate::compiler_frontend::compiler_messages::diagnostics) fn duplicate_public_export(
    name: StringId,
    previous_span: Option<SourceSpan>,
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    let draft = DiagnosticDraft::with_facts(&DUPLICATE_PUBLIC_EXPORT, span, |cursor| {
        cursor.push::<StringFact>(&name)?;
        Ok(())
    });
    match previous_span {
        Some(previous_span) => draft.with_label(Some(previous_span), None),
        None => draft,
    }
}

diagnostic_family! {
    PRIVATE_TYPE_IN_EXPORTED_API {
        internal: 3082,
        stable: "MOTH-RULE-0082",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "Private type exposed by exported API",
        facts: [exported_name: StringFact, private_type: TypeFact],
        labels: [],
        extra: None,
        ctor: generated private_type_in_exported_api,
        view: PrivateTypeInExportedApiView,
        render: render_private_type_in_exported_api,
    }
}

diagnostic_family! {
    INVALID_TRAIT_INCOMPATIBILITY {
        internal: 3084,
        stable: "MOTH-RULE-0084",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "Invalid trait incompatibility",
        facts: [
            subject_name: StringFact,
            incompatible_trait_name: OptStringFact,
            reason: UnitFact<InvalidTraitIncompatibilityReason>,
        ],
        labels: [],
        extra: None,
        ctor: generated invalid_trait_incompatibility,
        view: InvalidTraitIncompatibilityView,
        render: render_invalid_trait_incompatibility,
    }
}

diagnostic_family! {
    DUPLICATE_EXPORT_BLOCK {
        internal: 3085,
        stable: "MOTH-RULE-0085",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "Duplicate export block",
        facts: [],
        labels: [],
        extra: None,
        ctor: generated duplicate_export_block,
        view: DuplicateExportBlockView,
        render: render_title_only,
    }
}

pub(in crate::compiler_frontend::compiler_messages::diagnostics) fn invalid_parameter_contract(
    reason: InvalidParameterContractReason,
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    let (code, contract, nested_contract) = InvalidParameterContractCode::split(reason);
    DiagnosticDraft::with_facts(&INVALID_PARAMETER_CONTRACT, span, |cursor| {
        cursor.push::<UnitFact<InvalidParameterContractCode>>(&code)?;
        cursor.push::<OptUnitFact<ParameterContractKind>>(&contract)?;
        cursor.push::<OptUnitFact<ParameterContractKind>>(&nested_contract)
    })
}

pub(in crate::compiler_frontend::compiler_messages::diagnostics) fn invalid_generic_parameter(
    reason: InvalidGenericParameterReason,
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    let (code, found) = InvalidGenericParameterCode::split(&reason);
    DiagnosticDraft::with_facts(&INVALID_GENERIC_PARAMETER, span, |cursor| {
        cursor.push::<UnitFact<InvalidGenericParameterCode>>(&code)?;
        cursor.push::<OptTokenFact>(&found)
    })
}

pub(in crate::compiler_frontend::compiler_messages::diagnostics) fn invalid_this_usage(
    reason: InvalidThisUsageReason,
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    let (code, function_name) = InvalidThisUsageCode::split(reason);
    DiagnosticDraft::with_facts(&INVALID_THIS_USAGE, span, |cursor| {
        cursor.push::<UnitFact<InvalidThisUsageCode>>(&code)?;
        cursor.push::<OptStringFact>(&function_name)
    })
}

pub(in crate::compiler_frontend::compiler_messages::diagnostics) fn invalid_receiver_declaration(
    reason: InvalidReceiverDeclarationReason,
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    let (code, function_name, type_id) = InvalidReceiverDeclarationCode::split(reason);
    DiagnosticDraft::with_facts(&INVALID_RECEIVER_DECLARATION, span, |cursor| {
        cursor.push::<UnitFact<InvalidReceiverDeclarationCode>>(&code)?;
        cursor.push::<OptStringFact>(&function_name)?;
        cursor.push::<OptTypeFact>(&type_id)
    })
}

pub(in crate::compiler_frontend::compiler_messages::diagnostics) fn invalid_function_signature(
    reason: InvalidFunctionSignatureReason,
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    let (code, found) = InvalidFunctionSignatureCode::split(&reason);
    DiagnosticDraft::with_facts(&INVALID_FUNCTION_SIGNATURE, span, |cursor| {
        cursor.push::<UnitFact<InvalidFunctionSignatureCode>>(&code)?;
        cursor.push::<OptTokenFact>(&found)
    })
}

pub(in crate::compiler_frontend::compiler_messages::diagnostics) fn invalid_generic_instantiation(
    type_name: Option<StringId>,
    reason: InvalidGenericInstantiationReason,
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    let (shape, extra) = split_generic_instantiation(type_name, reason);
    let draft = DiagnosticDraft::with_facts(&INVALID_GENERIC_INSTANTIATION, span, |cursor| {
        cursor.push::<GenericInstantiationFact>(&shape)
    });
    draft.with_string_ids(extra)
}

pub(in crate::compiler_frontend::compiler_messages::diagnostics) fn conflicting_generic_inference(
    type_name: Option<StringId>,
    subject: GenericInferenceSubject,
    conflict: BindingConflict,
    parameter_name: StringId,
    current_evidence_span: Option<SourceSpan>,
    previous_evidence_span: Option<SourceSpan>,
) -> DiagnosticDraft {
    let draft = invalid_generic_instantiation(
        type_name,
        InvalidGenericInstantiationReason::ConflictingInference {
            subject,
            parameter_id: conflict.parameter_id,
            parameter_name,
            existing_type_id: conflict.existing_type_id,
            replacement_type_id: conflict.replacement_type_id,
        },
        current_evidence_span,
    );
    match previous_evidence_span {
        Some(span) => draft.with_label(
            Some(span),
            Some(DiagnosticLabelMessage::GenericInferencePreviousEvidence),
        ),
        None => draft,
    }
}

pub(in crate::compiler_frontend::compiler_messages::diagnostics) fn duplicate_trait_requirement(
    trait_name: StringId,
    requirement_name: StringId,
    first_span: Option<SourceSpan>,
    duplicate_span: Option<SourceSpan>,
) -> DiagnosticDraft {
    let draft =
        DiagnosticDraft::with_facts(&DUPLICATE_TRAIT_REQUIREMENT, duplicate_span, |cursor| {
            cursor.push::<StringFact>(&trait_name)?;
            cursor.push::<StringFact>(&requirement_name)
        });
    if first_span.is_some() {
        draft.with_label(
            first_span,
            Some(DiagnosticLabelMessage::PreviousDeclaration),
        )
    } else {
        draft
    }
}

/// The compact draft for a legacy generic, trait or API diagnostic in this group.
pub(super) fn from_legacy(diagnostic: &CompilerDiagnostic) -> Option<DiagnosticDraft> {
    let span = diagnostic.primary_span;
    match (diagnostic.kind, &diagnostic.payload) {
        (
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::InvalidGenericApplication),
            DiagnosticPayload::InvalidGenericApplication { reason },
        ) => Some(invalid_generic_application(*reason, span)),
        (
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::InvalidParameterContract),
            DiagnosticPayload::InvalidParameterContract { reason },
        ) => Some(invalid_parameter_contract(*reason, span)),
        (
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::InvalidGenericParameter),
            DiagnosticPayload::InvalidGenericParameter { reason },
        ) => Some(invalid_generic_parameter(reason.clone(), span)),
        (
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidThisUsage),
            DiagnosticPayload::InvalidThisUsage { reason },
        ) => Some(invalid_this_usage(*reason, span)),
        (
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidReceiverDeclaration),
            DiagnosticPayload::InvalidReceiverDeclaration { reason },
        ) => Some(invalid_receiver_declaration(*reason, span)),
        (
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidFunctionSignature),
            DiagnosticPayload::InvalidFunctionSignature { reason },
        ) => Some(invalid_function_signature(reason.clone(), span)),
        (
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidGenericInstantiation),
            DiagnosticPayload::InvalidGenericInstantiation { type_name, reason },
        ) => {
            let draft = match reason {
                InvalidGenericInstantiationReason::ConflictingInference {
                    subject,
                    parameter_id,
                    parameter_name,
                    existing_type_id,
                    replacement_type_id,
                } => conflicting_generic_inference(
                    *type_name,
                    *subject,
                    BindingConflict {
                        parameter_id: *parameter_id,
                        existing_type_id: *existing_type_id,
                        replacement_type_id: *replacement_type_id,
                    },
                    *parameter_name,
                    span,
                    None,
                ),
                reason => invalid_generic_instantiation(*type_name, reason.clone(), span),
            };
            Some(draft)
        }
        (
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidTraitConformance),
            DiagnosticPayload::InvalidTraitConformance {
                target_name,
                trait_name,
                reason,
            },
        ) => Some(invalid_trait_conformance(
            *target_name,
            *trait_name,
            reason.clone(),
            span,
        )),
        (
            DiagnosticKind::Rule(RuleDiagnosticKind::UnknownTrait),
            DiagnosticPayload::UnknownTrait { name },
        ) => Some(unknown_trait(*name, span)),
        (
            DiagnosticKind::Rule(RuleDiagnosticKind::DuplicateTraitRequirement),
            DiagnosticPayload::DuplicateTraitRequirement {
                trait_name,
                requirement_name,
            },
        ) => Some(duplicate_trait_requirement(
            *trait_name,
            *requirement_name,
            None,
            span,
        )),
        (
            DiagnosticKind::Rule(RuleDiagnosticKind::TraitPrivateSurfaceLeak),
            DiagnosticPayload::TraitPrivateSurfaceLeak {
                trait_name,
                surface_type,
            },
        ) => Some(trait_private_surface_leak(*trait_name, *surface_type, span)),
        (
            DiagnosticKind::Rule(RuleDiagnosticKind::ExportOutsideModuleRoot),
            DiagnosticPayload::None,
        ) => Some(export_outside_module_root(span)),
        (
            DiagnosticKind::Rule(RuleDiagnosticKind::UnsupportedTraitFeature),
            DiagnosticPayload::UnsupportedTraitFeature {
                trait_name,
                feature,
            },
        ) => Some(unsupported_trait_feature(*trait_name, *feature, span)),
        (
            DiagnosticKind::Rule(RuleDiagnosticKind::TraitNameUsedAsType),
            DiagnosticPayload::TraitNameUsedAsType { trait_name },
        ) => Some(trait_name_used_as_type(*trait_name, span)),
        (
            DiagnosticKind::Rule(RuleDiagnosticKind::GenericBoundPrivateSurfaceLeak),
            DiagnosticPayload::GenericBoundPrivateSurfaceLeak {
                owner_kind,
                owner_name,
                trait_name,
            },
        ) => Some(generic_bound_private_surface_leak(
            *owner_kind,
            *owner_name,
            *trait_name,
            span,
        )),
        (
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidTraitKeywordUsage),
            DiagnosticPayload::InvalidTraitKeywordUsage { reason },
        ) => Some(invalid_trait_keyword_usage(*reason, span)),
        (
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidTraitIncompatibility),
            DiagnosticPayload::InvalidTraitIncompatibility {
                subject_name,
                incompatible_trait_name,
                reason,
            },
        ) => Some(invalid_trait_incompatibility(
            *subject_name,
            *incompatible_trait_name,
            *reason,
            span,
        )),
        (
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidExportTarget),
            DiagnosticPayload::None,
        ) => Some(invalid_export_target(span)),
        (
            DiagnosticKind::Rule(RuleDiagnosticKind::DuplicatePublicExport),
            DiagnosticPayload::DuplicatePublicExport { name },
        ) => {
            // The outer adapter copies this existing message-less label once.
            Some(duplicate_public_export(*name, None, span))
        }
        (
            DiagnosticKind::Rule(RuleDiagnosticKind::PrivateTypeInExportedApi),
            DiagnosticPayload::PrivateTypeInExportedApi {
                exported_name,
                private_type,
            },
        ) => Some(private_type_in_exported_api(
            *exported_name,
            *private_type,
            span,
        )),
        (
            DiagnosticKind::Rule(RuleDiagnosticKind::DuplicateExportBlock),
            DiagnosticPayload::None,
        ) => Some(duplicate_export_block(span)),
        _ => None,
    }
}

pub(in crate::compiler_frontend::compiler_messages::diagnostics) const FAMILIES: &[FamilyRow] = &[
    INVALID_GENERIC_APPLICATION,
    INVALID_GENERIC_PARAMETER,
    INVALID_PARAMETER_CONTRACT,
    INVALID_THIS_USAGE,
    INVALID_RECEIVER_DECLARATION,
    INVALID_GENERIC_INSTANTIATION,
    INVALID_FUNCTION_SIGNATURE,
    UNKNOWN_TRAIT,
    DUPLICATE_TRAIT_REQUIREMENT,
    TRAIT_PRIVATE_SURFACE_LEAK,
    UNSUPPORTED_TRAIT_FEATURE,
    INVALID_TRAIT_CONFORMANCE,
    GENERIC_BOUND_PRIVATE_SURFACE_LEAK,
    TRAIT_NAME_USED_AS_TYPE,
    INVALID_TRAIT_KEYWORD_USAGE,
    EXPORT_OUTSIDE_MODULE_ROOT,
    INVALID_EXPORT_TARGET,
    DUPLICATE_PUBLIC_EXPORT,
    PRIVATE_TYPE_IN_EXPORTED_API,
    INVALID_TRAIT_INCOMPATIBILITY,
    DUPLICATE_EXPORT_BLOCK,
];
