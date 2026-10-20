//! Family rows for import diagnostics.
//!
//! WHAT: the unused-import (reserved) and missing-import-target pilots, in full.
//! The other 22 rows are identity rows for import families.
//! WHY: every row is in the import band, so they share one group table.

use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::facts::PathFact;
use crate::compiler_frontend::compiler_messages::diagnostics::schema::{
    FamilyRow, diagnostic_family,
};
use crate::compiler_frontend::compiler_messages::render::render_missing_import_target;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticCategory, DiagnosticKind, DiagnosticPayload, DiagnosticSeverity,
    ImportDiagnosticKind,
};
#[cfg(test)]
use crate::compiler_frontend::source::SourceSpan;

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
pub(crate) fn unused_import(span: Option<SourceSpan>) -> DiagnosticDraft {
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

/// Maps the legacy missing-import-target diagnostic onto its compact constructor. Declines other payloads.
pub(super) fn from_legacy(diagnostic: &CompilerDiagnostic) -> Option<DiagnosticDraft> {
    match (diagnostic.kind, &diagnostic.payload) {
        (
            DiagnosticKind::Import(ImportDiagnosticKind::MissingImportTarget),
            DiagnosticPayload::MissingImportTarget { path },
        ) => Some(missing_import_target(*path, diagnostic.primary_span)),
        _ => None,
    }
}

diagnostic_family! {
    identity DEPENDENCY_ALIAS_CASE_MISMATCH {
        internal: 4003,
        stable: "MOTH-IMPORT-0003",
        category: DiagnosticCategory::Import,
    }
}

diagnostic_family! {
    identity AMBIGUOUS_IMPORT_TARGET {
        internal: 4006,
        stable: "MOTH-IMPORT-0006",
        category: DiagnosticCategory::Import,
    }
}

diagnostic_family! {
    identity BARE_FILE_IMPORT {
        internal: 4007,
        stable: "MOTH-IMPORT-0007",
        category: DiagnosticCategory::Import,
    }
}

diagnostic_family! {
    identity DIRECT_SPECIAL_FILE_IMPORT {
        internal: 4008,
        stable: "MOTH-IMPORT-0008",
        category: DiagnosticCategory::Import,
    }
}

diagnostic_family! {
    identity IMPORT_NAME_COLLISION {
        internal: 4009,
        stable: "MOTH-IMPORT-0009",
        category: DiagnosticCategory::Import,
    }
}

diagnostic_family! {
    identity NOT_EXPORTED_BY_SOURCE_FILE {
        internal: 4010,
        stable: "MOTH-IMPORT-0010",
        category: DiagnosticCategory::Import,
    }
}

diagnostic_family! {
    identity NOT_EXPORTED_BY_PUBLIC_SURFACE {
        internal: 4011,
        stable: "MOTH-IMPORT-0011",
        category: DiagnosticCategory::Import,
    }
}

diagnostic_family! {
    identity MISSING_MODULE_ROOT_PUBLIC_SURFACE {
        internal: 4012,
        stable: "MOTH-IMPORT-0012",
        category: DiagnosticCategory::Import,
    }
}

diagnostic_family! {
    identity MISSING_PACKAGE_SYMBOL {
        internal: 4013,
        stable: "MOTH-IMPORT-0013",
        category: DiagnosticCategory::Import,
    }
}

diagnostic_family! {
    identity CROSS_MODULE_IMPORT_NOT_EXPORTED {
        internal: 4015,
        stable: "MOTH-IMPORT-0015",
        category: DiagnosticCategory::Import,
    }
}

diagnostic_family! {
    identity INVALID_IMPORT_PATH {
        internal: 4016,
        stable: "MOTH-IMPORT-0016",
        category: DiagnosticCategory::Import,
    }
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

pub(crate) const FAMILIES: &[FamilyRow] = &[
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
