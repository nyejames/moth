//! Family rows for type, borrow and target diagnostics.
//!
//! WHAT: family rows for type, borrow and target diagnostics, including reserved families.
//! Each row declares its facts and applicable label and renderer metadata.
//! WHY: type and borrow checks report over the same type and place facts, so their rows share one group.

use super::super::schema::DiagnosticCategory;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::facts::{
    OptPlaceFact, OptStringFact, OptTypeFact, PlaceFact, StringFact, TypeFact, UnitFact, unit_code,
};
use crate::compiler_frontend::compiler_messages::diagnostics::labels::{
    LabelDraftMessage, LabelMessageCode,
};
use crate::compiler_frontend::compiler_messages::diagnostics::schema::{
    FamilyRow, diagnostic_family,
};
use crate::compiler_frontend::compiler_messages::render::{
    render_empty_collection_type_ambiguity, render_incompatible_choice_comparison,
    render_invalid_mutable_access, render_invalid_route_output, render_multiple_mutable_borrows,
    render_shared_mutable_conflict, render_type_mismatch, render_unsupported_backend_feature,
    render_unsupported_external_function, render_unsupported_operator_types,
    render_use_of_uninitialized_local,
};
use crate::compiler_frontend::compiler_messages::{
    BorrowAccessKind, DiagnosticOperator, DiagnosticPlace, DiagnosticSeverity,
    IncompatibleChoiceComparisonReason, InvalidMutableAccessReason, InvalidRouteOutputReason,
    TypeMismatchContext, UnsupportedBackendFeatureReason,
};
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::string_interning::StringId;

unit_code! {
    impl TypeMismatchContext {
        Assignment = 1,
        Declaration = 2,
        ReturnValue = 3,
        FunctionArgument = 4,
        AssertionArgument = 5,
        ConstructorArgument = 6,
        ReceiverArgument = 7,
        Operator = 8,
        Condition = 9,
        CollectionElement = 10,
        StructFieldDefault = 11,
        TemplateInterpolation = 12,
        MatchScrutinee = 13,
        MatchPattern = 14,
        ErrorReturn = 15,
        Pattern = 16,
        General = 17,
    }
}

unit_code! {
    impl DiagnosticOperator {
        Add = 1,
        Subtract = 2,
        Multiply = 3,
        Divide = 4,
        IntDivide = 5,
        Modulus = 6,
        Exponent = 7,
        And = 8,
        Or = 9,
        GreaterThan = 10,
        GreaterThanOrEqual = 11,
        LessThan = 12,
        LessThanOrEqual = 13,
        Equality = 14,
        NotEqual = 15,
        Not = 16,
        Range = 17,
    }
}

unit_code! {
    impl BorrowAccessKind {
        Shared = 1,
        Mutable = 2,
        Move = 3,
    }
}

unit_code! {
    impl InvalidMutableAccessReason {
        ImmutablePlace = 1,
        OverlappingAccess = 2,
        AliasedValueRequiresExclusiveAccess = 3,
    }
}

unit_code! {
    impl InvalidRouteOutputReason {
        NotChoiceOrGeneric = 1,
    }
}

unit_code! {
    impl UnsupportedBackendFeatureReason {
        ConstructorRoutes = 1,
        HashmapConstruction = 2,
        HashmapOperation = 3,
        RuntimeCasts = 4,
        CheckedNumericOperations = 5,
        RecoverableNumericFailure = 6,
        FloatFormatting = 7,
        FloatBoundaryValidation = 8,
        GenericRuntimeValues = 9,
        FixedWidthScalarValues = 10,
        CrossModuleCalls = 11,
        RuntimeAssertionMessages = 12,
        ErrorValues = 13,
        FallibleControlFlow = 14,
        MutableFunctionParameters = 15,
        MutableLocalBindingAliases = 16,
        NumberValues = 17,
        VariantPayloadValues = 18,
        UnaryOperators = 19,
        NonEmptyCollections = 20,
        StructRangeVariantPayloadExpressions = 21,
        TemplateChunkTypes = 22,
        ProjectedPlaces = 23,
        LinkedFunctionTargets = 24,
        MatchTerminators = 25,
        ProjectedAssignmentTargets = 26,
    }
}

unit_code! {
    /// The reason code of a choice comparison; only the payload-equality code carries a field.
    pub(crate) enum IncompatibleChoiceComparisonCode {
        DifferentChoiceTypes = 1,
        ChoiceWithNonChoice = 2,
        PayloadEqualityNotSupported = 3,
    }
}

