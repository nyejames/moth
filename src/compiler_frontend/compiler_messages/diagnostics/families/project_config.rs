//! Family rows for project configuration diagnostics.
//!
//! WHAT: compact family rows for project-configuration diagnostics.
//! WHY: the group owns these stable codes and their typed views, constructors and renderers.

use super::super::schema::DiagnosticCategory;
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::facts::{
    OptCountFact, OptStringFact, PathFact, StringFact, UnitFact, unit_code,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::token_lexical::{
    NumberLiteralCode, OptFixedScalarFact, OptNumberScaleFact,
};
use crate::compiler_frontend::compiler_messages::diagnostics::labels::LabelMessageCode;
use crate::compiler_frontend::compiler_messages::diagnostics::schema::{
    FamilyRow, diagnostic_family,
};
use crate::compiler_frontend::compiler_messages::render::{
    render_circular_dependency, render_dependency_namespace_used_as_value,
    render_html_single_file_route_name_empty, render_invalid_compile_time_path,
    render_invalid_config, render_invalid_dependency_clause, render_invalid_page_metadata,
    render_legacy_dependency_clause, render_namespace_type_value_misuse,
    render_nested_dependency_traversal, render_project_context_escape,
    render_project_entry_extension_required, render_unsupported_builder_package,
};
use crate::compiler_frontend::compiler_messages::{
    DependencyClauseKind, DiagnosticSeverity, InvalidCompileTimePathReason, InvalidConfigReason,
    InvalidDependencyClauseReason, InvalidOutputFolderReason, InvalidPageMetadataReason,
    LegacyDependencyClauseReason, NamespaceTypeValueMisuseKind, ProjectContextEscapeReason,
};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::symbols::string_interning::StringId;
unit_code! {
    pub(crate) enum InvalidCompileTimePathCode {
        MissingTarget = 1,
        TargetIsDirectory = 2,
        TargetNotRegular = 3,
        CurrentDirectorySegment = 4,
        ParentDirectorySegment = 5,
        CaseMismatch = 6,
        EscapesModuleBoundary = 7,
        EscapesSymlink = 8,
        EscapesProjectRoot = 9,
    }
}

impl InvalidCompileTimePathCode {
    /// Returns the stored reason code for an invalid-path reason.
    pub(crate) fn of(reason: &InvalidCompileTimePathReason) -> Self {
        match reason {
            InvalidCompileTimePathReason::MissingTarget => Self::MissingTarget,
            InvalidCompileTimePathReason::TargetIsDirectory => Self::TargetIsDirectory,
            InvalidCompileTimePathReason::TargetNotRegular => Self::TargetNotRegular,
            InvalidCompileTimePathReason::CurrentDirectorySegment => Self::CurrentDirectorySegment,
            InvalidCompileTimePathReason::ParentDirectorySegment => Self::ParentDirectorySegment,
            InvalidCompileTimePathReason::CaseMismatch { .. } => Self::CaseMismatch,
            InvalidCompileTimePathReason::EscapesModuleBoundary => Self::EscapesModuleBoundary,
            InvalidCompileTimePathReason::EscapesSymlink => Self::EscapesSymlink,
            InvalidCompileTimePathReason::EscapesProjectRoot => Self::EscapesProjectRoot,
        }
    }

    /// Splits the data-bearing case-mismatch reason into its two string facts.
    pub(crate) fn split(
        reason: &InvalidCompileTimePathReason,
    ) -> (Self, Option<StringId>, Option<StringId>) {
        match reason {
            InvalidCompileTimePathReason::CaseMismatch { provided, expected } => {
                (Self::CaseMismatch, Some(*provided), Some(*expected))
            }
            _ => (Self::of(reason), None, None),
        }
    }

    /// Rebuilds the reason for its existing prose helper.
    pub(crate) fn to_reason(
        self,
        provided: Option<StringId>,
        expected: Option<StringId>,
    ) -> Option<InvalidCompileTimePathReason> {
        Some(match self {
            Self::MissingTarget => InvalidCompileTimePathReason::MissingTarget,
            Self::TargetIsDirectory => InvalidCompileTimePathReason::TargetIsDirectory,
            Self::TargetNotRegular => InvalidCompileTimePathReason::TargetNotRegular,
            Self::CurrentDirectorySegment => InvalidCompileTimePathReason::CurrentDirectorySegment,
            Self::ParentDirectorySegment => InvalidCompileTimePathReason::ParentDirectorySegment,
            Self::CaseMismatch => InvalidCompileTimePathReason::CaseMismatch {
                provided: provided?,
                expected: expected?,
            },
            Self::EscapesModuleBoundary => InvalidCompileTimePathReason::EscapesModuleBoundary,
            Self::EscapesSymlink => InvalidCompileTimePathReason::EscapesSymlink,
            Self::EscapesProjectRoot => InvalidCompileTimePathReason::EscapesProjectRoot,
        })
    }
}
unit_code! {
    impl DependencyClauseKind {
        Namespace = 1,
        NamespaceAlias = 2,
        DirectSelection = 3,
    }
}

unit_code! {
    impl InvalidDependencyClauseReason {
        MissingPath = 1,
        ExpectedPath = 2,
        ExpectedSelectionName = 3,
        MissingAlias = 4,
        ExpectedAliasName = 5,
        DuplicateSelectionName = 6,
        DuplicateSelectionLocalName = 7,
        LegacyBraceSelections = 8,
        MissingSelectionAfterComma = 9,
        MissingCommaBetweenSelections = 10,
        NamespaceAliasWithSelections = 11,
        InvalidSelectionDelimiter = 12,
        DependencyClauseNotAllowed = 13,
        ProjectGlobalsPathReserved = 14,
        ProjectGlobalsFacadeDependencyNotAllowed = 15,
        ProjectGlobalsReexportNotAllowed = 16,
        ProviderRequiresBinding = 17,
        ContinuationEnteredStatement = 18,
    }
}

unit_code! {
    impl LegacyDependencyClauseReason {
        ImportKeyword = 1,
    }
}
unit_code! {
    impl InvalidPageMetadataReason {
        NotAString = 1,
        DuplicateDeclaration = 2,
    }
}

unit_code! {
    impl NamespaceTypeValueMisuseKind {
        Namespace = 1,
        Type = 2,
        Value = 3,
    }
}

unit_code! {
    impl ProjectContextEscapeReason {
        ExportedDeclaration = 1,
        ReachableExecutable = 2,
    }
}

diagnostic_family! {
    INVALID_DEPENDENCY_CLAUSE {
        internal: 1019,
        stable: "MOTH-SYNTAX-0019",
        category: DiagnosticCategory::Syntax,
        severity: DiagnosticSeverity::Error,
        title: "Invalid dependency clause",
        facts: [
            clause_kind: UnitFact<DependencyClauseKind>,
            reason: UnitFact<InvalidDependencyClauseReason>,
        ],
        labels: [],
        extra: None,
        ctor: manual,
        reason_key: reason,
        view: InvalidDependencyClauseView,
        render: render_invalid_dependency_clause,
    }
}

/// Stores the clause kind and reason as typed facts.
pub(crate) fn invalid_dependency_clause(
    clause_kind: DependencyClauseKind,
    reason: InvalidDependencyClauseReason,
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    DiagnosticDraft::with_facts(&INVALID_DEPENDENCY_CLAUSE, span, |cursor| {
        cursor.push::<UnitFact<DependencyClauseKind>>(&clause_kind)?;
        cursor.push::<UnitFact<InvalidDependencyClauseReason>>(&reason)?;
        Ok(())
    })
}

diagnostic_family! {
    LEGACY_DEPENDENCY_CLAUSE {
        internal: 1035,
        stable: "MOTH-SYNTAX-0035",
        category: DiagnosticCategory::Syntax,
        severity: DiagnosticSeverity::Error,
        title: "Legacy dependency clause syntax",
        facts: [
            reason: UnitFact<LegacyDependencyClauseReason>,
            replacement: OptStringFact,
        ],
        labels: [],
        extra: None,
        ctor: manual,
        reason_key: reason,
        view: LegacyDependencyClauseView,
        render: render_legacy_dependency_clause,
    }
}

/// Builds the legacy clause diagnostic with its fixed reason and optional replacement.
pub(crate) fn legacy_dependency_clause(
    replacement: Option<StringId>,
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    let reason = LegacyDependencyClauseReason::ImportKeyword;
    DiagnosticDraft::with_facts(&LEGACY_DEPENDENCY_CLAUSE, span, |cursor| {
        cursor.push::<UnitFact<LegacyDependencyClauseReason>>(&reason)?;
        cursor.push::<OptStringFact>(&replacement)?;
        Ok(())
    })
}

diagnostic_family! {
    CIRCULAR_DEPENDENCY {
        internal: 3033,
        stable: "MOTH-RULE-0033",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "Circular dependency",
        facts: [path: PathFact,],
        labels: [],
        extra: None,
        ctor: manual,
        view: CircularDependencyView,
        render: render_circular_dependency,
    }
}

/// The circular-dependency draft stores its path as one fixed-width fact.
pub(crate) fn circular_dependency(path: PathId, span: Option<SourceSpan>) -> DiagnosticDraft {
    DiagnosticDraft::with_facts(&CIRCULAR_DEPENDENCY, span, |cursor| {
        cursor.push::<PathFact>(&path)?;
        Ok(())
    })
}

diagnostic_family! {
    UNSUPPORTED_BUILDER_PACKAGE {
        internal: 3060,
        stable: "MOTH-RULE-0060",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "Unsupported builder package",
        facts: [package_path: StringFact,],
        labels: [],
        extra: None,
        ctor: generated unsupported_builder_package,
        view: UnsupportedBuilderPackageView,
        render: render_unsupported_builder_package,
    }
}

diagnostic_family! {
    INVALID_PAGE_METADATA {
        internal: 3061,
        stable: "MOTH-RULE-0061",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "Invalid page metadata",
        facts: [
            key: StringFact,
            reason: UnitFact<InvalidPageMetadataReason>,
        ],
        labels: [],
        extra: None,
        ctor: generated invalid_page_metadata,
        reason_key: reason,
        view: InvalidPageMetadataView,
        render: render_invalid_page_metadata,
    }
}

diagnostic_family! {
    INVALID_COMPILE_TIME_PATH {
        internal: 3063,
        stable: "MOTH-RULE-0063",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "Invalid compile-time path",
        facts: [
            reason: UnitFact<InvalidCompileTimePathCode>,
            path: PathFact,
            provided: OptStringFact,
            expected: OptStringFact,
        ],
        labels: [],
        extra: None,
        ctor: manual,
        reason_key: reason,
        view: InvalidCompileTimePathView,
        render: render_invalid_compile_time_path,
    }
}

/// Encodes a compile-time-path reason and its optional case-mismatch labels.
pub(crate) fn invalid_compile_time_path(
    path: PathId,
    reason: InvalidCompileTimePathReason,
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    let (reason, provided, expected) = InvalidCompileTimePathCode::split(&reason);
    DiagnosticDraft::with_facts(&INVALID_COMPILE_TIME_PATH, span, |cursor| {
        cursor.push::<UnitFact<InvalidCompileTimePathCode>>(&reason)?;
        cursor.push::<PathFact>(&path)?;
        cursor.push::<OptStringFact>(&provided)?;
        cursor.push::<OptStringFact>(&expected)?;
        Ok(())
    })
}

diagnostic_family! {
    DEPENDENCY_NAMESPACE_USED_AS_VALUE {
        internal: 3065,
        stable: "MOTH-RULE-0065",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "Dependency namespace used as value",
        facts: [record_name: StringFact,],
        labels: [],
        extra: None,
        ctor: generated dependency_namespace_used_as_value,
        view: DependencyNamespaceUsedAsValueView,
        render: render_dependency_namespace_used_as_value,
    }
}

diagnostic_family! {
    NESTED_DEPENDENCY_TRAVERSAL {
        internal: 3066,
        stable: "MOTH-RULE-0066",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "Nested dependency-namespace traversal",
        facts: [record_name: StringFact,],
        labels: [],
        extra: None,
        ctor: manual,
        view: NestedDependencyTraversalView,
        render: render_nested_dependency_traversal,
    }
}

/// Builds the nested traversal diagnostic while preserving its record-name fact.
pub(crate) fn nested_dependency_traversal(
    record_name: StringId,
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    DiagnosticDraft::with_facts(&NESTED_DEPENDENCY_TRAVERSAL, span, |cursor| {
        cursor.push::<StringFact>(&record_name)?;
        Ok(())
    })
}

diagnostic_family! {
    NAMESPACE_TYPE_VALUE_MISUSE {
        internal: 3067,
        stable: "MOTH-RULE-0067",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "Namespace type/value misuse",
        facts: [
            name: StringFact,
            expected: UnitFact<NamespaceTypeValueMisuseKind>,
            found: UnitFact<NamespaceTypeValueMisuseKind>,
        ],
        labels: [],
        extra: None,
        ctor: generated namespace_type_value_misuse,
        view: NamespaceTypeValueMisuseView,
        render: render_namespace_type_value_misuse,
    }
}

diagnostic_family! {
    PROJECT_CONTEXT_ESCAPE {
        internal: 3086,
        stable: "MOTH-RULE-0086",
        category: DiagnosticCategory::Rule,
        severity: DiagnosticSeverity::Error,
        title: "Project context escapes package facade",
        facts: [reason: UnitFact<ProjectContextEscapeReason>,],
        labels: [],
        extra: None,
        ctor: generated project_context_escape,
        reason_key: reason,
        view: ProjectContextEscapeView,
        render: render_project_context_escape,
    }
}

unit_code! {
    pub(crate) enum InvalidConfigCode {
        MissingKey = 1,
        MissingConfigFile = 2,
        MissingProjectRecord = 3,
        MissingActiveBuilderSection = 4,
        DuplicateKey = 5,
        FunctionUnsupported = 6,
        TraitDeclarationUnsupported = 7,
        TraitConformanceUnsupported = 8,
        TraitIncompatibilityUnsupported = 9,
        NamedTypeUnsupported = 10,
        MutableBindingUnsupported = 11,
        PlainBindingUnsupported = 12,
        UnsupportedStatement = 13,
        StandaloneTemplateUnsupported = 14,
        MissingValue = 15,
        UnsupportedScalarValue = 16,
        NotCompileTimeConstant = 17,
        ValueCouldNotFold = 18,
        EmptyProjectSetting = 19,
        ConfigQualifierFixedField = 20,
        ConfigContractNameInvalid = 21,
        ConfigQualifierInvalidPlacement = 22,
        ProjectGlobalsNameReserved = 23,
        ConfigQualifierUnsupportedType = 24,
        ConfigContractConflict = 25,
        UnknownBuildConfigInput = 26,
        ConfigInputTypeMismatch = 27,
        ConfigInputOutOfRangeSeparatorNotBetweenDigits = 28,
        ConfigInputOutOfRangeMultipleDecimalPoints = 29,
        ConfigInputOutOfRangeDecimalPointNotAfterDigit = 30,
        ConfigInputOutOfRangeEndsWithSeparator = 31,
        ConfigInputOutOfRangeMissingFractionalDigits = 32,
        ConfigInputOutOfRangeUppercaseExponentMarker = 33,
        ConfigInputOutOfRangeMissingExponentDigits = 34,
        ConfigInputOutOfRangeInvalidExponentSignPlacement = 35,
        ConfigInputOutOfRangeInvalidSeparatorPlacement = 36,
        ConfigInputOutOfRangeParseOverflow = 37,
        ConfigInputOutOfRangeOutsideFixedScalarRange = 38,
        ConfigInputOutOfRangeNegativeUnsignedLiteral = 39,
        ConfigInputOutOfRangeNonFiniteFixedFloat = 40,
        ConfigInputOutOfRangeInexactNumberScale = 41,
        MissingConfigInput = 42,
        UnknownKey = 43,
        UnknownRecordField = 44,
        MissingRequiredRecordField = 45,
        InvalidConfigValueShape = 46,
        InvalidProjectSettingValue = 47,
        MissingHtmlHomepage = 48,
        DuplicateHtmlOutputPath = 49,
        ResourceOutputPathCollision = 50,
        ResourceOutputPathReserved = 51,
        ConfiguredEntryRootMissing = 52,
        EntryRootPackagePrefixCollision = 53,
        SourcePackageMissingRoot = 54,
        SourcePackageMultipleRoots = 55,
        NoRootModuleEntries = 56,
        MultipleModuleRootFiles = 57,
        ConfigImportUnsupported = 58,
        FileValuePathUnsupported = 59,
        SourceFileFolderCollision = 60,
        LegacyModuleRootFileName = 61,
        InvalidOutputFolderEmpty = 62,
        InvalidOutputFolderNonUtf8 = 63,
        InvalidOutputFolderAbsolutePathWithoutFolder = 64,
        InvalidOutputFolderAbsolutePathWithFolder = 65,
        InvalidOutputFolderRootOrPrefix = 66,
        InvalidOutputFolderParentDirectorySegmentWithoutFolder = 67,
        InvalidOutputFolderParentDirectorySegmentWithFolder = 68,
        InvalidOutputFolderCurrentDirectory = 69,
        InvalidOutputFolderInvalidPathComponentWithoutFolder = 70,
        InvalidOutputFolderInvalidPathComponentWithFolder = 71,
        InvalidOutputFolderInsideOrEqualToEntryRootWithoutFolder = 72,
        InvalidOutputFolderInsideOrEqualToEntryRootWithFolder = 73,
        InvalidOutputFolderResolvesOutsideProjectRootWithoutFolder = 74,
        InvalidOutputFolderResolvesOutsideProjectRootWithFolder = 75,
        OutputFoldersNotDistinct = 76,
        OutputManifestOwnerConflict = 77,
        InvalidArtefactOutputPathEmpty = 78,
        InvalidArtefactOutputPathNonUtf8 = 79,
        InvalidArtefactOutputPathAbsolutePath = 80,
        InvalidArtefactOutputPathRootOrPrefix = 81,
        InvalidArtefactOutputPathParentDirectorySegment = 82,
        InvalidArtefactOutputPathCurrentDirectory = 83,
        InvalidArtefactOutputPathInvalidPathComponent = 84,
        InvalidArtefactOutputPathInsideOrEqualToEntryRoot = 85,
        InvalidArtefactOutputPathResolvesOutsideProjectRoot = 86,
    }
}

impl InvalidConfigCode {
    /// Chooses the compact code that owns an invalid-config reason key.
    pub(crate) fn of(reason: &InvalidConfigReason) -> Self {
        match reason {
            InvalidConfigReason::MissingKey => Self::MissingKey,
            InvalidConfigReason::MissingConfigFile => Self::MissingConfigFile,
            InvalidConfigReason::MissingProjectRecord => Self::MissingProjectRecord,
            InvalidConfigReason::MissingActiveBuilderSection { .. } => {
                Self::MissingActiveBuilderSection
            }
            InvalidConfigReason::DuplicateKey => Self::DuplicateKey,
            InvalidConfigReason::FunctionUnsupported => Self::FunctionUnsupported,
            InvalidConfigReason::TraitDeclarationUnsupported => Self::TraitDeclarationUnsupported,
            InvalidConfigReason::TraitConformanceUnsupported => Self::TraitConformanceUnsupported,
            InvalidConfigReason::TraitIncompatibilityUnsupported => {
                Self::TraitIncompatibilityUnsupported
            }
            InvalidConfigReason::NamedTypeUnsupported => Self::NamedTypeUnsupported,
            InvalidConfigReason::MutableBindingUnsupported => Self::MutableBindingUnsupported,
            InvalidConfigReason::PlainBindingUnsupported => Self::PlainBindingUnsupported,
            InvalidConfigReason::UnsupportedStatement => Self::UnsupportedStatement,
            InvalidConfigReason::StandaloneTemplateUnsupported => {
                Self::StandaloneTemplateUnsupported
            }
            InvalidConfigReason::MissingValue => Self::MissingValue,
            InvalidConfigReason::UnsupportedScalarValue => Self::UnsupportedScalarValue,
            InvalidConfigReason::NotCompileTimeConstant => Self::NotCompileTimeConstant,
            InvalidConfigReason::ValueCouldNotFold => Self::ValueCouldNotFold,
            InvalidConfigReason::EmptyProjectSetting => Self::EmptyProjectSetting,
            InvalidConfigReason::ConfigQualifierFixedField => Self::ConfigQualifierFixedField,
            InvalidConfigReason::ConfigContractNameInvalid => Self::ConfigContractNameInvalid,
            InvalidConfigReason::ConfigQualifierInvalidPlacement => {
                Self::ConfigQualifierInvalidPlacement
            }
            InvalidConfigReason::ProjectGlobalsNameReserved => Self::ProjectGlobalsNameReserved,
            InvalidConfigReason::ConfigQualifierUnsupportedType => {
                Self::ConfigQualifierUnsupportedType
            }
            InvalidConfigReason::ConfigContractConflict { .. } => Self::ConfigContractConflict,
            InvalidConfigReason::UnknownBuildConfigInput { .. } => Self::UnknownBuildConfigInput,
            InvalidConfigReason::ConfigInputTypeMismatch { .. } => Self::ConfigInputTypeMismatch,
            InvalidConfigReason::ConfigInputOutOfRange { reason, .. } => {
                Self::from_number_literal_code(NumberLiteralCode::split(reason).0)
            }
            InvalidConfigReason::MissingConfigInput => Self::MissingConfigInput,
            InvalidConfigReason::UnknownKey { .. } => Self::UnknownKey,
            InvalidConfigReason::UnknownRecordField { .. } => Self::UnknownRecordField,
            InvalidConfigReason::MissingRequiredRecordField { .. } => {
                Self::MissingRequiredRecordField
            }
            InvalidConfigReason::InvalidConfigValueShape { .. } => Self::InvalidConfigValueShape,
            InvalidConfigReason::InvalidProjectSettingValue { .. } => {
                Self::InvalidProjectSettingValue
            }
            InvalidConfigReason::MissingHtmlHomepage { .. } => Self::MissingHtmlHomepage,
            InvalidConfigReason::DuplicateHtmlOutputPath { .. } => Self::DuplicateHtmlOutputPath,
            InvalidConfigReason::ResourceOutputPathCollision { .. } => {
                Self::ResourceOutputPathCollision
            }
            InvalidConfigReason::ResourceOutputPathReserved { .. } => {
                Self::ResourceOutputPathReserved
            }
            InvalidConfigReason::ConfiguredEntryRootMissing { .. } => {
                Self::ConfiguredEntryRootMissing
            }
            InvalidConfigReason::EntryRootPackagePrefixCollision { .. } => {
                Self::EntryRootPackagePrefixCollision
            }
            InvalidConfigReason::SourcePackageMissingRoot { .. } => Self::SourcePackageMissingRoot,
            InvalidConfigReason::SourcePackageMultipleRoots { .. } => {
                Self::SourcePackageMultipleRoots
            }
            InvalidConfigReason::NoRootModuleEntries { .. } => Self::NoRootModuleEntries,
            InvalidConfigReason::MultipleModuleRootFiles { .. } => Self::MultipleModuleRootFiles,
            InvalidConfigReason::ConfigImportUnsupported => Self::ConfigImportUnsupported,
            InvalidConfigReason::FileValuePathUnsupported => Self::FileValuePathUnsupported,
            InvalidConfigReason::SourceFileFolderCollision { .. } => {
                Self::SourceFileFolderCollision
            }
            InvalidConfigReason::LegacyModuleRootFileName { .. } => Self::LegacyModuleRootFileName,
            InvalidConfigReason::InvalidOutputFolder { folder, reason } => {
                Self::output_folder_variant(*reason, folder.is_some())
            }
            InvalidConfigReason::OutputFoldersNotDistinct { .. } => Self::OutputFoldersNotDistinct,
            InvalidConfigReason::OutputManifestOwnerConflict { .. } => {
                Self::OutputManifestOwnerConflict
            }
            InvalidConfigReason::InvalidArtefactOutputPath { reason, .. } => {
                Self::artefact_output_path_variant(*reason)
            }
        }
    }

    /// Returns the nested output-folder code used by the reason-key delegate.
    pub(crate) fn output_folder_code(reason: InvalidOutputFolderReason) -> Self {
        Self::output_folder_variant(reason, false)
    }

    fn output_folder_variant(reason: InvalidOutputFolderReason, has_folder: bool) -> Self {
        match reason {
            InvalidOutputFolderReason::Empty => Self::InvalidOutputFolderEmpty,
            InvalidOutputFolderReason::NonUtf8 => Self::InvalidOutputFolderNonUtf8,
            InvalidOutputFolderReason::AbsolutePath if has_folder => {
                Self::InvalidOutputFolderAbsolutePathWithFolder
            }
            InvalidOutputFolderReason::AbsolutePath => {
                Self::InvalidOutputFolderAbsolutePathWithoutFolder
            }
            InvalidOutputFolderReason::RootOrPrefix => Self::InvalidOutputFolderRootOrPrefix,
            InvalidOutputFolderReason::ParentDirectorySegment if has_folder => {
                Self::InvalidOutputFolderParentDirectorySegmentWithFolder
            }
            InvalidOutputFolderReason::ParentDirectorySegment => {
                Self::InvalidOutputFolderParentDirectorySegmentWithoutFolder
            }
            InvalidOutputFolderReason::CurrentDirectory => {
                Self::InvalidOutputFolderCurrentDirectory
            }
            InvalidOutputFolderReason::InvalidPathComponent if has_folder => {
                Self::InvalidOutputFolderInvalidPathComponentWithFolder
            }
            InvalidOutputFolderReason::InvalidPathComponent => {
                Self::InvalidOutputFolderInvalidPathComponentWithoutFolder
            }
            InvalidOutputFolderReason::InsideOrEqualToEntryRoot if has_folder => {
                Self::InvalidOutputFolderInsideOrEqualToEntryRootWithFolder
            }
            InvalidOutputFolderReason::InsideOrEqualToEntryRoot => {
                Self::InvalidOutputFolderInsideOrEqualToEntryRootWithoutFolder
            }
            InvalidOutputFolderReason::ResolvesOutsideProjectRoot if has_folder => {
                Self::InvalidOutputFolderResolvesOutsideProjectRootWithFolder
            }
            InvalidOutputFolderReason::ResolvesOutsideProjectRoot => {
                Self::InvalidOutputFolderResolvesOutsideProjectRootWithoutFolder
            }
        }
    }

    fn artefact_output_path_variant(reason: InvalidOutputFolderReason) -> Self {
        match reason {
            InvalidOutputFolderReason::Empty => Self::InvalidArtefactOutputPathEmpty,
            InvalidOutputFolderReason::NonUtf8 => Self::InvalidArtefactOutputPathNonUtf8,
            InvalidOutputFolderReason::AbsolutePath => Self::InvalidArtefactOutputPathAbsolutePath,
            InvalidOutputFolderReason::RootOrPrefix => Self::InvalidArtefactOutputPathRootOrPrefix,
            InvalidOutputFolderReason::ParentDirectorySegment => {
                Self::InvalidArtefactOutputPathParentDirectorySegment
            }
            InvalidOutputFolderReason::CurrentDirectory => {
                Self::InvalidArtefactOutputPathCurrentDirectory
            }
            InvalidOutputFolderReason::InvalidPathComponent => {
                Self::InvalidArtefactOutputPathInvalidPathComponent
            }
            InvalidOutputFolderReason::InsideOrEqualToEntryRoot => {
                Self::InvalidArtefactOutputPathInsideOrEqualToEntryRoot
            }
            InvalidOutputFolderReason::ResolvesOutsideProjectRoot => {
                Self::InvalidArtefactOutputPathResolvesOutsideProjectRoot
            }
        }
    }

    fn output_folder_shape(self) -> Option<(InvalidOutputFolderReason, bool)> {
        Some(match self {
            Self::InvalidOutputFolderEmpty => (InvalidOutputFolderReason::Empty, false),
            Self::InvalidOutputFolderNonUtf8 => (InvalidOutputFolderReason::NonUtf8, false),
            Self::InvalidOutputFolderAbsolutePathWithoutFolder => {
                (InvalidOutputFolderReason::AbsolutePath, false)
            }
            Self::InvalidOutputFolderAbsolutePathWithFolder => {
                (InvalidOutputFolderReason::AbsolutePath, true)
            }
            Self::InvalidOutputFolderRootOrPrefix => {
                (InvalidOutputFolderReason::RootOrPrefix, false)
            }
            Self::InvalidOutputFolderParentDirectorySegmentWithoutFolder => {
                (InvalidOutputFolderReason::ParentDirectorySegment, false)
            }
            Self::InvalidOutputFolderParentDirectorySegmentWithFolder => {
                (InvalidOutputFolderReason::ParentDirectorySegment, true)
            }
            Self::InvalidOutputFolderCurrentDirectory => {
                (InvalidOutputFolderReason::CurrentDirectory, false)
            }
            Self::InvalidOutputFolderInvalidPathComponentWithoutFolder => {
                (InvalidOutputFolderReason::InvalidPathComponent, false)
            }
            Self::InvalidOutputFolderInvalidPathComponentWithFolder => {
                (InvalidOutputFolderReason::InvalidPathComponent, true)
            }
            Self::InvalidOutputFolderInsideOrEqualToEntryRootWithoutFolder => {
                (InvalidOutputFolderReason::InsideOrEqualToEntryRoot, false)
            }
            Self::InvalidOutputFolderInsideOrEqualToEntryRootWithFolder => {
                (InvalidOutputFolderReason::InsideOrEqualToEntryRoot, true)
            }
            Self::InvalidOutputFolderResolvesOutsideProjectRootWithoutFolder => {
                (InvalidOutputFolderReason::ResolvesOutsideProjectRoot, false)
            }
            Self::InvalidOutputFolderResolvesOutsideProjectRootWithFolder => {
                (InvalidOutputFolderReason::ResolvesOutsideProjectRoot, true)
            }
            _ => return None,
        })
    }

    fn artefact_output_path_shape(self) -> Option<InvalidOutputFolderReason> {
        Some(match self {
            Self::InvalidArtefactOutputPathEmpty => InvalidOutputFolderReason::Empty,
            Self::InvalidArtefactOutputPathNonUtf8 => InvalidOutputFolderReason::NonUtf8,
            Self::InvalidArtefactOutputPathAbsolutePath => InvalidOutputFolderReason::AbsolutePath,
            Self::InvalidArtefactOutputPathRootOrPrefix => InvalidOutputFolderReason::RootOrPrefix,
            Self::InvalidArtefactOutputPathParentDirectorySegment => {
                InvalidOutputFolderReason::ParentDirectorySegment
            }
            Self::InvalidArtefactOutputPathCurrentDirectory => {
                InvalidOutputFolderReason::CurrentDirectory
            }
            Self::InvalidArtefactOutputPathInvalidPathComponent => {
                InvalidOutputFolderReason::InvalidPathComponent
            }
            Self::InvalidArtefactOutputPathInsideOrEqualToEntryRoot => {
                InvalidOutputFolderReason::InsideOrEqualToEntryRoot
            }
            Self::InvalidArtefactOutputPathResolvesOutsideProjectRoot => {
                InvalidOutputFolderReason::ResolvesOutsideProjectRoot
            }
            _ => return None,
        })
    }

    fn number_literal_code(self) -> Option<NumberLiteralCode> {
        Some(match self {
            Self::ConfigInputOutOfRangeSeparatorNotBetweenDigits => {
                NumberLiteralCode::SeparatorNotBetweenDigits
            }
            Self::ConfigInputOutOfRangeMultipleDecimalPoints => {
                NumberLiteralCode::MultipleDecimalPoints
            }
            Self::ConfigInputOutOfRangeDecimalPointNotAfterDigit => {
                NumberLiteralCode::DecimalPointNotAfterDigit
            }
            Self::ConfigInputOutOfRangeEndsWithSeparator => NumberLiteralCode::EndsWithSeparator,
            Self::ConfigInputOutOfRangeMissingFractionalDigits => {
                NumberLiteralCode::MissingFractionalDigits
            }
            Self::ConfigInputOutOfRangeUppercaseExponentMarker => {
                NumberLiteralCode::UppercaseExponentMarker
            }
            Self::ConfigInputOutOfRangeMissingExponentDigits => {
                NumberLiteralCode::MissingExponentDigits
            }
            Self::ConfigInputOutOfRangeInvalidExponentSignPlacement => {
                NumberLiteralCode::InvalidExponentSignPlacement
            }
            Self::ConfigInputOutOfRangeInvalidSeparatorPlacement => {
                NumberLiteralCode::InvalidSeparatorPlacement
            }
            Self::ConfigInputOutOfRangeParseOverflow => NumberLiteralCode::ParseOverflow,
            Self::ConfigInputOutOfRangeOutsideFixedScalarRange => {
                NumberLiteralCode::OutsideFixedScalarRange
            }
            Self::ConfigInputOutOfRangeNegativeUnsignedLiteral => {
                NumberLiteralCode::NegativeUnsignedLiteral
            }
            Self::ConfigInputOutOfRangeNonFiniteFixedFloat => {
                NumberLiteralCode::NonFiniteFixedFloat
            }
            Self::ConfigInputOutOfRangeInexactNumberScale => NumberLiteralCode::InexactNumberScale,
            _ => return None,
        })
    }

    fn from_number_literal_code(code: NumberLiteralCode) -> Self {
        match code {
            NumberLiteralCode::SeparatorNotBetweenDigits => {
                Self::ConfigInputOutOfRangeSeparatorNotBetweenDigits
            }
            NumberLiteralCode::MultipleDecimalPoints => {
                Self::ConfigInputOutOfRangeMultipleDecimalPoints
            }
            NumberLiteralCode::DecimalPointNotAfterDigit => {
                Self::ConfigInputOutOfRangeDecimalPointNotAfterDigit
            }
            NumberLiteralCode::EndsWithSeparator => Self::ConfigInputOutOfRangeEndsWithSeparator,
            NumberLiteralCode::MissingFractionalDigits => {
                Self::ConfigInputOutOfRangeMissingFractionalDigits
            }
            NumberLiteralCode::UppercaseExponentMarker => {
                Self::ConfigInputOutOfRangeUppercaseExponentMarker
            }
            NumberLiteralCode::MissingExponentDigits => {
                Self::ConfigInputOutOfRangeMissingExponentDigits
            }
            NumberLiteralCode::InvalidExponentSignPlacement => {
                Self::ConfigInputOutOfRangeInvalidExponentSignPlacement
            }
            NumberLiteralCode::InvalidSeparatorPlacement => {
                Self::ConfigInputOutOfRangeInvalidSeparatorPlacement
            }
            NumberLiteralCode::ParseOverflow => Self::ConfigInputOutOfRangeParseOverflow,
            NumberLiteralCode::OutsideFixedScalarRange => {
                Self::ConfigInputOutOfRangeOutsideFixedScalarRange
            }
            NumberLiteralCode::NegativeUnsignedLiteral => {
                Self::ConfigInputOutOfRangeNegativeUnsignedLiteral
            }
            NumberLiteralCode::NonFiniteFixedFloat => {
                Self::ConfigInputOutOfRangeNonFiniteFixedFloat
            }
            NumberLiteralCode::InexactNumberScale => Self::ConfigInputOutOfRangeInexactNumberScale,
        }
    }

    fn uses_outer_key(self) -> bool {
        matches!(
            self,
            Self::DuplicateKey
                | Self::PlainBindingUnsupported
                | Self::MissingValue
                | Self::UnsupportedScalarValue
                | Self::NotCompileTimeConstant
                | Self::ValueCouldNotFold
                | Self::EmptyProjectSetting
                | Self::ConfigQualifierFixedField
                | Self::ConfigContractNameInvalid
                | Self::ConfigContractConflict
                | Self::ConfigInputTypeMismatch
                | Self::ConfigInputOutOfRangeSeparatorNotBetweenDigits
                | Self::ConfigInputOutOfRangeMultipleDecimalPoints
                | Self::ConfigInputOutOfRangeDecimalPointNotAfterDigit
                | Self::ConfigInputOutOfRangeEndsWithSeparator
                | Self::ConfigInputOutOfRangeMissingFractionalDigits
                | Self::ConfigInputOutOfRangeUppercaseExponentMarker
                | Self::ConfigInputOutOfRangeMissingExponentDigits
                | Self::ConfigInputOutOfRangeInvalidExponentSignPlacement
                | Self::ConfigInputOutOfRangeInvalidSeparatorPlacement
                | Self::ConfigInputOutOfRangeParseOverflow
                | Self::ConfigInputOutOfRangeOutsideFixedScalarRange
                | Self::ConfigInputOutOfRangeNegativeUnsignedLiteral
                | Self::ConfigInputOutOfRangeNonFiniteFixedFloat
                | Self::ConfigInputOutOfRangeInexactNumberScale
                | Self::MissingConfigInput
                | Self::InvalidConfigValueShape
                | Self::InvalidProjectSettingValue
                | Self::InvalidOutputFolderEmpty
                | Self::InvalidOutputFolderNonUtf8
                | Self::InvalidOutputFolderAbsolutePathWithoutFolder
                | Self::InvalidOutputFolderAbsolutePathWithFolder
                | Self::InvalidOutputFolderRootOrPrefix
                | Self::InvalidOutputFolderParentDirectorySegmentWithoutFolder
                | Self::InvalidOutputFolderParentDirectorySegmentWithFolder
                | Self::InvalidOutputFolderCurrentDirectory
                | Self::InvalidOutputFolderInvalidPathComponentWithoutFolder
                | Self::InvalidOutputFolderInvalidPathComponentWithFolder
                | Self::InvalidOutputFolderInsideOrEqualToEntryRootWithoutFolder
                | Self::InvalidOutputFolderInsideOrEqualToEntryRootWithFolder
                | Self::InvalidOutputFolderResolvesOutsideProjectRootWithoutFolder
                | Self::InvalidOutputFolderResolvesOutsideProjectRootWithFolder
        )
    }
}

diagnostic_family! {
    INVALID_CONFIG {
        internal: 6001,
        stable: "MOTH-CONFIG-0001",
        category: DiagnosticCategory::Config,
        severity: DiagnosticSeverity::Error,
        title: "Invalid config",
        facts: [
            reason: UnitFact<InvalidConfigCode>,
            provided_argument_index: OptCountFact,
            fixed_scalar: OptFixedScalarFact,
            number_scale: OptNumberScaleFact,
        ],
        labels: [
            LabelMessageCode::PREVIOUS_DECLARATION,
        ],
        // StringIds follows each reason's string-field order; a rendered outer key is last.
        // Candidate arrays occupy the remainder after their fixed string-field prefixes.
        extra: StringIds,
        ctor: manual,
        reason_key: reason,
        view: InvalidConfigView,
        render: render_invalid_config,
    }
}

/// Builds an invalid-config draft from the key and reason inputs.
pub(crate) fn invalid_config_reason(
    key: Option<StringId>,
    reason: InvalidConfigReason,
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    let code = InvalidConfigCode::of(&reason);
    let provided_argument_index = match &reason {
        InvalidConfigReason::UnknownBuildConfigInput {
            provided_argument_index,
            ..
        }
        | InvalidConfigReason::ConfigInputTypeMismatch {
            provided_argument_index,
            ..
        }
        | InvalidConfigReason::ConfigInputOutOfRange {
            provided_argument_index,
            ..
        } => *provided_argument_index,
        _ => None,
    };
    let (fixed_scalar, number_scale) = match &reason {
        InvalidConfigReason::ConfigInputOutOfRange { reason, .. } => {
            let (_, fixed_scalar, number_scale) = NumberLiteralCode::split(reason);
            (fixed_scalar, number_scale)
        }
        _ => (None, None),
    };
    let string_ids = invalid_config_string_ids(key, &reason, code);

    DiagnosticDraft::with_facts(&INVALID_CONFIG, span, |cursor| {
        cursor.push::<UnitFact<InvalidConfigCode>>(&code)?;
        cursor.push::<OptCountFact>(&provided_argument_index)?;
        cursor.push::<OptFixedScalarFact>(&fixed_scalar)?;
        cursor.push::<OptNumberScaleFact>(&number_scale)?;
        Ok(())
    })
    .with_string_ids(string_ids)
}

/// Encodes the reason's displayed strings in its fixed row-defined order.
fn invalid_config_string_ids(
    key: Option<StringId>,
    reason: &InvalidConfigReason,
    code: InvalidConfigCode,
) -> Vec<StringId> {
    let mut strings = Vec::new();
    match reason {
        InvalidConfigReason::MissingActiveBuilderSection { section } => strings.push(*section),
        InvalidConfigReason::ConfigContractConflict { first, conflicting } => {
            strings.extend([*first, *conflicting])
        }
        InvalidConfigReason::UnknownBuildConfigInput { key, .. }
        | InvalidConfigReason::UnknownKey { key } => strings.push(*key),
        InvalidConfigReason::ConfigInputTypeMismatch {
            provided, expected, ..
        } => strings.extend([*provided, *expected]),
        InvalidConfigReason::ConfigInputOutOfRange { literal, .. } => strings.push(*literal),
        InvalidConfigReason::UnknownRecordField { record, field }
        | InvalidConfigReason::MissingRequiredRecordField { record, field } => {
            strings.extend([*record, *field]);
        }
        InvalidConfigReason::InvalidConfigValueShape { expected } => strings.push(*expected),
        InvalidConfigReason::InvalidProjectSettingValue { value, expected } => {
            strings.extend([*value, *expected]);
        }
        InvalidConfigReason::MissingHtmlHomepage { entry_root }
        | InvalidConfigReason::ConfiguredEntryRootMissing { entry_root }
        | InvalidConfigReason::NoRootModuleEntries { entry_root } => strings.push(*entry_root),
        InvalidConfigReason::DuplicateHtmlOutputPath {
            output_path,
            entry_point,
            existing_entry_point,
        } => strings.extend([*output_path, *entry_point, *existing_entry_point]),
        InvalidConfigReason::ResourceOutputPathCollision {
            output_path,
            existing_origin,
            conflicting_origin,
        } => strings.extend([*output_path, *existing_origin, *conflicting_origin]),
        InvalidConfigReason::ResourceOutputPathReserved {
            output_path,
            origin,
            artefact_kind,
        } => strings.extend([*output_path, *origin, *artefact_kind]),
        InvalidConfigReason::InvalidArtefactOutputPath {
            output_path,
            artefact_kind,
            ..
        } => strings.extend([*output_path, *artefact_kind]),
        InvalidConfigReason::EntryRootPackagePrefixCollision {
            prefix,
            entry_folder,
        } => strings.extend([*prefix, *entry_folder]),
        InvalidConfigReason::SourcePackageMissingRoot { prefix, root } => {
            strings.extend([*prefix, *root]);
        }
        InvalidConfigReason::SourcePackageMultipleRoots {
            prefix,
            root,
            candidates,
        } => {
            strings.extend([*prefix, *root]);
            strings.extend(candidates.iter().copied());
        }
        InvalidConfigReason::MultipleModuleRootFiles {
            directory,
            candidates,
        } => {
            strings.push(*directory);
            strings.extend(candidates.iter().copied());
        }
        InvalidConfigReason::SourceFileFolderCollision {
            file_name,
            folder_name,
            directory,
        } => strings.extend([*file_name, *folder_name, *directory]),
        InvalidConfigReason::LegacyModuleRootFileName {
            file_name,
            directory,
        } => strings.extend([*file_name, *directory]),
        InvalidConfigReason::InvalidOutputFolder { folder, .. } => {
            if matches!(code.output_folder_shape(), Some((_, true)))
                && let Some(folder) = folder
            {
                strings.push(*folder);
            }
        }
        InvalidConfigReason::OutputFoldersNotDistinct {
            dev_folder,
            release_folder,
        } => strings.extend([*dev_folder, *release_folder]),
        InvalidConfigReason::OutputManifestOwnerConflict {
            output_root,
            existing_builder,
            existing_profile,
            active_builder,
            active_profile,
        } => strings.extend([
            *output_root,
            *existing_builder,
            *existing_profile,
            *active_builder,
            *active_profile,
        ]),
        _ => {}
    }
    if code.uses_outer_key()
        && let Some(key) = key
    {
        strings.push(key);
    }
    strings
}

struct InvalidConfigStringIds<'a> {
    remaining: &'a [StringId],
}

