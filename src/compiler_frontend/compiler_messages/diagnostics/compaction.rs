//! Compaction turns a finished `DiagnosticDraft` into one stored record, or refuses it when an arena is full.
//!
//! WHAT: `plan` checks every ceiling the draft would touch and changes nothing. `write` then appends the draft's
//! parts and its record, and it cannot fail. A refused draft leaves the arenas as they were, apart from the latch
//! that marks the store full and the capacity record that replaces the draft.
//! WHY: a half-written record would point at arena rows that do not exist. Splitting the work into a check and a
//! write keeps every stored record complete. A full store then reports one capacity record rather than failing again
//! for each later diagnostic.

use super::draft::DiagnosticDraft;
use super::extras::{
    DiagnosticExtraDraft, DiagnosticExtraId, DiagnosticExtraRecord, DiagnosticListRange,
    ExtraDataDraft, ExtraDataKind, ExtraDataRef,
};
use super::foreign::{ForeignSiteDraft, ForeignSiteRecord, ForeignSlot};
use super::labels::{
    DraftType, GenericSubstitutionRecord, LabelMessageCode, SecondaryDiagnosticLabel,
};
use super::record::DiagnosticRecord;
use super::store::{DiagnosticId, DiagnosticStore, PRIMARY_SELECTOR};
use crate::compiler_frontend::compiler_messages::{CapacityResource, DiagnosticCapacityResource};
use crate::compiler_frontend::source::FrozenIdentityHandle;

/// The outcome of compacting one draft.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CompactOutcome {
    /// The draft is stored at this id.
    Stored(DiagnosticId),
    /// The draft is not stored. The resource names the arena that refused it. The store is full from now on, and its
    /// capacity record is already in place.
    Rejected(DiagnosticCapacityResource),
}

impl DiagnosticStore {
    /// Stores `draft` as one record, or refuses it when an arena is full.
    ///
    /// The draft's types must already be captured. A pending mark is a caller bug, so it panics rather than storing a
    /// raw `TypeId` where a display ID belongs.
    pub(super) fn compact(&mut self, draft: DiagnosticDraft) -> CompactOutcome {
        if let Some(resource) = self.full {
            return CompactOutcome::Rejected(resource);
        }
        assert_captured(&draft);
        assert_label_roles(&draft);
        match self.plan(&draft) {
            Ok(()) => CompactOutcome::Stored(self.write(draft)),
            Err(resource) => {
                self.latch_full(resource, &draft);
                CompactOutcome::Rejected(resource)
            }
        }
    }
}

/// Panics when the draft still holds a pending type word. Such a word is a raw `TypeId`, not a display ID.
fn assert_captured(draft: &DiagnosticDraft) {
    assert!(
        draft.core.pending_facts == 0,
        "compaction needs `capture_types` to run first: a fact still holds a raw type"
    );
    let Some(extra) = draft.extra.as_deref() else {
        return;
    };
    assert!(
        extra.labels.iter().all(|label| label.pending_types == 0),
        "compaction needs `capture_types` to run first: a label still holds a raw type"
    );
    assert!(
        extra
            .substitutions
            .iter()
            .all(|entry| matches!(entry.concrete, DraftType::Captured(_))),
        "compaction needs `capture_types` to run first: a substitution still holds a raw type"
    );
}

/// Panics when a label's message is not one the family permits. A family permits its declared roles and the universal
/// roles, so an undeclared role is a producer bug and never reaches the store.
fn assert_label_roles(draft: &DiagnosticDraft) {
    let Some(extra) = draft.extra.as_deref() else {
        return;
    };
    for label in &extra.labels {
        assert!(
            draft.core.family.permits_label(label.message),
            "label role {} is not declared by family {}",
            label.message.name(),
            draft.core.family.descriptor.code
        );
    }
}

/// Fails with `resource` when `needed` more rows would pass `limit`. `used` is the arena's current length.
fn check_room(
    used: usize,
    needed: usize,
    limit: usize,
    resource: DiagnosticCapacityResource,
) -> Result<(), DiagnosticCapacityResource> {
    if used.saturating_add(needed) <= limit {
        Ok(())
    } else {
        Err(resource)
    }
}

impl DiagnosticStore {
    /// Checks every ceiling the draft would touch, in arena order. Changes nothing.
    fn plan(&self, draft: &DiagnosticDraft) -> Result<(), DiagnosticCapacityResource> {
        let no_parts = DiagnosticExtraDraft::default();
        let extra = draft.extra.as_deref().unwrap_or(&no_parts);

        let label_data_rows = extra
            .labels
            .iter()
            .filter(|label| label.message.carries_data())
            .count();
        check_room(
            self.label_data.len(),
            label_data_rows,
            self.limits.label_data,
            DiagnosticCapacityResource::LabelData,
        )?;
        check_room(
            self.labels.len(),
            extra.labels.len(),
            self.limits.labels,
            DiagnosticCapacityResource::Labels,
        )?;

        let string_count = match &extra.data {
            ExtraDataDraft::None => 0,
            ExtraDataDraft::StringIds(ids) => ids.len(),
        };
        check_room(
            self.string_ids.len(),
            string_count,
            self.limits.string_ids,
            DiagnosticCapacityResource::StringIds,
        )?;
        check_room(
            self.substitutions.len(),
            extra.substitutions.len(),
            self.limits.substitutions,
            DiagnosticCapacityResource::Substitutions,
        )?;
        check_room(
            self.foreign_sites.len(),
            extra.foreign_sites.len(),
            self.limits.foreign_sites,
            DiagnosticCapacityResource::ForeignSites,
        )?;
        let new_contexts = self.count_new_identity_contexts(&extra.foreign_sites);
        check_room(
            self.identity_contexts.len(),
            new_contexts,
            self.limits.identity_contexts,
            DiagnosticCapacityResource::IdentityContexts,
        )?;

        let needs_extra =
            !extra.labels.is_empty() || !extra.foreign_sites.is_empty() || string_count > 0;
        check_room(
            self.extras.len(),
            usize::from(needs_extra),
            self.limits.extras,
            DiagnosticCapacityResource::Extras,
        )?;

        // One record slot stays free for the capacity record that replaces a refused draft.
        check_room(
            self.records.len(),
            2,
            self.limits.records,
            DiagnosticCapacityResource::Records,
        )
    }

