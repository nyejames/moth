//! Borrowed read-only view over one stored compact diagnostic record.
//!
//! WHAT: pairs a stored record with its family row and the frozen type-display snapshot that the
//! record's type facts index into. Family views decode the record's facts into named fields.
//! WHY: render functions read typed facts instead of the legacy payload enum, so the compact
//! record is the source of truth for every compact-rendered diagnostic. The view owns no data and
//! is built per render call.

use super::foreign::SiteRef;
use super::labels::DurableLabel;
use super::registry::family_for_code;
use super::schema::FamilyRow;
use super::store::{DiagnosticId, DiagnosticStore, RecordRef};
use crate::compiler_frontend::compiler_messages::DiagnosticSeverity;
use crate::compiler_frontend::compiler_messages::diagnostic_types::type_display_snapshot::TypeDisplaySnapshot;
use crate::compiler_frontend::symbols::string_interning::StringId;

/// Decodes one stored record into a family's typed facts.
///
/// WHAT: every family view struct implements this trait. Decoding returns `None` when the record
/// belongs to a different family.
/// WHY: render functions receive named, decoded fields, and a wrong-family read fails loudly at the
/// dispatch boundary rather than producing a silently wrong message.
pub(super) trait FamilyView: Sized {
    fn from_record(record: RecordRef<'_>) -> Option<Self>;
}

#[derive(Clone, Copy)]
pub(crate) struct DiagnosticView<'a> {
    record: RecordRef<'a>,
    family: &'static FamilyRow,
    types: &'a TypeDisplaySnapshot,
}

impl<'a> DiagnosticView<'a> {
    pub(super) fn new(
        store: &'a DiagnosticStore,
        id: DiagnosticId,
        types: &'a TypeDisplaySnapshot,
    ) -> Self {
        let record = store.record(id);
        let family = family_for_code(record.code())
            .expect("compaction only stores records whose code names a family row");
        Self {
            record,
            family,
            types,
        }
    }

    pub(crate) fn family(&self) -> &'static FamilyRow {
        self.family
    }

    pub(crate) fn types(&self) -> &'a TypeDisplaySnapshot {
        self.types
    }

    pub(crate) fn string_ids(&self) -> &'a [StringId] {
        self.record.string_ids()
    }

    /// The severity the record reports: its override, or the family default when no override is stored.
    pub(crate) fn severity(&self) -> DiagnosticSeverity {
        self.record
            .severity()
            .resolve(self.family.default_severity())
    }

    /// The primary site: foreign when the record names a foreign primary, otherwise local.
    pub(crate) fn primary_site(&self) -> SiteRef<'a> {
        self.record.primary()
    }

    /// The secondary labels in the order the draft added them, each with its site and decoded message.
    pub(crate) fn durable_labels(self) -> impl Iterator<Item = DurableLabel<'a>> {
        self.record.durable_labels()
    }

    pub(super) fn family_view<View: FamilyView>(&self) -> Option<View> {
        View::from_record(self.record)
    }
}
