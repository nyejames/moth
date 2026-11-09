//! Per-occurrence resolved targets for expression-qualified references.
//!
//! WHAT: stores the declaration one authored `PATH SYMBOL` occurrence selects, keyed by the
//!       occurrence's `(SourceId, PathSyntaxId)`.
//! WHY: Stage 3 ordering and AST expression lowering consume the target binding proved, not the
//!      authored spelling. A qualified occurrence installs no file-local name, so this lookup
//!      lives beside file visibility instead of inside it and supplies no name-based surface.
//! MUST NOT: publish a namespace, mint a local alias, or re-resolve a selected name by text.

use crate::compiler_frontend::external_packages::ExternalSymbolId;
use crate::compiler_frontend::paths::path_syntax::PathSyntaxId;
use crate::compiler_frontend::semantic_identity::OriginDeclarationId;
use crate::compiler_frontend::source::{SourceId, SourceSpan};
use crate::compiler_frontend::symbols::path_interner::PathId;
use rustc_hash::FxHashMap;

/// One resolved qualified-reference target.
///
/// WHAT: distinguishes a module-local declaration path from an imported provider declaration and
///       an external package symbol.
/// WHY: AST lowers a module-local target through the ordinary declaration table, an imported
///      provider target through its stable origin plus the consumer-local path already used by
///      direct-selection binding, and an external target by its stable symbol identity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ResolvedQualifiedTarget {
    /// A declaration authored in this module; the AST declaration table owns this exact path.
    Source { declaration_path: PathId },
    /// A declaration supplied by a completed provider interface.
    Imported {
        origin: OriginDeclarationId,
        /// The consumer-local path direct-selection binding uses for this imported declaration.
        local_path: PathId,
    },
    /// A symbol supplied by a registered external package provider.
    External { symbol_id: ExternalSymbolId },
}

impl ResolvedQualifiedTarget {
    /// Remap every consumer-local path identity after a path-fork merge.
    ///
    /// A source target names its declaration and an imported target names the consumer-local
    /// path direct-selection binding uses; both live in the merged path domain. External targets
    /// carry only a stable symbol identity.
    pub(crate) fn remap_path_ids(
        &mut self,
        remap: &crate::compiler_frontend::symbols::path_interner::PathIdRemap,
    ) {
        match self {
            Self::Source { declaration_path } => {
                *declaration_path = remap.get(*declaration_path);
            }
            Self::Imported { local_path, .. } => {
                *local_path = remap.get(*local_path);
            }
            Self::External { .. } => {}
        }
    }
}

/// One occurrence's resolved target and the authored span it must report diagnostics at.
#[derive(Clone, Debug)]
pub(crate) struct ResolvedQualifiedReference {
    pub(crate) target: ResolvedQualifiedTarget,
    pub(crate) selected_span: SourceSpan,
}

/// Per-module qualified-reference lookup keyed by owning source and authored path row.
///
/// WHY: path rows are source-local, so the owning `SourceId` disambiguates handles from different
/// prepared files without remapping; both identities survive the preparation rebinding boundary.
pub(crate) type QualifiedReferenceTargets =
    FxHashMap<(SourceId, PathSyntaxId), ResolvedQualifiedReference>;
