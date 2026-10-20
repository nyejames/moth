//! Family rows for import diagnostics.
//!
//! WHAT: the unused-import (reserved) and missing-import-target pilots, in full.
//! The other 22 rows are identity rows for import families.
//! WHY: every row is in the import band, so they share one group table.

use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::facts::{
    OptStringFact, PathFact, StringFact, UnitFact, unit_code,
};
use crate::compiler_frontend::compiler_messages::diagnostics::labels::LabelMessageCode;
use crate::compiler_frontend::compiler_messages::diagnostics::schema::{
    FamilyRow, diagnostic_family,
};
use crate::compiler_frontend::compiler_messages::render::{
    render_ambiguous_import_target, render_bare_file_import,
    render_cross_module_import_not_exported, render_dependency_alias_case_mismatch,
    render_direct_special_file_import, render_import_name_collision, render_invalid_import_path,
    render_missing_import_target, render_missing_module_root_public_surface,
    render_missing_package_symbol, render_not_exported_by_public_surface,
    render_not_exported_by_source_file,
};
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticCategory, DiagnosticKind, DiagnosticLabelMessage,
    DiagnosticPayload, DiagnosticSeverity, ImportDiagnosticKind, ImportPublicSurfaceType,
    InvalidImportPathReason,
};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathId;
use crate::compiler_frontend::symbols::string_interning::StringId;
unit_code! {
    impl ImportPublicSurfaceType {
        SourcePackage = 1,
        ModuleRoot = 2,
    }
}
unit_code! {
    pub(crate) enum InvalidImportPathCode {
        PublicRoot = 1,
        CurrentDirectorySegment = 2,
        ParentDirectorySegment = 3,
        EscapesProjectRoot = 4,
        EscapesSourcePackageRoot = 5,
        CaseMismatch = 6,
    }
}

impl InvalidImportPathCode {
    /// Returns the compact code corresponding to a legacy invalid-import-path reason.
    pub(crate) fn of(reason: &InvalidImportPathReason) -> Self {
        match reason {
            InvalidImportPathReason::PublicRoot => Self::PublicRoot,
            InvalidImportPathReason::CurrentDirectorySegment => Self::CurrentDirectorySegment,
            InvalidImportPathReason::ParentDirectorySegment => Self::ParentDirectorySegment,
            InvalidImportPathReason::EscapesProjectRoot => Self::EscapesProjectRoot,
            InvalidImportPathReason::EscapesSourcePackageRoot => Self::EscapesSourcePackageRoot,
            InvalidImportPathReason::CaseMismatch { .. } => Self::CaseMismatch,
        }
    }

    /// Splits the data-bearing case-mismatch reason into its two optional string facts.
    pub(crate) fn split(
        reason: &InvalidImportPathReason,
    ) -> (Self, Option<StringId>, Option<StringId>) {
        match reason {
            InvalidImportPathReason::CaseMismatch { provided, expected } => {
                (Self::CaseMismatch, Some(*provided), Some(*expected))
            }
            _ => (Self::of(reason), None, None),
        }
    }

    /// Rebuilds the legacy reason for its existing prose helper.
    pub(crate) fn to_reason(
        self,
        provided: Option<StringId>,
        expected: Option<StringId>,
    ) -> Option<InvalidImportPathReason> {
        Some(match self {
            Self::PublicRoot => InvalidImportPathReason::PublicRoot,
            Self::CurrentDirectorySegment => InvalidImportPathReason::CurrentDirectorySegment,
            Self::ParentDirectorySegment => InvalidImportPathReason::ParentDirectorySegment,
            Self::EscapesProjectRoot => InvalidImportPathReason::EscapesProjectRoot,
            Self::EscapesSourcePackageRoot => InvalidImportPathReason::EscapesSourcePackageRoot,
            Self::CaseMismatch => InvalidImportPathReason::CaseMismatch {
                provided: provided?,
                expected: expected?,
            },
        })
    }
}

