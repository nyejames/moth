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
use super::labels::{DraftType, LabelDraftMessage, encode_label};
use super::record::SeverityOverride;
use super::schema::FamilyRow;
use crate::compiler_frontend::compiler_messages::diagnostic_types::{
    TypeDisplayCapacityError, TypeDisplayId,
};
use crate::compiler_frontend::compiler_messages::{
    CapacityResource, DiagnosticCapacityResource, DiagnosticSeverity,
};
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::source::{FrozenIdentityHandle, SourceSpan};
use crate::compiler_frontend::symbols::path_interner::{PathId, PathIdRemap};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringIdRemap};

/// A diagnostic under construction. Only `DiagnosticStore::compact` turns it into a stored record.
#[derive(Debug)]
pub struct DiagnosticDraft {
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

    /// Replaces this draft with the capacity stand-in, retaining owner handles for boundary identity selection.
    pub(super) fn faulted_by(mut self, resource: DiagnosticCapacityResource) -> Self {
        let span = self.capacity_span();
        let mut owner_handles = self
            .extra
            .take()
            .map(|boxed| {
                let mut extra = *boxed;
                extra
                    .owner_handles
                    .extend(extra.foreign_sites.into_iter().map(|site| site.context));
                extra.owner_handles
            })
            .unwrap_or_default();
        let mut fault = Self::capacity_fault(span, CapacityResource::Store(resource));
        if !owner_handles.is_empty() {
            fault
                .extra
                .get_or_insert_with(Box::default)
                .owner_handles
                .append(&mut owner_handles);
        }
        fault
    }
}

impl DiagnosticDraft {
    /// Records `severity` as this draft's override. A severity equal to the family default stores nothing, so the
    /// record keeps the descriptor's severity. A faulted draft keeps its capacity severity.
    pub(crate) fn with_severity(mut self, severity: DiagnosticSeverity) -> Self {
        if !self.core.faulted {
            self.core.severity =
                SeverityOverride::for_severity(severity, self.core.family.default_severity());
        }
        self
    }

    pub(crate) fn severity(&self) -> DiagnosticSeverity {
        self.core
            .severity
            .resolve(self.core.family.default_severity())
    }

