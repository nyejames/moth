//! Prepared and resolved structural file references.
//!
//! WHAT: classifies authored path rows that are not dependency clauses into graph-active file
//! references, and later holds the Stage 0 resolved physical targets those references consume.
//! WHY: every file-value path must be graph-active before AST, without a second tokenization or
//! an expression parse. Preparation owns classification; Stage 0 owns filesystem resolution;
//! AST interprets an already-resolved target and is given no filesystem resolver.
//!
//! Path syntax rows stay spelling and location only. This table does not store resource identity,
//! output placement, hashes or byte contents.

use crate::compiler_frontend::compiler_messages::source_location::SourceLocation;
use crate::compiler_frontend::paths::path_syntax::{PathSyntaxId, PathSyntaxTable};
use crate::compiler_frontend::paths::resource_identity::PortableResourcePath;
use crate::compiler_frontend::symbols::identity::FileId;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringIdRemap, StringTable};
use rustc_hash::{FxHashMap, FxHashSet};

/// Shallow classification of one non-dependency path row.
///
/// Classification never inspects the surrounding expression. An extensionless path is retained so
/// AST can diagnose it; a `.moth` path is retained so AST can issue the no-file-value diagnostic
/// without the file entering the semantic source set.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum PreparedFileReferenceClass {
    SiteRoot,
    ContentSource,
    ResourceFile,
    SourceKindNoFileValue,
    Extensionless,
}

/// One graph-active file-value path occurrence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PreparedFileReference {
    pub(crate) source_file: Option<FileId>,
    pub(crate) path_syntax: PathSyntaxId,
    pub(crate) location: SourceLocation,
    pub(crate) class: PreparedFileReferenceClass,
}

/// File-local table of structural file references, in authored path-row order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct PreparedFileReferenceTable {
    references: Vec<PreparedFileReference>,
}

impl PreparedFileReferenceTable {
    pub(crate) fn references(&self) -> &[PreparedFileReference] {
        &self.references
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &PreparedFileReference> {
        self.references.iter()
    }

    pub(crate) fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        for reference in &mut self.references {
            reference.location.remap_string_ids(remap);
        }
    }

    pub(crate) fn rebind_source_identity(&mut self, file_id: FileId, logical_path: &InternedPath) {
        for reference in &mut self.references {
            reference.source_file = Some(file_id);
            reference.location.rebind_source_identity(logical_path);
        }
    }
}

/// Classify every path row that a dependency clause did not consume.
///
/// WHAT: one walk of the file-owned path table. Dependency-clause rows keep their binding role
/// and are excluded from the file-value family so one authored occurrence has one semantic role.
/// WHY: graph activity follows the authored path token, including tokens inside broken
/// expressions, unused constants and unmaterialised generic bodies.
pub(crate) fn classify_prepared_file_references(
    path_syntax: &PathSyntaxTable,
    consumed_by_dependency_clauses: impl IntoIterator<Item = PathSyntaxId>,
    source_file: Option<FileId>,
    string_table: &StringTable,
) -> PreparedFileReferenceTable {
    let consumed: FxHashSet<PathSyntaxId> = consumed_by_dependency_clauses
        .into_iter()
        .filter(|id| !id.is_none())
        .collect();

    let mut references = Vec::new();
    for (path_id, row) in path_syntax.iter() {
        if consumed.contains(&path_id) {
            continue;
        }

        references.push(PreparedFileReference {
            source_file,
            path_syntax: path_id,
            location: row.location.clone(),
            class: classify_authored_path(&row.root, string_table),
        });
    }

    PreparedFileReferenceTable { references }
}

fn classify_authored_path(
    root: &InternedPath,
    string_table: &StringTable,
) -> PreparedFileReferenceClass {
    if root.is_empty() {
        return PreparedFileReferenceClass::SiteRoot;
    }

    let Some(name) = root.name_str(string_table) else {
        return PreparedFileReferenceClass::Extensionless;
    };

    match explicit_extension(name) {
        Some("mtf") | Some("md") => PreparedFileReferenceClass::ContentSource,
        Some("moth") => PreparedFileReferenceClass::SourceKindNoFileValue,
        Some(_) => PreparedFileReferenceClass::ResourceFile,
        None => PreparedFileReferenceClass::Extensionless,
    }
}

fn explicit_extension(name: &str) -> Option<&str> {
    match name.rfind('.') {
        Some(0) | None => None,
        Some(dot) if dot + 1 < name.len() => Some(&name[dot + 1..]),
        Some(_) => None,
    }
}

// Resolved targets are published by Stage 0 and consumed by AST. They are declared beside
// classification so the handoff types have one owner before the resolver is wired.
/// Dense handle for one Stage 0 resolved file reference.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ResolvedFileReferenceId(u32);

/// Build-owned physical resource source created by Stage 0.
///
/// This is a build input, not a semantic origin. AST creates `StableResourceOriginId` later and
/// associates it only when the module publishes successfully.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ResourceSourceId(u32);

impl ResourceSourceId {
    pub(crate) fn from_index(index: usize) -> Self {
        Self(index as u32)
    }

    pub(crate) fn index(self) -> usize {
        self.0 as usize
    }
}

/// Physical target Stage 0 resolved for one prepared file reference.
///
/// AST consumes this and never rediscovers the filesystem target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ResolvedFileReferenceTarget {
    ContentSource {
        source: FileId,
    },
    ResourceSource {
        source: ResourceSourceId,
        owner_relative_path: PortableResourcePath,
    },
    IdentifiedSourceKind,
}

