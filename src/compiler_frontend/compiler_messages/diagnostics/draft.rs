//! Compact diagnostic drafts: what a constructor builds before compaction stores it.
//!
//! WHAT: a draft is a fixed-size core (family, span, fact words and pending marks) plus an optional boxed extra
//! part for labels, foreign sites and data lists. Constructors fill a draft; `DiagnosticStore::compact` consumes
//! it and writes the record.
//! WHY: the core is small and copyable, so the common case needs no allocation beyond the store's own growth.
//! Type words stay pending until `capture_types` rewrites them, so a draft can be built before the type display
//! table that will hold its types exists.

use super::extras::{DiagnosticExtraDraft, ExtraDataDraft};
use super::facts::{
    ABSENT_TYPE_WORD, FACT_WORDS, FactCapacityError, FactCursor, IdRemap, NO_TYPE_WORD,
    type_display_word,
};
use super::families::token_lexical::{COMPILER_CAPACITY_EXCEEDED, CapacityResourceFact};
use super::foreign::{ForeignSiteDraft, ForeignSlot};
use super::labels::{DraftType, encode_label};
use super::record::SeverityOverride;
use super::schema::FamilyRow;
use crate::compiler_frontend::compiler_messages::diagnostic_types::{
    TypeDisplayCapacityError, TypeDisplayId,
};
use crate::compiler_frontend::compiler_messages::{
    CapacityResource, DiagnosticCapacityResource, DiagnosticLabelMessage, DiagnosticSeverity,
};
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathIdRemap;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringIdRemap};

/// A diagnostic under construction. Only `DiagnosticStore::compact` turns it into a stored record.
#[derive(Debug)]
pub(super) struct DiagnosticDraft {
    pub(super) core: DraftCore,
    pub(super) extra: Option<Box<DiagnosticExtraDraft>>,
}

const _: () = assert!(
    size_of::<DiagnosticDraft>() <= 48,
    "diagnostic drafts stay within 48 bytes"
);

/// The fixed-size part of a draft.
#[derive(Clone, Copy, Debug)]
pub(super) struct DraftCore {
    pub(super) family: &'static FamilyRow,
    pub(super) span: Option<SourceSpan>,
    pub(super) facts: [u32; FACT_WORDS],
    /// Exact mask of fact words that hold type words, whether pending or captured.
    pub(super) type_facts: u8,
    /// Exact mask of fact words still holding raw `TypeId`s; the family mask may be a superset.
    pub(super) pending_facts: u8,
    /// Set once a value failed to encode. The draft then stores as a capacity record and takes no more parts.
    pub(super) faulted: bool,
    /// The severity override the record carries. Constructors keep `Default`, which keeps the family's severity.
    pub(super) severity: SeverityOverride,
}

impl DiagnosticDraft {
    /// Starts a draft for `family` and fills its fact words through `fill`.
    ///
    /// A value that cannot be encoded does not panic. The draft becomes a `COMPILER_CAPACITY_EXCEEDED` record at
    /// the same span, so the user still sees a compiler limit where the diagnostic was.
    pub(super) fn with_facts(
        family: &'static FamilyRow,
        span: Option<SourceSpan>,
        fill: impl FnOnce(&mut FactCursor) -> Result<(), FactCapacityError>,
    ) -> Self {
        let mut cursor = FactCursor::new();
        match fill(&mut cursor) {
            Ok(()) => {
                let (facts, pending_facts) = cursor.finish_for(family);
                Self {
                    core: DraftCore {
                        family,
                        span,
                        facts,
                        type_facts: pending_facts,
                        pending_facts,
                        faulted: false,
                        severity: SeverityOverride::Default,
                    },
                    extra: None,
                }
            }
            Err(FactCapacityError) => Self::capacity_fault(
                span,
                CapacityResource::Store(DiagnosticCapacityResource::FactValue),
            ),
        }
    }

