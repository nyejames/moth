//! Family rows for choice and control-flow diagnostics.
//!
//! WHAT: the family rows, constructors and typed views for choice and control-flow diagnostics.
//! WHY: related choice and control-flow diagnostics share one schema table.

use super::super::schema::DiagnosticCategory;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::facts::{
    OptCountFact, OptStringFact, OptTypeFact, UnitFact, unit_code,
};
use crate::compiler_frontend::compiler_messages::diagnostics::labels::LabelMessageCode;
use crate::compiler_frontend::compiler_messages::diagnostics::schema::{
    FamilyRow, diagnostic_family,
};
use crate::compiler_frontend::compiler_messages::diagnostics::view::DiagnosticView;
use crate::compiler_frontend::compiler_messages::render::{
    render_compile_time_evaluation_error, render_invalid_choice_variant,
    render_invalid_control_flow_statement, render_invalid_fallible_handling,
    render_invalid_loop_header, render_invalid_match_arm, render_invalid_match_guard,
    render_invalid_match_pattern, render_invalid_standalone_statement,
    render_invalid_statement_position, render_non_exhaustive_match, render_unreachable_match_arm,
};
use crate::compiler_frontend::compiler_messages::{
    CompileTimeEvaluationErrorReason, DiagnosticSeverity, InvalidChoiceVariantReason,
    InvalidControlFlowStatementReason, InvalidFallibleHandlingReason, InvalidLoopHeaderReason,
    InvalidMatchArmReason, InvalidMatchGuardReason, InvalidMatchPatternReason,
    InvalidStandaloneStatementReason, InvalidStatementPositionReason, NonExhaustiveMatchReason,
};
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::string_interning::StringId;

unit_code! {
    impl NonExhaustiveMatchReason {
        MissingElseArm = 1,
        MissingVariants = 2,
        GuardedArmsRequireElse = 3,
        MissingOptionPatterns = 4,
    }
}
unit_code! {
    impl InvalidMatchGuardReason {
        SideEffectingPrelude = 1,
    }
}
unit_code! {
    impl InvalidStandaloneStatementReason {
        FieldRead = 1,
        Expression = 2,
        StandaloneTemplate = 3,
    }
}
unit_code! {
    impl InvalidMatchArmReason {
        SemicolonDelimiter = 1,
        LegacyColonSyntax = 2,
        LegacyElseSyntax = 3,
        InvalidArrow = 4,
        ArmMustStartNewLine = 5,
        ExpectedArmHeader = 6,
    }
}

unit_code! {
    impl InvalidStatementPositionReason {
        UnexpectedComma = 1,
        UnexpectedCloseParenthesis = 2,
        UnexpectedCloseCurly = 3,
        UnexpectedPipe = 4,
        UnexpectedArrow = 5,
        UnexpectedWildcard = 6,
        AnonymousDeclaredRegion = 7,
        GenericParameterOutsideDeclarationHeader = 8,
        UnexpectedOf = 9,
        UnexpectedScopeCloseInExpression = 10,
        UnexpectedScopeCloseInTemplate = 11,
    }
}
unit_code! {
    impl InvalidChoiceVariantReason {
        EmptyRecordBody = 1,
        RecursiveDeclaration = 2,
        ConstructorStyleNotSupported = 3,
        PayloadShorthandNotSupported = 4,
        UnexpectedSeparator = 5,
        MissingVariants = 6,
        UnknownVariant = 7,
        UnitVariantWithParentheses = 8,
        UnitVariantAsConstructor = 9,
        PayloadVariantMissingArguments = 10,
    }
}

unit_code! {
    impl InvalidControlFlowStatementReason {
        ElseOutsideIfOrMatch = 1,
        ElseIfUnsupported = 2,
        BreakOutsideLoop = 3,
        ContinueOutsideLoop = 4,
        ReturnOutsideFunction = 5,
        ReturnBangOutsideErrorFunction = 6,
        ExpectedColonAfterCondition = 7,
        ExpectedConditionAfterIf = 8,
        UnexpectedEndOfFileInMatch = 9,
        CaseRequiredBeforeElse = 10,
        DuplicateElseArm = 11,
        ExpectedFatArrow = 12,
        InlineValueIfMultiline = 13,
        InlineValueIfElseThen = 14,
        ValueIfMissingElse = 15,
        ValueIfBranchFallsThrough = 16,
        ValueIfNoProducingPath = 17,
        ValueBlockOutsideReceiver = 18,
        ValueIfOptionNonePredicate = 19,
        ValueIfOptionLiteralPredicate = 20,
        ExpectedValueAfterThen = 21,
        ExpectedValueAfterElse = 22,
    }
}