    /// The family descriptor's stable code, for code-level classification without decoding facts.
    pub(crate) fn code(&self) -> &'static str {
        self.core.family.descriptor.code
    }

    pub(crate) fn primary_span(&self) -> Option<SourceSpan> {
        self.core.span
    }

    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "P4.6b producer migrations use draft span and label-owner builders"
        )
    )]
    pub(crate) fn set_primary_span(&mut self, span: Option<SourceSpan>) {
        self.core.span = span;
    }

    pub(crate) fn has_label_at(&self, span: Option<SourceSpan>) -> bool {
        self.extra
            .as_ref()
            .is_some_and(|extra| extra.labels.iter().any(|label| label.span == span))
    }
    pub(crate) fn has_label_at_with_owner(
        &self,
        span: Option<SourceSpan>,
        owner: Option<&FrozenIdentityHandle>,
    ) -> bool {
        self.extra.as_ref().is_some_and(|extra| {
            extra.labels.iter().enumerate().any(|(index, label)| {
                if label.span != span {
                    return false;
                }
                let label_owner = extra
                    .foreign_sites
                    .iter()
                    .find(|site| matches!(site.slot, ForeignSlot::Label(label_index) if label_index == index))
                    .map(|site| &site.context);
                match (label_owner, owner) {
                    (None, None) => true,
                    (None, Some(_)) | (Some(_), None) => false,
                    (Some(label_owner), Some(owner)) => {
                        match (label_owner.domain(), owner.domain()) {
                            (Some(label_domain), Some(owner_domain)) => {
                                label_domain == owner_domain
                            }
                            _ => label_owner == owner,
                        }
                    }
                }
            })
        })
    }

    pub(crate) fn label_count(&self) -> usize {
        self.extra.as_ref().map_or(0, |extra| extra.labels.len())
    }

    pub(crate) fn label_span(&self, index: usize) -> Option<SourceSpan> {
        self.extra
            .as_ref()?
            .labels
            .get(index)
            .and_then(|label| label.span)
    }

    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "P4.6b producer migrations use draft span and label-owner builders"
        )
    )]
    pub(crate) fn set_label_span(&mut self, index: usize, span: Option<SourceSpan>) {
        if let Some(label) = self
            .extra
            .as_deref_mut()
            .and_then(|extra| extra.labels.get_mut(index))
        {
            label.span = span;
        }
    }

    pub(crate) fn rebind_source_ids(
        &mut self,
        rebind: &mut impl FnMut(
            crate::compiler_frontend::source::SourceId,
        ) -> Result<
            crate::compiler_frontend::source::SourceId,
            crate::compiler_frontend::compiler_messages::compiler_errors::CompilerError,
        >,
    ) -> Result<(), crate::compiler_frontend::compiler_messages::compiler_errors::CompilerError>
    {
        fn rebind_span(
            span: &mut Option<SourceSpan>,
            rebind: &mut impl FnMut(
                crate::compiler_frontend::source::SourceId,
            ) -> Result<
                crate::compiler_frontend::source::SourceId,
                crate::compiler_frontend::compiler_messages::compiler_errors::CompilerError,
            >,
        ) -> Result<(), crate::compiler_frontend::compiler_messages::compiler_errors::CompilerError>
        {
            if let Some(span) = span {
                *span = SourceSpan::new(rebind(span.source())?, span.local());
            }
            Ok(())
        }

        rebind_span(&mut self.core.span, rebind)?;
        if let Some(extra) = self.extra.as_deref_mut() {
            for label in &mut extra.labels {
                rebind_span(&mut label.span, rebind)?;
            }
        }
        Ok(())
    }

    /// Adds a secondary label in this draft's own identity.
    pub(crate) fn with_label(
        self,
        span: Option<SourceSpan>,
        message: Option<LabelDraftMessage>,
    ) -> Self {
        let index = self.label_count();
        self.with_label_at(index, span, message)
    }

    /// Inserts a secondary label before the label at `index`.
    pub(crate) fn with_label_at(
        mut self,
        index: usize,
        span: Option<SourceSpan>,
        message: Option<LabelDraftMessage>,
    ) -> Self {
        if self.core.faulted {
            return self;
        }
        assert!(
            index <= self.label_count(),
            "a draft label is inserted at or before an existing label"
        );
        let label = {
            let extra = self.extra.get_or_insert_with(Box::default);
            encode_label(span, message, &mut extra.substitutions)
        };
        let label = match label {
            Ok(label) => label,
            Err(FactCapacityError) => {
                return self.faulted_by(DiagnosticCapacityResource::FactValue);
            }
        };
        let extra = self
            .extra
            .as_deref_mut()
            .expect("label insertion initialized extra");
        extra.labels.insert(index, label);
        for site in &mut extra.foreign_sites {
            if let ForeignSlot::Label(label_index) = &mut site.slot
                && *label_index >= index
            {
                *label_index += 1;
            }
        }
        self
    }

    /// Marks a span as belonging to another identity.
    pub(super) fn with_foreign(mut self, site: ForeignSiteDraft) -> Self {
        if self.core.faulted {
            if matches!(site.slot, ForeignSlot::Primary) {
                self.core.span = None;
            }
            self.extra
                .get_or_insert_with(Box::default)
                .owner_handles
                .push(site.context);
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

    pub(crate) fn with_primary_frozen_identity_handle(
        mut self,
        handle: FrozenIdentityHandle,
    ) -> Self {
        self.set_foreign_slot(ForeignSlot::Primary, handle, false);
        self
    }

    pub(crate) fn set_primary_frozen_identity_handle_if_missing(
        &mut self,
        handle: FrozenIdentityHandle,
    ) {
        self.set_foreign_slot(ForeignSlot::Primary, handle, true);
    }
    pub(crate) fn primary_frozen_identity_handle(&self) -> Option<&FrozenIdentityHandle> {
        self.extra
            .as_ref()?
            .foreign_sites
            .iter()
            .find(|site| matches!(site.slot, ForeignSlot::Primary))
            .map(|site| &site.context)
    }

    pub(crate) fn replace_primary_frozen_identity_handle(
        &mut self,
        handle: Option<FrozenIdentityHandle>,
    ) {
        if self.core.faulted {
            if let Some(handle) = handle {
                self.extra
                    .get_or_insert_with(Box::default)
                    .owner_handles
                    .push(handle);
            }
            return;
        }
        if let Some(extra) = self.extra.as_deref_mut() {
            extra
                .foreign_sites
                .retain(|site| !matches!(site.slot, ForeignSlot::Primary));
        }
        if let Some(handle) = handle {
            self.set_foreign_slot(ForeignSlot::Primary, handle, false);
        }
    }

    pub(crate) fn attach_frozen_identity_handle_if_missing(
        &mut self,
        handle: FrozenIdentityHandle,
    ) {
        self.set_primary_frozen_identity_handle_if_missing(handle.clone());
        let label_count = self.extra.as_ref().map_or(0, |extra| extra.labels.len());
        for index in 0..label_count {
            if self
                .extra
                .as_ref()
                .is_some_and(|extra| extra.labels[index].span.is_some())
            {
                self.set_label_frozen_identity_handle_if_missing(index, handle.clone());
            }
        }
    }

    #[cfg_attr(
        not(test),
        allow(
            dead_code,
            reason = "P4.6b producer migrations use draft span and label-owner builders"
        )
    )]
    pub(crate) fn with_label_frozen_identity_handle(
        mut self,
        index: usize,
        handle: FrozenIdentityHandle,
    ) -> Self {
        self.set_foreign_slot(ForeignSlot::Label(index), handle, false);
        self
    }

    pub(crate) fn set_label_frozen_identity_handle_if_missing(
        &mut self,
        index: usize,
        handle: FrozenIdentityHandle,
    ) {
        let label_count = self.extra.as_ref().map_or(0, |extra| extra.labels.len());
        assert!(
            index < label_count,
            "a frozen label owner names an existing label"
        );
        self.set_foreign_slot(ForeignSlot::Label(index), handle, true);
    }

    fn set_foreign_slot(
        &mut self,
        slot: ForeignSlot,
        handle: FrozenIdentityHandle,
        if_missing: bool,
    ) {
        if self.core.faulted {
            if matches!(slot, ForeignSlot::Primary) {
                self.core.span = None;
            }
            self.extra
                .get_or_insert_with(Box::default)
                .owner_handles
                .push(handle);
            return;
        }
        let extra = self.extra.get_or_insert_with(Box::default);
        if let ForeignSlot::Label(index) = slot {
            assert!(
                index < extra.labels.len(),
                "a foreign label site names a label that was added first"
            );
        }
        if let Some(site) = extra
            .foreign_sites
            .iter_mut()
            .find(|site| site.slot == slot)
        {
            if !if_missing {
                site.context = handle;
            }
        } else {
            extra.foreign_sites.push(ForeignSiteDraft {
                slot,
                context: handle,
            });
        }
    }

    /// Returns every frozen identity owner carried by this draft, including owners retained after a capacity fault.
    pub(super) fn visit_identity_handles(&self, mut visit: impl FnMut(&FrozenIdentityHandle)) {
        if let Some(extra) = self.extra.as_deref() {
            for site in &extra.foreign_sites {
                visit(&site.context);
            }
            for handle in &extra.owner_handles {
                visit(handle);
            }
        }
    }

    /// Appends path IDs encoded by the family's fact codecs.
    pub(crate) fn append_required_paths(&self, paths: &mut Vec<PathId>) {
        let mut offset = 0;
        for fact in self.core.family.layout {
            let end = offset + usize::from(fact.words);
            (fact.append_path_ids)(&self.core.facts[offset..end], paths);
            offset = end;
        }
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
    pub(crate) fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        if remap.is_identity() {
            return;
        }
        self.remap_draft(IdRemap::Strings(remap));
    }

    /// Rewrites every path ID in this draft through `remap`, which must come from the interner the draft was
    /// built against. An identity remap changes nothing.
    pub(crate) fn remap_path_ids(&mut self, remap: &PathIdRemap) {
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
            let resource = DiagnosticCapacityResource::FactValue;
            let mut replacement =
                Self::capacity_fault(self.capacity_span(), CapacityResource::Store(resource));
            std::mem::swap(self, &mut replacement);
            *self = replacement.faulted_by(resource);
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
