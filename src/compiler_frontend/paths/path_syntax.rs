//! Source-owned path syntax table.
//!
//! WHAT: one dense table per tokenized source owns every authored path row. A path token carries
//! one `PathSyntaxId` handle into this table instead of an expanded per-leaf payload.
//! WHY: path syntax owns paths only. Dependency selections are ordinary identifier, comma and
//! alias tokens handled by the dependency-clause parser; they never become path rows or selection
//! trees.
//!
//! A row deliberately stores a [`LocalSpan`], not a [`SourceSpan`]. The table owns the source
//! identity once at its lifecycle boundary; token-aware lookup checks that identity before
//! accepting a same-index handle from another source.

use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::instrumentation::{FrontendCounter, add_frontend_counter};
use crate::compiler_frontend::source::{LocalSpan, SourceId, SourceSpan};
use crate::compiler_frontend::symbols::path_interner::{PathId, PathIdRemap};

/// Dense file-local handle into a [`PathSyntaxTable`].
///
/// `PathSyntaxId::NONE` is the absent marker (zero) and is never a valid row.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PathSyntaxId(u32);

impl PathSyntaxId {
    /// Absent marker: no path row. Not a valid identity.
    #[cfg(test)]
    pub const NONE: Self = Self(0);

    /// Checked construction from the packed representation.
    ///
    /// Zero is reserved for the absent marker, so it is not accepted as a row identity.
    pub const fn try_from_raw(raw: u32) -> Option<Self> {
        if raw == 0 { None } else { Some(Self(raw)) }
    }

    /// Checked construction from a zero-based row index.
    pub const fn try_from_index(index: usize) -> Option<Self> {
        if index >= u32::MAX as usize {
            return None;
        }
        Some(Self((index as u32) + 1))
    }

    /// The packed handle value. `0` is returned only for [`Self::NONE`].
    pub const fn raw(self) -> u32 {
        self.0
    }

    pub const fn is_none(self) -> bool {
        self.0 == 0
    }

    /// Return the zero-based row index, or `None` for the absent marker.
    pub const fn index(self) -> Option<usize> {
        if self.0 == 0 {
            None
        } else {
            Some((self.0 - 1) as usize)
        }
    }

    // Internal spelling retained at call sites that construct IDs while iterating a Vec.
    fn from_index(index: usize) -> Option<Self> {
        Self::try_from_index(index)
    }
}

/// Capacity exhaustion while allocating one dense path row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathSyntaxCapacityError {
    /// The one-based `u32` handle domain cannot address another row.
    TableFull,
}

/// Fallible path-row construction errors.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathSyntaxError {
    Capacity(PathSyntaxCapacityError),
    Frozen,
    ForeignSource {
        expected: SourceId,
        actual: SourceId,
    },
}

impl From<PathSyntaxCapacityError> for PathSyntaxError {
    fn from(error: PathSyntaxCapacityError) -> Self {
        Self::Capacity(error)
    }
}

/// One authored path row: the complete path identity and its exact source-local span.
///
/// Path syntax owns no dependency selections. The root is the full authored path; the local span
/// is interpreted only with the owning [`PathSyntaxTable`] source identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PathSyntax {
    pub root: PathId,
    pub span: LocalSpan,
}

/// Dense source-owned store of authored path rows.
#[derive(Clone, Debug, Default)]
pub struct PathSyntaxTable {
    paths: Vec<PathSyntax>,
    owner_source: Option<SourceId>,
    frozen: bool,
}

impl PathSyntaxTable {
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct the table with its source identity already attached.
    pub fn with_source(source: SourceId) -> Self {
        Self {
            paths: Vec::new(),
            owner_source: Some(source),
            frozen: false,
        }
    }

    #[cfg(test)]
    pub(crate) fn paths(&self) -> &[PathSyntax] {
        &self.paths
    }

    /// Bytes occupied by authored path rows currently used by this source.
    #[cfg(feature = "data_layout_memory_probe")]
    pub(crate) fn used_bytes(&self) -> usize {
        self.paths
            .len()
            .saturating_mul(std::mem::size_of::<PathSyntax>())
    }

    /// Bytes reserved by the backing authored-path row vector.
    #[cfg(feature = "data_layout_memory_probe")]
    pub(crate) fn storage_bytes(&self) -> usize {
        self.paths
            .capacity()
            .saturating_mul(std::mem::size_of::<PathSyntax>())
    }
    /// The source identity owned by this table, when rows have been attached to one.
    #[cfg(test)]
    pub fn owner_source(&self) -> Option<SourceId> {
        self.owner_source
    }

    /// Mark the table immutable. `PreparedFilePathSyntax` calls this at its one freeze boundary.
    pub fn freeze(&mut self) {
        self.frozen = true;
    }

    #[cfg(test)]
    pub fn is_frozen(&self) -> bool {
        self.frozen
    }