impl<'a> InvalidConfigStringIds<'a> {
    fn new(remaining: &'a [StringId]) -> Self {
        Self { remaining }
    }

    fn take(&mut self) -> Option<StringId> {
        let (first, remaining) = self.remaining.split_first()?;
        self.remaining = remaining;
        Some(*first)
    }

    fn take_remaining(&mut self) -> Vec<StringId> {
        let remaining = self.remaining.to_vec();
        self.remaining = &[];
        remaining
    }

    fn finish_key(&mut self, uses_outer_key: bool) -> Option<Option<StringId>> {
        if !uses_outer_key {
            return self.remaining.is_empty().then_some(None);
        }

        match self.remaining {
            [] => Some(None),
            [key] => {
                self.remaining = &[];
                Some(Some(*key))
            }
            _ => None,
        }
    }
}

/// Rebuilds an invalid-config reason from its typed code and ordered string list.
pub(crate) fn decode_invalid_config(
    view: &InvalidConfigView,
    string_ids: &[StringId],
) -> Option<(Option<StringId>, InvalidConfigReason)> {
    let code = view.reason;
    let mut strings = InvalidConfigStringIds::new(string_ids);
    let reason = match code {
        number_code @ (InvalidConfigCode::ConfigInputOutOfRangeSeparatorNotBetweenDigits
        | InvalidConfigCode::ConfigInputOutOfRangeMultipleDecimalPoints
        | InvalidConfigCode::ConfigInputOutOfRangeDecimalPointNotAfterDigit
        | InvalidConfigCode::ConfigInputOutOfRangeEndsWithSeparator
        | InvalidConfigCode::ConfigInputOutOfRangeMissingFractionalDigits
        | InvalidConfigCode::ConfigInputOutOfRangeUppercaseExponentMarker
        | InvalidConfigCode::ConfigInputOutOfRangeMissingExponentDigits
        | InvalidConfigCode::ConfigInputOutOfRangeInvalidExponentSignPlacement
        | InvalidConfigCode::ConfigInputOutOfRangeInvalidSeparatorPlacement
        | InvalidConfigCode::ConfigInputOutOfRangeParseOverflow
        | InvalidConfigCode::ConfigInputOutOfRangeOutsideFixedScalarRange
        | InvalidConfigCode::ConfigInputOutOfRangeNegativeUnsignedLiteral
        | InvalidConfigCode::ConfigInputOutOfRangeNonFiniteFixedFloat
        | InvalidConfigCode::ConfigInputOutOfRangeInexactNumberScale) => {
            InvalidConfigReason::ConfigInputOutOfRange {
                literal: strings.take()?,
                reason: number_code
                    .number_literal_code()?
                    .to_reason(view.fixed_scalar, view.number_scale)?,
                provided_argument_index: view.provided_argument_index,
            }
        }
        folder_code @ (InvalidConfigCode::InvalidOutputFolderEmpty
        | InvalidConfigCode::InvalidOutputFolderNonUtf8
        | InvalidConfigCode::InvalidOutputFolderAbsolutePathWithoutFolder
        | InvalidConfigCode::InvalidOutputFolderAbsolutePathWithFolder
        | InvalidConfigCode::InvalidOutputFolderRootOrPrefix
        | InvalidConfigCode::InvalidOutputFolderParentDirectorySegmentWithoutFolder
        | InvalidConfigCode::InvalidOutputFolderParentDirectorySegmentWithFolder
        | InvalidConfigCode::InvalidOutputFolderCurrentDirectory
        | InvalidConfigCode::InvalidOutputFolderInvalidPathComponentWithoutFolder
        | InvalidConfigCode::InvalidOutputFolderInvalidPathComponentWithFolder
        | InvalidConfigCode::InvalidOutputFolderInsideOrEqualToEntryRootWithoutFolder
        | InvalidConfigCode::InvalidOutputFolderInsideOrEqualToEntryRootWithFolder
        | InvalidConfigCode::InvalidOutputFolderResolvesOutsideProjectRootWithoutFolder
        | InvalidConfigCode::InvalidOutputFolderResolvesOutsideProjectRootWithFolder) => {
            let (reason, has_folder) = folder_code.output_folder_shape()?;
            InvalidConfigReason::InvalidOutputFolder {
                folder: if has_folder {
                    Some(strings.take()?)
                } else {
                    None
                },
                reason,
            }
        }
        InvalidConfigCode::MissingKey => InvalidConfigReason::MissingKey,
        InvalidConfigCode::MissingConfigFile => InvalidConfigReason::MissingConfigFile,
        InvalidConfigCode::MissingProjectRecord => InvalidConfigReason::MissingProjectRecord,
        InvalidConfigCode::MissingActiveBuilderSection => {
            InvalidConfigReason::MissingActiveBuilderSection {
                section: strings.take()?,
            }
        }
        InvalidConfigCode::DuplicateKey => InvalidConfigReason::DuplicateKey,
        InvalidConfigCode::FunctionUnsupported => InvalidConfigReason::FunctionUnsupported,
        InvalidConfigCode::TraitDeclarationUnsupported => {
            InvalidConfigReason::TraitDeclarationUnsupported
        }
        InvalidConfigCode::TraitConformanceUnsupported => {
            InvalidConfigReason::TraitConformanceUnsupported
        }
        InvalidConfigCode::TraitIncompatibilityUnsupported => {
            InvalidConfigReason::TraitIncompatibilityUnsupported
        }
        InvalidConfigCode::NamedTypeUnsupported => InvalidConfigReason::NamedTypeUnsupported,
        InvalidConfigCode::MutableBindingUnsupported => {
            InvalidConfigReason::MutableBindingUnsupported
        }
        InvalidConfigCode::PlainBindingUnsupported => {
            InvalidConfigReason::PlainBindingUnsupported
        }
        InvalidConfigCode::UnsupportedStatement => InvalidConfigReason::UnsupportedStatement,
        InvalidConfigCode::StandaloneTemplateUnsupported => {
            InvalidConfigReason::StandaloneTemplateUnsupported
        }
        InvalidConfigCode::MissingValue => InvalidConfigReason::MissingValue,
        InvalidConfigCode::UnsupportedScalarValue => InvalidConfigReason::UnsupportedScalarValue,
        InvalidConfigCode::NotCompileTimeConstant => {
            InvalidConfigReason::NotCompileTimeConstant
        }
        InvalidConfigCode::ValueCouldNotFold => InvalidConfigReason::ValueCouldNotFold,
        InvalidConfigCode::EmptyProjectSetting => InvalidConfigReason::EmptyProjectSetting,
        InvalidConfigCode::ConfigQualifierFixedField => {
            InvalidConfigReason::ConfigQualifierFixedField
        }
        InvalidConfigCode::ConfigContractNameInvalid => {
            InvalidConfigReason::ConfigContractNameInvalid
        }
        InvalidConfigCode::ConfigQualifierInvalidPlacement => {
            InvalidConfigReason::ConfigQualifierInvalidPlacement
        }
        InvalidConfigCode::ProjectGlobalsNameReserved => {
            InvalidConfigReason::ProjectGlobalsNameReserved
        }
        InvalidConfigCode::ConfigQualifierUnsupportedType => {
            InvalidConfigReason::ConfigQualifierUnsupportedType
        }
        InvalidConfigCode::ConfigContractConflict => {
            InvalidConfigReason::ConfigContractConflict {
                first: strings.take()?,
                conflicting: strings.take()?,
            }
        }
        InvalidConfigCode::UnknownBuildConfigInput => {
            InvalidConfigReason::UnknownBuildConfigInput {
                key: strings.take()?,
                provided_argument_index: view.provided_argument_index,
            }
        }
        InvalidConfigCode::ConfigInputTypeMismatch => {
            InvalidConfigReason::ConfigInputTypeMismatch {
                provided: strings.take()?,
                expected: strings.take()?,
                provided_argument_index: view.provided_argument_index,
            }
        }
        InvalidConfigCode::MissingConfigInput => InvalidConfigReason::MissingConfigInput,
        InvalidConfigCode::UnknownKey => InvalidConfigReason::UnknownKey {
            key: strings.take()?,
        },
        InvalidConfigCode::UnknownRecordField => InvalidConfigReason::UnknownRecordField {
            record: strings.take()?,
            field: strings.take()?,
        },
        InvalidConfigCode::MissingRequiredRecordField => {
            InvalidConfigReason::MissingRequiredRecordField {
                record: strings.take()?,
                field: strings.take()?,
            }
        }
        InvalidConfigCode::InvalidConfigValueShape => {
            InvalidConfigReason::InvalidConfigValueShape {
                expected: strings.take()?,
            }
        }
        InvalidConfigCode::InvalidProjectSettingValue => {
            InvalidConfigReason::InvalidProjectSettingValue {
                value: strings.take()?,
                expected: strings.take()?,
            }
        }
        InvalidConfigCode::MissingHtmlHomepage => InvalidConfigReason::MissingHtmlHomepage {
            entry_root: strings.take()?,
        },
        InvalidConfigCode::DuplicateHtmlOutputPath => {
            InvalidConfigReason::DuplicateHtmlOutputPath {
                output_path: strings.take()?,
                entry_point: strings.take()?,
                existing_entry_point: strings.take()?,
            }
        }
        InvalidConfigCode::ResourceOutputPathCollision => {
            InvalidConfigReason::ResourceOutputPathCollision {
                output_path: strings.take()?,
                existing_origin: strings.take()?,
                conflicting_origin: strings.take()?,
            }
        }
        InvalidConfigCode::ResourceOutputPathReserved => {
            InvalidConfigReason::ResourceOutputPathReserved {
                output_path: strings.take()?,
                origin: strings.take()?,
                artefact_kind: strings.take()?,
            }
        }
        artefact_code @ (InvalidConfigCode::InvalidArtefactOutputPathEmpty
        | InvalidConfigCode::InvalidArtefactOutputPathNonUtf8
        | InvalidConfigCode::InvalidArtefactOutputPathAbsolutePath
        | InvalidConfigCode::InvalidArtefactOutputPathRootOrPrefix
        | InvalidConfigCode::InvalidArtefactOutputPathParentDirectorySegment
        | InvalidConfigCode::InvalidArtefactOutputPathCurrentDirectory
        | InvalidConfigCode::InvalidArtefactOutputPathInvalidPathComponent
        | InvalidConfigCode::InvalidArtefactOutputPathInsideOrEqualToEntryRoot
        | InvalidConfigCode::InvalidArtefactOutputPathResolvesOutsideProjectRoot) => {
            InvalidConfigReason::InvalidArtefactOutputPath {
                output_path: strings.take()?,
                artefact_kind: strings.take()?,
                reason: artefact_code.artefact_output_path_shape()?,
            }
        }
        InvalidConfigCode::ConfiguredEntryRootMissing => {
            InvalidConfigReason::ConfiguredEntryRootMissing {
                entry_root: strings.take()?,
            }
        }
        InvalidConfigCode::EntryRootPackagePrefixCollision => {
            InvalidConfigReason::EntryRootPackagePrefixCollision {
                prefix: strings.take()?,
                entry_folder: strings.take()?,
            }
        }
        InvalidConfigCode::SourcePackageMissingRoot => {
            InvalidConfigReason::SourcePackageMissingRoot {
                prefix: strings.take()?,
                root: strings.take()?,
            }
        }
        InvalidConfigCode::SourcePackageMultipleRoots => {
            InvalidConfigReason::SourcePackageMultipleRoots {
                prefix: strings.take()?,
                root: strings.take()?,
                candidates: strings.take_remaining(),
            }
        }
        InvalidConfigCode::NoRootModuleEntries => InvalidConfigReason::NoRootModuleEntries {
            entry_root: strings.take()?,
        },
        InvalidConfigCode::MultipleModuleRootFiles => {
            InvalidConfigReason::MultipleModuleRootFiles {
                directory: strings.take()?,
                candidates: strings.take_remaining(),
            }
        }
        InvalidConfigCode::ConfigImportUnsupported => {
            InvalidConfigReason::ConfigImportUnsupported
        }
        InvalidConfigCode::FileValuePathUnsupported => {
            InvalidConfigReason::FileValuePathUnsupported
        }
        InvalidConfigCode::SourceFileFolderCollision => {
            InvalidConfigReason::SourceFileFolderCollision {
                file_name: strings.take()?,
                folder_name: strings.take()?,
                directory: strings.take()?,
            }
        }
        InvalidConfigCode::LegacyModuleRootFileName => {
            InvalidConfigReason::LegacyModuleRootFileName {
                file_name: strings.take()?,
                directory: strings.take()?,
            }
        }
        InvalidConfigCode::OutputFoldersNotDistinct => {
            InvalidConfigReason::OutputFoldersNotDistinct {
                dev_folder: strings.take()?,
                release_folder: strings.take()?,
            }
        }
        InvalidConfigCode::OutputManifestOwnerConflict => {
            InvalidConfigReason::OutputManifestOwnerConflict {
                output_root: strings.take()?,
                existing_builder: strings.take()?,
                existing_profile: strings.take()?,
                active_builder: strings.take()?,
                active_profile: strings.take()?,
            }
        }
    };
    let key = strings.finish_key(code.uses_outer_key())?;
    Some((key, reason))
}