    /// The stand-in draft for a value the store could not hold. It is an ordinary `COMPILER_CAPACITY_EXCEEDED`
    /// record with no labels or foreign sites, so it reaches the user as a compiler limit at `span`.
    pub(super) fn capacity_fault(span: Option<SourceSpan>, resource: CapacityResource) -> Self {
        let mut cursor = FactCursor::new();
        cursor
            .push::<CapacityResourceFact>(&resource)
            .expect("a capacity resource always fits in one fact word");
        let (facts, pending_facts) = cursor.finish_for(&COMPILER_CAPACITY_EXCEEDED);
        Self {
            core: DraftCore {
                family: &COMPILER_CAPACITY_EXCEEDED,
                span,
                facts,
                type_facts: pending_facts,
                pending_facts,
                faulted: true,
                severity: SeverityOverride::Default,
            },
            extra: None,
        }
    }

    /// The primary span a capacity record may keep. A foreign primary keeps its span in the draft, but the capacity
    /// record has no foreign-site row for it, so that span is dropped rather than reported as local.
    pub(super) fn capacity_span(&self) -> Option<SourceSpan> {
        let foreign_primary = self.extra.as_ref().is_some_and(|extra| {
            extra
                .foreign_sites
                .iter()
                .any(|site| matches!(site.slot, ForeignSlot::Primary))
        });
        if foreign_primary {
            None
        } else {
            self.core.span
        }
    }

    /// Replaces this draft with the capacity stand-in, keeping the draft's local primary span.
    fn faulted_by(self, resource: DiagnosticCapacityResource) -> Self {
        Self::capacity_fault(self.capacity_span(), CapacityResource::Store(resource))
    }
}

impl DiagnosticDraft {
    /// Records `severity` as this draft's override. A severity equal to the family default stores nothing, so the
    /// record keeps the descriptor's severity. A faulted draft keeps its capacity severity.
    pub(super) fn with_severity(mut self, severity: DiagnosticSeverity) -> Self {
        if !self.core.faulted {
            self.core.severity =
                SeverityOverride::for_severity(severity, self.core.family.default_severity());
        }
        self
    }

    /// Adds a secondary label in this draft's own identity. A label from another identity takes its span here too,
    /// then goes through `with_foreign`, which records the identity and leaves the span in place.
    /// A label without a message is encoded as `NONE`, so it still points at its span.
    pub(super) fn with_label(
        mut self,
        span: Option<SourceSpan>,
        message: Option<DiagnosticLabelMessage>,
    ) -> Self {
        if self.core.faulted {
            return self;
        }
        let extra = self.extra.get_or_insert_with(Box::default);
        match encode_label(span, message, &mut extra.substitutions) {
            Ok(label) => {
                extra.labels.push(label);
                self
            }
            Err(FactCapacityError) => self.faulted_by(DiagnosticCapacityResource::FactValue),
        }
    }

    /// Marks a span as belonging to another identity. The span stays in the draft's primary or label field. The
    /// foreign-site row records which slot it occupies and which identity context owns it.
    pub(super) fn with_foreign(mut self, site: ForeignSiteDraft) -> Self {
        if self.core.faulted {
            // A capacity-fault draft carries no ownership rows, so its foreign primary is dropped
            // like the stand-in record's, rather than being read later as a local span.
            if matches!(site.slot, ForeignSlot::Primary) {
                self.core.span = None;
            }
            return self;
        }
        if let ForeignSlot::Label(index) = site.slot {
            let label_count = self.extra.as_ref().map_or(0, |extra| extra.labels.len());
            assert!(
                index < label_count,
                "a foreign label site names a label that was added first"
            );
        }
        self.extra
            .get_or_insert_with(Box::default)
            .foreign_sites
            .push(site);
        self
    }

