//! Cold extra data for diagnostics with labels, foreign sites or string lists.
//!
//! WHAT: a record with extra data points at a `DiagnosticExtraRecord`, which holds ranges into the label,
//! foreign-site and string-id arenas. The draft types that carry this data before compaction live here too.
//! WHY: the hot record stays 32 bytes, and diagnostics without extra data pay nothing for these arenas.

use super::facts::{FactCapacityError, IdRemap};
use super::foreign::ForeignSiteDraft;
use super::labels::{DraftLabel, DraftSubstitution};
use crate::compiler_frontend::symbols::string_interning::StringId;
use std::num::NonZeroU32;
use std::ops::Range;

/// A range into one arena. `start` and `len` count entries, not bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct DiagnosticListRange {
    start: u32,
    len: u32,
}

impl DiagnosticListRange {
    /// Compaction checks every arena against its u32-sized limit before it builds a range from its offsets.
    pub(super) fn new(start: usize, len: usize) -> Self {
        Self {
            start: u32::try_from(start)
                .expect("compaction checked the arena offset against its limit"),
            len: u32::try_from(len).expect("compaction checked the arena length against its limit"),
        }
    }

    pub(super) fn range(self) -> Range<usize> {
        let start = self.start as usize;
        start..start + self.len as usize
    }
}

/// The kind of list an extra record's data points at.
#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ExtraDataKind {
    None = 0,
    StringIds = 1,
}

/// An extra record's data: the kind of list it holds and where that list lives.
#[derive(Clone, Copy, Debug)]
pub(super) struct ExtraDataRef {
    pub(super) kind: ExtraDataKind,
    pub(super) list: DiagnosticListRange,
}

const _: () = assert!(
    size_of::<ExtraDataRef>() == 12,
    "extra data refs are locked at 12 bytes"
);

/// Cold per-diagnostic data, reached through a record's extra id.
#[derive(Clone, Copy, Debug)]
pub(super) struct DiagnosticExtraRecord {
    pub(super) labels: DiagnosticListRange,
    pub(super) foreign_sites: DiagnosticListRange,
    pub(super) data: ExtraDataRef,
}

const _: () = assert!(
    size_of::<DiagnosticExtraRecord>() == 28,
    "extra records are locked at 28 bytes"
);

/// A one-based index into the extra-record arena. Records store it as an `Option`, so zero means no extra data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct DiagnosticExtraId(NonZeroU32);

impl DiagnosticExtraId {
    /// Names the extra record at zero-based `index`. Compaction keeps the arena below its u32 limit.
    pub(super) fn from_index(index: usize) -> Self {
        let one_based =
            u32::try_from(index + 1).expect("compaction keeps the extra arena below its u32 limit");
        Self(NonZeroU32::new(one_based).expect("one-based indices are never zero"))
    }

    pub(super) fn index(self) -> usize {
        (self.0.get() - 1) as usize
    }
}

/// Extra data a draft carries besides its labels and foreign sites.
#[derive(Debug, Default)]
pub(super) enum ExtraDataDraft {
    #[default]
    None,
    StringIds(Vec<StringId>),
}

/// The cold half of a draft. It is boxed inside `DiagnosticDraft`, so diagnostics without extras stay small.
#[derive(Debug, Default)]
pub(super) struct DiagnosticExtraDraft {
    pub(super) labels: Vec<DraftLabel>,
    pub(super) foreign_sites: Vec<ForeignSiteDraft>,
    /// Entries of generic substitution labels. Each such label's data names a run of these entries.
    pub(super) substitutions: Vec<DraftSubstitution>,
    pub(super) data: ExtraDataDraft,
}

impl ExtraDataDraft {
    /// Rewrites the string IDs of a string-list extra. A path remap leaves them as they are.
    pub(super) fn remap_ids(&mut self, remap: IdRemap<'_>) {
        if let (Self::StringIds(ids), IdRemap::Strings(strings)) = (self, remap) {
            for id in ids {
                *id = strings.get(*id);
            }
        }
    }
}

impl DiagnosticExtraDraft {
    /// Rewrites the identity words of this extra part: label and substitution words, then string IDs.
    /// Foreign sites are not remapped, since their identity belongs to another context.
    pub(super) fn remap_ids(&mut self, remap: IdRemap<'_>) -> Result<(), FactCapacityError> {
        for label in &mut self.labels {
            label.remap_ids(remap)?;
        }
        for substitution in &mut self.substitutions {
            substitution.remap_ids(remap)?;
        }
        self.data.remap_ids(remap);
        Ok(())
    }
}