impl IncompatibleChoiceComparisonCode {
    /// Splits a reason into its code, optional field name and optional field type.
    pub(crate) fn split(
        reason: &IncompatibleChoiceComparisonReason,
    ) -> (Self, Option<StringId>, Option<TypeId>) {
        match reason {
            IncompatibleChoiceComparisonReason::DifferentChoiceTypes => {
                (Self::DifferentChoiceTypes, None, None)
            }
            IncompatibleChoiceComparisonReason::ChoiceWithNonChoice => {
                (Self::ChoiceWithNonChoice, None, None)
            }
            IncompatibleChoiceComparisonReason::PayloadEqualityNotSupported {
                field_name,
                field_type,
            } => (
                Self::PayloadEqualityNotSupported,
                Some(*field_name),
                Some(*field_type),
            ),
        }
    }
}

diagnostic_family! {
    TYPE_MISMATCH {
        internal: 2001,
        stable: "MOTH-TYPE-0001",
        category: DiagnosticCategory::Type,
        severity: DiagnosticSeverity::Error,
        title: "Type mismatch",
        facts: [expected: TypeFact, found: TypeFact, context: UnitFact<TypeMismatchContext>],
        labels: [],
        extra: None,
        ctor: generated type_mismatch,
        view: TypeMismatchView,
        render: render_type_mismatch,
    }
}

diagnostic_family! {
    USE_OF_UNINITIALIZED_LOCAL {
        internal: 5009,
        stable: "MOTH-BORROW-0009",
        category: DiagnosticCategory::Borrow,
        severity: DiagnosticSeverity::Error,
        title: "Use of uninitialized local",
        facts: [place: PlaceFact],
        labels: [],
        extra: None,
        ctor: manual,
        view: UseOfUninitializedLocalView,
        render: render_use_of_uninitialized_local,
    }
}

/// The uninitialised-local draft stores the place in its compact place fact.
pub(crate) fn use_of_uninitialized_local(
    place: DiagnosticPlace,
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    DiagnosticDraft::with_facts(&USE_OF_UNINITIALIZED_LOCAL, span, |cursor| {
        cursor.push::<PlaceFact>(&place)
    })
}

pub(crate) fn incompatible_choice_comparison(
    reason: IncompatibleChoiceComparisonReason,
    lhs: TypeId,
    rhs: TypeId,
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    let (code, field_name, field_type) = IncompatibleChoiceComparisonCode::split(&reason);
    DiagnosticDraft::with_facts(&INCOMPATIBLE_CHOICE_COMPARISON, span, |cursor| {
        cursor.push::<UnitFact<IncompatibleChoiceComparisonCode>>(&code)?;
        cursor.push::<TypeFact>(&lhs)?;
        cursor.push::<TypeFact>(&rhs)?;
        cursor.push::<OptTypeFact>(&field_type)
    })
    .with_string_ids(field_name.into_iter().collect())
}

/// Decodes the code-determined field-name list for a choice-comparison reason.
pub(crate) fn incompatible_choice_comparison_field_name(
    code: IncompatibleChoiceComparisonCode,
    string_ids: &[StringId],
) -> Option<StringId> {
    match (code, string_ids) {
        (IncompatibleChoiceComparisonCode::PayloadEqualityNotSupported, [field_name]) => {
            Some(*field_name)
        }
        (
            IncompatibleChoiceComparisonCode::DifferentChoiceTypes
            | IncompatibleChoiceComparisonCode::ChoiceWithNonChoice,
            [],
        ) => None,
        _ => crate::compiler_bug!(
            CompilerStage::DiagnosticReporting,
            "choice-comparison code fixes its field-name list"
        ),
    }
}

pub(crate) fn multiple_mutable_borrows(
    place: DiagnosticPlace,
    conflicting_place: Option<DiagnosticPlace>,
    existing_span: Option<SourceSpan>,
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    let draft = DiagnosticDraft::with_facts(&MULTIPLE_MUTABLE_BORROWS, span, |cursor| {
        cursor.push::<PlaceFact>(&place)?;
        cursor.push::<OptPlaceFact>(&conflicting_place)
    });
    with_conflicting_access_label(draft, existing_span)
}

pub(crate) fn shared_mutable_conflict(
    place: DiagnosticPlace,
    existing_access: BorrowAccessKind,
    requested_access: BorrowAccessKind,
    conflicting_place: Option<DiagnosticPlace>,
    existing_span: Option<SourceSpan>,
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    let draft = DiagnosticDraft::with_facts(&SHARED_MUTABLE_CONFLICT, span, |cursor| {
        cursor.push::<PlaceFact>(&place)?;
        cursor.push::<UnitFact<BorrowAccessKind>>(&existing_access)?;
        cursor.push::<UnitFact<BorrowAccessKind>>(&requested_access)?;
        cursor.push::<OptPlaceFact>(&conflicting_place)
    });
    with_conflicting_access_label(draft, existing_span)
}

