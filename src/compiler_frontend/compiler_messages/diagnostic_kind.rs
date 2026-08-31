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

    #[cfg(test)]
    pub(crate) fn all() -> Vec<Self> {
        let mut kinds = Vec::new();

        kinds.extend(SyntaxDiagnosticKind::all().map(DiagnosticKind::Syntax));
        kinds.extend(TypeDiagnosticKind::all().map(DiagnosticKind::Type));
        kinds.extend(RuleDiagnosticKind::all().map(DiagnosticKind::Rule));
        kinds.extend(ImportDiagnosticKind::all().map(DiagnosticKind::Import));
        kinds.extend(BorrowDiagnosticKind::all().map(DiagnosticKind::Borrow));
        kinds.extend(ConfigDiagnosticKind::all().map(DiagnosticKind::Config));
        kinds.extend(InfrastructureDiagnosticKind::all().map(DiagnosticKind::Infrastructure));
        kinds.extend(DeferredFeatureDiagnosticKind::all().map(DiagnosticKind::DeferredFeature));

        kinds
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
}

#[cfg(test)]
impl SyntaxDiagnosticKind {
    pub(crate) fn all() -> impl Iterator<Item = Self> {
        [
            Self::ExpectedToken,
            Self::UnexpectedToken,
            Self::UnexpectedTrailingComma,
            Self::MalformedCssTemplate,
            Self::MalformedHtmlTemplate,
            Self::UnterminatedStringLiteral,
            Self::InvalidCharacter,
            Self::InvalidNumberLiteral,
            Self::InvalidCharLiteral,
            Self::InvalidStyleDirective,
            Self::InvalidIdentifier,
            Self::MissingClosingDelimiter,
            Self::UnexpectedTokenInDeclaration,
            Self::InvalidTypeAnnotation,
            Self::InvalidGenericApplication,
            Self::InvalidCollectionType,
            Self::InvalidMapType,
            Self::InvalidMapLiteral,
            Self::UnexpectedEndOfFile,
            Self::InvalidPath,
            Self::InvalidDependencyClause,
            Self::LegacyDependencyClause,
            Self::InvalidGenericParameter,
            Self::InvalidTemplateDirective,
            Self::InvalidTemplateStructure,
            Self::InvalidExpression,
            Self::MissingOperatorOperand,
            Self::InvalidStandaloneStatement,
            Self::ExpectedSymbolStatement,
            Self::MissingCollectionItem,
            Self::InvalidMatchArm,
            Self::InvalidLoopHeader,
            Self::InvalidStatementPosition,
            Self::CommonSyntaxMistake,
            Self::UnescapedImplicitTemplateClose,
            Self::InvalidStringEscape,
        ]
        .into_iter()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TypeDiagnosticKind {
    TypeMismatch,
    EmptyCollectionTypeAmbiguity,
    UnsupportedOperatorTypes,
    InvalidFallibleOperand,
    IncompatibleChoiceComparison,
}

#[cfg(test)]
impl TypeDiagnosticKind {
    pub(crate) fn all() -> impl Iterator<Item = Self> {
        [
            Self::TypeMismatch,
            Self::EmptyCollectionTypeAmbiguity,
            Self::UnsupportedOperatorTypes,
            Self::InvalidFallibleOperand,
            Self::IncompatibleChoiceComparison,
        ]
        .into_iter()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RuleDiagnosticKind {
    UnknownName,
    DuplicateDeclaration,
    UnusedVariable,
    UnusedFunction,
    UnusedType,
    UnusedConstant,
    UnusedFunctionArgument,
    UnusedFunctionReturnValue,
    UnusedFunctionParameter,
    UnusedFunctionParameterDefaultValue,
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
    DuplicateExportBlock,
    PrivateTypeInExportedApi,
}

#[cfg(test)]
impl RuleDiagnosticKind {
    pub(crate) fn all() -> impl Iterator<Item = Self> {
        [
            Self::UnknownName,
            Self::DuplicateDeclaration,
            Self::UnusedVariable,
            Self::UnusedFunction,
            Self::UnusedType,
            Self::UnusedConstant,
            Self::UnusedFunctionArgument,
            Self::UnusedFunctionReturnValue,
            Self::UnusedFunctionParameter,
            Self::UnusedFunctionParameterDefaultValue,
            Self::IdentifierNamingConvention,
            Self::UnreachableMatchArm,
            Self::InvalidTopLevelRuntimeStatement,
            Self::ReservedBuiltinName,
            Self::InvalidSignatureMember,
            Self::InvalidChoiceVariant,
            Self::InvalidStructDefaultValue,
            Self::MissingDeclarationInitializer,
            Self::CircularDependency,
            Self::UnknownValueName,
            Self::UnknownTypeName,
            Self::ValueUsedAsType,
            Self::TypeUsedAsValue,
            Self::ShadowedName,
            Self::ReservedNameCollision,
            Self::InvalidThisUsage,
            Self::InvalidReceiverDeclaration,
            Self::InvalidControlFlowStatement,
            Self::InvalidDeclaration,
            Self::InvalidAssignmentTarget,
            Self::InvalidMultiBind,
            Self::InvalidBuiltinCall,
            Self::InvalidCast,
            Self::InvalidReceiverCall,
            Self::InvalidCopyTarget,
            Self::InvalidFieldAccess,
            Self::InvalidMatchPattern,
            Self::NonExhaustiveMatch,
            Self::InvalidFallibleHandling,
            Self::InvalidTemplateSlot,
            Self::CompileTimeEvaluationError,
            Self::InvalidCallShape,
            Self::InvalidReturnShape,
            Self::InvalidFunctionSignature,
            Self::InvalidGenericInstantiation,
            Self::UnsupportedExternalFunction,
            Self::InvalidRangeOperand,
            Self::UnsupportedBuilderPackage,
            Self::InvalidPageMetadata,
            Self::InvalidCompileTimePath,
            Self::DependencyNamespaceUsedAsValue,
            Self::ConstRecordUsedAsValue,
            Self::NestedDependencyTraversal,
            Self::NamespaceTypeValueMisuse,
            Self::UnknownTrait,
            Self::DuplicateTraitRequirement,
            Self::TraitPrivateSurfaceLeak,
            Self::UnsupportedTraitFeature,
            Self::InvalidTraitConformance,
            Self::InvalidTraitIncompatibility,
            Self::GenericBoundPrivateSurfaceLeak,
            Self::TraitNameUsedAsType,
            Self::InvalidTraitKeywordUsage,
            Self::ExportOutsideModuleRoot,
            Self::InvalidExportTarget,
            Self::DuplicatePublicExport,
            Self::DuplicateExportBlock,
            Self::PrivateTypeInExportedApi,
        ]
        .into_iter()
    }
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
    InvalidMothTemplateApiScopeItem,
    MothTemplateInputsShareNoCommonAncestor,
    DuplicateMothTemplateInputPath,
    UnsupportedExternalExtension,
    InvalidExternalModule,
}

#[cfg(test)]
impl ImportDiagnosticKind {
    pub(crate) fn all() -> impl Iterator<Item = Self> {
        [
            Self::UnusedImport,
            Self::DependencyAliasCaseMismatch,
            Self::MissingImportTarget,
            Self::AmbiguousImportTarget,
            Self::BareFileImport,
            Self::DirectSpecialFileImport,
            Self::ImportNameCollision,
            Self::NotExportedBySourceFile,
            Self::NotExportedByPublicSurface,
            Self::MissingModuleRootPublicSurface,
            Self::MissingPackageSymbol,
            Self::CrossModuleImportNotExported,
            Self::InvalidImportPath,
            Self::DirectSymbolPathImport,
            Self::InvalidNamespaceDefaultName,
            Self::DuplicateImportSurfaceMember,
            Self::ExplicitMothExtension,
            Self::ExplicitSourceExtension,
            Self::UnsupportedSourceFileKind,
            Self::InvalidSourceFileEntry,
            Self::InvalidMothTemplateApiScopeItem,
            Self::DuplicateMothTemplateInputPath,
            Self::UnsupportedExternalExtension,
            Self::InvalidExternalModule,
        ]
        .into_iter()
    }
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

#[cfg(test)]
impl BorrowDiagnosticKind {
    pub(crate) fn all() -> impl Iterator<Item = Self> {
        [
            Self::BorrowConflict,
            Self::MultipleMutableBorrows,
            Self::SharedMutableConflict,
            Self::UseAfterPossibleMove,
            Self::MoveWhileBorrowed,
            Self::WholeObjectBorrowConflict,
            Self::InvalidMutableAccess,
            Self::UseOfUninitializedLocal,
        ]
        .into_iter()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ConfigDiagnosticKind {
    InvalidConfig,
}

#[cfg(test)]
impl ConfigDiagnosticKind {
    pub(crate) fn all() -> impl Iterator<Item = Self> {
        [Self::InvalidConfig].into_iter()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum InfrastructureDiagnosticKind {
    InfrastructureFailure,
}

#[cfg(test)]
impl InfrastructureDiagnosticKind {
    pub(crate) fn all() -> impl Iterator<Item = Self> {
        [Self::InfrastructureFailure].into_iter()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DeferredFeatureDiagnosticKind {
    DeferredFeature,
}

#[cfg(test)]
impl DeferredFeatureDiagnosticKind {
    pub(crate) fn all() -> impl Iterator<Item = Self> {
        [Self::DeferredFeature].into_iter()
    }
}
