//! Freezes one batch of pending diagnostics into per-domain reports.
//!
//! WHAT: `PendingDiagnostics` holds drafts in push order, each tagged with the identity domain that owns its spans.
//! `freeze` installs every donor identity handle, compacts each draft into its domain's store, and records one run
//! per contiguous stretch of one domain's records.
//!
//! WHY: runs keep the batch's push order across domains without moving records between stores. One flat draft list
//! avoids a vector per domain, and the stores never clone a draft.

use std::sync::Arc;

use super::draft::DiagnosticDraft;
use super::foreign::{ForeignSiteDraft, ForeignSlot};
use super::report::{DiagnosticReport, DiagnosticReportSet, DiagnosticRun};
use super::resource::DiagnosticLimits;
use super::store::DiagnosticStore;
use crate::compiler_frontend::compiler_messages::diagnostic_types::TypeDisplayCapacityError;
use crate::compiler_frontend::compiler_messages::diagnostic_types::type_display_snapshot::TypeDisplaySnapshot;
use crate::compiler_frontend::semantic_identity::StablePackageIdentity;
use crate::compiler_frontend::source::{
    FrozenIdentityContext, FrozenSourceDatabase, SourceDatabase,
};
use crate::compiler_frontend::symbols::path_interner::PathTable;
use crate::compiler_frontend::symbols::string_interning::{FrozenStringTable, StringTable};

/// Where one identity domain's spans resolve.
#[cfg_attr(not(test), allow(dead_code, reason = "P4.5 S3 boundary cut"))]
pub(super) enum DomainSource {
    /// The domain's loaded sources, frozen with the batch's shared string table.
    Sources(SourceDatabase),
    /// A path trie with no source slots. Its spans resolve to no position.
    Unowned(Arc<PathTable>),
    /// A context its owner froze already. The batch keeps that context and its own string table.
    Frozen(Arc<FrozenIdentityContext>),
}

impl DomainSource {
    #[cfg_attr(not(test), allow(dead_code, reason = "P4.5 S3 boundary cut"))]
    fn into_context(self, strings: &Arc<FrozenStringTable>) -> Arc<FrozenIdentityContext> {
        match self {
            Self::Sources(sources) => Arc::new(FrozenIdentityContext::from_shared_strings(
                Arc::clone(strings),
                sources,
            )),
            Self::Unowned(paths) => Arc::new(FrozenIdentityContext::from_frozen_sources(
                Arc::clone(strings),
                FrozenSourceDatabase::with_paths(paths),
            )),
            Self::Frozen(context) => context,
        }
    }
}

/// One identity domain's accumulated state. Its index is the `u32` that `add_domain` returned.
#[cfg_attr(not(test), allow(dead_code, reason = "P4.5 S3 boundary cut"))]
struct DomainAccumulator {
    identity: Option<StablePackageIdentity>,
    source: DomainSource,
    types: TypeDisplaySnapshot,
}

/// One draft and the index of the domain that owns its spans.
#[cfg_attr(not(test), allow(dead_code, reason = "P4.5 S3 boundary cut"))]
struct PendingDraft {
    domain: u32,
    draft: DiagnosticDraft,
}

/// The drafts of one boundary call, in push order, with the identity domains they name.
#[derive(Default)]
#[cfg_attr(not(test), allow(dead_code, reason = "P4.5 S3 boundary cut"))]
pub(super) struct PendingDiagnostics {
    domains: Vec<DomainAccumulator>,
    drafts: Vec<PendingDraft>,
}

impl PendingDiagnostics {
    /// Registers an identity domain and returns the index that its drafts name.
    #[cfg_attr(not(test), allow(dead_code, reason = "P4.5 S3 boundary cut"))]
    pub(super) fn add_domain(
        &mut self,
        identity: Option<StablePackageIdentity>,
        source: DomainSource,
    ) -> u32 {
        let index =
            u32::try_from(self.domains.len()).expect("diagnostic domain indices fit in u32");
        self.domains.push(DomainAccumulator {
            identity,
            source,
            types: TypeDisplaySnapshot::default(),
        });
        index
    }

    /// Queues `draft` in push order. Its spans belong to the domain that `domain` names.
    #[cfg_attr(not(test), allow(dead_code, reason = "P4.5 S3 boundary cut"))]
    pub(super) fn push(&mut self, domain: u32, draft: DiagnosticDraft) {
        assert!(
            (domain as usize) < self.domains.len(),
            "diagnostic drafts must name a registered domain"
        );
        self.drafts.push(PendingDraft { domain, draft });
    }