    /// Attaches the string IDs that a family declares as its extra data. An empty list stores nothing.
    pub(super) fn with_string_ids(mut self, ids: Vec<StringId>) -> Self {
        if self.core.faulted || ids.is_empty() {
            return self;
        }
        if u32::try_from(ids.len()).is_err() {
            return self.faulted_by(DiagnosticCapacityResource::FactValue);
        }
        self.extra.get_or_insert_with(Box::default).data = ExtraDataDraft::StringIds(ids);
        self
    }
}

impl DiagnosticDraft {
    /// Rewrites the pending type words of this draft through `rewrite`, which maps each raw `TypeId` to its display ID.
    ///
    /// The type-word masks keep the concrete slot shape after capture, so type-free words such as a reason code
    /// stay untouched. All-or-nothing: the new words are computed into temporaries first, and the draft changes
    /// only when every rewrite succeeds. A failed call leaves the draft exactly as it was. Callers map the error
    /// to their own capacity handling: the render adapter expects success, since one diagnostic's types always fit
    /// the id space. The P4.6 producer emits TypeIds while it owns the type environment, so capture happens here.
    pub(super) fn capture_types(
        &mut self,
        rewrite: &mut impl FnMut(TypeId) -> Result<TypeDisplayId, TypeDisplayCapacityError>,
    ) -> Result<(), TypeDisplayCapacityError> {
        let mut facts = self.core.facts;
        for (index, word) in facts.iter_mut().enumerate() {
            if is_marked(self.core.pending_facts, index) {
                *word = captured_word(rewrite, *word)?;
            }
        }

        let mut label_updates = Vec::new();
        let mut substitution_updates = Vec::new();
        if let Some(extra) = self.extra.as_deref() {
            for (index, label) in extra.labels.iter().enumerate() {
                if label.pending_types == 0 {
                    continue;
                }
                let mut data = label.data;
                for (slot, word) in data.iter_mut().enumerate() {
                    if is_marked(label.pending_types, slot) {
                        *word = captured_word(rewrite, *word)?;
                    }
                }
                label_updates.push((index, data));
            }
            for (index, entry) in extra.substitutions.iter().enumerate() {
                if let DraftType::Pending(word) = entry.concrete {
                    substitution_updates.push((index, captured_word(rewrite, word)?));
                }
            }
        }

        // Every rewrite has succeeded, so the commit below cannot fail.
        self.core.facts = facts;
        self.core.pending_facts = 0;
        if let Some(extra) = self.extra.as_deref_mut() {
            for (index, data) in label_updates {
                let label = &mut extra.labels[index];
                label.data = data;
                label.pending_types = 0;
            }
            for (index, word) in substitution_updates {
                extra.substitutions[index].concrete = DraftType::Captured(word);
            }
        }
        Ok(())
    }

    /// Rebase every captured type word after its snapshot is appended to another snapshot.
    ///
    /// The first pass checks every present word before any write, so overflow leaves the draft unchanged.
    pub(super) fn rebase_type_displays(
        &mut self,
        base: u32,
    ) -> Result<(), TypeDisplayCapacityError> {
        assert_eq!(
            self.core.pending_facts, 0,
            "type-display rebasing needs `capture_types` to run first"
        );
        if let Some(extra) = self.extra.as_deref() {
            assert!(
                extra.labels.iter().all(|label| label.pending_types == 0),
                "type-display rebasing needs `capture_types` to run first"
            );
            assert!(
                extra
                    .substitutions
                    .iter()
                    .all(|entry| matches!(&entry.concrete, DraftType::Captured(_))),
                "type-display rebasing needs `capture_types` to run first"
            );
        }

        for (index, &word) in self.core.facts.iter().enumerate() {
            if is_marked(self.core.type_facts, index) {
                rebased_type_word(word, base)?;
            }
        }

        if let Some(extra) = self.extra.as_deref() {
            for label in &extra.labels {
                for (slot, &word) in label.data.iter().enumerate() {
                    if is_marked(label.type_words, slot) {
                        rebased_type_word(word, base)?;
                    }
                }
            }
            for entry in &extra.substitutions {
                let DraftType::Captured(word) = entry.concrete else {
                    unreachable!("the rebase preflight rejects pending substitution types")
                };
                rebased_type_word(word, base)?;
            }
        }

        for (index, word) in self.core.facts.iter_mut().enumerate() {
            if is_marked(self.core.type_facts, index) {
                *word = rebased_type_word(*word, base)
                    .expect("type-display rebase preflight checked every fact");
            }
        }

        if let Some(extra) = self.extra.as_deref_mut() {
            for label in &mut extra.labels {
                for (slot, word) in label.data.iter_mut().enumerate() {
                    if is_marked(label.type_words, slot) {
                        *word = rebased_type_word(*word, base)
                            .expect("type-display rebase preflight checked every label");
                    }
                }
            }
            for entry in &mut extra.substitutions {
                let DraftType::Captured(word) = entry.concrete else {
                    unreachable!("the rebase preflight rejects pending substitution types")
                };
                entry.concrete = DraftType::Captured(
                    rebased_type_word(word, base)
                        .expect("type-display rebase preflight checked every substitution"),
                );
            }
        }

        Ok(())
    }
}

