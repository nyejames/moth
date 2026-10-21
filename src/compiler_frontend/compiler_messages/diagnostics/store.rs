//! The compact store of compacted diagnostics: fixed-size records plus shared arenas for their cold parts.
//!
//! WHAT: each compacted draft becomes one `DiagnosticRecord`. Labels, foreign sites, string lists and generic
//! substitutions live in arenas that a record's extra data reaches through index ranges.
//! WHY: a diagnostic costs one record push and no heap allocation of its own. The arenas only grow by appending, so
//! an index or range keeps its meaning once it has been handed out.

use super::extras::{DiagnosticExtraRecord, ExtraDataKind};
use super::facts::FactReader;
use super::foreign::{ForeignSiteRecord, SiteRef};
use super::labels::{DurableLabel, GenericSubstitutionRecord, SecondaryDiagnosticLabel};
use super::record::{DiagnosticCode, DiagnosticRecord, SeverityOverride};
use super::resource::DiagnosticLimits;
use crate::compiler_frontend::compiler_messages::DiagnosticCapacityResource;
use crate::compiler_frontend::source::{FrozenIdentityHandle, SourceSpan};
use crate::compiler_frontend::symbols::string_interning::StringId;

/// Selector of a record's primary foreign site. A label's foreign site uses `label index + 1`, so the two never
/// collide and a record's primary is always found under selector zero.
pub(super) const PRIMARY_SELECTOR: u32 = 0;

/// The compacted diagnostics of one compilation. Ids are positions in push order and never change.
#[derive(Debug)]
pub(super) struct DiagnosticStore {
    pub(super) records: Vec<DiagnosticRecord>,
    pub(super) extras: Vec<DiagnosticExtraRecord>,
    pub(super) labels: Vec<SecondaryDiagnosticLabel>,
    /// Two-word rows for labels whose code carries data: a string, a type display, a substitution run or an origin.
    pub(super) label_data: Vec<[u32; 2]>,
    pub(super) string_ids: Vec<StringId>,
    pub(super) substitutions: Vec<GenericSubstitutionRecord>,
    pub(super) foreign_sites: Vec<ForeignSiteRecord>,
    pub(super) identity_contexts: Vec<FrozenIdentityHandle>,
    pub(super) limits: DiagnosticLimits,
    /// The resource whose ceiling latched the store. Once set, every later compaction is rejected.
    pub(super) full: Option<DiagnosticCapacityResource>,
}

/// The position of a stored record. Compaction only appends, so an id never changes meaning.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct DiagnosticId(u32);

impl DiagnosticId {
    /// `index` is below the record limit, which keeps it inside `u32`.
    pub(super) fn from_index(index: usize) -> Self {
        Self(u32::try_from(index).expect("the record limit keeps every id inside u32"))
    }

    pub(super) fn index(self) -> usize {
        self.0 as usize
    }
}

impl DiagnosticStore {
    /// An empty store with production ceilings.
    #[cfg(test)]
    pub(super) fn new() -> Self {
        Self::with_limits(DiagnosticLimits::production())
    }

    /// An empty store with the given ceilings. A store needs room for a draft and for the capacity record that
    /// replaces it, so it must allow at least two records.
    pub(super) fn with_limits(limits: DiagnosticLimits) -> Self {
        assert!(
            limits.records >= 2,
            "a store must hold a draft and the capacity record that replaces it"
        );
        Self {
            records: Vec::new(),
            extras: Vec::new(),
            labels: Vec::new(),
            label_data: Vec::new(),
            string_ids: Vec::new(),
            substitutions: Vec::new(),
            foreign_sites: Vec::new(),
            identity_contexts: Vec::new(),
            limits,
            full: None,
        }
    }

    /// The number of records the store holds.
    #[cfg_attr(not(test), allow(dead_code, reason = "P4.5 S3 boundary cut"))]
    pub(super) fn len(&self) -> usize {
        self.records.len()
    }

    /// The record at `id`, read through the store's arenas.
    pub(super) fn record(&self, id: DiagnosticId) -> RecordRef<'_> {
        RecordRef {
            store: self,
            record: &self.records[id.index()],
        }
    }
}

/// A stored record with the store that owns its arenas. Copyable, so views can pass it along freely.
#[derive(Clone, Copy, Debug)]
pub(super) struct RecordRef<'a> {
    store: &'a DiagnosticStore,
    record: &'a DiagnosticRecord,
}

impl<'a> RecordRef<'a> {
    pub(super) fn code(self) -> DiagnosticCode {
        self.record.code
    }

    /// The severity override the record stores. `Default` keeps the family descriptor's severity.
    pub(super) fn severity(self) -> SeverityOverride {
        self.record.severity
    }

    /// A reader over the fact words. Each view reads the words its family declares, in layout order.
    pub(super) fn facts(self) -> FactReader<'a> {
        FactReader::new(&self.record.facts)
    }

    /// The primary site: foreign when a foreign-site row names the primary selector, otherwise local. The span stays
    /// in the record either way, so a foreign primary reads its span from the record.
    pub(super) fn primary(self) -> SiteRef<'a> {
        self.site(self.record.primary, PRIMARY_SELECTOR)
    }

    /// The site of secondary label `index`: foreign when a foreign-site row names the label, otherwise local. The span
    /// stays in the label either way.
    pub(super) fn label_site(self, index: usize) -> SiteRef<'a> {
        let span = self.labels().get(index).and_then(|label| label.span());
        self.site(span, index as u32 + 1)
    }

    /// Pairs an in-place span with its foreign-site row. A row with no span has nothing to point at, so it reads as
    /// absent.
    fn site(self, span: Option<SourceSpan>, selector: u32) -> SiteRef<'a> {
        match (span, self.foreign_site(selector)) {
            (Some(span), Some(site)) => SiteRef::Foreign {
                span,
                context: &self.store.identity_contexts[site.context as usize],
            },
            (Some(span), None) => SiteRef::Local(span),
            (None, _) => SiteRef::Absent,
        }
    }

    /// The secondary labels, in the order the draft added them.
    pub(super) fn labels(self) -> &'a [SecondaryDiagnosticLabel] {
        match self.extra() {
            Some(extra) => &self.store.labels[extra.labels.range()],
            None => &[],
        }
    }

    /// The secondary labels in the order the draft added them, each with its site and decoded message.
    pub(super) fn durable_labels(self) -> impl Iterator<Item = DurableLabel<'a>> {
        self.labels()
            .iter()
            .enumerate()
            .map(move |(index, label)| DurableLabel {
                spanned: label.span().is_some(),
                site: self.label_site(index),
                message: label.message(self.store),
            })
    }

    /// The string list a family declares as extra data, or an empty slice.
    pub(super) fn string_ids(self) -> &'a [StringId] {
        let Some(extra) = self.extra() else {
            return &[];
        };
        match extra.data.kind {
            ExtraDataKind::StringIds => &self.store.string_ids[extra.data.list.range()],
            ExtraDataKind::None => &[],
        }
    }

    fn extra(self) -> Option<&'a DiagnosticExtraRecord> {
        self.record.extra.map(|id| &self.store.extras[id.index()])
    }

    /// The foreign site stored under `selector`, when the record has one.
    fn foreign_site(self, selector: u32) -> Option<&'a ForeignSiteRecord> {
        let extra = self.extra()?;
        self.store.foreign_sites[extra.foreign_sites.range()]
            .iter()
            .find(|site| site.selector == selector)
    }
}