pub(crate) fn invalid_mutable_access(
    place: DiagnosticPlace,
    reason: InvalidMutableAccessReason,
    conflicting_place: Option<DiagnosticPlace>,
    conflicting_span: Option<SourceSpan>,
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    let draft = DiagnosticDraft::with_facts(&INVALID_MUTABLE_ACCESS, span, |cursor| {
        cursor.push::<PlaceFact>(&place)?;
        cursor.push::<UnitFact<InvalidMutableAccessReason>>(&reason)?;
        cursor.push::<OptPlaceFact>(&conflicting_place)
    });
    with_conflicting_access_label(draft, conflicting_span)
}

fn with_conflicting_access_label(
    draft: DiagnosticDraft,
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    match span {
        Some(span) => draft.with_label(Some(span), Some(LabelDraftMessage::ConflictingAccess)),
        None => draft,
    }
}

diagnostic_family! {
    EMPTY_COLLECTION_TYPE_AMBIGUITY {
        internal: 2002,
        stable: "MOTH-TYPE-0002",
        category: DiagnosticCategory::Type,
        severity: DiagnosticSeverity::Error,
        title: "Empty collection type ambiguity",
        facts: [],
        labels: [],
        extra: None,
        ctor: generated empty_collection_type_ambiguity,
        view: EmptyCollectionTypeAmbiguityView,
        render: render_empty_collection_type_ambiguity,
    }
}

diagnostic_family! {
    UNSUPPORTED_OPERATOR_TYPES {
        internal: 2003,
        stable: "MOTH-TYPE-0003",
        category: DiagnosticCategory::Type,
        severity: DiagnosticSeverity::Error,
        title: "Unsupported operator types",
        facts: [
            operator: UnitFact<DiagnosticOperator>,
            lhs: TypeFact,
            rhs: OptTypeFact,
        ],
        labels: [],
        extra: None,
        ctor: generated unsupported_operator_types,
        view: UnsupportedOperatorTypesView,
        render: render_unsupported_operator_types,
    }
}

diagnostic_family! {
    INCOMPATIBLE_CHOICE_COMPARISON {
        internal: 2005,
        stable: "MOTH-TYPE-0005",
        category: DiagnosticCategory::Type,
        severity: DiagnosticSeverity::Error,
        title: "Incompatible choice comparison",
        facts: [
            reason: UnitFact<IncompatibleChoiceComparisonCode>,
            lhs: TypeFact,
            rhs: TypeFact,
            field_type: OptTypeFact,
        ],
        labels: [],
        extra: StringIds,
        ctor: manual,
        reason_key: reason,
        view: IncompatibleChoiceComparisonView,
        render: render_incompatible_choice_comparison,
    }
}

diagnostic_family! {
    INVALID_ROUTE_OUTPUT {
        internal: 2006,
        stable: "MOTH-TYPE-0006",
        category: DiagnosticCategory::Type,
        severity: DiagnosticSeverity::Error,
        title: "Invalid route output type",
        facts: [
            reason: UnitFact<InvalidRouteOutputReason>,
            output: TypeFact,
        ],
        labels: [],
        extra: None,
        ctor: generated invalid_route_output,
        reason_key: reason,
        view: InvalidRouteOutputView,
        render: render_invalid_route_output,
    }
}

diagnostic_family! {
    UNSUPPORTED_EXTERNAL_FUNCTION {
        internal: 3058,
        stable: "MOTH-RULE-0058",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "Unsupported external function",
        facts: [
            function_name: StringFact,
            package_path: OptStringFact,
            backend_name: StringFact,
        ],
        labels: [],
        extra: None,
        ctor: generated unsupported_external_function,
        view: UnsupportedExternalFunctionView,
        render: render_unsupported_external_function,
    }
}

diagnostic_family! {
    UNSUPPORTED_BACKEND_FEATURE {
        internal: 3064,
        stable: "MOTH-RULE-0064",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "Unsupported backend feature",
        facts: [
            backend_name: StringFact,
            reason: UnitFact<UnsupportedBackendFeatureReason>,
        ],
        labels: [],
        extra: None,
        ctor: generated unsupported_backend_feature,
        reason_key: reason,
        view: UnsupportedBackendFeatureView,
        render: render_unsupported_backend_feature,
    }
}

diagnostic_family! {
    BORROW_CONFLICT {
        internal: 5001,
        stable: "MOTH-BORROW-0001",
        category: DiagnosticCategory::Borrow,
        severity: DiagnosticSeverity::Error,
        title: "Access conflict",
        facts: [
            place: PlaceFact,
            existing_access: UnitFact<BorrowAccessKind>,
            requested_access: UnitFact<BorrowAccessKind>,
        ],
        labels: [],
        extra: None,
        ctor: manual,
        #[allow(dead_code, reason = "reserved: no production producer or renderer uses this view")]
        view: BorrowConflictView,
        render: none,
        reserved,
    }
}