unit_code! {
    impl InvalidMatchPatternReason {
        WildcardNotSupported = 1,
        AsNotValid = 2,
        NegativeLiteralNotNumeric = 3,
        LiteralTypeUnsupported = 4,
        ScrutineeTypeUnsupportedForRelational = 5,
        UnitVariantHasPayload = 6,
        PayloadVariantNeedsBindings = 7,
        CaptureBindingMustBeFieldName = 8,
        ExpectedLocalBindingAfterAs = 9,
        AliasMustBeLocalBinding = 10,
        DuplicateCaptureBinding = 11,
        TooManyCaptureBindings = 12,
        CaptureBindingNameMismatch = 13,
        TooFewCaptureBindings = 14,
        QualifierDoesNotMatchScrutinee = 15,
        ExpectedVariantNameAfterQualifier = 16,
        MustUseVariantNamesNotLiterals = 17,
        MustStartWithVariantName = 18,
        UnknownVariant = 19,
        CaptureBindingShadowsVariable = 20,
        NonePatternRequiresOptionalScrutinee = 21,
        OptionValuePatternRequiresEquality = 22,
        BareCaptureOnOptionalScrutinee = 23,
        OptionPresentCaptureOnNonOptional = 24,
        EmptyOptionPresentCapture = 25,
        OptionPresentCaptureTypeAnnotation = 26,
        MissingClosingPipe = 27,
        ExpectedBindingInOptionPresentCapture = 28,
    }
}

unit_code! {
    pub(crate) enum InvalidLoopHeaderCode {
        EmptyHeader = 1,
        MissingColon = 2,
        RemovedInSyntax = 3,
        MissingClosingPipe = 4,
        MalformedBindingPipes = 5,
        MissingSourceBeforeBindings = 6,
        EmptyBindingList = 7,
        ThisBinding = 8,
        BindingMustBeSymbol = 9,
        MissingBindingComma = 10,
        TrailingBindingComma = 11,
        BareSingleBinding = 12,
        BareDualBinding = 13,
        TooManyBindings = 14,
        DuplicateBindingName = 15,
        BindingAlreadyDeclared = 16,
        CollectionSourceNotCollection = 17,
        MissingRangeSeparator = 18,
        MissingRangeEndBound = 19,
        MissingRangeStep = 20,
        FloatRangeMissingStep = 21,
        ZeroRangeStep = 22,
        ExpectedHeaderExpression = 23,
    }
}

impl InvalidLoopHeaderCode {
    pub(crate) fn of(reason: &InvalidLoopHeaderReason) -> Self {
        match reason {
            InvalidLoopHeaderReason::EmptyHeader => Self::EmptyHeader,
            InvalidLoopHeaderReason::MissingColon => Self::MissingColon,
            InvalidLoopHeaderReason::RemovedInSyntax => Self::RemovedInSyntax,
            InvalidLoopHeaderReason::MissingClosingPipe => Self::MissingClosingPipe,
            InvalidLoopHeaderReason::MalformedBindingPipes => Self::MalformedBindingPipes,
            InvalidLoopHeaderReason::MissingSourceBeforeBindings => {
                Self::MissingSourceBeforeBindings
            }
            InvalidLoopHeaderReason::EmptyBindingList => Self::EmptyBindingList,
            InvalidLoopHeaderReason::ThisBinding => Self::ThisBinding,
            InvalidLoopHeaderReason::BindingMustBeSymbol => Self::BindingMustBeSymbol,
            InvalidLoopHeaderReason::MissingBindingComma => Self::MissingBindingComma,
            InvalidLoopHeaderReason::TrailingBindingComma => Self::TrailingBindingComma,
            InvalidLoopHeaderReason::BareSingleBinding => Self::BareSingleBinding,
            InvalidLoopHeaderReason::BareDualBinding => Self::BareDualBinding,
            InvalidLoopHeaderReason::TooManyBindings => Self::TooManyBindings,
            InvalidLoopHeaderReason::DuplicateBindingName => Self::DuplicateBindingName,
            InvalidLoopHeaderReason::BindingAlreadyDeclared => Self::BindingAlreadyDeclared,
            InvalidLoopHeaderReason::CollectionSourceNotCollection { .. } => {
                Self::CollectionSourceNotCollection
            }
            InvalidLoopHeaderReason::MissingRangeSeparator => Self::MissingRangeSeparator,
            InvalidLoopHeaderReason::MissingRangeEndBound => Self::MissingRangeEndBound,
            InvalidLoopHeaderReason::MissingRangeStep => Self::MissingRangeStep,
            InvalidLoopHeaderReason::FloatRangeMissingStep => Self::FloatRangeMissingStep,
            InvalidLoopHeaderReason::ZeroRangeStep => Self::ZeroRangeStep,
            InvalidLoopHeaderReason::ExpectedHeaderExpression => Self::ExpectedHeaderExpression,
        }
    }