diagnostic_family! {
    pilot UNUSED_IMPORT {
        internal: 4002,
        stable: "MOTH-IMPORT-0002",
        category: DiagnosticCategory::Import,
        severity: DiagnosticSeverity::Warning,
        title: "Unused dependency binding",
        facts: [],
        labels: [],
        extra: None,
        ctor: manual,
        #[allow(dead_code, reason = "reserved (D9): no producer or renderer builds this view")]
        view: UnusedImportView,
        render: none,
        reserved,
    }
}

/// The unused-import draft. No production path produces it yet, so only tests construct it.
#[cfg(test)]
pub(in crate::compiler_frontend::compiler_messages::diagnostics) fn unused_import(
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    DiagnosticDraft::with_facts(&UNUSED_IMPORT, span, |_cursor| Ok(()))
}

diagnostic_family! {
    pilot MISSING_IMPORT_TARGET {
        internal: 4005,
        stable: "MOTH-IMPORT-0005",
        category: DiagnosticCategory::Import,
        severity: DiagnosticSeverity::Error,
        title: "Missing dependency target",
        facts: [path: PathFact],
        labels: [],
        extra: None,
        ctor: generated missing_import_target,
        view: MissingImportTargetView,
        render: render_missing_import_target,
    }
}

/// Maps legacy import diagnostics onto their compact constructors.
pub(super) fn from_legacy(diagnostic: &CompilerDiagnostic) -> Option<DiagnosticDraft> {
    match (diagnostic.kind, &diagnostic.payload) {
        (
            DiagnosticKind::Import(ImportDiagnosticKind::MissingImportTarget),
            DiagnosticPayload::MissingImportTarget { path },
        ) => Some(missing_import_target(*path, diagnostic.primary_span)),
        (
            DiagnosticKind::Import(ImportDiagnosticKind::BareFileImport),
            DiagnosticPayload::BareFileImport { path },
        ) => Some(bare_file_import(*path, diagnostic.primary_span)),
        (
            DiagnosticKind::Import(ImportDiagnosticKind::AmbiguousImportTarget),
            DiagnosticPayload::AmbiguousImportTarget { path },
        ) => Some(ambiguous_import_target(*path, diagnostic.primary_span)),
        (
            DiagnosticKind::Import(ImportDiagnosticKind::DirectSpecialFileImport),
            DiagnosticPayload::DirectSpecialFileImport { path },
        ) => Some(direct_special_file_import(*path, diagnostic.primary_span)),
        (
            DiagnosticKind::Import(ImportDiagnosticKind::DependencyAliasCaseMismatch),
            DiagnosticPayload::DependencyAliasCaseMismatch { alias, symbol },
        ) => Some(dependency_alias_case_mismatch(
            *alias,
            *symbol,
            diagnostic.primary_span,
        )),
        (
            DiagnosticKind::Import(ImportDiagnosticKind::ImportNameCollision),
            DiagnosticPayload::ImportNameCollision { name },
        ) => {
            // The outer legacy adapter copies the existing labels after mapping the payload facts.
            Some(import_name_collision(*name, None, diagnostic.primary_span))
        }
        (
            DiagnosticKind::Import(ImportDiagnosticKind::NotExportedBySourceFile),
            DiagnosticPayload::NotExportedBySourceFile { symbol_path },
        ) => Some(not_exported_by_source_file(
            *symbol_path,
            diagnostic.primary_span,
        )),
        (
            DiagnosticKind::Import(ImportDiagnosticKind::NotExportedByPublicSurface),
            DiagnosticPayload::NotExportedByPublicSurface {
                requested_path,
                public_surface_name,
                public_surface_type,
            },
        ) => Some(not_exported_by_public_surface(
            *requested_path,
            *public_surface_name,
            *public_surface_type,
            diagnostic.primary_span,
        )),
        (
            DiagnosticKind::Import(ImportDiagnosticKind::MissingModuleRootPublicSurface),
            DiagnosticPayload::MissingModuleRootPublicSurface { symbol_path },
        ) => Some(missing_module_root_public_surface(
            *symbol_path,
            diagnostic.primary_span,
        )),
        (
            DiagnosticKind::Import(ImportDiagnosticKind::MissingPackageSymbol),
            DiagnosticPayload::MissingPackageSymbol {
                symbol,
                package_path,
            },
        ) => Some(missing_package_symbol(
            *symbol,
            *package_path,
            diagnostic.primary_span,
        )),
        (
            DiagnosticKind::Import(ImportDiagnosticKind::CrossModuleImportNotExported),
            DiagnosticPayload::CrossModuleImportNotExported { symbol_path },
        ) => Some(cross_module_import_not_exported(
            *symbol_path,
            diagnostic.primary_span,
        )),
        (
            DiagnosticKind::Import(ImportDiagnosticKind::InvalidImportPath),
            DiagnosticPayload::InvalidImportPath { path, reason },
        ) => Some(invalid_import_path(*path, *reason, diagnostic.primary_span)),
        _ => None,
    }
}

