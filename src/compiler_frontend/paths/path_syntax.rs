//! File-owned path syntax table.
//!
//! WHAT: one dense table per tokenized file owns every authored path row. A path token
//!       carries one `PathSyntaxId` handle into this table instead of an expanded per-leaf
//!       payload.
//! WHY: path syntax owns paths only. Dependency selections are ordinary identifier, comma and
//!       alias tokens handled by the dependency-clause parser; they never become path rows or
//!       selection trees.

use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::instrumentation::{FrontendCounter, add_frontend_counter};
use crate::compiler_frontend::source::{SourceId, SourceSpan};
use crate::compiler_frontend::symbols::path_interner::{PathId, PathIdRemap};
use crate::compiler_frontend::tokenizer::tokens::{Token, TokenKind};
use rustc_hash::FxHashMap;

/// Dense file-local handle into a `PathSyntaxTable`.
///
/// `PathSyntaxId::NONE` is the absent marker (zero) and is never a valid row.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PathSyntaxId(u32);

impl PathSyntaxId {
    /// Absent marker: no path row. Not a valid identity.
    pub const NONE: PathSyntaxId = PathSyntaxId(0);

    fn from_index(index: usize) -> Self {
        Self((index as u32) + 1)
    }

    pub fn is_none(self) -> bool {
        self == Self::NONE
    }

    fn index(self) -> Option<usize> {
        self.0.checked_sub(1).map(|index| index as usize)
    }
}

/// One authored path row: the complete path spelling and its exact source span.
///
/// Path syntax owns no dependency selections. The root is the full authored path; the span carries
/// the source identity that owns the token bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PathSyntax {
    pub root: PathId,
    pub span: SourceSpan,
}

/// Dense file-local store of authored path rows.
#[derive(Clone, Debug, Default)]
pub struct PathSyntaxTable {
    paths: Vec<PathSyntax>,
}

impl PathSyntaxTable {
    pub fn new() -> Self {
        Self::default()
    }

    #[cfg(test)]
    pub(crate) fn paths(&self) -> &[PathSyntax] {
        &self.paths
    }

    /// Walk every authored path row with its dense handle.
    ///
    /// File-reference classification uses this instead of scanning source text or parsing
    /// expressions, so graph activity stays a syntax-table fact.
    pub(crate) fn iter(&self) -> impl Iterator<Item = (PathSyntaxId, &PathSyntax)> {
        self.paths
            .iter()
            .enumerate()
            .map(|(index, path)| (PathSyntaxId::from_index(index), path))
    }

