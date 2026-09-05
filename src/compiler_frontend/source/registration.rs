//! Source candidates handed to compiler identity assignment.
//!
//! The registration index is the single compiler-facing handoff for every source lane. It borrows
//! canonical paths from the discovery owner and carries only the compact candidate rows needed to
//! assign compiler [`SourceId`] values, so the compiler does not reconstruct a second discovery or
//! ownership table. Each row also carries the authored [`SourceKind`] classified from the lexical
//! file name, because canonicalize can resolve a recognized spelling onto a target whose extension
//! would name a different kind.
//!
//! Two ordering authorities meet at this boundary. Stage 0 already sorts rows by
//! `SourceLogicalIdentity` (module origin, then module-relative path, rooted before unrooted)
//! because it owns the per-source ownership inventory that key needs. The compiler preserves that
//! order. Lanes that discover sources by traversal own no such inventory, so the compiler orders
//! their rows by canonical logical path. Producers hand over rows, never sort keys: identity order
//! is the compiler's to decide, so no producer can drift from it.

use super::SourceKind;
use std::path::Path;

/// One registration candidate: the canonical IO path plus the authored lexical kind.
#[derive(Debug)]
struct SourceRegistrationRow<'a> {
    canonical_path: &'a Path,
    kind: SourceKind,
}

/// Source candidates for one project or source-package identity boundary.
///
/// Stage 0 hands rows already sorted by logical identity. Discovered lanes hand rows in whatever
/// order they walked. The compiler constructor chosen at this boundary is the authority that
/// either preserves Stage 0 order or sorts by canonical logical path.
///
/// [`SourceId`]: super::SourceId
#[derive(Debug)]
pub(crate) struct SourceRegistrationIndex<'a> {
    rows: Vec<SourceRegistrationRow<'a>>,
}

impl<'a> SourceRegistrationIndex<'a> {
    /// Collect registration rows in the producing owner's order.
    pub(crate) fn from_rows<I>(rows: I) -> Self
    where
        I: IntoIterator<Item = (&'a Path, SourceKind)>,
    {
        Self {
            rows: rows
                .into_iter()
                .map(|(canonical_path, kind)| SourceRegistrationRow {
                    canonical_path,
                    kind,
                })
                .collect(),
        }
    }

    /// The canonical source paths in the order this owner produced them.
    pub(crate) fn canonical_paths(&self) -> impl ExactSizeIterator<Item = &'a Path> + '_ {
        self.rows.iter().map(|row| row.canonical_path)
    }

    /// Canonical paths paired with the authored kind classified for each row.
    pub(crate) fn rows(&self) -> impl ExactSizeIterator<Item = (&'a Path, SourceKind)> + '_ {
        self.rows.iter().map(|row| (row.canonical_path, row.kind))
    }
}