    /// The number of distinct contexts in `sites` that the store does not hold yet. Sites that name one context
    /// count once.
    fn count_new_identity_contexts(&self, sites: &[ForeignSiteDraft]) -> usize {
        let mut fresh = 0;
        for (index, site) in sites.iter().enumerate() {
            let stored = self.identity_contexts.contains(&site.context);
            let earlier = sites[..index]
                .iter()
                .any(|other| other.context == site.context);
            if !stored && !earlier {
                fresh += 1;
            }
        }
        fresh
    }

    /// The index of `context` in the store's identity table. An unseen context is moved into the table.
    fn identity_index(&mut self, context: FrozenIdentityHandle) -> u32 {
        let known = self
            .identity_contexts
            .iter()
            .position(|stored| *stored == context);
        let index = match known {
            Some(index) => index,
            None => {
                self.identity_contexts.push(context);
                self.identity_contexts.len() - 1
            }
        };
        u32::try_from(index).expect("the identity limit keeps every index inside u32")
    }
}

impl DiagnosticStore {
    /// Appends the draft's parts and then its record. Cannot fail: `plan` has checked every ceiling.
    fn write(&mut self, draft: DiagnosticDraft) -> DiagnosticId {
        let DiagnosticDraft { core, extra } = draft;
        let DiagnosticExtraDraft {
            labels,
            foreign_sites,
            owner_handles: _owner_handles,
            substitutions,
            data,
        } = extra.map_or_else(DiagnosticExtraDraft::default, |boxed| *boxed);

        let substitution_base = u32::try_from(self.substitutions.len())
            .expect("the substitution limit keeps every index inside u32");
        for entry in substitutions {
            let DraftType::Captured(type_word) = entry.concrete else {
                unreachable!("`assert_captured` runs before the write phase")
            };
            self.substitutions.push(GenericSubstitutionRecord {
                parameter: entry.parameter,
                type_word,
            });
        }

        let label_start = self.labels.len();
        let label_count = labels.len();
        for label in labels {
            let data_index = if label.message.carries_data() {
                let mut row = label.data;
                if label.message == LabelMessageCode::GENERIC_INSTANTIATION_SUBSTITUTIONS {
                    row[0] += substitution_base;
                }
                let index = u32::try_from(self.label_data.len())
                    .expect("the label data limit keeps every index inside 24 bits");
                self.label_data.push(row);
                index
            } else {
                0
            };
            self.labels.push(SecondaryDiagnosticLabel::pack(
                label.span,
                label.message,
                data_index,
            ));
        }

        let string_start = self.string_ids.len();
        let (data_kind, string_count) = match data {
            ExtraDataDraft::StringIds(ids) if !ids.is_empty() => {
                assert!(
                    core.family.extra == ExtraDataKind::StringIds,
                    "only a family that declares string extra data may carry string IDs"
                );
                let count = ids.len();
                self.string_ids.extend(ids);
                (ExtraDataKind::StringIds, count)
            }
            _ => (ExtraDataKind::None, 0),
        };

        let site_start = self.foreign_sites.len();
        let site_count = foreign_sites.len();
        for site in foreign_sites {
            let selector = match site.slot {
                ForeignSlot::Primary => PRIMARY_SELECTOR,
                ForeignSlot::Label(index) => {
                    u32::try_from(index + 1).expect("a label index fits in u32")
                }
            };
            let context = self.identity_index(site.context);
            self.foreign_sites
                .push(ForeignSiteRecord { selector, context });
        }

        // Must agree with the `needs_extra` rule in `plan`, or the extras ceiling could be overrun by one.
        let has_extra = label_count > 0 || site_count > 0 || string_count > 0;
        let extra_id = if has_extra {
            let index = self.extras.len();
            self.extras.push(DiagnosticExtraRecord {
                labels: DiagnosticListRange::new(label_start, label_count),
                foreign_sites: DiagnosticListRange::new(site_start, site_count),
                data: ExtraDataRef {
                    kind: data_kind,
                    list: DiagnosticListRange::new(string_start, string_count),
                },
            });
            Some(DiagnosticExtraId::from_index(index))
        } else {
            None
        };

        self.records.push(DiagnosticRecord::new(
            core.family.code,
            core.severity,
            core.span,
            core.facts,
            extra_id,
        ));
        DiagnosticId::from_index(self.records.len() - 1)
    }

    /// Latches the store full and appends the capacity record that stands in for the refused draft. The stand-in keeps
    /// the draft's local primary span (see `DiagnosticDraft::capacity_span`). It needs no ceiling check: every stored
    /// record leaves one record slot free (see `plan`).
    fn latch_full(&mut self, resource: DiagnosticCapacityResource, draft: &DiagnosticDraft) {
        let span = draft.capacity_span();
        let stand_in = DiagnosticDraft::capacity_fault(span, CapacityResource::Store(resource));
        self.full = Some(resource);
        self.records.push(DiagnosticRecord::new(
            stand_in.core.family.code,
            stand_in.core.severity,
            span,
            stand_in.core.facts,
            None,
        ));
    }
}
