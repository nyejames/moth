//! Fixed 32-byte diagnostic records.
//!
//! WHAT: a record holds its stable code, a severity override, the primary span, four fact words and the id of
//! its optional extra record.
//! WHY: one flat record per diagnostic keeps the hot path small. Labels, foreign sites and string lists live in
//! side arenas and are reached through the extra id, so diagnostics without them pay nothing for those arenas.

use super::extras::DiagnosticExtraId;
use super::facts::FACT_WORDS;
use crate::compiler_frontend::compiler_messages::DiagnosticSeverity;
use crate::compiler_frontend::source::SourceSpan;
use std::num::NonZeroU16;

/// A stable internal diagnostic code. Zero is never a valid code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DiagnosticCode(NonZeroU16);

impl DiagnosticCode {
    /// Panics on zero. Family rows are checked at compile time, so this panic is unreachable for real rows.
    pub(crate) const fn new(code: u16) -> Self {
        match NonZeroU16::new(code) {
            Some(code) => Self(code),
            None => panic!("diagnostic codes are never zero"),
        }
    }

    pub(crate) const fn get(self) -> u16 {
        self.0.get()
    }
}

/// A per-record severity override. `Default` keeps the family descriptor's severity.
#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum SeverityOverride {
    #[default]
    Default = 0,
    Error = 1,
    Warning = 2,
    Note = 3,
}

impl SeverityOverride {
    /// The override that stores `severity` on a family whose default is `default`. A severity equal to the default
    /// stores `Default`, so the record keeps the descriptor's severity.
    pub(super) fn for_severity(severity: DiagnosticSeverity, default: DiagnosticSeverity) -> Self {
        if severity == default {
            return Self::Default;
        }
        match severity {
            DiagnosticSeverity::Error => Self::Error,
            DiagnosticSeverity::Warning => Self::Warning,
            DiagnosticSeverity::Note => Self::Note,
        }
    }

    /// The severity a record reports: the stored override, or the family default when no override is stored.
    pub(super) fn resolve(self, default: DiagnosticSeverity) -> DiagnosticSeverity {
        match self {
            Self::Default => default,
            Self::Error => DiagnosticSeverity::Error,
            Self::Warning => DiagnosticSeverity::Warning,
            Self::Note => DiagnosticSeverity::Note,
        }
    }
}

/// One compact diagnostic. Its size is locked at 32 bytes: the fields take 31 bytes, and
/// the tail pads them to the 4-byte alignment of the span and fact words.
#[derive(Debug)]
pub(crate) struct DiagnosticRecord {
    pub(super) code: DiagnosticCode,
    pub(super) severity: SeverityOverride,
    pub(super) primary: Option<SourceSpan>,
    pub(super) facts: [u32; FACT_WORDS],
    pub(super) extra: Option<DiagnosticExtraId>,
}

impl DiagnosticRecord {
    /// Builds a record. `severity` is the draft's override, and `Default` keeps the family's severity.
    pub(super) fn new(
        code: DiagnosticCode,
        severity: SeverityOverride,
        primary: Option<SourceSpan>,
        facts: [u32; FACT_WORDS],
        extra: Option<DiagnosticExtraId>,
    ) -> Self {
        Self {
            code,
            severity,
            primary,
            facts,
            extra,
        }
    }
}

const _: () = assert!(
    size_of::<DiagnosticRecord>() == 32,
    "diagnostic records are locked at 32 bytes"
);
const _: () = assert!(
    size_of::<Option<SourceSpan>>() == 8,
    "the primary span must keep its niche"
);
