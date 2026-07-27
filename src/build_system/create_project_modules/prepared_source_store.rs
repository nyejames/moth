//! Project-boundary prepared-source store indexed by dense `SourceId`.
//!
//! WHAT: one slot per project source, transitioning `Unprepared` -> `Prepared`. Each prepared
//!       slot retains source text, tokens (for `.moth`) and structural provider references. The
//!       store is shared across all entry traversals in one project boundary so each project
//!       source is read, tokenized and prepared at most once.
//! WHY: replaces per-entry path-keyed `ScannedImportSource` caches with one shared project owner.
//!      The live reachable traversal prepares sources lazily and stores them here; semantic-input
//!      assembly projects `PreparedSourceInput` values from the retained slots instead of
//!      consuming a second per-entry project cache. Package-boundary sources remain outside this
//!      store until their own indexed store and consumer land.

use crate::builder_surface::SourceFileKind;
use crate::compiler_frontend::paths::const_paths::StructuralProviderReference;
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::FileTokens;

use std::path::Path;

use super::import_scanning::{ScannedImportSource, scan_imports_with_source};
use super::prepared_source::PreparedSourceInput;
use super::source_discovery_error::SourceDiscoveryError;
use super::source_loading::extract_source_code;
use super::source_tree_index::{SourceClassification, SourceId, SourceTreeIndex};

/// One project-boundary prepared-source store.
///
/// Project sources are indexed by dense `SourceId` from the project `SourceTreeIndex`. Each slot
/// transitions at most once from `Unprepared` to `Prepared`; sources from another compilation
/// boundary never enter this store.
///
/// The store is populated serially during classification (or the single-entry serial path) and then
/// shared read-only across Rayon workers. Workers call [`Self::get_project_moth_imports`], which only reads
/// already-prepared slots; an unprepared slot during a worker traversal is an invariant violation
/// because classification traverses every entry before workers start.
pub(super) struct PreparedSourceStore {
    project_slots: Vec<PreparedSourceSlot>,
}

/// One project-boundary slot addressed by dense `SourceId`.
#[derive(Clone)]
enum PreparedSourceSlot {
    Unprepared,
    Prepared(PreparedSourceEntry),
}

/// Retained prepared source data for one source file.
///
/// `tokens` is `Some` for `.moth` files (scanned and tokenized during traversal) and `None` for
/// `.mtf`/`.md` files (loaded during assembly without tokenization). `imports` is non-empty only
/// for `.moth` files; `.mtf`/`.md` files have no structural provider references.
#[derive(Clone)]
struct PreparedSourceEntry {
    source_code: String,
    source_kind: SourceFileKind,
    tokens: Option<Box<FileTokens>>,
    imports: Vec<StructuralProviderReference>,
}

/// Whether a `.moth` source was freshly read or reused from the store.
#[derive(Debug, PartialEq)]
pub(super) enum MothScanOrigin {
    FreshRead { source_byte_count: usize },
    ReusedFromStore,
}

impl PreparedSourceStore {
    /// Create a store with one `Unprepared` slot per project source.
    pub(super) fn new(source_count: usize) -> Self {
        Self {
            project_slots: vec![PreparedSourceSlot::Unprepared; source_count],
        }
    }

    /// Prepare or reuse one `.moth` source during traversal, returning its structural provider
    /// references.
    ///
    /// If the slot is already `Prepared` the retained imports are returned without re-reading or
    /// re-tokenizing. If the slot is `Unprepared` the source is read, tokenized and stored, and the
    /// fresh imports are returned. Non-project sources (no `SourceId`) are stored by canonical path.
    pub(super) fn prepare_or_get_project_moth_imports(
        &mut self,
        source_id: SourceId,
        canonical_path: &Path,
        style_directives: &StyleDirectiveRegistry,
        string_table: &mut StringTable,
    ) -> Result<(Vec<StructuralProviderReference>, MothScanOrigin), SourceDiscoveryError> {
        let slot = &mut self.project_slots[source_id.index()];

        if let PreparedSourceSlot::Prepared(entry) = slot {
            return Ok((entry.imports.clone(), MothScanOrigin::ReusedFromStore));
        }

        let scanned = scan_imports_with_source(canonical_path, style_directives, string_table)?;
        let imports = scanned.imports.clone();
        let source_byte_count = scanned.source_code.len();
        *slot =
            PreparedSourceSlot::Prepared(scanned_source_to_entry(scanned, SourceFileKind::Moth));
        Ok((imports, MothScanOrigin::FreshRead { source_byte_count }))
    }

