//! Ordered source candidates handed from Stage 0 discovery to compiler identity assignment.
//!
//! The registration index borrows canonical paths from the filesystem discovery owner. It carries
//! only the compact ordered candidate rows needed to assign compiler [`SourceId`] values, so the
//! compiler does not reconstruct a second discovery or ownership table.

use std::path::Path;

/// Ordered source candidates for one project or source-package identity boundary.
///
/// Stage 0 sorts these rows by its stable logical source identity before handing them to the
/// compiler. The compiler preserves that order while assigning [`SourceId`] values, even when the
/// resolver's display logical paths would sort differently for a filesystem path list.
///
/// [`SourceId`]: super::SourceId
#[derive(Debug)]
pub(crate) struct SourceRegistrationIndex<'a> {
    canonical_paths: Vec<&'a Path>,
}

impl<'a> SourceRegistrationIndex<'a> {
    /// Build registration rows from an already sorted canonical source sequence.
    pub(crate) fn from_ordered_paths<I>(canonical_paths: I) -> Self
    where
        I: IntoIterator<Item = &'a Path>,
    {
        Self {
            canonical_paths: canonical_paths.into_iter().collect(),
        }
    }

    /// The canonical source paths in the order the compiler must assign identities.
    pub(crate) fn canonical_paths(&self) -> std::iter::Copied<std::slice::Iter<'_, &'a Path>> {
        self.canonical_paths.iter().copied()
    }
}
