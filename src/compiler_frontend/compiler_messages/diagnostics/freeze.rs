//! Freezes one batch of pending diagnostics into per-domain reports.
//!
//! WHAT: `PendingDiagnostics` holds drafts in push order, each tagged with the identity domain that owns its spans and
//! with the domain whose context resolves its identity-less foreign sites. `freeze` installs donor identity handles
//! under an `InstallMode`, compacts each draft into its domain's store, and records one run per contiguous stretch of
//! one domain's records.
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
use crate::compiler_frontend::symbols::string_interning::FrozenStringTable;

/// Where one identity domain's spans resolve.
pub(super) enum DomainSource {
    /// The domain's loaded sources, frozen with the batch's shared string table.
    Sources(SourceDatabase),
    /// A path trie with no source slots. Its spans resolve to no position.
    Unowned(Arc<PathTable>),
    /// The context of the earlier domain at this index. The domain keeps its own identity, so one database that
    /// several rows reference keeps one context.
    Shares(u32),
}

impl DomainSource {
    /// Whether the domain's spans resolve through a source database rather than a bare path trie.
    fn owns_database(&self) -> bool {
        !matches!(self, Self::Unowned(_))
    }

    /// Builds the domain's context. `earlier` holds the contexts of every lower domain index, in index order.
    fn into_context(
        self,
        strings: &Arc<FrozenStringTable>,
        earlier: &[Arc<FrozenIdentityContext>],
    ) -> Arc<FrozenIdentityContext> {
        match self {
            Self::Sources(sources) => Arc::new(FrozenIdentityContext::from_shared_strings(
                Arc::clone(strings),
                sources,
            )),
            Self::Unowned(paths) => Arc::new(FrozenIdentityContext::from_frozen_sources(
                Arc::clone(strings),
                FrozenSourceDatabase::with_paths(paths),
            )),
            Self::Shares(index) => Arc::clone(&earlier[index as usize]),
        }
    }
}

/// One identity domain's accumulated state. Its index is the `u32` that `add_domain` returned.
struct DomainAccumulator {
    identity: Option<StablePackageIdentity>,
    source: DomainSource,
    types: TypeDisplaySnapshot,
}

/// One draft, the index of the domain that owns its spans, and the index of the domain whose context resolves its
/// identity-less foreign sites.
struct PendingDraft {
    domain: u32,
    none_domain: Option<u32>,
    draft: DiagnosticDraft,
}

/// The drafts of one boundary call, in push order, with the identity domains they name.
#[derive(Default)]
pub(super) struct PendingDiagnostics {
    domains: Vec<DomainAccumulator>,
    drafts: Vec<PendingDraft>,
}

const _: () = assert!(std::mem::size_of::<PendingDiagnostics>() <= 48);

/// Whether a freeze installs identity handles by identity.
///
/// `Frozen` is the vessel legacy would have frozen: each handle that names an identity takes that domain's context,
/// and an owned draft whose named identity matches no domain is a D12 invariant. `Unfrozen` reproduces legacy
/// non-frozen rendering: identity-named handles stay uninstalled and render without a position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InstallMode {
    Frozen,
    Unfrozen,
}

impl PendingDiagnostics {
    /// Registers an identity domain and returns the index that its drafts name.
    ///
    /// `Shares(j)` must name a lower index than the new domain's own index. Any other `j` is a D12 invariant.
    pub(super) fn add_domain(
        &mut self,
        identity: Option<StablePackageIdentity>,
        source: DomainSource,
    ) -> u32 {
        let index =
            u32::try_from(self.domains.len()).expect("diagnostic domain indices fit in u32");
        if let DomainSource::Shares(earlier) = &source {
            assert!(
                *earlier < index,
                "diagnostic domains may share only the context of an earlier domain"
            );
        }
        self.domains.push(DomainAccumulator {
            identity,
            source,
            types: TypeDisplaySnapshot::default(),
        });
        index
    }