    /// Appends one domain's type-display snapshot and returns the record index that it starts at.
    #[cfg_attr(not(test), allow(dead_code, reason = "P4.5 S3 boundary cut"))]
    pub(super) fn append_types(
        &mut self,
        domain: u32,
        types: TypeDisplaySnapshot,
    ) -> Result<u32, TypeDisplayCapacityError> {
        self.domains
            .get_mut(domain as usize)
            .expect("diagnostic drafts must name a registered domain")
            .types
            .append(types)
    }

    /// Freezes the batch into one report set.
    ///
    /// Donor handles install first, so a refused draft still resolves its foreign sites. Each domain's drafts compact
    /// into that domain's store. Each stretch of one domain's records becomes one run, in push order. A stretch that
    /// continues the last run in the same domain extends that run.
    #[cfg_attr(not(test), allow(dead_code, reason = "P4.5 S3 boundary cut"))]
    pub(super) fn freeze(
        self,
        strings: StringTable,
        limits: DiagnosticLimits,
    ) -> DiagnosticReportSet {
        let Self { domains, drafts } = self;
        let shared = Arc::new(strings.freeze());

        let mut identities = Vec::with_capacity(domains.len());
        let mut contexts = Vec::with_capacity(domains.len());
        let mut types = Vec::with_capacity(domains.len());
        for domain in domains {
            identities.push(domain.identity);
            contexts.push(domain.source.into_context(&shared));
            types.push(domain.types);
        }

        for pending in &drafts {
            if let Some(extra) = &pending.draft.extra {
                for site in &extra.foreign_sites {
                    install_site(site, pending.domain, &identities, &contexts);
                }
            }
        }

        let mut stores: Vec<DiagnosticStore> = contexts
            .iter()
            .map(|_| DiagnosticStore::with_limits(limits))
            .collect();
        let mut runs: Vec<DiagnosticRun> = Vec::new();
        let mut queue = drafts.into_iter().peekable();
        while let Some(first) = queue.next() {
            let domain = first.domain;
            let store = &mut stores[domain as usize];
            let start = store.len();
            store.compact(first.draft);
            while let Some(next) = queue.next_if(|pending| pending.domain == domain) {
                store.compact(next.draft);
            }
            let len = store.len() - start;
            if len > 0 {
                match runs.last_mut() {
                    Some(run)
                        if run.report == domain
                            && run.first as usize + run.len as usize == start =>
                    {
                        run.len += to_u32(len);
                    }
                    _ => runs.push(DiagnosticRun {
                        report: domain,
                        first: to_u32(start),
                        len: to_u32(len),
                    }),
                }
            }
        }

        let reports = contexts
            .into_iter()
            .zip(stores)
            .zip(types)
            .map(|((context, diagnostics), snapshot)| {
                DiagnosticReport::new(context, diagnostics, snapshot)
            })
            .collect();
        DiagnosticReportSet::new(reports, runs)
    }
}

/// Points one foreign site's donor handle at the context that its identity names.
///
/// A handle already installed is left alone. A domain-less handle takes the context of its draft's own domain, and
/// only when that domain's identity is `None`. Otherwise it stays uninstalled and never panics. A handle that names a
/// domain takes the context of the first domain, in index order, whose identity is that domain. A named domain that no
/// identity matches is a bug: a primary or label site must reach the frozen render boundary with its donor identity.
#[cfg_attr(not(test), allow(dead_code, reason = "P4.5 S3 boundary cut"))]
fn install_site(
    site: &ForeignSiteDraft,
    draft_domain: u32,
    identities: &[Option<StablePackageIdentity>],
    contexts: &[Arc<FrozenIdentityContext>],
) {
    let handle = &site.context;
    if handle.get().is_some() {
        return;
    }
    let context = match handle.domain() {
        None => {
            let own = draft_domain as usize;
            if identities[own].is_some() {
                return;
            }
            &contexts[own]
        }
        Some(domain) => {
            let Some(index) = identities
                .iter()
                .position(|identity| identity.as_ref() == Some(domain))
            else {
                match site.slot {
                    ForeignSlot::Primary => panic!(
                        "diagnostic primary reached the frozen render boundary without its donor identity"
                    ),
                    ForeignSlot::Label(_) => panic!(
                        "diagnostic label reached the frozen render boundary without its donor identity"
                    ),
                }
            };
            &contexts[index]
        }
    };
    handle
        .install(Arc::clone(context))
        .expect("frozen identity handle was assigned concurrently");
}

#[cfg_attr(not(test), allow(dead_code, reason = "P4.5 S3 boundary cut"))]
fn to_u32(index: usize) -> u32 {
    u32::try_from(index).expect("diagnostic run indices fit in u32")
}
