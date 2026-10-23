//! Compiler message models and render-boundary helpers.
//!
//! WHAT: owns typed user-facing diagnostics, internal/tooling error transport, exact source spans,
//! stage-local diagnostic bags, boundary aggregation, and final renderers.
//! WHY: compiler stages should exchange structured facts while CLI, dev-server, test, and tool
//! boundaries decide how those facts become user-visible text.
//!
//! `CompilerDiagnostic` is the normal source/config/import/type/rule/borrow diagnostic path.
//! `DiagnosticBag` accumulates stage-local diagnostics and warnings in production order.
//! `PreparedFailure` is the owner-edge capture that
//! pairs diagnostics with the `StringTable` and optional type render context needed for prose.
//! `CompilerError` is reserved for internal compiler, filesystem, backend, and dev-server
//! infrastructure failures.

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
pub(crate) mod diagnostic_token;
pub(crate) mod diagnostic_types;
pub(crate) mod diagnostics;
pub(crate) mod display_messages;
pub(crate) mod prepared_batch;
pub(crate) mod render;
pub(crate) mod trait_keyword_diagnostics;

pub(crate) use crate::compiler_frontend::tokenizer::tokens::TokenDescriptorPayload;
#[cfg(test)]
pub(crate) use crate::compiler_frontend::tokenizer::tokens::TokenTag;
pub(crate) use compiler_diagnostic::CompilerDiagnostic;
pub(crate) use diagnostic_bag::{DiagnosticBag, PreparedFailure};
pub(crate) use diagnostic_descriptor::DiagnosticDescriptor;
#[cfg(test)]
pub(crate) use diagnostic_identity::DiagnosticIdentity;
pub(crate) use diagnostic_identity::is_well_formed_reason_key;
pub(crate) use diagnostic_kind::{
    BorrowDiagnosticKind, ConfigDiagnosticKind, DeferredFeatureDiagnosticKind, DiagnosticCategory,
    DiagnosticKind, ImportDiagnosticKind, InfrastructureDiagnosticKind, RuleDiagnosticKind,
    SyntaxDiagnosticKind, TypeDiagnosticKind,
};
pub(crate) use diagnostic_label::{DiagnosticLabel, DiagnosticLabelMessage, DiagnosticLabelStyle};
pub(crate) use diagnostic_payload::{
    AstCapacityResource, BorrowAccessKind, BuiltinFailureOriginKind, BuiltinFailureWitness,
    CapacityResource, ChoiceVariantSeparatorGap, CommonSyntaxMistakeReason,
    CompileTimeEvaluationErrorReason, CssTemplateWarning, DeferredFeatureReason,
    DependencyClauseKind, DiagnosticCapacityResource, DiagnosticCompoundAssignmentOperator,
    DiagnosticOperator, DiagnosticPayload, DiagnosticPlace, GenericApplicationErrorReason,
    GenericInferenceSubject, HirCapacityResource, HtmlTemplateWarning, ImportPublicSurfaceType,
    IncompatibleChoiceComparisonReason, InvalidAssignmentTargetReason, InvalidBuiltinCallReason,
    InvalidCallShapeReason, InvalidCastReason, InvalidChoiceVariantReason,
    InvalidCollectionTypeReason, InvalidCompileTimePathReason, InvalidConfigReason,
    InvalidControlFlowStatementReason, InvalidCopyTargetReason, InvalidDeclarationReason,
    InvalidDependencyClauseReason, InvalidExpressionReason, InvalidExternalModuleReason,
    InvalidFallibleHandlingReason, InvalidFieldAccessReason, InvalidFunctionSignatureReason,
    InvalidGenericInstantiationReason, InvalidGenericParameterReason, InvalidImportPathReason,
    InvalidLoopHeaderReason, InvalidMapLiteralReason, InvalidMapTypeReason, InvalidMatchArmReason,
    InvalidMatchPatternReason, InvalidMultiBindReason, InvalidMutableAccessReason,
    InvalidOutputFolderReason, InvalidPageMetadataReason, InvalidParameterContractReason,
    InvalidRangeOperandReason, InvalidReceiverCallReason, InvalidReceiverDeclarationReason,
    InvalidReturnShapeReason, InvalidRouteOutputReason, InvalidSignatureMemberReason,
    InvalidStandaloneStatementReason, InvalidStatementPositionReason, InvalidStringEscapeReason,
    InvalidTemplateDirectiveReason, InvalidTemplateSlotReason, InvalidTemplateStructureReason,
    InvalidThisUsageReason, InvalidTraitConformanceReason, InvalidTraitIncompatibilityReason,
    InvalidTraitKeywordUsageReason, InvalidTypeAnnotationReason, LegacyDependencyClauseReason,
    MalformedTemplateReason, MissingWhitespace, NameNamespace, NamespaceTypeValueMisuseKind,
    NamingConvention, NonExhaustiveMatchReason, OperatorOperandPosition, ParameterContractKind,
    PathKind, ProjectContextEscapeReason, PublicGenericOwnerKind, RangeOperandKind,
    ReceiverCallKind, ReservedNameOwner, SourceSpanCapacityResource, SymbolicSpacingConstruct,
    SymbolicSpacingError, TypeAnnotationContext, TypeMismatchContext,
    UnsupportedBackendFeatureReason, ValueReceiverContext,
};
pub(crate) use diagnostic_severity::DiagnosticSeverity;
pub(crate) use diagnostic_token::DiagnosticToken;
pub(crate) use diagnostics::draft::DiagnosticDraft;
pub(crate) use diagnostics::labels::GenericSubstitutionDiagnostic;

#[cfg(test)]
#[path = "tests/diagnostic_model_tests.rs"]
mod diagnostic_model_tests;
#[cfg(test)]
#[path = "tests/diagnostic_token_projection_tests.rs"]
mod diagnostic_token_projection_tests;

#[cfg(test)]
#[path = "tests/prepared_batch_tests.rs"]
mod prepared_batch_tests;

#[cfg(test)]
#[path = "tests/type_rendering_tests.rs"]
mod type_rendering_tests;

#[cfg(test)]
#[path = "tests/boundary_failure_tests.rs"]
mod boundary_failure_tests;