    pub(crate) fn split(reason: &InvalidLoopHeaderReason) -> (Self, Option<TypeId>) {
        match reason {
            InvalidLoopHeaderReason::CollectionSourceNotCollection { found_type } => {
                (Self::CollectionSourceNotCollection, *found_type)
            }
            _ => (Self::of(reason), None),
        }
    }

    pub(crate) fn to_reason(self, found_type: Option<TypeId>) -> InvalidLoopHeaderReason {
        match self {
            Self::EmptyHeader => InvalidLoopHeaderReason::EmptyHeader,
            Self::MissingColon => InvalidLoopHeaderReason::MissingColon,
            Self::RemovedInSyntax => InvalidLoopHeaderReason::RemovedInSyntax,
            Self::MissingClosingPipe => InvalidLoopHeaderReason::MissingClosingPipe,
            Self::MalformedBindingPipes => InvalidLoopHeaderReason::MalformedBindingPipes,
            Self::MissingSourceBeforeBindings => {
                InvalidLoopHeaderReason::MissingSourceBeforeBindings
            }
            Self::EmptyBindingList => InvalidLoopHeaderReason::EmptyBindingList,
            Self::ThisBinding => InvalidLoopHeaderReason::ThisBinding,
            Self::BindingMustBeSymbol => InvalidLoopHeaderReason::BindingMustBeSymbol,
            Self::MissingBindingComma => InvalidLoopHeaderReason::MissingBindingComma,
            Self::TrailingBindingComma => InvalidLoopHeaderReason::TrailingBindingComma,
            Self::BareSingleBinding => InvalidLoopHeaderReason::BareSingleBinding,
            Self::BareDualBinding => InvalidLoopHeaderReason::BareDualBinding,
            Self::TooManyBindings => InvalidLoopHeaderReason::TooManyBindings,
            Self::DuplicateBindingName => InvalidLoopHeaderReason::DuplicateBindingName,
            Self::BindingAlreadyDeclared => InvalidLoopHeaderReason::BindingAlreadyDeclared,
            Self::CollectionSourceNotCollection => {
                InvalidLoopHeaderReason::CollectionSourceNotCollection { found_type }
            }
            Self::MissingRangeSeparator => InvalidLoopHeaderReason::MissingRangeSeparator,
            Self::MissingRangeEndBound => InvalidLoopHeaderReason::MissingRangeEndBound,
            Self::MissingRangeStep => InvalidLoopHeaderReason::MissingRangeStep,
            Self::FloatRangeMissingStep => InvalidLoopHeaderReason::FloatRangeMissingStep,
            Self::ZeroRangeStep => InvalidLoopHeaderReason::ZeroRangeStep,
            Self::ExpectedHeaderExpression => InvalidLoopHeaderReason::ExpectedHeaderExpression,
        }
    }
}

unit_code! {
    pub(crate) enum InvalidFallibleHandlingCode {
        CatchOutsideBoundary = 1,
        ExpectedCatchBlockOrHandler = 2,
        ExpectedCatchHandlerOpeningPipe = 3,
        ExpectedCatchHandlerIdentifier = 4,
        ExpectedCatchHandlerClosingPipe = 5,
        ExpectedCatchHandlerColon = 6,
        EmptyCatchHandlerBinding = 7,
        MultipleCatchHandlerBindings = 8,
        FallbackValuesForErrorOnlyResult = 9,
        CatchOnNonFallible = 10,
        CatchOnOptional = 11,
        BangOnNonFallible = 12,
        BangOnOptional = 13,
        FunctionHasNoErrorSlot = 14,
        EntryHasIncompatibleError = 15,
        NotOptionExpression = 16,
        FunctionHasNoOptionalReturn = 17,
        FunctionHasMultipleSuccessReturns = 18,
        OptionPropagationReturnTypeMismatch = 19,
        OptionPropagationCatchConflict = 20,
        ExplicitPropagationCatchConflict = 21,
        CatchHandlerConflicts = 22,
        CatchHandlerCanFallThrough = 23,
        InlineCatchMultiline = 24,
        AssertionMessageCannotEscape = 25,
        ThenWithNoActiveValueTarget = 26,
        ThenCrossesBlockedConstruct = 27,
        ThenRequiresValues = 28,
        DirectOptionFallbackSyntax = 29,
        UnhandledErrorReturn = 30,
        SuccessValueDiscarded = 31,
        IncompatibleCatchErrorTypes = 32,
        CustomErrorMixedWithImplicitFailure = 33,
        UnhandledBuiltinFailureInCustomErrorFunction = 34,
        UnhandledBuiltinFailureInExportedFunction = 35,
    }
}

