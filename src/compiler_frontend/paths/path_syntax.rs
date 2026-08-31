//! File-owned path syntax table.
//!
//! WHAT: one dense table per tokenized file owns every authored path row. A path token
//!       carries one `PathSyntaxId` handle into this table instead of an expanded per-leaf
//!       payload.
//! WHY: path syntax owns paths only. Dependency selections are ordinary identifier, comma and
//!       alias tokens handled by the dependency-clause parser; they never become path rows or
//!       selection trees.

use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::source_location::SourceLocation;
use crate::compiler_frontend::instrumentation::{FrontendCounter, add_frontend_counter};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringIdRemap};
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

/// One authored path row: the complete path spelling and its source location.
///
/// Path syntax owns no dependency selections. The root is the full authored path; the
/// location is the token span that introduced it.
#[derive(Clone, Debug)]
pub struct PathSyntax {
    pub root: InternedPath,
    pub location: SourceLocation,
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
    /// WHAT: rejects absent, out-of-range and same-index handles that index a different
    ///       file-owned table or a row whose location is not the consumed token.
    /// WHY: `PathSyntaxId` is a dense table-local index. Bounds checking cannot detect a
    ///      valid index from file A used against a non-empty table for file B.
    pub(crate) fn try_path_for_token(
        &self,
        path_id: PathSyntaxId,
        token_location: &SourceLocation,
    ) -> Result<&PathSyntax, CompilerError> {
        let row = self.try_path(path_id)?;
        if row.location != *token_location {
            return Err(CompilerError::compiler_error(
                "path syntax row does not belong to the consumed path token",
            ));
        }
        Ok(row)
    }

    /// Append one authored path row and return its handle.
    pub fn push(&mut self, root: InternedPath, location: SourceLocation) -> PathSyntaxId {
        self.paths.push(PathSyntax { root, location });
        add_frontend_counter(FrontendCounter::PathSyntaxRowCount, 1);
        PathSyntaxId::from_index(self.paths.len() - 1)
    }

    /// Remap every interned string in this table once.
    ///
    /// The file-owned table is the single path-token remap owner; path tokens
    /// themselves carry only handles and are not walked again.
    pub fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        self.try_remap_string_ids(&mut |id| {
            Ok::<StringId, std::convert::Infallible>(remap.get(id))
        })
        .expect("ordinary string-ID remapping is infallible");
    }

    /// Remap every string-bearing field through the table's one exhaustive walker.
    ///
    /// WHAT: covers path roots and locations in place.
    /// WHY: normal source merging and frozen generic materialisation must use the same owner so
    ///      a future path payload cannot bypass one remap lane.
    pub(crate) fn try_remap_string_ids<E>(
        &mut self,
        map: &mut impl FnMut(StringId) -> Result<StringId, E>,
    ) -> Result<(), E> {
        for path in &mut self.paths {
            path.root.try_remap_string_ids(map)?;
            path.location.try_remap_string_ids(map)?;
        }
        Ok(())
    }

    /// Rebind every table location scope to a module logical path once.
    pub fn rebind_source_identity(&mut self, logical_path: &InternedPath) {
        for path in &mut self.paths {
            path.location.scope = logical_path.clone();
        }
    }

    /// Validate the dense table independently of any consuming token stream.
    ///
    /// This is intentionally an internal invariant boundary. Malformed handles indicate
    /// compiler-retained corruption, not source syntax a user can author.
    pub(crate) fn validate_structure(&self) -> Result<(), CompilerError> {
        for path in &self.paths {
            validate_path_location(&path.location, &path.location.scope, "path row")?;
        }
        Ok(())
    }

    /// Validate file-owned locations after final source identity is known.
    pub(crate) fn validate_file_owned_locations(
        &self,
        expected_source: &InternedPath,
    ) -> Result<(), CompilerError> {
        self.validate_structure()?;
        for path in &self.paths {
            validate_path_location(&path.location, expected_source, "path row")?;
        }
        Ok(())
    }

    /// Validate every path handle carried by one retained token slice.
    ///
    /// WHAT: checks the table topology before resolving each non-expanded `TokenKind::Path`
    ///       payload through its dense file-local handle.
    /// WHY: prepared-file freezing and persistent generic materialisation both retain token
    ///      slices independently of their construction owner. Keeping this check beside the
    ///      canonical table prevents a stale handle from reaching a later panic-only lookup.
    pub(crate) fn validate_token_handles(&self, tokens: &[Token]) -> Result<(), CompilerError> {
        self.validate_structure()?;
        for token in tokens {
            if let TokenKind::Path(path_id) = token.kind {
                self.try_path_for_token(path_id, &token.location)?;
            }
        }
        Ok(())
    }

    /// Validate one retained token slice against its owning source identity.
    ///
    /// WHAT: combines dense-path handle validation with source-scope and span checks for every
    ///       token in a retained slice.
    /// WHY: prepared headers and persistent generic bodies retain token slices independently of
    ///      the tokenizer. They must not be able to pair a valid path handle with a stale source
    ///      scope and reach a later parser through internally inconsistent retained state.
    pub(crate) fn validate_file_tokens(
        &self,
        tokens: &[Token],
        expected_source: &InternedPath,
        role: &str,
    ) -> Result<(), CompilerError> {
        self.validate_token_handles(tokens)?;

        for token in tokens {
            validate_path_location(&token.location, expected_source, role)?;
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
            location: source_path.location.clone(),
        });
        Ok(PathSyntaxId::from_index(self.paths.len() - 1))
    }
}

fn validate_path_location(
    location: &SourceLocation,
    expected_source: &InternedPath,
    role: &str,
) -> Result<(), CompilerError> {
    if &location.scope != expected_source {
        return Err(CompilerError::compiler_error(format!(
            "{role} location does not use the prepared file's source identity"
        )));
    }
    let start = (
        location.start_pos.line_number,
        location.start_pos.char_column,
    );
    let end = (location.end_pos.line_number, location.end_pos.char_column);
    if start > end {
        return Err(CompilerError::compiler_error(format!(
            "{role} location has an inverted source span"
        )));
    }
    Ok(())
}
