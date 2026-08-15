//! Compiler message models and render-boundary helpers.
//!
//! WHAT: owns typed user-facing diagnostics, internal/tooling error transport, source locations,
//! stage-local diagnostic bags, boundary aggregation, and final renderers.
//! WHY: compiler stages should exchange structured facts while CLI, dev-server, test, and tool
//! boundaries decide how those facts become user-visible text.
//!
//! `CompilerDiagnostic` is the normal source/config/import/type/rule/borrow diagnostic path.
//! `CompilerMessages` is the ordered boundary container that carries diagnostics with the
//! `StringTable` and optional type render context needed for prose. `CompilerError` is reserved
//! for internal compiler, filesystem, backend, and dev-server infrastructure failures.

pub(crate) mod compiler_dev_logging;
pub(crate) mod compiler_diagnostic;
pub(crate) mod compiler_errors;
pub(crate) mod deferred_feature_diagnostics;
pub(crate) mod diagnostic_bag;
pub(crate) mod diagnostic_descriptor;
pub(crate) mod diagnostic_identity;
pub(crate) mod diagnostic_kind;
mod diagnostic_kind_descriptors;
pub(crate) mod diagnostic_label;
pub(crate) mod diagnostic_payload;
pub(crate) mod diagnostic_severity;
pub(crate) mod display_messages;
pub(crate) mod module_diagnostics;
pub(crate) mod render;
pub(crate) mod source_location;
pub(crate) mod trait_keyword_diagnostics;

pub(crate) use compiler_diagnostic::CompilerDiagnostic;
pub(crate) use diagnostic_bag::DiagnosticBag;
pub(crate) use diagnostic_descriptor::DiagnosticDescriptor;
pub(crate) use diagnostic_identity::{DiagnosticIdentity, is_well_formed_reason_key};
pub(crate) use diagnostic_kind::{
    BorrowDiagnosticKind, ConfigDiagnosticKind, DeferredFeatureDiagnosticKind, DiagnosticCategory,
    DiagnosticKind, ImportDiagnosticKind, InfrastructureDiagnosticKind, RuleDiagnosticKind,
    SyntaxDiagnosticKind, TypeDiagnosticKind,
};
pub(crate) use diagnostic_label::{
    DiagnosticLabel, DiagnosticLabelMessage, DiagnosticLabelStyle, GenericSubstitutionDiagnostic,
};
pub(crate) use diagnostic_payload::{
    BorrowAccessKind, CommonSyntaxMistakeReason, CompileTimeEvaluationErrorReason,
    DeferredFeatureReason, DependencyClauseKind, DiagnosticCompoundAssignmentOperator,
    DiagnosticOperator, DiagnosticPayload, DiagnosticPlace, GenericApplicationErrorReason,
    GenericInferenceSubject, ImportPublicSurfaceType, IncompatibleChoiceComparisonReason,
    InvalidAssignmentTargetReason, InvalidBuiltinCallReason, InvalidCallShapeReason,
    InvalidCastReason, InvalidChoiceVariantReason, InvalidCollectionTypeReason,
    InvalidCompileTimePathReason, InvalidConfigReason, InvalidControlFlowStatementReason,
    InvalidCopyTargetReason, InvalidDeclarationReason, InvalidDependencyClauseReason,
    InvalidExpressionReason, InvalidFallibleHandlingReason, InvalidFallibleOperandReason,
    InvalidFieldAccessReason, InvalidFunctionSignatureReason, InvalidGenericInstantiationReason,
    InvalidGenericParameterReason, InvalidImportPathReason, InvalidLoopHeaderReason,
    InvalidMapLiteralReason, InvalidMapTypeReason, InvalidMatchArmReason,
    InvalidMatchPatternReason, InvalidMultiBindReason, InvalidMutableAccessReason,
    InvalidOutputFolderReason, InvalidPackageFolderReason, InvalidPageMetadataReason,
    InvalidReceiverCallReason, InvalidReceiverDeclarationReason, InvalidReturnShapeReason,
    InvalidSignatureMemberReason, InvalidStandaloneStatementReason, InvalidStatementPositionReason,
    InvalidStringEscapeReason, InvalidTemplateDirectiveReason, InvalidTemplateSlotReason,
    InvalidTemplateStructureReason, InvalidThisUsageReason, InvalidTraitConformanceReason,
    InvalidTraitIncompatibilityReason, InvalidTraitKeywordUsageReason, InvalidTypeAnnotationReason,
    LegacyDependencyClauseReason, MissingWhitespace, NameNamespace, NamespaceTypeValueMisuseKind,
    NamingConvention, NonExhaustiveMatchReason, NumberLiteralErrorReason, OperatorOperandPosition,
    PathKind, RangeOperandKind, ReceiverCallKind, ReservedNameOwner, SymbolicSpacingConstruct,
    SymbolicSpacingError, TypeAnnotationContext, TypeMismatchContext,
    UnsupportedBackendFeatureReason, UnsupportedOperatorCategory,
};
pub(crate) use diagnostic_severity::DiagnosticSeverity;
pub(crate) use module_diagnostics::ModuleDiagnostics;

#[cfg(test)]
#[path = "tests/diagnostic_model_tests.rs"]
mod diagnostic_model_tests;

#[cfg(test)]
#[path = "tests/module_diagnostics_tests.rs"]
mod module_diagnostics_tests;

#[cfg(test)]
#[path = "tests/type_rendering_tests.rs"]
mod type_rendering_tests;