diagnostic_family! {
    pilot DEPENDENCY_ALIAS_CASE_MISMATCH {
        internal: 4003,
        stable: "MOTH-IMPORT-0003",
        category: DiagnosticCategory::Import,
        severity: DiagnosticSeverity::Warning,
        title: "Dependency alias case mismatch",
        facts: [alias: StringFact, symbol: StringFact],
        labels: [],
        extra: None,
        ctor: generated dependency_alias_case_mismatch,
        view: DependencyAliasCaseMismatchView,
        render: render_dependency_alias_case_mismatch,
    }
}

diagnostic_family! {
    pilot AMBIGUOUS_IMPORT_TARGET {
        internal: 4006,
        stable: "MOTH-IMPORT-0006",
        category: DiagnosticCategory::Import,
        severity: DiagnosticSeverity::Error,
        title: "Ambiguous dependency target",
        facts: [path: PathFact],
        labels: [],
        extra: None,
        ctor: generated ambiguous_import_target,
        view: AmbiguousImportTargetView,
        render: render_ambiguous_import_target,
    }
}

diagnostic_family! {
    pilot BARE_FILE_IMPORT {
        internal: 4007,
        stable: "MOTH-IMPORT-0007",
        category: DiagnosticCategory::Import,
        severity: DiagnosticSeverity::Error,
        title: "Bare file dependency",
        facts: [path: PathFact],
        labels: [],
        extra: None,
        ctor: generated bare_file_import,
        view: BareFileImportView,
        render: render_bare_file_import,
    }
}

diagnostic_family! {
    pilot DIRECT_SPECIAL_FILE_IMPORT {
        internal: 4008,
        stable: "MOTH-IMPORT-0008",
        category: DiagnosticCategory::Import,
        severity: DiagnosticSeverity::Error,
        title: "Direct special-file dependency",
        facts: [path: PathFact],
        labels: [],
        extra: None,
        ctor: generated direct_special_file_import,
        view: DirectSpecialFileImportView,
        render: render_direct_special_file_import,
    }
}

diagnostic_family! {
    pilot IMPORT_NAME_COLLISION {
        internal: 4009,
        stable: "MOTH-IMPORT-0009",
        category: DiagnosticCategory::Import,
        severity: DiagnosticSeverity::Error,
        title: "Dependency binding name collision",
        facts: [name: StringFact],
        labels: [LabelMessageCode::PREVIOUS_DECLARATION],
        extra: None,
        ctor: manual,
        view: ImportNameCollisionView,
        render: render_import_name_collision,
    }
}

/// Stores a colliding binding name and its prior-declaration label.
pub(in crate::compiler_frontend::compiler_messages::diagnostics) fn import_name_collision(
    name: StringId,
    previous_span: Option<SourceSpan>,
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    let draft = DiagnosticDraft::with_facts(&IMPORT_NAME_COLLISION, span, |cursor| {
        cursor.push::<StringFact>(&name)?;
        Ok(())
    });
    match previous_span {
        Some(previous_span) => draft.with_label(
            Some(previous_span),
            Some(DiagnosticLabelMessage::PreviousDeclaration),
        ),
        None => draft,
    }
}