impl InvalidFallibleHandlingCode {
    pub(crate) fn of(reason: &InvalidFallibleHandlingReason) -> Self {
        Self::split(reason).0
    }

    pub(crate) fn split(
        reason: &InvalidFallibleHandlingReason,
    ) -> (Self, Option<TypeId>, Option<TypeId>, Option<usize>) {
        match reason {
            InvalidFallibleHandlingReason::EntryHasIncompatibleError {
                produced_error_type,
                entry_error_type,
            } => (
                Self::EntryHasIncompatibleError,
                Some(*produced_error_type),
                Some(*entry_error_type),
                None,
            ),
            InvalidFallibleHandlingReason::FunctionHasMultipleSuccessReturns { count } => (
                Self::FunctionHasMultipleSuccessReturns,
                None,
                None,
                Some(*count),
            ),
            InvalidFallibleHandlingReason::IncompatibleCatchErrorTypes {
                first_error_type_id,
                second_error_type_id,
                ..
            } => (
                Self::IncompatibleCatchErrorTypes,
                Some(*first_error_type_id),
                Some(*second_error_type_id),
                None,
            ),
            InvalidFallibleHandlingReason::CustomErrorMixedWithImplicitFailure {
                error_type_id,
                ..
            } => (
                Self::CustomErrorMixedWithImplicitFailure,
                Some(*error_type_id),
                None,
                None,
            ),
            InvalidFallibleHandlingReason::UnhandledBuiltinFailureInCustomErrorFunction {
                error_type_id,
                witness,
            } => (
                Self::UnhandledBuiltinFailureInCustomErrorFunction,
                Some(*error_type_id),
                None,
                Some(
                    usize::try_from(witness.elided_call_hops)
                        .expect("call-hop counts fit in usize"),
                ),
            ),
            InvalidFallibleHandlingReason::UnhandledBuiltinFailureInExportedFunction {
                witness,
            } => (
                Self::UnhandledBuiltinFailureInExportedFunction,
                None,
                None,
                Some(
                    usize::try_from(witness.elided_call_hops)
                        .expect("call-hop counts fit in usize"),
                ),
            ),
            _ => (Self::of_unit(reason), None, None, None),
        }
    }

