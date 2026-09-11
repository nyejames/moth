//! Diagnostic kind taxonomy and stable descriptor mapping.
//!
//! WHAT: groups diagnostics by compiler domain and derives category, code, title, and default
//! severity from the kind.
//! WHY: categories should not be stored redundantly on diagnostics; the kind is the source of
//! truth for grouping and render metadata.

use crate::compiler_frontend::compiler_messages::{DiagnosticDescriptor, DiagnosticSeverity};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DiagnosticKind {
    Syntax(SyntaxDiagnosticKind),
    Type(TypeDiagnosticKind),
    Rule(RuleDiagnosticKind),
    Import(ImportDiagnosticKind),
    Borrow(BorrowDiagnosticKind),
    Config(ConfigDiagnosticKind),
    Infrastructure(InfrastructureDiagnosticKind),
    DeferredFeature(DeferredFeatureDiagnosticKind),
}

impl DiagnosticKind {
    pub(crate) fn descriptor(self) -> DiagnosticDescriptor {
        super::diagnostic_kind_descriptors::descriptor_for_kind(self)
    }

    pub(crate) fn category(self) -> DiagnosticCategory {
        match self {
            DiagnosticKind::Syntax(_) => DiagnosticCategory::Syntax,
            DiagnosticKind::Type(_) => DiagnosticCategory::Type,
            DiagnosticKind::Rule(_) => DiagnosticCategory::Rule,
            DiagnosticKind::Import(_) => DiagnosticCategory::Import,
            DiagnosticKind::Borrow(_) => DiagnosticCategory::Borrow,
            DiagnosticKind::Config(_) => DiagnosticCategory::Config,
            DiagnosticKind::Infrastructure(_) => DiagnosticCategory::Infrastructure,
            DiagnosticKind::DeferredFeature(_) => DiagnosticCategory::DeferredFeature,
        }
    }

    pub(crate) fn code(self) -> &'static str {
        self.descriptor().code
    }

    pub(crate) fn default_severity(self) -> DiagnosticSeverity {
        self.descriptor().default_severity
    }
}

// -------------------------
//  Diagnostic Kind Enums
// -------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum DiagnosticCategory {
    Syntax,
    Type,
    Rule,
    Import,
    Borrow,
    Config,
    Infrastructure,
    DeferredFeature,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SyntaxDiagnosticKind {
    ExpectedToken,
    UnexpectedToken,
    UnexpectedTrailingComma,
    MalformedCssTemplate,
    MalformedHtmlTemplate,
    UnterminatedStringLiteral,
    InvalidCharacter,
    InvalidNumberLiteral,
    InvalidCharLiteral,
    InvalidStyleDirective,
    InvalidIdentifier,
    MissingClosingDelimiter,
    UnexpectedTokenInDeclaration,
    InvalidTypeAnnotation,
    InvalidGenericApplication,
    InvalidCollectionType,
    InvalidMapType,
    InvalidMapLiteral,
    UnexpectedEndOfFile,
    InvalidPath,
    InvalidDependencyClause,
    LegacyDependencyClause,
    InvalidGenericParameter,
    InvalidTemplateDirective,
    InvalidTemplateStructure,
    InvalidExpression,
    MissingOperatorOperand,
    InvalidStandaloneStatement,
    ExpectedSymbolStatement,
    MissingCollectionItem,
    InvalidMatchArm,
    InvalidLoopHeader,
    InvalidStatementPosition,
    CommonSyntaxMistake,
    UnescapedImplicitTemplateClose,
    InvalidStringEscape,
    SourceSpanCapacity,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TypeDiagnosticKind {
    TypeMismatch,
    EmptyCollectionTypeAmbiguity,
    UnsupportedOperatorTypes,
    InvalidFallibleOperand,
    IncompatibleChoiceComparison,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RuleDiagnosticKind {
    UnknownName,
    DuplicateDeclaration,
    IdentifierNamingConvention,
    UnreachableMatchArm,
    InvalidTopLevelRuntimeStatement,
    ReservedBuiltinName,
    InvalidSignatureMember,
    InvalidChoiceVariant,
    InvalidStructDefaultValue,
    MissingDeclarationInitializer,
    CircularDependency,
    UnknownValueName,
    UnknownTypeName,
    ValueUsedAsType,
    TypeUsedAsValue,
    ShadowedName,
    ReservedNameCollision,
    InvalidThisUsage,
    InvalidReceiverDeclaration,
    InvalidControlFlowStatement,
    InvalidDeclaration,
    InvalidAssignmentTarget,
    InvalidMultiBind,
    InvalidBuiltinCall,
    InvalidCast,
    InvalidReceiverCall,
    InvalidCopyTarget,
    InvalidFieldAccess,
    InvalidMatchPattern,
    NonExhaustiveMatch,
    InvalidFallibleHandling,
    InvalidTemplateSlot,
    CompileTimeEvaluationError,
    InvalidCallShape,
    InvalidReturnShape,
    InvalidFunctionSignature,
    InvalidGenericInstantiation,
    UnsupportedExternalFunction,
    InvalidRangeOperand,
    UnsupportedBuilderPackage,
    UnsupportedBackendFeature,
    InvalidPageMetadata,
    InvalidCompileTimePath,
    DependencyNamespaceUsedAsValue,
    ConstRecordUsedAsValue,
    NestedDependencyTraversal,
    NamespaceTypeValueMisuse,
    UnknownTrait,
    DuplicateTraitRequirement,
    TraitPrivateSurfaceLeak,
    UnsupportedTraitFeature,
    InvalidTraitConformance,
    InvalidTraitIncompatibility,
    GenericBoundPrivateSurfaceLeak,
    TraitNameUsedAsType,
    InvalidTraitKeywordUsage,
    ExportOutsideModuleRoot,
    InvalidExportTarget,
    DuplicatePublicExport,
    PrivateTypeInExportedApi,
    DuplicateExportBlock,
    ProjectContextEscape,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ImportDiagnosticKind {
    UnusedImport,
    DependencyAliasCaseMismatch,
    MissingImportTarget,
    AmbiguousImportTarget,
    BareFileImport,
    DirectSpecialFileImport,
    ImportNameCollision,
    NotExportedBySourceFile,
    NotExportedByPublicSurface,
    MissingModuleRootPublicSurface,
    MissingPackageSymbol,
    CrossModuleImportNotExported,
    InvalidImportPath,
    DirectSymbolPathImport,
    InvalidNamespaceDefaultName,
    DuplicateImportSurfaceMember,
    ExplicitMothExtension,
    ExplicitSourceExtension,
    UnsupportedSourceFileKind,
    InvalidSourceFileEntry,
    MothTemplateInputsShareNoCommonAncestor,
    DuplicateMothTemplateInputPath,
    UnsupportedExternalExtension,
    InvalidExternalModule,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BorrowDiagnosticKind {
    BorrowConflict,
    MultipleMutableBorrows,
    SharedMutableConflict,
    UseAfterPossibleMove,
    MoveWhileBorrowed,
    WholeObjectBorrowConflict,
    InvalidMutableAccess,
    UseOfUninitializedLocal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ConfigDiagnosticKind {
    InvalidConfig,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InfrastructureDiagnosticKind {
    InfrastructureFailure,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DeferredFeatureDiagnosticKind {
    DeferredFeature,
}