diagnostic_family! {
    pilot NOT_EXPORTED_BY_SOURCE_FILE {
        internal: 4010,
        stable: "MOTH-IMPORT-0010",
        category: DiagnosticCategory::Import,
        severity: DiagnosticSeverity::Error,
        title: "Not exported by source file",
        facts: [symbol_path: PathFact],
        labels: [],
        extra: None,
        ctor: generated not_exported_by_source_file,
        view: NotExportedBySourceFileView,
        render: render_not_exported_by_source_file,
    }
}

diagnostic_family! {
    pilot NOT_EXPORTED_BY_PUBLIC_SURFACE {
        internal: 4011,
        stable: "MOTH-IMPORT-0011",
        category: DiagnosticCategory::Import,
        severity: DiagnosticSeverity::Error,
        title: "Not exported by public surface",
        facts: [
            requested_path: PathFact,
            public_surface_name: StringFact,
            public_surface_type: UnitFact<ImportPublicSurfaceType>,
        ],
        labels: [],
        extra: None,
        ctor: generated not_exported_by_public_surface,
        view: NotExportedByPublicSurfaceView,
        render: render_not_exported_by_public_surface,
    }
}

diagnostic_family! {
    pilot MISSING_MODULE_ROOT_PUBLIC_SURFACE {
        internal: 4012,
        stable: "MOTH-IMPORT-0012",
        category: DiagnosticCategory::Import,
        severity: DiagnosticSeverity::Error,
        title: "Missing module-root public surface",
        facts: [symbol_path: PathFact],
        labels: [],
        extra: None,
        ctor: generated missing_module_root_public_surface,
        view: MissingModuleRootPublicSurfaceView,
        render: render_missing_module_root_public_surface,
    }
}

diagnostic_family! {
    pilot MISSING_PACKAGE_SYMBOL {
        internal: 4013,
        stable: "MOTH-IMPORT-0013",
        category: DiagnosticCategory::Import,
        severity: DiagnosticSeverity::Error,
        title: "Missing package symbol",
        facts: [symbol: StringFact, package_path: StringFact],
        labels: [],
        extra: None,
        ctor: generated missing_package_symbol,
        view: MissingPackageSymbolView,
        render: render_missing_package_symbol,
    }
}

diagnostic_family! {
    pilot CROSS_MODULE_IMPORT_NOT_EXPORTED {
        internal: 4015,
        stable: "MOTH-IMPORT-0015",
        category: DiagnosticCategory::Import,
        severity: DiagnosticSeverity::Error,
        title: "Cross-module dependency not exported",
        facts: [symbol_path: PathFact],
        labels: [],
        extra: None,
        ctor: generated cross_module_import_not_exported,
        view: CrossModuleImportNotExportedView,
        render: render_cross_module_import_not_exported,
    }
}

diagnostic_family! {
    pilot INVALID_IMPORT_PATH {
        internal: 4016,
        stable: "MOTH-IMPORT-0016",
        category: DiagnosticCategory::Import,
        severity: DiagnosticSeverity::Error,
        title: "Invalid dependency path",
        facts: [
            path: PathFact,
            reason: UnitFact<InvalidImportPathCode>,
            provided: OptStringFact,
            expected: OptStringFact,
        ],
        labels: [],
        extra: None,
        ctor: manual,
        view: InvalidImportPathView,
        render: render_invalid_import_path,
    }
}

/// Stores an invalid path reason code and its optional case-mismatch names.
pub(in crate::compiler_frontend::compiler_messages::diagnostics) fn invalid_import_path(
    path: PathId,
    reason: InvalidImportPathReason,
    span: Option<SourceSpan>,
) -> DiagnosticDraft {
    let (reason, provided, expected) = InvalidImportPathCode::split(&reason);
    DiagnosticDraft::with_facts(&INVALID_IMPORT_PATH, span, |cursor| {
        cursor.push::<PathFact>(&path)?;
        cursor.push::<UnitFact<InvalidImportPathCode>>(&reason)?;
        cursor.push::<OptStringFact>(&provided)?;
        cursor.push::<OptStringFact>(&expected)?;
        Ok(())
    })
}