    fn of_unit(reason: &InvalidFallibleHandlingReason) -> Self {
        match reason {
            InvalidFallibleHandlingReason::CatchOutsideBoundary => Self::CatchOutsideBoundary,
            InvalidFallibleHandlingReason::ExpectedCatchBlockOrHandler => {
                Self::ExpectedCatchBlockOrHandler
            }
            InvalidFallibleHandlingReason::ExpectedCatchHandlerOpeningPipe => {
                Self::ExpectedCatchHandlerOpeningPipe
            }
            InvalidFallibleHandlingReason::ExpectedCatchHandlerIdentifier => {
                Self::ExpectedCatchHandlerIdentifier
            }
            InvalidFallibleHandlingReason::ExpectedCatchHandlerClosingPipe => {
                Self::ExpectedCatchHandlerClosingPipe
            }
            InvalidFallibleHandlingReason::ExpectedCatchHandlerColon => {
                Self::ExpectedCatchHandlerColon
            }
            InvalidFallibleHandlingReason::EmptyCatchHandlerBinding => {
                Self::EmptyCatchHandlerBinding
            }
            InvalidFallibleHandlingReason::MultipleCatchHandlerBindings => {
                Self::MultipleCatchHandlerBindings
            }
            InvalidFallibleHandlingReason::FallbackValuesForErrorOnlyResult => {
                Self::FallbackValuesForErrorOnlyResult
            }
            InvalidFallibleHandlingReason::CatchOnNonFallible => Self::CatchOnNonFallible,
            InvalidFallibleHandlingReason::CatchOnOptional => Self::CatchOnOptional,
            InvalidFallibleHandlingReason::BangOnNonFallible => Self::BangOnNonFallible,
            InvalidFallibleHandlingReason::BangOnOptional => Self::BangOnOptional,
            InvalidFallibleHandlingReason::FunctionHasNoErrorSlot => Self::FunctionHasNoErrorSlot,
            InvalidFallibleHandlingReason::NotOptionExpression => Self::NotOptionExpression,
            InvalidFallibleHandlingReason::FunctionHasNoOptionalReturn => {
                Self::FunctionHasNoOptionalReturn
            }
            InvalidFallibleHandlingReason::OptionPropagationReturnTypeMismatch => {
                Self::OptionPropagationReturnTypeMismatch
            }
            InvalidFallibleHandlingReason::OptionPropagationCatchConflict => {
                Self::OptionPropagationCatchConflict
            }
            InvalidFallibleHandlingReason::ExplicitPropagationCatchConflict => {
                Self::ExplicitPropagationCatchConflict
            }
            InvalidFallibleHandlingReason::CatchHandlerConflicts => Self::CatchHandlerConflicts,
            InvalidFallibleHandlingReason::CatchHandlerCanFallThrough => {
                Self::CatchHandlerCanFallThrough
            }
            InvalidFallibleHandlingReason::InlineCatchMultiline => Self::InlineCatchMultiline,
            InvalidFallibleHandlingReason::AssertionMessageCannotEscape => {
                Self::AssertionMessageCannotEscape
            }
            InvalidFallibleHandlingReason::ThenWithNoActiveValueTarget => {
                Self::ThenWithNoActiveValueTarget
            }
            InvalidFallibleHandlingReason::ThenCrossesBlockedConstruct => {
                Self::ThenCrossesBlockedConstruct
            }
            InvalidFallibleHandlingReason::ThenRequiresValues => Self::ThenRequiresValues,
            InvalidFallibleHandlingReason::DirectOptionFallbackSyntax => {
                Self::DirectOptionFallbackSyntax
            }
            InvalidFallibleHandlingReason::UnhandledErrorReturn => Self::UnhandledErrorReturn,
            InvalidFallibleHandlingReason::SuccessValueDiscarded => Self::SuccessValueDiscarded,
            InvalidFallibleHandlingReason::EntryHasIncompatibleError { .. }
            | InvalidFallibleHandlingReason::FunctionHasMultipleSuccessReturns { .. }
            | InvalidFallibleHandlingReason::IncompatibleCatchErrorTypes { .. }
            | InvalidFallibleHandlingReason::CustomErrorMixedWithImplicitFailure { .. }
            | InvalidFallibleHandlingReason::UnhandledBuiltinFailureInCustomErrorFunction {
                ..
            }
            | InvalidFallibleHandlingReason::UnhandledBuiltinFailureInExportedFunction { .. } => {
                crate::compiler_bug!(
                    CompilerStage::DiagnosticReporting,
                    "data-bearing fallible reasons are split before unit mapping"
                )
            }
        }
    }