diagnostic_family! {
    MULTIPLE_MUTABLE_BORROWS {
        internal: 5002,
        stable: "MOTH-BORROW-0002",
        category: DiagnosticCategory::Borrow,
        severity: DiagnosticSeverity::Error,
        title: "Conflicting mutable access",
        facts: [place: PlaceFact, conflicting_place: OptPlaceFact],
        labels: [LabelMessageCode::CONFLICTING_ACCESS],
        extra: None,
        ctor: manual,
        view: MultipleMutableBorrowsView,
        render: render_multiple_mutable_borrows,
    }
}

diagnostic_family! {
    SHARED_MUTABLE_CONFLICT {
        internal: 5003,
        stable: "MOTH-BORROW-0003",
        category: DiagnosticCategory::Borrow,
        severity: DiagnosticSeverity::Error,
        title: "Shared and mutable access conflict",
        facts: [
            place: PlaceFact,
            existing_access: UnitFact<BorrowAccessKind>,
            requested_access: UnitFact<BorrowAccessKind>,
            conflicting_place: OptPlaceFact,
        ],
        labels: [LabelMessageCode::CONFLICTING_ACCESS],
        extra: None,
        ctor: manual,
        view: SharedMutableConflictView,
        render: render_shared_mutable_conflict,
    }
}

diagnostic_family! {
    USE_AFTER_POSSIBLE_MOVE {
        internal: 5004,
        stable: "MOTH-BORROW-0004",
        category: DiagnosticCategory::Borrow,
        severity: DiagnosticSeverity::Error,
        title: "Use after possible move",
        facts: [place: PlaceFact],
        labels: [],
        extra: None,
        ctor: manual,
        #[allow(dead_code, reason = "reserved: no production producer or renderer uses this view")]
        view: UseAfterPossibleMoveView,
        render: none,
        reserved,
    }
}

diagnostic_family! {
    MOVE_WHILE_BORROWED {
        internal: 5005,
        stable: "MOTH-BORROW-0005",
        category: DiagnosticCategory::Borrow,
        severity: DiagnosticSeverity::Error,
        title: "Ownership transfer conflicts with active access",
        facts: [
            place: PlaceFact,
            existing_access: UnitFact<BorrowAccessKind>,
        ],
        labels: [],
        extra: None,
        ctor: manual,
        #[allow(dead_code, reason = "reserved: no production producer or renderer uses this view")]
        view: MoveWhileBorrowedView,
        render: none,
        reserved,
    }
}

diagnostic_family! {
    WHOLE_OBJECT_BORROW_CONFLICT {
        internal: 5006,
        stable: "MOTH-BORROW-0006",
        category: DiagnosticCategory::Borrow,
        severity: DiagnosticSeverity::Error,
        title: "Whole-value access conflict",
        facts: [whole_place: PlaceFact, part_place: PlaceFact],
        labels: [],
        extra: None,
        ctor: manual,
        #[allow(dead_code, reason = "reserved: no production producer or renderer uses this view")]
        view: WholeObjectBorrowConflictView,
        render: none,
        reserved,
    }
}

diagnostic_family! {
    INVALID_MUTABLE_ACCESS {
        internal: 5007,
        stable: "MOTH-BORROW-0007",
        category: DiagnosticCategory::Borrow,
        severity: DiagnosticSeverity::Error,
        title: "Invalid mutable access",
        facts: [
            place: PlaceFact,
            reason: UnitFact<InvalidMutableAccessReason>,
            conflicting_place: OptPlaceFact,
        ],
        labels: [LabelMessageCode::CONFLICTING_ACCESS],
        extra: None,
        ctor: manual,
        reason_key: reason,
        view: InvalidMutableAccessView,
        render: render_invalid_mutable_access,
    }
}

pub(in crate::compiler_frontend::compiler_messages::diagnostics) const FAMILIES: &[FamilyRow] = &[
    TYPE_MISMATCH,
    USE_OF_UNINITIALIZED_LOCAL,
    EMPTY_COLLECTION_TYPE_AMBIGUITY,
    UNSUPPORTED_OPERATOR_TYPES,
    INCOMPATIBLE_CHOICE_COMPARISON,
    INVALID_ROUTE_OUTPUT,
    UNSUPPORTED_EXTERNAL_FUNCTION,
    UNSUPPORTED_BACKEND_FEATURE,
    BORROW_CONFLICT,
    MULTIPLE_MUTABLE_BORROWS,
    SHARED_MUTABLE_CONFLICT,
    USE_AFTER_POSSIBLE_MOVE,
    MOVE_WHILE_BORROWED,
    WHOLE_OBJECT_BORROW_CONFLICT,
    INVALID_MUTABLE_ACCESS,
];