diagnostic_family! {
    identity DIRECT_SYMBOL_PATH_IMPORT {
        internal: 4017,
        stable: "MOTH-IMPORT-0017",
        category: DiagnosticCategory::Import,
    }
}

diagnostic_family! {
    identity INVALID_NAMESPACE_DEFAULT_NAME {
        internal: 4018,
        stable: "MOTH-IMPORT-0018",
        category: DiagnosticCategory::Import,
    }
}

diagnostic_family! {
    identity DUPLICATE_IMPORT_SURFACE_MEMBER {
        internal: 4019,
        stable: "MOTH-IMPORT-0019",
        category: DiagnosticCategory::Import,
    }
}

diagnostic_family! {
    identity EXPLICIT_MOTH_EXTENSION {
        internal: 4020,
        stable: "MOTH-IMPORT-0020",
        category: DiagnosticCategory::Import,
    }
}

diagnostic_family! {
    identity UNSUPPORTED_EXTERNAL_EXTENSION {
        internal: 4021,
        stable: "MOTH-IMPORT-0021",
        category: DiagnosticCategory::Import,
    }
}

diagnostic_family! {
    identity INVALID_EXTERNAL_MODULE {
        internal: 4022,
        stable: "MOTH-IMPORT-0022",
        category: DiagnosticCategory::Import,
    }
}

diagnostic_family! {
    identity EXPLICIT_SOURCE_EXTENSION {
        internal: 4024,
        stable: "MOTH-IMPORT-0024",
        category: DiagnosticCategory::Import,
    }
}

diagnostic_family! {
    identity UNSUPPORTED_SOURCE_FILE_KIND {
        internal: 4025,
        stable: "MOTH-IMPORT-0025",
        category: DiagnosticCategory::Import,
    }
}

diagnostic_family! {
    identity INVALID_SOURCE_FILE_ENTRY {
        internal: 4026,
        stable: "MOTH-IMPORT-0026",
        category: DiagnosticCategory::Import,
    }
}

diagnostic_family! {
    identity DUPLICATE_MOTH_TEMPLATE_INPUT_PATH {
        internal: 4028,
        stable: "MOTH-IMPORT-0028",
        category: DiagnosticCategory::Import,
    }
}

diagnostic_family! {
    identity MOTH_TEMPLATE_INPUTS_SHARE_NO_COMMON_ANCESTOR {
        internal: 4029,
        stable: "MOTH-IMPORT-0029",
        category: DiagnosticCategory::Import,
    }
}

pub(in crate::compiler_frontend::compiler_messages::diagnostics) const FAMILIES: &[FamilyRow] = &[
    UNUSED_IMPORT,
    MISSING_IMPORT_TARGET,
    DEPENDENCY_ALIAS_CASE_MISMATCH,
    AMBIGUOUS_IMPORT_TARGET,
    BARE_FILE_IMPORT,
    DIRECT_SPECIAL_FILE_IMPORT,
    IMPORT_NAME_COLLISION,
    NOT_EXPORTED_BY_SOURCE_FILE,
    NOT_EXPORTED_BY_PUBLIC_SURFACE,
    MISSING_MODULE_ROOT_PUBLIC_SURFACE,
    MISSING_PACKAGE_SYMBOL,
    CROSS_MODULE_IMPORT_NOT_EXPORTED,
    INVALID_IMPORT_PATH,
    DIRECT_SYMBOL_PATH_IMPORT,
    INVALID_NAMESPACE_DEFAULT_NAME,
    DUPLICATE_IMPORT_SURFACE_MEMBER,
    EXPLICIT_MOTH_EXTENSION,
    UNSUPPORTED_EXTERNAL_EXTENSION,
    INVALID_EXTERNAL_MODULE,
    EXPLICIT_SOURCE_EXTENSION,
    UNSUPPORTED_SOURCE_FILE_KIND,
    INVALID_SOURCE_FILE_ENTRY,
    DUPLICATE_MOTH_TEMPLATE_INPUT_PATH,
    MOTH_TEMPLATE_INPUTS_SHARE_NO_COMMON_ANCESTOR,
];
