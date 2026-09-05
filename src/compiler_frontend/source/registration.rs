//! Ordered source candidates handed from Stage 0 discovery to compiler identity assignment.
//!
//! The registration index borrows canonical paths from the filesystem discovery owner. It carries
//! only the compact ordered candidate rows needed to assign compiler [`SourceId`] values, so the
//! compiler does not reconstruct a second discovery or ownership table. Each row also carries the
//! authored [`SourceKind`] Stage 0 classified from the lexical file name, because canonicalize can
//! resolve a recognized spelling onto a target whose extension would name a different kind.

use super::SourceKind;
use std::path::Path;

/// One ordered registration candidate: the canonical IO path plus the authored lexical kind.
#[derive(Debug)]
struct SourceRegistrationRow<'a> {
    canonical_path: &'a Path,
    kind: SourceKind,
}

/// Ordered source candidates for one project or source-package identity boundary.
///
/// Stage 0 sorts these rows by its stable logical source identity before handing them to the
/// compiler. The compiler preserves that order while assigning [`SourceId`] values, even when the
/// resolver's display logical paths would sort differently for a filesystem path list.
///
/// [`SourceId`]: super::SourceId
#[derive(Debug)]
pub(crate) struct SourceRegistrationIndex<'a> {
    rows: Vec<SourceRegistrationRow<'a>>,
}

impl<'a> SourceRegistrationIndex<'a> {
    /// Build registration rows from an already sorted canonical source sequence.
    pub(crate) fn from_ordered_rows<I>(rows: I) -> Self
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

    /// The canonical source paths in the order the compiler must assign identities.
    pub(crate) fn canonical_paths(&self) -> impl ExactSizeIterator<Item = &'a Path> + '_ {
        self.rows.iter().map(|row| row.canonical_path)
    }

    /// Canonical paths paired with the authored kind Stage 0 classified for each row.
    pub(crate) fn rows(&self) -> impl ExactSizeIterator<Item = (&'a Path, SourceKind)> + '_ {
        self.rows.iter().map(|row| (row.canonical_path, row.kind))
    }
}