    /// Get the structural provider references from an already-prepared `.moth` source.
    ///
    /// Used by provider-free workers after classification has prepared every reachable source.
    /// An unprepared slot is an invariant violation because classification traverses all entries.
    pub(super) fn get_project_moth_imports(
        &self,
        source_id: SourceId,
    ) -> Result<Vec<StructuralProviderReference>, SourceDiscoveryError> {
        match &self.project_slots[source_id.index()] {
            PreparedSourceSlot::Prepared(entry) => Ok(entry.imports.clone()),
            _ => Err(SourceDiscoveryError::from(
                crate::compiler_frontend::compiler_errors::CompilerError::compiler_error(
                    "Provider-free worker reached a Moth file absent from the prepared source store",
                ),
            )),
        }
    }

    /// Project one `PreparedSourceInput` from the store for a project source, preparing the slot
    /// lazily if it is still `Unprepared` (`.mtf`/`.md` loaded during assembly).
    pub(super) fn project_project_prepared_input(
        &mut self,
        source_id: SourceId,
        source_tree_index: &SourceTreeIndex,
        string_table: &mut StringTable,
    ) -> Result<PreparedSourceInput, SourceDiscoveryError> {
        let record = source_tree_index.source(source_id);
        let slot = &mut self.project_slots[source_id.index()];

        if let PreparedSourceSlot::Prepared(entry) = slot {
            return Ok(prepared_input_from_entry(record.canonical_path(), entry));
        }

        let SourceClassification::CompilerSemantic(source_kind) = record.classification() else {
            return Err(SourceDiscoveryError::from(
                crate::compiler_frontend::compiler_errors::CompilerError::compiler_error(format!(
                    "Project source ID {} is not compiler semantic",
                    source_id.index(),
                )),
            ));
        };

        let source_code = extract_source_code(record.canonical_path(), string_table)?;
        let entry = PreparedSourceEntry {
            source_code: source_code.clone(),
            source_kind: *source_kind,
            tokens: None,
            imports: Vec::new(),
        };
        *slot = PreparedSourceSlot::Prepared(entry);
        Ok(prepared_input_from_entry(
            record.canonical_path(),
            slot_entry(slot),
        ))
    }
}

/// Extract a reference to the prepared entry from a slot, panicking if not prepared.
fn slot_entry(slot: &PreparedSourceSlot) -> &PreparedSourceEntry {
    match slot {
        PreparedSourceSlot::Prepared(entry) => entry,
        _ => unreachable!("slot was just set to Prepared"),
    }
}

/// Convert a `ScannedImportSource` into a `PreparedSourceEntry`.
fn scanned_source_to_entry(
    scanned: ScannedImportSource,
    source_kind: SourceFileKind,
) -> PreparedSourceEntry {
    PreparedSourceEntry {
        source_code: scanned.source_code,
        source_kind,
        tokens: Some(Box::new(scanned.tokens)),
        imports: scanned.imports,
    }
}

/// Build a `PreparedSourceInput` from a prepared entry.
fn prepared_input_from_entry(
    canonical_path: &Path,
    entry: &PreparedSourceEntry,
) -> PreparedSourceInput {
    match entry.source_kind {
        SourceFileKind::Moth => PreparedSourceInput::Moth {
            source_code: entry.source_code.clone(),
            source_path: canonical_path.to_path_buf(),
            tokens: entry
                .tokens
                .clone()
                .expect("a prepared Moth entry always carries tokens"),
        },
        SourceFileKind::MothTemplate => PreparedSourceInput::MothTemplate {
            source_code: entry.source_code.clone(),
            source_path: canonical_path.to_path_buf(),
        },
        SourceFileKind::PlainMarkdown => PreparedSourceInput::PlainMarkdown {
            source_code: entry.source_code.clone(),
            source_path: canonical_path.to_path_buf(),
        },
    }
}