/// The settled Stage 0 outcome for one authored file-reference occurrence.
///
/// User-authored target failures are retained beside the path identity so a surrounding AST
/// syntax diagnostic remains primary. Infrastructure failures abort through the enclosing
/// `Result` boundary instead of being represented as user diagnostics.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ResolvedFileReferenceOutcome {
    /// The authored path is structural but has no physical input target (`@/` or an
    /// extensionless path retained for AST's typed diagnostic).
    NoPhysicalTarget,
    Target(ResolvedFileReferenceTarget),
    Diagnostic(Box<crate::compiler_frontend::compiler_messages::CompilerDiagnostic>),
}

/// Module-compilation table pairing prepared path rows with Stage 0 resolved targets.
#[derive(Clone, Debug, Default)]
pub(crate) struct ResolvedFileReferenceTable {
    targets: Vec<ResolvedFileReference>,
    by_key: FxHashMap<(FileId, PathSyntaxId), ResolvedFileReferenceId>,
}

/// One resolved file-value occurrence, keyed by the preparing file and its path-syntax handle.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ResolvedFileReference {
    pub(crate) source_file: FileId,
    pub(crate) path_syntax: PathSyntaxId,
    pub(crate) class: PreparedFileReferenceClass,
    pub(crate) outcome: ResolvedFileReferenceOutcome,
}

impl ResolvedFileReferenceTable {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn push(
        &mut self,
        reference: ResolvedFileReference,
    ) -> Result<ResolvedFileReferenceId, crate::compiler_frontend::compiler_errors::CompilerError>
    {
        let key = (reference.source_file, reference.path_syntax);
        if self.by_key.contains_key(&key) {
            return Err(
                crate::compiler_frontend::compiler_errors::CompilerError::compiler_error(format!(
                    "duplicate resolved file reference for FileId {} and PathSyntaxId {:?}",
                    reference.source_file.0, reference.path_syntax
                )),
            );
        }

        let reference_id = ResolvedFileReferenceId(self.targets.len() as u32);
        self.targets.push(reference);
        self.by_key.insert(key, reference_id);
        Ok(reference_id)
    }

    pub(crate) fn get(
        &self,
        source_file: FileId,
        path_syntax: PathSyntaxId,
    ) -> Option<&ResolvedFileReference> {
        let reference_id = self.by_key.get(&(source_file, path_syntax))?;
        self.targets.get(reference_id.0 as usize)
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &ResolvedFileReference> {
        self.targets.iter()
    }

    pub(crate) fn validate(
        &self,
    ) -> Result<(), crate::compiler_frontend::compiler_errors::CompilerError> {
        if self.targets.len() != self.by_key.len() {
            return Err(
                crate::compiler_frontend::compiler_errors::CompilerError::compiler_error(
                    "resolved file-reference table has an inconsistent key index",
                ),
            );
        }
        for reference in self.iter() {
            let Some(indexed) = self.get(reference.source_file, reference.path_syntax) else {
                return Err(
                    crate::compiler_frontend::compiler_errors::CompilerError::compiler_error(
                        "resolved file-reference table lost a composite-key row",
                    ),
                );
            };
            if !std::ptr::eq(indexed, reference) {
                return Err(
                    crate::compiler_frontend::compiler_errors::CompilerError::compiler_error(
                        "resolved file-reference table composite key points at the wrong row",
                    ),
                );
            }

            let valid_outcome = match reference.class {
                PreparedFileReferenceClass::SiteRoot
                | PreparedFileReferenceClass::Extensionless => {
                    matches!(
                        reference.outcome,
                        ResolvedFileReferenceOutcome::NoPhysicalTarget
                            | ResolvedFileReferenceOutcome::Diagnostic(_)
                    )
                }
                PreparedFileReferenceClass::ContentSource => matches!(
                    reference.outcome,
                    ResolvedFileReferenceOutcome::Target(
                        ResolvedFileReferenceTarget::ContentSource { .. }
                    ) | ResolvedFileReferenceOutcome::Diagnostic(_)
                ),
                PreparedFileReferenceClass::ResourceFile => matches!(
                    reference.outcome,
                    ResolvedFileReferenceOutcome::Target(
                        ResolvedFileReferenceTarget::ResourceSource { .. }
                    ) | ResolvedFileReferenceOutcome::Diagnostic(_)
                ),
                PreparedFileReferenceClass::SourceKindNoFileValue => matches!(
                    reference.outcome,
                    ResolvedFileReferenceOutcome::Target(
                        ResolvedFileReferenceTarget::IdentifiedSourceKind
                    ) | ResolvedFileReferenceOutcome::Diagnostic(_)
                ),
            };
            if !valid_outcome {
                return Err(
                    crate::compiler_frontend::compiler_errors::CompilerError::compiler_error(
                        "resolved file-reference class does not match its settled outcome",
                    ),
                );
            }
        }

        for (key, reference_id) in &self.by_key {
            let Some(reference) = self.targets.get(reference_id.0 as usize) else {
                return Err(
                    crate::compiler_frontend::compiler_errors::CompilerError::compiler_error(
                        "resolved file-reference table key points outside its rows",
                    ),
                );
            };
            if (reference.source_file, reference.path_syntax) != *key {
                return Err(
                    crate::compiler_frontend::compiler_errors::CompilerError::compiler_error(
                        "resolved file-reference table key disagrees with its row",
                    ),
                );
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "tests/file_reference_tests.rs"]
mod tests;