    pub(crate) fn unit_reason(self) -> Option<InvalidFallibleHandlingReason> {
        Some(match self {
            Self::CatchOutsideBoundary => InvalidFallibleHandlingReason::CatchOutsideBoundary,
            Self::ExpectedCatchBlockOrHandler => {
                InvalidFallibleHandlingReason::ExpectedCatchBlockOrHandler
            }
            Self::ExpectedCatchHandlerOpeningPipe => {
                InvalidFallibleHandlingReason::ExpectedCatchHandlerOpeningPipe
            }
            Self::ExpectedCatchHandlerIdentifier => {
                InvalidFallibleHandlingReason::ExpectedCatchHandlerIdentifier
            }
            Self::ExpectedCatchHandlerClosingPipe => {
                InvalidFallibleHandlingReason::ExpectedCatchHandlerClosingPipe
            }
            Self::ExpectedCatchHandlerColon => {
                InvalidFallibleHandlingReason::ExpectedCatchHandlerColon
            }
            Self::EmptyCatchHandlerBinding => {
                InvalidFallibleHandlingReason::EmptyCatchHandlerBinding
            }
            Self::MultipleCatchHandlerBindings => {
                InvalidFallibleHandlingReason::MultipleCatchHandlerBindings
            }
            Self::FallbackValuesForErrorOnlyResult => {
                InvalidFallibleHandlingReason::FallbackValuesForErrorOnlyResult
            }
            Self::CatchOnNonFallible => InvalidFallibleHandlingReason::CatchOnNonFallible,
            Self::CatchOnOptional => InvalidFallibleHandlingReason::CatchOnOptional,
            Self::BangOnNonFallible => InvalidFallibleHandlingReason::BangOnNonFallible,
            Self::BangOnOptional => InvalidFallibleHandlingReason::BangOnOptional,
            Self::FunctionHasNoErrorSlot => InvalidFallibleHandlingReason::FunctionHasNoErrorSlot,
            Self::NotOptionExpression => InvalidFallibleHandlingReason::NotOptionExpression,
            Self::FunctionHasNoOptionalReturn => {
                InvalidFallibleHandlingReason::FunctionHasNoOptionalReturn
            }
            Self::OptionPropagationReturnTypeMismatch => {
                InvalidFallibleHandlingReason::OptionPropagationReturnTypeMismatch
            }
            Self::OptionPropagationCatchConflict => {
                InvalidFallibleHandlingReason::OptionPropagationCatchConflict
            }
            Self::ExplicitPropagationCatchConflict => {
                InvalidFallibleHandlingReason::ExplicitPropagationCatchConflict
            }
            Self::CatchHandlerConflicts => InvalidFallibleHandlingReason::CatchHandlerConflicts,
            Self::CatchHandlerCanFallThrough => {
                InvalidFallibleHandlingReason::CatchHandlerCanFallThrough
            }
            Self::InlineCatchMultiline => InvalidFallibleHandlingReason::InlineCatchMultiline,
            Self::AssertionMessageCannotEscape => {
                InvalidFallibleHandlingReason::AssertionMessageCannotEscape
            }
            Self::ThenWithNoActiveValueTarget => {
                InvalidFallibleHandlingReason::ThenWithNoActiveValueTarget
            }
            Self::ThenCrossesBlockedConstruct => {
                InvalidFallibleHandlingReason::ThenCrossesBlockedConstruct
            }
            Self::ThenRequiresValues => InvalidFallibleHandlingReason::ThenRequiresValues,
            Self::DirectOptionFallbackSyntax => {
                InvalidFallibleHandlingReason::DirectOptionFallbackSyntax
            }
            Self::UnhandledErrorReturn => InvalidFallibleHandlingReason::UnhandledErrorReturn,
            Self::SuccessValueDiscarded => InvalidFallibleHandlingReason::SuccessValueDiscarded,
            Self::EntryHasIncompatibleError
            | Self::FunctionHasMultipleSuccessReturns
            | Self::IncompatibleCatchErrorTypes
            | Self::CustomErrorMixedWithImplicitFailure
            | Self::UnhandledBuiltinFailureInCustomErrorFunction
            | Self::UnhandledBuiltinFailureInExportedFunction => return None,
        })
    }
}

unit_code! {
    impl CompileTimeEvaluationErrorReason {
        IntegerOverflow = 1,
        FloatOverflow = 2,
        DivideByZero = 3,
        InvalidExponent = 4,
        InvalidOperatorForType = 5,
        ConstantSelfReference = 6,
        ConstantNotVisible = 7,
        NonConstantReferenceInConstant = 8,
        SameFileForwardConstantReference = 9,
        ConstantInitializerNotFoldable = 10,
        ExternalNonScalarConstantInConstantContext = 11,
        NonCompileTimeFieldInConstantContext = 12,
        NoneLiteralRequiresOptionalTypeContext = 13,
        ExternalTypeConstructionNotSupported = 14,
        StructFieldDefaultNotFoldable = 15,
        EvaluationOutputBoundExceeded = 16,
        StructuralStringRequiresFinalText = 17,
    }
}

diagnostic_family! {
    UNREACHABLE_MATCH_ARM {
        internal: 3022,
        stable: "MOTH-RULE-0022",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Warning,
        title: "Unreachable match arm",
        facts: [],
        labels: [],
        extra: None,
        ctor: generated unreachable_match_arm,
        view: UnreachableMatchArmView,
        render: render_unreachable_match_arm,
    }
}

/// Stores an invalid choice-variant draft, including its available-name list as string ids.
pub(crate) fn invalid_choice_variant(
    reason: InvalidChoiceVariantReason,
    choice_name: Option<StringId>,
    variant_name: Option<StringId>,
    available_variants: Vec<StringId>,
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    DiagnosticDraft::with_facts(&INVALID_CHOICE_VARIANT, span, |cursor| {
        cursor.push::<UnitFact<InvalidChoiceVariantReason>>(&reason)?;
        cursor.push::<OptStringFact>(&choice_name)?;
        cursor.push::<OptStringFact>(&variant_name)
    })
    .with_string_ids(available_variants)
}