    /// Queues `draft` in push order. Its spans belong to the domain that `domain` names. Its identity-less foreign
    /// sites resolve through the domain that `none_domain` names, when one is given.
    pub(super) fn push(&mut self, domain: u32, none_domain: Option<u32>, draft: DiagnosticDraft) {
        let count = self.domains.len();
        assert!(
            (domain as usize) < count && none_domain.is_none_or(|none| (none as usize) < count),
            "diagnostic drafts must name a registered domain"
        );
        self.drafts.push(PendingDraft {
            domain,
            none_domain,
            draft,
        });
    }

    /// Appends one domain's type-display snapshot and returns the record index that it starts at.
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

    /// Freezes the batch into one report set over the batch's frozen string table.
    ///
    /// Donor handles install first, so a refused draft still resolves its foreign sites. Each domain's drafts compact
    /// into that domain's store. Each stretch of one domain's records becomes one run, in push order. A stretch that
    /// continues the last run in the same domain extends that run.
    pub(super) fn freeze(
        self,
        strings: FrozenStringTable,
        limits: DiagnosticLimits,
        install: InstallMode,
    ) -> DiagnosticReportSet {
        let Self { domains, drafts } = self;
        let shared = Arc::new(strings);

        let mut identities = Vec::with_capacity(domains.len());
        let mut contexts: Vec<Arc<FrozenIdentityContext>> = Vec::with_capacity(domains.len());
        let mut database_owned = Vec::with_capacity(domains.len());
        let mut types = Vec::with_capacity(domains.len());
        for DomainAccumulator {
            identity,
            source,
            types: snapshot,
        } in domains
        {
            database_owned.push(source.owns_database());
            let context = source.into_context(&shared, &contexts);
            contexts.push(context);
            identities.push(identity);
            types.push(snapshot);
        }

        install_donor_handles(&drafts, &identities, &contexts, &database_owned, install);

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

/// Installs the donor handle of every foreign site that the batch's drafts carry.
fn install_donor_handles(
    drafts: &[PendingDraft],
    identities: &[Option<StablePackageIdentity>],
    contexts: &[Arc<FrozenIdentityContext>],
    database_owned: &[bool],
    install: InstallMode,
) {
    for pending in drafts {
        if let Some(extra) = &pending.draft.extra {
            for site in &extra.foreign_sites {
                install_site(site, pending, identities, contexts, database_owned, install);
            }
        }
    }
}

/// Points one foreign site's donor handle at the context that its identity names.
///
/// A handle already installed is left alone. A domain-less handle takes the context of its draft's `none_domain` in
/// both modes, and stays uninstalled when there is none. In `Frozen` mode, a handle that names an identity takes the
/// context of the first domain, in index order, whose identity matches. A named identity that no domain matches is a
/// bug when the draft's own domain owns a database: a primary or label site must reach the frozen render boundary with
/// its donor identity. Otherwise the handle stays uninstalled and renders without a position. `Unfrozen` mode installs
/// no identity-named handle.
fn install_site(
    site: &ForeignSiteDraft,
    pending: &PendingDraft,
    identities: &[Option<StablePackageIdentity>],
    contexts: &[Arc<FrozenIdentityContext>],
    database_owned: &[bool],
    install: InstallMode,
) {
    let handle = &site.context;
    if handle.get().is_some() {
        return;
    }
    let context = match handle.domain() {
        None => match pending.none_domain {
            Some(domain) => &contexts[domain as usize],
            None => return,
        },
        Some(domain) => {
            if install == InstallMode::Unfrozen {
                return;
            }
            match identities
                .iter()
                .position(|identity| identity.as_ref() == Some(domain))
            {
                Some(index) => &contexts[index],
                None if database_owned[pending.domain as usize] => match site.slot {
                    ForeignSlot::Primary => panic!(
                        "diagnostic primary reached the frozen render boundary without its donor identity"
                    ),
                    ForeignSlot::Label(_) => panic!(
                        "diagnostic label reached the frozen render boundary without its donor identity"
                    ),
                },
                None => return,
            }
        }
    };
    handle
        .install(Arc::clone(context))
        .expect("frozen identity handle was assigned concurrently");
}

fn to_u32(index: usize) -> u32 {
    u32::try_from(index).expect("diagnostic run indices fit in u32")
}