diagnostic_family! {
    HTML_SINGLE_FILE_ROUTE_NAME_EMPTY {
        internal: 6002,
        stable: "MOTH-CONFIG-0002",
        category: DiagnosticCategory::Config,
        severity: DiagnosticSeverity::Error,
        title: "Invalid HTML single-file route",
        facts: [entry_path: StringFact],
        labels: [],
        extra: None,
        ctor: generated html_single_file_route_name_empty,
        view: HtmlSingleFileRouteNameEmptyView,
        render: render_html_single_file_route_name_empty,
    }
}

diagnostic_family! {
    PROJECT_ENTRY_EXTENSION_REQUIRED {
        internal: 6003,
        stable: "MOTH-CONFIG-0003",
        category: DiagnosticCategory::Config,
        severity: DiagnosticSeverity::Error,
        title: "Invalid project entry file",
        facts: [entry_path: StringFact],
        labels: [],
        extra: None,
        ctor: generated project_entry_extension_required,
        view: ProjectEntryExtensionRequiredView,
        render: render_project_entry_extension_required,
    }
}

pub(in crate::compiler_frontend::compiler_messages::diagnostics) const FAMILIES: &[FamilyRow] = &[
    INVALID_DEPENDENCY_CLAUSE,
    LEGACY_DEPENDENCY_CLAUSE,
    CIRCULAR_DEPENDENCY,
    UNSUPPORTED_BUILDER_PACKAGE,
    INVALID_PAGE_METADATA,
    INVALID_COMPILE_TIME_PATH,
    DEPENDENCY_NAMESPACE_USED_AS_VALUE,
    NESTED_DEPENDENCY_TRAVERSAL,
    NAMESPACE_TYPE_VALUE_MISUSE,
    PROJECT_CONTEXT_ESCAPE,
    INVALID_CONFIG,
    HTML_SINGLE_FILE_ROUTE_NAME_EMPTY,
    PROJECT_ENTRY_EXTENSION_REQUIRED,
];