/// Whether bit `bit` of a pending mask is set.
fn is_marked(mask: u8, bit: usize) -> bool {
    mask & (1 << bit) != 0
}

/// Rebases one present display word. `NO_TYPE_WORD` is the absent-type representation.
fn rebased_type_word(word: u32, base: u32) -> Result<u32, TypeDisplayCapacityError> {
    if word == NO_TYPE_WORD {
        return Ok(NO_TYPE_WORD);
    }
    word.checked_add(base).ok_or(TypeDisplayCapacityError)
}

/// Rewrites one pending word to its display word. The absent sentinel captures as `NO_TYPE_WORD` without a rewrite.
fn captured_word(
    rewrite: &mut impl FnMut(TypeId) -> Result<TypeDisplayId, TypeDisplayCapacityError>,
    word: u32,
) -> Result<u32, TypeDisplayCapacityError> {
    if word == ABSENT_TYPE_WORD {
        return Ok(NO_TYPE_WORD);
    }
    let display = rewrite(TypeId(word))?;
    type_display_word(display).map_err(|FactCapacityError| TypeDisplayCapacityError)
}

impl DiagnosticDraft {
    /// Rewrites every string ID in this draft through `remap`, which must come from the table the draft was
    /// built against. An identity remap changes nothing.
    pub(super) fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        if remap.is_identity() {
            return;
        }
        self.remap_draft(IdRemap::Strings(remap));
    }

    /// Rewrites every path ID in this draft through `remap`, which must come from the interner the draft was
    /// built against. An identity remap changes nothing.
    pub(super) fn remap_path_ids(&mut self, remap: &PathIdRemap) {
        if remap.is_identity() {
            return;
        }
        self.remap_draft(IdRemap::Paths(remap));
    }

    /// Applies `remap` to the fact words and the extra part. A value the remap cannot store turns the draft into
    /// the capacity stand-in at the same local span, as `with_facts` does for an unencodable value. Spans are
    /// left alone: a `SourceId` names a source record, not a string or a path.
    fn remap_draft(&mut self, remap: IdRemap<'_>) {
        if self.rewrite_draft(remap).is_err() {
            let span = self.capacity_span();
            *self = Self::capacity_fault(
                span,
                CapacityResource::Store(DiagnosticCapacityResource::FactValue),
            );
        }
    }

    /// Walks the family layout, rewriting each fact's words in place, then the extra part.
    fn rewrite_draft(&mut self, remap: IdRemap<'_>) -> Result<(), FactCapacityError> {
        let mut offset = 0;
        for fact in self.core.family.layout {
            let end = offset + usize::from(fact.words);
            (fact.remap_ids)(&mut self.core.facts[offset..end], remap)?;
            offset = end;
        }
        if let Some(extra) = self.extra.as_deref_mut() {
            extra.remap_ids(remap)?;
        }
        Ok(())
    }
}