    /// Read one path row through a fallible boundary.
    ///
    /// WHAT: returns `CompilerError` for absent or out-of-range handles so authored-source
    ///       parsing cannot panic on malformed retained state.
    /// WHY: stale, absent or out-of-range path handles are internal compiler corruption, not
    ///      user syntax. Production callers propagate the infrastructure error rather than
    ///      expecting validation.
    pub fn try_path(&self, id: PathSyntaxId) -> Result<&PathSyntax, CompilerError> {
        let Some(index) = id.index() else {
            return Err(CompilerError::compiler_error(
                "path syntax table received the absent PathSyntaxId marker",
            ));
        };
        self.paths.get(index).ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "path syntax handle {} is outside a table of {} rows",
                id.0,
                self.paths.len()
            ))
        })
    }

    /// Read one path row and prove it belongs to the token currently being consumed.
    ///
    /// The token span carries its source identity explicitly, so a same-index handle from another
    /// file-owned table cannot pass this check even when byte offsets happen to match.
    pub(crate) fn try_path_for_token(
        &self,
        path_id: PathSyntaxId,
        token_span: SourceSpan,
    ) -> Result<&PathSyntax, CompilerError> {
        let row = self.try_path(path_id)?;
        if row.span != token_span {
            return Err(CompilerError::compiler_error(
                "path syntax row does not belong to the consumed path token",
            ));
        }
        Ok(row)
    }

    /// Append one authored path row and return its handle.
    pub fn push(&mut self, root: PathId, span: SourceSpan) -> PathSyntaxId {
        self.paths.push(PathSyntax { root, span });
        add_frontend_counter(FrontendCounter::PathSyntaxRowCount, 1);
        PathSyntaxId::from_index(self.paths.len() - 1)
    }

    /// Remap every complete-path identity in this table once.
    ///
    /// WHAT: rewrites each authored row's `PathId` through a worker-local merge remap.
    /// WHY: path rows are interned against a chunk-local fork; the canonical chunk merge
    ///      re-interns those nodes into the module fork and must rewrite rows that still
    ///      address worker-local suffixes. Source spans contain no path identities.
    pub fn remap_path_ids(&mut self, remap: &PathIdRemap) {
        if remap.is_identity() {
            return;
        }
        for path in &mut self.paths {
            path.root = remap.get(path.root);
        }
    }

    /// Rebind every path row to the finalized source identity without changing its local range.
    pub fn rebind_source_identity(&mut self, source: SourceId) {
        for path in &mut self.paths {
            path.span = SourceSpan::new(source, path.span.local());
        }
    }

    /// Validate the dense table independently of any consuming token stream.
    pub(crate) fn validate_structure(&self) -> Result<(), CompilerError> {
        Ok(())
    }

    /// Validate file-owned spans after final source identity is known.
    pub(crate) fn validate_file_owned_locations(
        &self,
        expected_source: SourceId,
    ) -> Result<(), CompilerError> {
        self.validate_structure()?;
        for path in &self.paths {
            if path.span.source() != expected_source {
                return Err(CompilerError::compiler_error(
                    "path row span does not use the prepared file's source identity",
                ));
            }
        }
        Ok(())
    }

    /// Validate every path handle carried by one retained token slice.
    pub(crate) fn validate_token_handles(&self, tokens: &[Token]) -> Result<(), CompilerError> {
        self.validate_structure()?;
        for token in tokens {
            if let TokenKind::Path(path_id) = token.kind {
                let row = self.try_path(path_id)?;
                if row.span.local() != token.span {
                    return Err(CompilerError::compiler_error(
                        "path syntax row does not belong to the consumed path token",
                    ));
                }
            }
        }
        Ok(())
    }

    /// Validate one retained token slice against its owning source identity.
    pub(crate) fn validate_file_tokens(
        &self,
        tokens: &[Token],
        expected_source: SourceId,
        role: &str,
    ) -> Result<(), CompilerError> {
        self.validate_token_handles(tokens)?;

        for token in tokens {
            if let TokenKind::Path(path_id) = token.kind {
                let span = SourceSpan::new(expected_source, token.span);
                let row = self.try_path_for_token(path_id, span)?;
                if row.span.source() != expected_source {
                    return Err(CompilerError::compiler_error(format!(
                        "{role} path span does not use the prepared file's source identity"
                    )));
                }
            }
        }
        Ok(())
    }

    /// Capture the canonical subset required by one persistent generic artefact.
    ///
    /// WHAT: copies only path rows referenced by the frozen generic body and rewrites that
    ///       body's handles to its compact table.
    /// WHY: persistent generic artefacts outlive their prepared source, so they are the sole
    ///      deliberate exception to the one-table-per-prepared-file rule. Ordinary header and
    ///      AST substreams share the frozen source table and must never call this API.
    pub(crate) fn capture_persistent_generic_subset(
        &self,
        tokens: &mut [Token],
    ) -> Result<(PathSyntaxTable, FxHashMap<PathSyntaxId, PathSyntaxId>), CompilerError> {
        // Validate every token against this table before copying. Persistent capture must not
        // depend on the caller having already proved handle ownership.
        self.validate_token_handles(tokens)?;

        let mut subset = PathSyntaxTable::new();
        let mut old_to_new: FxHashMap<PathSyntaxId, PathSyntaxId> = FxHashMap::default();

        for token in tokens {
            let TokenKind::Path(path_handle) = &mut token.kind else {
                continue;
            };
            let old_id = *path_handle;
            if old_id.is_none() {
                return Err(CompilerError::compiler_error(
                    "persistent generic body contains an absent PathSyntaxId marker",
                ));
            }

            let new_id = match old_to_new.get(&old_id) {
                Some(new_id) => *new_id,
                None => {
                    let new_id = subset.copy_persistent_path_from(self, old_id)?;
                    old_to_new.insert(old_id, new_id);
                    new_id
                }
            };
            *path_handle = new_id;
        }

        if !old_to_new.is_empty() {
            add_frontend_counter(
                FrontendCounter::PersistentGenericPathSyntaxSubsetCopyCount,
                1,
            );
            add_frontend_counter(
                FrontendCounter::PersistentGenericPathSyntaxRowCopyCount,
                subset.paths.len(),
            );
        }
        Ok((subset, old_to_new))
    }

    fn copy_persistent_path_from(
        &mut self,
        source: &PathSyntaxTable,
        id: PathSyntaxId,
    ) -> Result<PathSyntaxId, CompilerError> {
        let source_path = source.try_path(id)?;
        self.paths.push(PathSyntax {
            root: source_path.root.clone(),
            span: source_path.span,
        });
        Ok(PathSyntaxId::from_index(self.paths.len() - 1))
    }
}