    /// Walk every authored path row with its dense handle in stable row order.
    pub(crate) fn iter(&self) -> impl Iterator<Item = (PathSyntaxId, &PathSyntax)> {
        self.paths
            .iter()
            .enumerate()
            .filter_map(|(index, path)| Some((PathSyntaxId::from_index(index)?, path)))
    }

    /// Read one path row through a fallible boundary.
    ///
    /// Out-of-range handles are retained-state failures and therefore remain on the
    /// infrastructure lane rather than becoming source diagnostics. The absent marker is a
    /// compiler bug because `try_from_raw` and `try_from_index` never issue it.
    pub fn try_path(&self, id: PathSyntaxId) -> Result<&PathSyntax, InfrastructureFailure> {
        let Some(index) = id.index() else {
            crate::compiler_bug!(
                CompilerStage::DependencyResolution,
                "path syntax table received PathSyntaxId::NONE; try_from_raw and try_from_index never issue raw 0"
            );
        };
        self.paths.get(index).ok_or_else(|| {
            InfrastructureFailure::internal(
                CompilerStage::DependencyResolution,
                format!(
                    "path syntax handle {} is outside a table of {} rows",
                    id.raw(),
                    self.paths.len()
                ),
            )
        })
    }

    /// Read one path row and prove it belongs to the token currently being consumed.
    ///
    /// The token carries a source-qualified span. A same-index handle from another source's table
    /// therefore cannot pass even when byte offsets happen to match.
    pub(crate) fn try_path_for_token(
        &self,
        path_id: PathSyntaxId,
        token_span: SourceSpan,
    ) -> Result<&PathSyntax, InfrastructureFailure> {
        let row = self.try_path(path_id)?;
        if let Some(owner_source) = self.owner_source
            && owner_source != token_span.source()
        {
            crate::compiler_bug!(
                CompilerStage::DependencyResolution,
                "path syntax row does not belong to the consumed path token",
            );
        }
        if row.span != token_span.local() {
            crate::compiler_bug!(
                CompilerStage::DependencyResolution,
                "path syntax row does not belong to the consumed path token",
            );
        }
        Ok(row)
    }

    /// Checked append for a table whose source owner is already known.
    pub fn try_push_local(
        &mut self,
        root: PathId,
        span: LocalSpan,
    ) -> Result<PathSyntaxId, PathSyntaxError> {
        if self.frozen {
            return Err(PathSyntaxError::Frozen);
        }
        let id = PathSyntaxId::try_from_index(self.paths.len())
            .ok_or(PathSyntaxCapacityError::TableFull)?;
        self.paths.push(PathSyntax { root, span });
        add_frontend_counter(FrontendCounter::PathSyntaxRowCount, 1);
        Ok(id)
    }

    /// Checked append that validates or installs the source owner.
    pub fn try_push_for_source(
        &mut self,
        root: PathId,
        source: SourceId,
        span: LocalSpan,
    ) -> Result<PathSyntaxId, PathSyntaxError> {
        if let Some(expected) = self.owner_source
            && expected != source
        {
            return Err(PathSyntaxError::ForeignSource {
                expected,
                actual: source,
            });
        }
        if self.owner_source.is_none() {
            self.owner_source = Some(source);
        }
        self.try_push_local(root, span)
    }

    /// Remap every complete-path identity in this table once.
    pub fn remap_path_ids(&mut self, remap: &PathIdRemap) {
        if remap.is_identity() {
            return;
        }
        for path in &mut self.paths {
            path.root = remap.get(path.root);
        }
    }

    /// Rebind the single source owner without changing any local byte range.
    pub fn rebind_source_identity(&mut self, source: SourceId) {
        self.owner_source = Some(source);
    }

    /// Validate the dense table independently of any consuming token stream.
    pub(crate) fn validate_structure(&self) -> Result<(), InfrastructureFailure> {
        // try_from_index issues indices below u32::MAX, so exactly u32::MAX rows is legal.
        if self.paths.len() > u32::MAX as usize {
            crate::compiler_bug!(
                CompilerStage::DependencyResolution,
                "path syntax table row count exceeds the handle domain; try_push_local refuses PathSyntaxId::try_from_index overflow"
            );
        }
        if !self.paths.is_empty() && self.owner_source.is_none() {
            return Err(InfrastructureFailure::internal(
                CompilerStage::DependencyResolution,
                "path syntax table has rows but no source owner",
            ));
        }
        Ok(())
    }

    /// Validate the source owner after final identity is known.
    pub(crate) fn validate_file_owned_locations(
        &self,
        expected_source: SourceId,
    ) -> Result<(), InfrastructureFailure> {
        self.validate_structure()?;
        if let Some(owner_source) = self.owner_source
            && owner_source != expected_source
        {
            return Err(InfrastructureFailure::internal(
                CompilerStage::DependencyResolution,
                "path table does not use the prepared file's source identity",
            ));
        }
        Ok(())
    }
}
#[cfg(feature = "data_layout_memory_probe")]
impl Drop for PathSyntaxTable {
    fn drop(&mut self) {
        crate::compiler_frontend::instrumentation::release_source_path_table(self);
    }
}