/// Returns the available-variant list stored by this family.
pub(crate) fn invalid_choice_variant_available_variants<'a>(
    diagnostic: &DiagnosticView<'a>,
) -> &'a [StringId] {
    diagnostic.string_ids()
}

/// Stores an invalid loop-header draft with the collection-source type in a pending optional type fact.
pub(crate) fn invalid_loop_header(
    reason: InvalidLoopHeaderReason,
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    let (code, found_type) = InvalidLoopHeaderCode::split(&reason);
    DiagnosticDraft::with_facts(&INVALID_LOOP_HEADER, span, |cursor| {
        cursor.push::<UnitFact<InvalidLoopHeaderCode>>(&code)?;
        cursor.push::<OptTypeFact>(&found_type)
    })
}

/// Stores fallible-handling facts in a code plus two optional types and one per-code count.
pub(crate) fn invalid_fallible_handling(
    reason: InvalidFallibleHandlingReason,
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    let (code, first_error_type, second_error_type, count) =
        InvalidFallibleHandlingCode::split(&reason);
    DiagnosticDraft::with_facts(&INVALID_FALLIBLE_HANDLING, span, |cursor| {
        cursor.push::<UnitFact<InvalidFallibleHandlingCode>>(&code)?;
        cursor.push::<OptTypeFact>(&first_error_type)?;
        cursor.push::<OptTypeFact>(&second_error_type)?;
        cursor.push::<OptCountFact>(&count)
    })
}
diagnostic_family! {
    NON_EXHAUSTIVE_MATCH {
        internal: 3050,
        stable: "MOTH-RULE-0050",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "Non-exhaustive match",
        facts: [reason: UnitFact<NonExhaustiveMatchReason>],
        labels: [],
        extra: StringIds,
        ctor: manual,
        reason_key: reason,
        view: NonExhaustiveMatchView,
        render: render_non_exhaustive_match,
    }
}

/// The non-exhaustive-match draft. The missing variant names are stored as the extra string IDs.
pub(crate) fn non_exhaustive_match(
    reason: NonExhaustiveMatchReason,
    missing_variants: Vec<StringId>,
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    DiagnosticDraft::with_facts(&NON_EXHAUSTIVE_MATCH, span, |cursor| {
        cursor.push::<UnitFact<NonExhaustiveMatchReason>>(&reason)
    })
    .with_string_ids(missing_variants)
}

diagnostic_family! {
    INVALID_STANDALONE_STATEMENT {
        internal: 1025,
        stable: "MOTH-SYNTAX-0025",
        category: DiagnosticCategory::Syntax,
        severity: DiagnosticSeverity::Error,
        title: "Invalid standalone statement",
        facts: [reason: UnitFact<InvalidStandaloneStatementReason>],
        labels: [],
        extra: None,
        ctor: generated invalid_standalone_statement,
        reason_key: reason,
        view: InvalidStandaloneStatementView,
        render: render_invalid_standalone_statement,
    }
}

diagnostic_family! {
    INVALID_MATCH_ARM {
        internal: 1028,
        stable: "MOTH-SYNTAX-0028",
        category: DiagnosticCategory::Syntax,
        severity: DiagnosticSeverity::Error,
        title: "Invalid match arm",
        facts: [reason: UnitFact<InvalidMatchArmReason>],
        labels: [],
        extra: None,
        ctor: generated invalid_match_arm,
        reason_key: reason,
        view: InvalidMatchArmView,
        render: render_invalid_match_arm,
    }
}

diagnostic_family! {
    INVALID_LOOP_HEADER {
        internal: 1029,
        stable: "MOTH-SYNTAX-0029",
        category: DiagnosticCategory::Syntax,
        severity: DiagnosticSeverity::Error,
        title: "Invalid loop header",
        facts: [
            reason: UnitFact<InvalidLoopHeaderCode>,
            found_type: OptTypeFact,
        ],
        labels: [],
        extra: None,
        ctor: manual,
        reason_key: reason,
        view: InvalidLoopHeaderView,
        render: render_invalid_loop_header,
    }
}

diagnostic_family! {
    INVALID_STATEMENT_POSITION {
        internal: 1030,
        stable: "MOTH-SYNTAX-0030",
        category: DiagnosticCategory::Syntax,
        severity: DiagnosticSeverity::Error,
        title: "Invalid statement position",
        facts: [reason: UnitFact<InvalidStatementPositionReason>],
        labels: [],
        extra: None,
        ctor: generated invalid_statement_position,
        reason_key: reason,
        view: InvalidStatementPositionView,
        render: render_invalid_statement_position,
    }
}

