//! Project-boundary prepared-source store indexed by dense `SourceId`.
//!
//! WHAT: one slot per project source, transitioning `Unprepared` -> `Prepared`. Each prepared
//!       slot retains source text, tokens (for `.moth`) and structural provider references. The
//!       store is shared across all entry traversals in one project boundary so each project
//!       source is read, tokenized and prepared at most once.
//! WHY: replaces per-entry path-keyed `ScannedImportSource` caches with one shared project owner.
//!      Canonical header discovery prepares sources lazily and projects `PreparedSourceInput`
//!      values from retained slots instead of running a second per-entry lexical scan.

use crate::builder_surface::SourceFileKind;
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::interned_path::{InternedPath, NonUtf8PathComponent};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::lexer::tokenize;
use crate::compiler_frontend::tokenizer::tokens::FileTokens;
use crate::compiler_frontend::tokenizer::tokens::TokenizerEntryMode;

use std::path::Path;

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
/// The canonical Stage 0 traversal populates this store serially and reuses prepared slots across
/// module jobs, so a source shared by multiple modules is not read or tokenized again.
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
}

impl PreparedSourceStore {
    /// Create a store with one `Unprepared` slot per project source.
    pub(super) fn new(source_count: usize) -> Self {
        Self {
            project_slots: vec![PreparedSourceSlot::Unprepared; source_count],
        }
    }

    /// Prepare or reuse one indexed compiler source.
    ///
    /// Moth sources are read and tokenized once here. Header syntax preparation consumes the
    /// retained tokens and returns structural provider references, so this store never runs a
    /// parallel lexical import scan.
    pub(super) fn prepare_or_get_project_input(
        &mut self,
        source_id: SourceId,
        source_tree_index: &SourceTreeIndex,
        style_directives: &StyleDirectiveRegistry,
        string_table: &mut StringTable,
    ) -> Result<PreparedSourceInput, SourceDiscoveryError> {
        let slot = &mut self.project_slots[source_id.index()];

        if let PreparedSourceSlot::Prepared(entry) = slot {
            return Ok(prepared_input_from_entry(
                source_tree_index.source(source_id).canonical_path(),
                entry,
            ));
        }

        let record = source_tree_index.source(source_id);
        let SourceClassification::CompilerSemantic(source_kind) = record.classification() else {
            return Err(SourceDiscoveryError::from(
                crate::compiler_frontend::compiler_errors::CompilerError::compiler_error(format!(
                    "Project source ID {} is not compiler semantic",
                    source_id.index(),
                )),
            ));
        };
        let source_code = extract_source_code(record.canonical_path(), string_table)?;
        let tokens = if *source_kind == SourceFileKind::Moth {
            let interned_path = InternedPath::try_from_filesystem_path(
                record.canonical_path(),
                string_table,
            )
            .map_err(|NonUtf8PathComponent { path }| {
                SourceDiscoveryError::from(
                    crate::compiler_frontend::compiler_errors::CompilerError::file_error(
                        &path,
                        format!(
                            "Source file path {path:?} contains a non-UTF-8 component; Moth identity requires UTF-8 paths."
                        ),
                        string_table,
                    ),
                )
            })?;
            Some(Box::new(
                tokenize(
                    &source_code,
                    &interned_path,
                    TokenizerEntryMode::SourceFile,
                    style_directives,
                    string_table,
                    None,
                )
                .map_err(SourceDiscoveryError::Diagnostic)?,
            ))
        } else {
            None
        };
        *slot = PreparedSourceSlot::Prepared(PreparedSourceEntry {
            source_code,
            source_kind: *source_kind,
            tokens,
        });
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