diagnostic_family! {
    INVALID_CHOICE_VARIANT {
        internal: 3029,
        stable: "MOTH-RULE-0029",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "Invalid choice variant",
        facts: [
            reason: UnitFact<InvalidChoiceVariantReason>,
            choice_name: OptStringFact,
            variant_name: OptStringFact,
        ],
        labels: [],
        extra: StringIds,
        ctor: manual,
        reason_key: reason,
        view: InvalidChoiceVariantView,
        render: render_invalid_choice_variant,
    }
}

diagnostic_family! {
    INVALID_CONTROL_FLOW_STATEMENT {
        internal: 3042,
        stable: "MOTH-RULE-0042",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "Invalid control flow statement",
        facts: [reason: UnitFact<InvalidControlFlowStatementReason>],
        labels: [],
        extra: None,
        ctor: generated invalid_control_flow_statement,
        reason_key: reason,
        view: InvalidControlFlowStatementView,
        render: render_invalid_control_flow_statement,
    }
}

diagnostic_family! {
    INVALID_MATCH_PATTERN {
        internal: 3049,
        stable: "MOTH-RULE-0049",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "Invalid match pattern",
        facts: [
            reason: UnitFact<InvalidMatchPatternReason>,
            variant_name: OptStringFact,
            scrutinee_name: OptStringFact,
        ],
        labels: [],
        extra: None,
        ctor: generated invalid_match_pattern,
        reason_key: reason,
        view: InvalidMatchPatternView,
        render: render_invalid_match_pattern,
    }
}

diagnostic_family! {
    INVALID_MATCH_GUARD {
        internal: 3088,
        stable: "MOTH-RULE-0088",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "Invalid match guard",
        facts: [reason: UnitFact<InvalidMatchGuardReason>],
        labels: [],
        extra: None,
        ctor: generated invalid_match_guard,
        reason_key: reason,
        view: InvalidMatchGuardView,
        render: render_invalid_match_guard,
    }
}

diagnostic_family! {
    INVALID_FALLIBLE_HANDLING {
        internal: 3051,
        stable: "MOTH-RULE-0051",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "Invalid fallible handling",
        facts: [
            reason: UnitFact<InvalidFallibleHandlingCode>,
            first_error_type: OptTypeFact,
            second_error_type: OptTypeFact,
            count: OptCountFact,
        ],
        labels: [
            LabelMessageCode::TYPED_FAILURE_PRODUCER,
            LabelMessageCode::IMPLICIT_FAILURE_PRODUCER,
            LabelMessageCode::BUILTIN_FAILURE_CALL,
            LabelMessageCode::BUILTIN_FAILURE_ORIGIN,
            LabelMessageCode::BUILTIN_WRITE_BACK_ORIGIN,
        ],
        extra: None,
        ctor: manual,
        reason_key: reason,
        view: InvalidFallibleHandlingView,
        render: render_invalid_fallible_handling,
    }
}

diagnostic_family! {
    COMPILE_TIME_EVALUATION_ERROR {
        internal: 3053,
        stable: "MOTH-RULE-0053",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "Compile-time evaluation error",
        facts: [
            reason: UnitFact<CompileTimeEvaluationErrorReason>,
            operation: OptStringFact,
        ],
        labels: [LabelMessageCode::CONFIG_INPUT_ORIGIN],
        extra: None,
        ctor: generated compile_time_evaluation_error,
        reason_key: reason,
        view: CompileTimeEvaluationErrorView,
        render: render_compile_time_evaluation_error,
    }
}

pub(in crate::compiler_frontend::compiler_messages::diagnostics) const FAMILIES: &[FamilyRow] = &[
    UNREACHABLE_MATCH_ARM,
    NON_EXHAUSTIVE_MATCH,
    INVALID_STANDALONE_STATEMENT,
    INVALID_MATCH_ARM,
    INVALID_LOOP_HEADER,
    INVALID_STATEMENT_POSITION,
    INVALID_CHOICE_VARIANT,
    INVALID_CONTROL_FLOW_STATEMENT,
    INVALID_MATCH_PATTERN,
    INVALID_FALLIBLE_HANDLING,
    INVALID_MATCH_GUARD,
    COMPILE_TIME_EVALUATION_ERROR,
];
