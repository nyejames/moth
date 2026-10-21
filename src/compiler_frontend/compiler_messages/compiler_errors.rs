//! # Compiler Error Handling System
//!
//! This module owns `CompilerError` (internal/tooling failures) and `CompilerMessages`
//! (render-boundary aggregation). User-facing source diagnostics live in
//! `compiler_diagnostic.rs` as `CompilerDiagnostic`.
//!
//! ## Architecture
//!
//! ```text
//! Frontend/compiler stages
//!   -> CompilerDiagnostic { kind, severity, primary_span, primary owner, labels, payload }
//!   -> DiagnosticBag accumulates one or many diagnostics locally
//!   -> CompilerMessages owns ordered diagnostics + frozen identity range rows plus
//!      transitional table/source rows at stage/build boundaries
//!   -> renderers produce terminal/dev-server/terse output
//!
//! CompilerError
//!   -> target ownership: internal/tooling/compiler failure only
//!   -> printed through one central helper
//!   -> no normal Moth source, syntax, type, rule, import, config-source,
//!      or borrow diagnostics
//! ```
//!
//! User-facing diagnostics must use typed `CompilerDiagnostic` constructors in
//! `compiler_diagnostic.rs`.
//!
//! ### What is still allowed
//! - `return_compiler_error!` — for internal compiler bugs only.
//! - `return_hir_transformation_error!` — for HIR lowering failures (compiler bugs).
//! - `return_file_error!` — for filesystem failures before source representation.
//!
//! ## Error Types
//!
//! `ErrorType` classifies internal/tooling failures that still use `CompilerError`.
//!
//! Categories:
//! - **HirTransformation / Backend** — compiler-internal lowering failures.
//! - **Compiler** — internal bugs (not user's fault).
//! - **File** — filesystem errors.
//! - **DevServer** — development server infrastructure failures.
//!
//! ## Design Principles
//!
//! ### Frozen Identity Contexts Plus Transitional StringTable Rows
//! Diagnostics preserve interned payload scopes, so top-level renderers and file-adjacent helpers
//! resolve paths through range-bound frozen identity snapshots when present, falling back to
//! the shared transitional `StringTable` for the current build or parse lifecycle.
//!
//! ### Structured Payloads
//! `CompilerDiagnostic` carries typed payloads (`DiagnosticPayload`) instead of rendered strings.
//! Renderers at the boundary resolve interned IDs and enums into human prose.
//!
//! ### Consistent Patterns
//! - Stage-local accumulation: `DiagnosticBag`.
//! - Boundary transport: `CompilerMessages`.
//! - Internal failure: `CompilerError` + immediate print.
//!
//! ## Error Flow Through Compilation Pipeline
//!
//! ```text
//! Source Code
//!     ↓
//! Tokenizer → CompilerDiagnostic (Syntax)
//!     ↓
//! Header Parser → CompilerDiagnostic (Syntax / Import / Rule)
//!     ↓
//! Dependency Sort → CompilerDiagnostic (Rule)
//!     ↓
//! AST Builder → CompilerDiagnostic (Type / Rule)
//!     ↓
//! HIR Builder → CompilerError (HirTransformation) — internal only
//!     ↓
//! Borrow Checker → CompilerDiagnostic (Borrow) + side-table facts
//!     ↓
//! Backend Lowering → CompilerError (Backend) — internal only
//!     ↓
//! CompilerMessages (ordered diagnostics + render contexts + transitional StringTable)
//! ```

use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
#[cfg(test)]
use crate::compiler_frontend::compiler_messages::DiagnosticSeverity;
use crate::compiler_frontend::compiler_messages::diagnostics::freeze::InstallMode;
use crate::compiler_frontend::compiler_messages::diagnostics::report::DiagnosticReportSet;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::semantic_identity::StablePackageIdentity;
use crate::compiler_frontend::source::{
    FrozenIdentityContext, FrozenIdentityHandle, SourceDatabase, SourceSpan, SpanCapacityError,
};
use crate::compiler_frontend::symbols::path_interner::{PathId, PathIdRemap, PathTable};
use crate::compiler_frontend::symbols::string_interning::{StringIdRemap, StringTable};
use std::collections::HashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;

// -------------------------
//  Compiler Message Set
// -------------------------

#[derive(Debug)]
pub struct CompilerMessages {
    /// Ordered diagnostics at a build/render boundary.
    ///
    /// WHAT: stores errors and warnings in the order the compiler produced them.
    /// WHY: renderers, tests, dev-server summaries, and CLI output all consume this one sequence
    /// instead of consulting parallel message stores.
    pub(crate) diagnostics: Vec<CompilerDiagnostic>,

    /// Ordered infrastructure causes beside the diagnostic stream. Several causes still describe
    /// one aborted compilation. Cold ownership keeps common diagnostics and this vessel compact.
    pub(crate) infrastructure_errors: Option<Box<InfrastructureErrors>>,

    /// Interned diagnostic strings owned by this boundary.
    ///
    /// The table is separately allocated because the message vessel crosses many `Result`
    /// boundaries and must remain below the large-error threshold without boxing individual
    /// `CompilerDiagnostic` values.
    pub string_table: Box<StringTable>,

    /// Freeze state of this vessel, read by the boundary converter.
    ///
    /// WHAT: `Frozen` once a production boundary has finalised the identity rows its diagnostics
    /// render against; `Unfrozen` while the vessel is premerge or carries no such rows.
    /// WHY: the mode travels with the vessel through merges, so the converter takes no separate
    /// install argument. A merge that absorbs a frozen vessel becomes frozen.
    pub(crate) install_mode: InstallMode,

    /// Per-diagnostic source snapshots used by diagnostic renderers (transitional).
    ///
    /// A message set may aggregate diagnostics from several independently retained source
    /// databases, so this association is range-bound rather than one flat database for the whole
    /// set. Each range is shifted alongside its diagnostics during aggregation.
    pub(crate) render_source_contexts: Vec<RenderSourceContext>,

    /// Module-local type tables used only by diagnostic renderers.
    ///
    /// WHAT: carries semantic type lookup tables beside the diagnostics produced with them.
    /// WHY: type diagnostics store `TypeId`s. Renderers need the matching module environment for
    /// each diagnostic index, but individual diagnostics must not own that environment.
    ///
    /// Boundary shape: this is intentionally owned by `CompilerMessages` only on failed module or
    /// build boundaries where diagnostics outlive the AST/HIR owner that still has the active
    /// `TypeEnvironment`. Successful builds carry the module type table in `Module`, not here.
    pub(crate) render_type_contexts: Vec<RenderTypeContext>,
    /// Per-diagnostic path-table snapshots used by transitional diagnostic renderers.
    ///
    /// A row is retained only when a direct path payload or type-render context consumes paths.
    /// Path-free diagnostics without type contexts retain no caller-supplied table. Retained rows
    /// remain separate from source/frozen contexts because diagnosed lanes may finish before a
    /// source database or boundary path builder exists.
    pub(crate) render_path_contexts: Option<Box<[RenderPathContext]>>,
}

const _: () = assert!(std::mem::size_of::<CompilerMessages>() <= 128);
#[derive(Debug, Clone)]
pub(crate) struct RenderTypeContext {
    /// Range whose `TypeId` payloads resolve against this mutable environment. Aggregation shifts
    ///       the range and remaps it with its string table before the vessel's single boundary
    ///       conversion captures the type-display snapshot.
    pub(crate) diagnostic_range: Range<usize>,
    pub(crate) type_environment: TypeEnvironment,
}
#[derive(Debug, Clone)]
pub(crate) struct RenderSourceContext {
    pub(crate) diagnostic_range: Range<usize>,
    pub(crate) source_database: Arc<SourceDatabase>,
    pub(crate) domain: Option<StablePackageIdentity>,
}
#[derive(Debug, Clone)]
pub(crate) struct RenderPathContext {
    pub(crate) diagnostic_range: Range<usize>,
    pub(crate) path_table: Arc<PathTable>,
}

/// Rare failure causes and their explicitly paired source owners. Cause ranges, unlike diagnostic
/// ranges, remain meaningful when a failed compilation produced no diagnostic at all.
#[derive(Debug)]
pub(crate) struct InfrastructureErrors {
    pub(crate) primary: CompilerError,
    pub(crate) additional: Vec<CompilerError>,
    source_contexts: Vec<InfrastructureSourceContext>,
}

/// Boundary failure carrying frozen user diagnostics and the transitional infrastructure lane.
#[derive(Debug)]
pub struct BoundaryFailure {
    pub(crate) reports: DiagnosticReportSet,
    /// Transitional infrastructure lane deleted by Phase 5 slices 5C, 5E, W5 and 5F.
    pub(crate) infrastructure: Option<Box<InfrastructureErrors>>,
}

const _: () = assert!(std::mem::size_of::<BoundaryFailure>() <= 56);

impl BoundaryFailure {
    /// Count error records plus one for the infrastructure lane, matching `CompilerMessages`.
    pub(crate) fn error_count(&self) -> usize {
        let record_errors = usize::try_from(self.reports.counts().errors)
            .expect("a report set's error count fits in usize");
        record_errors + usize::from(self.has_infrastructure_error())
    }

    /// Count warning records, matching `CompilerMessages`.
    pub(crate) fn warning_count(&self) -> usize {
        usize::try_from(self.reports.counts().warnings)
            .expect("a report set's warning count fits in usize")
    }

    /// Return whether the infrastructure lane or any error record is present.
    pub(crate) fn has_errors(&self) -> bool {
        self.has_infrastructure_error() || self.reports.counts().errors > 0
    }

    /// Return whether any warning record is present.
    pub(crate) fn has_warnings(&self) -> bool {
        self.reports.counts().warnings > 0
    }

    /// Return whether the transitional infrastructure lane is populated.
    pub(crate) fn has_infrastructure_error(&self) -> bool {
        self.infrastructure.is_some()
    }

    /// Borrow the transitional infrastructure lane when it is populated.
    pub(crate) fn infrastructure_errors(&self) -> Option<&InfrastructureErrors> {
        self.infrastructure.as_deref()
    }
}

#[derive(Debug)]
struct InfrastructureSourceContext {
    cause_range: Range<usize>,
    owner: InfrastructureSourceOwner,
}

#[derive(Debug)]
enum InfrastructureSourceOwner {
    Source(Arc<SourceDatabase>),
    #[cfg_attr(
        not(feature = "data_layout_memory_probe"),
        allow(dead_code, reason = "P4.5 S3.3b build_project")
    )]
    Frozen(Arc<FrozenIdentityContext>),
}

impl InfrastructureErrors {
    pub(crate) fn new(primary: CompilerError) -> Self {
        Self {
            primary,
            additional: Vec::new(),
            source_contexts: Vec::new(),
        }
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &CompilerError> {
        std::iter::once(&self.primary).chain(self.additional.iter())
    }

    #[cfg(feature = "data_layout_memory_probe")]
    pub(crate) fn frozen_identities(&self) -> impl Iterator<Item = &Arc<FrozenIdentityContext>> {
        self.source_contexts
            .iter()
            .filter_map(|context| match &context.owner {
                InfrastructureSourceOwner::Frozen(identity) => Some(identity),
                InfrastructureSourceOwner::Source(_) => None,
            })
    }

    #[cfg(feature = "data_layout_memory_probe")]
    pub(crate) fn source_databases(&self) -> impl Iterator<Item = &Arc<SourceDatabase>> {
        self.source_contexts
            .iter()
            .filter_map(|context| match &context.owner {
                InfrastructureSourceOwner::Source(source) => Some(source),
                InfrastructureSourceOwner::Frozen(_) => None,
            })
    }

    /// Frozen causes already belong to an immutable identity domain and must not be rebound.
    pub(crate) fn unfrozen_iter_mut(&mut self) -> impl Iterator<Item = &mut CompilerError> {
        let contexts = &self.source_contexts;
        std::iter::once(&mut self.primary)
            .chain(self.additional.iter_mut())
            .enumerate()
            .filter_map(move |(index, error)| {
                let frozen = contexts.iter().any(|context| {
                    context.cause_range.contains(&index)
                        && matches!(context.owner, InfrastructureSourceOwner::Frozen(_))
                });
                (!frozen).then_some(error)
            })
    }

    pub(crate) fn push(&mut self, error: CompilerError) {
        self.additional.push(error);
    }

    pub(crate) fn append(&mut self, other: Self) {
        let shift = 1 + self.additional.len();
        self.additional.push(other.primary);
        self.additional.extend(other.additional);
        self.source_contexts
            .extend(other.source_contexts.into_iter().map(|mut context| {
                context.cause_range.start += shift;
                context.cause_range.end += shift;
                context
            }));
    }

    pub(crate) fn set_source_database(&mut self, source_database: Arc<SourceDatabase>) {
        // Attach only unpaired causes. A later project fallback must not replace a package owner.
        for index in 0..1 + self.additional.len() {
            if !self
                .source_contexts
                .iter()
                .any(|context| context.cause_range.contains(&index))
            {
                self.source_contexts.push(InfrastructureSourceContext {
                    cause_range: index..index + 1,
                    owner: InfrastructureSourceOwner::Source(Arc::clone(&source_database)),
                });
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn source_database_for_cause(&self, index: usize) -> Option<&SourceDatabase> {
        self.source_contexts.iter().find_map(|context| {
            if context.cause_range.contains(&index)
                && let InfrastructureSourceOwner::Source(source) = &context.owner
            {
                return Some(source.as_ref());
            }
            None
        })
    }

    #[cfg(test)]
    pub(crate) fn frozen_identity_for_cause(&self, index: usize) -> Option<&FrozenIdentityContext> {
        self.source_contexts.iter().find_map(|context| {
            if context.cause_range.contains(&index)
                && let InfrastructureSourceOwner::Frozen(identity) = &context.owner
            {
                return Some(identity.as_ref());
            }
            None
        })
    }

    /// Move the source-owned cause rows out so the boundary converter can re-home their databases.
    pub(crate) fn take_source_rows(&mut self) -> Vec<(Range<usize>, Arc<SourceDatabase>)> {
        std::mem::take(&mut self.source_contexts)
            .into_iter()
            .map(|context| match context.owner {
                InfrastructureSourceOwner::Source(database) => (context.cause_range, database),
                InfrastructureSourceOwner::Frozen(_) => {
                    panic!("pre-frozen infrastructure source owner reached the boundary converter")
                }
            })
            .collect()
    }

    /// Restore the cause rows as frozen owners once the converter has built their domains.
    pub(crate) fn set_frozen_source_rows(
        &mut self,
        rows: Vec<(Range<usize>, Arc<FrozenIdentityContext>)>,
    ) {
        assert!(
            self.source_contexts.is_empty(),
            "frozen cause rows replace only the rows taken for the converter"
        );
        self.source_contexts = rows
            .into_iter()
            .map(|(cause_range, identity)| InfrastructureSourceContext {
                cause_range,
                owner: InfrastructureSourceOwner::Frozen(identity),
            })
            .collect();
    }
}

impl From<CompilerError> for Box<InfrastructureErrors> {
    fn from(error: CompilerError) -> Self {
        Box::new(InfrastructureErrors::new(error))
    }
}

impl CompilerMessages {
    pub fn empty(string_table: StringTable) -> Self {
        Self {
            diagnostics: Vec::new(),
            infrastructure_errors: None,
            string_table: Box::new(string_table),
            install_mode: InstallMode::Unfrozen,
            render_source_contexts: Vec::new(),
            render_type_contexts: Vec::new(),
            render_path_contexts: None,
        }
    }

    pub(crate) fn from_diagnostics(
        diagnostics: Vec<CompilerDiagnostic>,
        string_table: StringTable,
    ) -> Self {
        Self {
            diagnostics,
            infrastructure_errors: None,
            string_table: Box::new(string_table),
            install_mode: InstallMode::Unfrozen,
            render_source_contexts: Vec::new(),
            render_type_contexts: Vec::new(),
            render_path_contexts: None,
        }
    }

    /// Retain the path table that issued diagnostics before the local compiler owner is dropped.
    ///
    /// The snapshot is range-bound to the diagnostics it serves. A caller-supplied table is
    /// retained only when a renderer consumes path identities: a diagnostic payload renders a
    /// `PathId`, or a retained type context needs the table for nominal type names. Path-free
    /// diagnostics without type contexts have no path consumer, so their supplied table is
    /// discarded rather than retained merely as incidental context.
    ///
    /// When retention is required, pairing validates both the path identities the payloads
    /// render (each payload path and every ancestor its resolver walks) and every non-root node
    /// in the table. The latter is required because later aggregation remaps the complete
    /// retained table, including nodes no current payload dereferences. A missing required path
    /// or a component the paired string table cannot resolve returns a typed `CompilerError`
    /// instead of silently dropping or corrupting the context. Same-index ownership across
    /// foreign domains is not detectable from bare tables; the provider/materialisation boundary
    /// solves it by retaining each table with its source `FrozenStringTable` and rebasing by
    /// spelling.
    #[cfg(test)]
    pub(crate) fn attach_path_table_if_missing(
        &mut self,
        path_table: Arc<PathTable>,
    ) -> Result<(), CompilerError> {
        if self.diagnostics.is_empty()
            || self
                .render_path_contexts
                .as_ref()
                .is_some_and(|contexts| !contexts.is_empty())
        {
            return Ok(());
        }

        let mut required_paths = Vec::new();
        for diagnostic in &self.diagnostics {
            diagnostic
                .payload
                .required_path_payload_paths(&mut required_paths);
        }
        if required_paths.is_empty() && self.render_type_contexts.is_empty() {
            return Ok(());
        }

        if let Some(index) =
            unpaired_path_table_component_index(&path_table, &self.string_table, &required_paths)
        {
            return Err(CompilerError::compiler_error(format!(
                "diagnostic path table is not pairable with this string table: path node \
                 {index} is unaddressable or carries a component StringId the string table \
                 cannot resolve"
            )));
        }

        self.render_path_contexts = Some(
            vec![RenderPathContext {
                diagnostic_range: 0..self.diagnostics.len(),
                path_table,
            }]
            .into_boxed_slice(),
        );
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn has_infrastructure_error(&self) -> bool {
        self.infrastructure_errors.is_some()
    }

    /// Explicit first-cause access for consumers whose contract concerns only the primary failure.
    #[cfg(test)]
    pub(crate) fn primary_infrastructure_error(&self) -> Option<&CompilerError> {
        self.infrastructure_errors
            .as_ref()
            .map(|errors| &errors.primary)
    }

    #[cfg(test)]
    pub(crate) fn infrastructure_errors(&self) -> Option<&InfrastructureErrors> {
        self.infrastructure_errors.as_deref()
    }

    /// Append one cause without merging its message or provenance into an unrelated failure.
    pub(crate) fn set_infrastructure_error(&mut self, error: CompilerError) {
        match self.infrastructure_errors.as_mut() {
            Some(errors) => errors.push(error),
            None => self.infrastructure_errors = Some(error.into()),
        }
    }

    pub(crate) fn append_infrastructure_errors(&mut self, errors: Box<InfrastructureErrors>) {
        match self.infrastructure_errors.as_mut() {
            Some(existing) => existing.append(*errors),
            None => self.infrastructure_errors = Some(errors),
        }
    }

    #[cfg(test)]
    pub fn has_errors(&self) -> bool {
        self.infrastructure_errors.is_some()
            || self
                .diagnostics
                .iter()
                .any(|d| d.severity == DiagnosticSeverity::Error)
    }

    /// Count diagnostics with `Error` severity.
    #[cfg(test)]
    pub fn error_count(&self) -> usize {
        self.diagnostics
            .iter()
            .filter(|d| d.severity == DiagnosticSeverity::Error)
            .count()
            + usize::from(self.infrastructure_errors.is_some())
    }

    /// Count diagnostics with `Warning` severity.
    #[cfg(test)]
    pub fn warning_count(&self) -> usize {
        self.diagnostics
            .iter()
            .filter(|d| d.severity == DiagnosticSeverity::Warning)
            .count()
    }

    #[cfg(test)]
    pub fn has_warnings(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.severity == DiagnosticSeverity::Warning)
    }

    /// Iterate over every diagnostic in compiler production order.
    ///
    /// WHAT: exposes the single boundary diagnostic stream without implying an error-only mirror.
    /// WHY: renderers and reports often need to preserve ordering while applying their own
    /// severity policy locally.
    pub(crate) fn diagnostics(&self) -> impl Iterator<Item = &CompilerDiagnostic> {
        self.diagnostics.iter()
    }

    /// Borrow the ordered diagnostic stream for render helpers that need a slice.
    pub(crate) fn diagnostic_slice(&self) -> &[CompilerDiagnostic] {
        &self.diagnostics
    }

    /// Record that this vessel's identity rows are final, so the boundary converter installs them.
    ///
    /// WHAT: sets the install mode to `Frozen`. The mode travels with the vessel through merges.
    /// WHY: production boundaries freeze once through the run accumulator; this setter now only
    /// builds frozen legacy fixtures for the parity oracle and render tests.
    #[cfg(test)]
    pub(crate) fn mark_frozen(&mut self) {
        self.install_mode = InstallMode::Frozen;
    }

    /// Append already-structured diagnostics while preserving current order.
    pub(crate) fn extend_diagnostics(
        &mut self,
        diagnostics: impl IntoIterator<Item = CompilerDiagnostic>,
    ) {
        self.diagnostics.extend(diagnostics);
    }
    /// Fill missing diagnostic span owners without replacing mixed-domain provenance.
    pub(crate) fn set_frozen_identity_handle_if_missing(
        &mut self,
        frozen_identity_handle: FrozenIdentityHandle,
    ) {
        for diagnostic in &mut self.diagnostics {
            diagnostic.attach_frozen_identity_handle_if_missing(frozen_identity_handle.clone());
        }
    }

    /// Iterate over diagnostics with `Error` severity.
    #[cfg(test)]
    pub(crate) fn error_diagnostics(&self) -> impl Iterator<Item = &CompilerDiagnostic> {
        self.diagnostics
            .iter()
            .filter(|d| d.severity == DiagnosticSeverity::Error)
    }

    /// Return the first error-severity diagnostic, preserving diagnostic order.
    #[cfg(test)]
    pub(crate) fn first_error(&self) -> Option<&CompilerDiagnostic> {
        self.error_diagnostics().next()
    }

    /// Wrap a single `CompilerDiagnostic` into a `CompilerMessages` container with no warnings.
    ///
    /// WHY: frontend stages emit `CompilerDiagnostic` values directly and need a clean boundary
    /// conversion into the message container expected by build-system callers.
    pub fn from_diagnostic(diagnostic: CompilerDiagnostic, string_table: StringTable) -> Self {
        Self {
            diagnostics: vec![diagnostic],
            infrastructure_errors: None,
            string_table: Box::new(string_table),
            install_mode: InstallMode::Unfrozen,
            render_source_contexts: Vec::new(),
            render_type_contexts: Vec::new(),
            render_path_contexts: None,
        }
    }

    /// Wrap a single `CompilerDiagnostic` while cloning the caller's active `StringTable`.
    pub fn from_diagnostic_ref(diagnostic: CompilerDiagnostic, string_table: &StringTable) -> Self {
        Self::from_diagnostic(diagnostic, string_table.clone())
    }
    /// Carry a single `CompilerError` on the outer infrastructure lane with no warnings.
    ///
    /// WHY: Several build/backend modules need to surface a `CompilerError` through the richer
    /// `CompilerMessages` type at a boundary. Centralising this avoids repeated inline struct
    /// literals scattered across callers. The error stays typed: no diagnostic payload is
    /// fabricated and its message, source span, host path, type and metadata are preserved as-is.
    ///
    /// The table remains owned by `CompilerMessages` for diagnostic payloads and render contexts;
    /// it is never attached to or remapped through the infrastructure error.
    pub fn from_error(error: CompilerError, string_table: StringTable) -> Self {
        Self {
            diagnostics: Vec::new(),
            infrastructure_errors: Some(error.into()),
            string_table: Box::new(string_table),
            install_mode: InstallMode::Unfrozen,
            render_source_contexts: Vec::new(),
            render_type_contexts: Vec::new(),
            render_path_contexts: None,
        }
    }

    /// Wrap one error while cloning the caller's active `StringTable`.
    ///
    /// WHAT: snapshots the current table state into the returned message container.
    /// WHY: frontend/build boundaries often only borrow the shared table, but diagnostics still
    /// need the full interned payload context accumulated so far.
    pub fn from_error_ref(error: CompilerError, string_table: &StringTable) -> Self {
        Self::from_error(error, string_table.clone())
    }

    /// Carry already-collected warnings plus one infrastructure error on the outer lane.
    ///
    /// WHAT: carries forward the caller's warning set on the diagnostic stream and clones the
    /// current `StringTable`.
    /// WHY: these helpers receive warnings that were produced before the failure. Keeping that
    /// order makes `CompilerMessages` a true production-order diagnostic stream with the typed
    /// failure reported beside it. The error retains its own typed provenance and does not
    /// borrow or remap the warning table.
    pub fn from_error_with_warnings(
        error: CompilerError,
        warning_diagnostics: Vec<CompilerDiagnostic>,
        string_table: &StringTable,
    ) -> Self {
        Self {
            diagnostics: warning_diagnostics,
            infrastructure_errors: Some(error.into()),
            string_table: Box::new(string_table.clone()),
            install_mode: InstallMode::Unfrozen,
            render_source_contexts: Vec::new(),
            render_type_contexts: Vec::new(),
            render_path_contexts: None,
        }
    }

    /// Wrap already-collected warnings plus one typed diagnostic while preserving table context.
    ///
    /// WHAT: carries forward the caller's warning set and a clone of the current `StringTable`,
    /// then stores the typed boundary diagnostic directly in `diagnostics`.
    /// WHY: frontend stages that emit `CompilerDiagnostic` need to preserve structured payloads
    /// so that boundary renderers can resolve `StringId` values through the shared `StringTable`.
    pub fn from_diagnostic_with_warnings(
        diagnostic: CompilerDiagnostic,
        warning_diagnostics: Vec<CompilerDiagnostic>,
        string_table: &StringTable,
    ) -> Self {
        let mut diagnostics = warning_diagnostics;
        diagnostics.push(diagnostic);
        Self {
            diagnostics,
            infrastructure_errors: None,
            string_table: Box::new(string_table.clone()),
            install_mode: InstallMode::Unfrozen,
            render_source_contexts: Vec::new(),
            render_type_contexts: Vec::new(),
            render_path_contexts: None,
        }
    }

    /// Carry a filesystem failure while retaining the caller's table for any surrounding
    /// diagnostics.
    pub fn file_error(path: &Path, msg: impl Into<String>, string_table: &StringTable) -> Self {
        Self::from_error(CompilerError::file_error(path, msg), string_table.clone())
    }

    /// Pair the producing source owner with current diagnostics and previously unpaired causes.
    /// Infrastructure ownership uses cause indices, never a fabricated diagnostic-zero row.
    /// Source rows remain mutable through aggregation; the boundary converter remaps their
    /// identity domains before freezing reports in the vessel's single conversion.
    pub(crate) fn set_source_database(&mut self, source_database: Arc<SourceDatabase>) {
        if let Some(errors) = self.infrastructure_errors.as_mut() {
            errors.set_source_database(Arc::clone(&source_database));
        }
        if !self.diagnostics.is_empty() {
            self.render_source_contexts.push(RenderSourceContext {
                diagnostic_range: 0..self.diagnostics.len(),
                source_database,
                domain: None,
            });
        }
    }

    /// Install precomputed source contexts for diagnostics appended at `diagnostic_offset`.
    ///
    /// Successful-build warnings are collected before the final project-owned warnings are
    /// appended, while output failures may prepend their own diagnostics. Shifting each retained
    /// range at installation keeps the context indices tied to the warning vector in either case.
    pub(crate) fn install_source_contexts(
        &mut self,
        source_contexts: impl IntoIterator<Item = RenderSourceContext>,
        diagnostic_offset: usize,
    ) {
        self.render_source_contexts
            .extend(source_contexts.into_iter().map(|mut source_context| {
                source_context.diagnostic_range.start += diagnostic_offset;
                source_context.diagnostic_range.end += diagnostic_offset;
                source_context
            }));
    }

    pub(crate) fn with_type_context_for_all_diagnostics(
        mut self,
        type_environment: TypeEnvironment,
    ) -> Self {
        if !self.diagnostics.is_empty() {
            self.render_type_contexts.push(RenderTypeContext {
                diagnostic_range: 0..self.diagnostics.len(),
                type_environment,
            });
        }
        self
    }

    /// Prepend diagnostics that were produced before this message set.
    ///
    /// WHAT: shifts every stored source, type and path render-context range forward by the
    /// prepended length.
    /// WHY: frontend/build aggregation often carries warnings from earlier stages into a later
    /// failure. Those warnings must stay before the failure without disconnecting diagnostics from
    /// their retained source snapshot or render type table.
    // The compiler-source-token-and-diagnostic-data-layout plan retains direct-template warning
    // contexts. Test-gated template pipelines and diagnostic aggregation tests consume this method.
    #[cfg_attr(not(test), allow(dead_code, reason = "P4.6 tests move to typed views"))]
    pub(crate) fn prepend_diagnostics_preserving_context(
        &mut self,
        prior_diagnostics: impl IntoIterator<Item = CompilerDiagnostic>,
    ) {
        let mut prior_diagnostics = prior_diagnostics.into_iter().collect::<Vec<_>>();
        let shift = prior_diagnostics.len();

        if shift == 0 {
            return;
        }

        prior_diagnostics.append(&mut self.diagnostics);
        self.diagnostics = prior_diagnostics;

        for source_context in &mut self.render_source_contexts {
            source_context.diagnostic_range.start += shift;
            source_context.diagnostic_range.end += shift;
        }

        for type_context in &mut self.render_type_contexts {
            type_context.diagnostic_range.start += shift;
            type_context.diagnostic_range.end += shift;
        }
        if let Some(path_contexts) = self.render_path_contexts.as_mut() {
            for path_context in path_contexts.iter_mut() {
                path_context.diagnostic_range.start += shift;
                path_context.diagnostic_range.end += shift;
            }
        }
    }
    /// Append another boundary message set while preserving its render contexts.
    ///
    /// WHAT: moves diagnostics and all source, type and path render-context ranges from `messages`
    ///       into this set, offsets the appended ranges by the current diagnostic count, and
    ///       remaps premerge facts into this set's table.
    /// WHY: this is the retired production merge. It survives only as the parity oracle that S4a's
    ///      accumulator tests compare against; every production tail now appends to the run
    ///      accumulator and freezes once.
    #[cfg(test)]
    pub(crate) fn append_messages_preserving_context(&mut self, messages: CompilerMessages) {
        let CompilerMessages {
            mut diagnostics,
            infrastructure_errors,
            string_table,
            install_mode,
            mut render_source_contexts,
            mut render_type_contexts,
            mut render_path_contexts,
        } = messages;
        let remap = self.string_table.merge_from(string_table.as_ref());
        remap_diagnostics(&mut diagnostics, &remap);
        remap_type_contexts(&mut render_type_contexts, &remap);
        remap_path_contexts(render_path_contexts.as_deref_mut(), &remap);

        let shift = self.diagnostics.len();
        self.diagnostics.append(&mut diagnostics);

        for mut source_context in render_source_contexts.drain(..) {
            source_context.diagnostic_range.start += shift;
            source_context.diagnostic_range.end += shift;
            self.render_source_contexts.push(source_context);
        }

        if let Some(mut path_contexts) = render_path_contexts {
            for path_context in &mut path_contexts {
                path_context.diagnostic_range.start += shift;
                path_context.diagnostic_range.end += shift;
            }
            let mut combined = self
                .render_path_contexts
                .take()
                .map(|boxed| boxed.into_vec())
                .unwrap_or_default();
            combined.extend(path_contexts.into_vec());
            self.render_path_contexts = Some(combined.into_boxed_slice());
        }
        for mut type_context in render_type_contexts {
            type_context.diagnostic_range.start += shift;
            type_context.diagnostic_range.end += shift;
            self.render_type_contexts.push(type_context);
        }

        if install_mode == InstallMode::Frozen {
            self.mark_frozen();
        }

        if let Some(errors) = infrastructure_errors {
            self.append_infrastructure_errors(errors);
        }
    }
    /// Remap premerge facts into another mutable string-table domain.
    ///
    /// Diagnostics, type contexts and path contexts all move through the supplied mutable-table
    /// remap. The boundary converter owns identity installation, so no fact is exempt here.
    pub fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        remap_diagnostics(&mut self.diagnostics, remap);
        remap_type_contexts(&mut self.render_type_contexts, remap);
        remap_path_contexts(self.render_path_contexts.as_deref_mut(), remap);
    }

    /// Remap complete logical paths for diagnostics and renderer-owned type contexts.
    ///
    /// Diagnostics and type environments belong to the worker fork and are rewritten through the
    /// same `PathIdRemap` used by the rest of the module facts.
    pub fn remap_path_ids(&mut self, remap: &PathIdRemap) {
        if remap.is_identity() {
            return;
        }
        for diagnostic in &mut self.diagnostics {
            diagnostic.remap_path_ids(remap);
        }
        for context in &mut self.render_type_contexts {
            context.type_environment.remap_path_ids(remap);
        }
    }

    /// Resolve the source database that produced one diagnostic.
    ///
    /// The first matching association wins, and that ordering is load-bearing: build and check
    /// attach the project database over the whole message set as a fallback, so a package's own
    /// association must already be present to take precedence. Rendering a colliding logical path
    /// against the wrong database shows a different file's text.
    pub(crate) fn source_database_for_diagnostic(
        &self,
        diagnostic_index: usize,
    ) -> Option<&SourceDatabase> {
        self.render_source_contexts
            .iter()
            .find(|source_context| source_context.diagnostic_range.contains(&diagnostic_index))
            .map(|source_context| source_context.source_database.as_ref())
    }
    pub(crate) fn path_table_for_diagnostic(&self, diagnostic_index: usize) -> Option<&PathTable> {
        self.render_path_contexts
            .as_deref()
            .and_then(|contexts| {
                contexts
                    .iter()
                    .find(|context| context.diagnostic_range.contains(&diagnostic_index))
            })
            .map(|context| context.path_table.as_ref())
    }

    pub(crate) fn type_environment_for_diagnostic(
        &self,
        diagnostic_index: usize,
    ) -> Option<&TypeEnvironment> {
        self.render_type_contexts
            .iter()
            .find(|type_context| type_context.diagnostic_range.contains(&diagnostic_index))
            .map(|type_context| &type_context.type_environment)
    }
    /// Rebuild the legacy diagnostic context for test-only render parity checks.
    ///
    /// Production rendering uses compact report views; this adapter remains only for legacy
    /// baselines until P4.6 moves the remaining tests to typed views.
    #[cfg(test)]
    pub(crate) fn diagnostic_render_context(
        &self,
        diagnostic_index: usize,
    ) -> crate::compiler_frontend::compiler_messages::render::DiagnosticRenderContext<'_> {
        crate::compiler_frontend::compiler_messages::render::DiagnosticRenderContext::new(
            self.string_table.as_ref(),
        )
        .with_optional_type_environment(self.type_environment_for_diagnostic(diagnostic_index))
        .with_optional_source_database(self.source_database_for_diagnostic(diagnostic_index))
        .with_optional_path_table(self.path_table_for_diagnostic(diagnostic_index))
    }

    #[cfg(test)]
    pub(crate) fn render_type_contexts(&self) -> &[RenderTypeContext] {
        &self.render_type_contexts
    }
}

/// Find the first defect that makes retaining a path table unsafe.
///
/// WHAT: one pairing check shared by the diagnosed batch and message-set attach boundaries.
/// WHY: `PathTable::contains` only proves numeric addressability, while later aggregation
///      remaps every component in a retained table. This checks each payload path and every
///      ancestor its renderer walks for addressability and string resolution, then checks every
///      non-root table node so unused components are also safe for that complete remap.
///      Same-index ownership across foreign domains is not detectable from bare
///      `PathTable`+`StringTable` pairs and is intentionally not attempted here: the
///      provider/materialisation boundary retains each path table with its source
///      `FrozenStringTable` and re-interns by spelling through
///      [`ModuleMaterialisationContext::rebased_for_requester`]. Payloads without path
///      identities contribute no required chains; callers skip retention entirely when they
///      have no path payloads and no type-render contexts. The root node's component is
///      deliberately uninterpreted and is skipped.
pub(crate) fn unpaired_path_table_component_index(
    path_table: &PathTable,
    string_table: &StringTable,
    required_paths: &[PathId],
) -> Option<usize> {
    let unpaired_path = required_paths
        .iter()
        .copied()
        .flat_map(|path| {
            std::iter::successors(Some(path), |&current| path_table.try_parent(current))
        })
        .find(|&path| {
            path != PathId::ROOT
                && path_table
                    .try_component(path)
                    .is_none_or(|component| string_table.try_resolve(component).is_none())
        })
        .map(PathId::index);
    if unpaired_path.is_some() {
        return unpaired_path;
    }

    (1..path_table.len()).find_map(|index| {
        let path = PathId::try_from_index(index)?;
        let component = path_table.try_component(path)?;
        string_table
            .try_resolve(component)
            .is_none()
            .then_some(index)
    })
}

fn remap_diagnostics(diagnostics: &mut [CompilerDiagnostic], remap: &StringIdRemap) {
    if remap.is_identity() {
        return;
    }

    for diagnostic in diagnostics {
        diagnostic.remap_string_ids(remap);
    }
}

fn remap_type_contexts(type_contexts: &mut [RenderTypeContext], remap: &StringIdRemap) {
    if remap.is_identity() {
        return;
    }

    for context in type_contexts {
        context.type_environment.remap_string_ids(remap);
    }
}

/// Remap the string IDs of each path table in place.
///
/// A path table shared with another owner is copied on write by `Arc::make_mut`.
fn remap_path_contexts(path_contexts: Option<&mut [RenderPathContext]>, remap: &StringIdRemap) {
    if remap.is_identity() {
        return;
    }
    let Some(path_contexts) = path_contexts else {
        return;
    };

    for context in path_contexts {
        Arc::make_mut(&mut context.path_table).remap_string_ids(remap);
    }
}

#[derive(Debug, Eq, Hash, PartialEq, Clone)]
pub enum CompilerErrorMetadataKey {
    CompilationStage,
    OutputRejectionReason,

    // Optional guidance for direct internal/tooling error rendering.
    PrimarySuggestion,
    AlternativeSuggestion,
    SuggestedReplacement,
    SuggestedInsertion,
    SuggestedLocation,
}

// -------------------------
//  Internal Compiler Error
// -------------------------

#[derive(Debug, Clone)]
pub struct CompilerError {
    pub msg: String,

    /// Exact source provenance when this failure has an authored source range.
    pub source_span: Option<SourceSpan>,

    /// Host filesystem path for failures that occur before a source span exists.
    pub host_path: Option<PathBuf>,

    pub error_type: ErrorType,

    // Structured guidance for internal/tooling failures. User-facing diagnostics carry typed
    // payload facts on `CompilerDiagnostic` instead of using this string map.
    pub metadata: HashMap<CompilerErrorMetadataKey, String>,
}

impl CompilerError {
    pub fn new(
        msg: impl Into<String>,
        source_span: Option<SourceSpan>,
        error_type: ErrorType,
    ) -> CompilerError {
        CompilerError {
            msg: msg.into(),
            source_span,
            host_path: None,
            error_type,
            metadata: HashMap::new(),
        }
    }

    /// Classify an unrepresentable source-span end as a compiler invariant failure.
    ///
    /// Extended-table exhaustion is converted into a typed source diagnostic by
    /// `CompilerDiagnostic::from_span_capacity_error`; this constructor is reserved for the
    /// accepted-source-size invariant that cannot represent `start + length`.
    pub(crate) fn source_span_capacity(
        error: SpanCapacityError,
        source_span: Option<SourceSpan>,
    ) -> Self {
        debug_assert!(
            matches!(
                error.reason(),
                crate::compiler_frontend::source::SpanCapacityReason::EndUnrepresentable
            ),
            "only unrepresentable span ends use the compiler-error capacity lane",
        );
        let message = format!(
            "span at byte offset {} with length {} ends past u32::MAX in a source \
             that was accepted as addressable; this is a compiler bug",
            error.start(),
            error.length(),
        );
        Self::new(message, source_span, ErrorType::Compiler)
    }

    /// Attach structured guidance metadata to this error and return it for chaining.
    ///
    /// WHAT: replaces the metadata map wholesale.
    /// WHY: infrastructure boundaries recover an error's metadata alongside its message and
    ///      typed provenance, so a single builder keeps that reconstruction readable without
    ///      repeated `new_metadata_entry` calls.
    pub fn with_metadata(mut self, metadata: HashMap<CompilerErrorMetadataKey, String>) -> Self {
        self.metadata = metadata;
        self
    }

    pub fn with_error_type(mut self, error_type: ErrorType) -> Self {
        self.error_type = error_type;
        self
    }

    pub fn new_metadata_entry(&mut self, key: CompilerErrorMetadataKey, value: String) {
        self.metadata.insert(key, value);
    }

    /// Create a thread panic error (internal compiler_frontend issue).
    pub fn new_thread_panic(msg: impl Into<String>) -> Self {
        Self::new(msg, None, ErrorType::Compiler)
    }

    /// Create a compiler_frontend error (internal bug, not user's fault).
    // Existing backend and frontend invariant checks use `CompilerError::compiler_error(...)`
    // as the direct constructor for infrastructure diagnostics.
    #[allow(clippy::self_named_constructors)]
    pub fn compiler_error(msg: impl Into<String>) -> Self {
        Self::new(msg, None, ErrorType::Compiler)
    }

    /// Create a filesystem error from a host path.
    pub fn file_error(path: &Path, msg: impl Into<String>) -> Self {
        CompilerError {
            msg: msg.into(),
            source_span: None,
            host_path: Some(path.to_path_buf()),
            error_type: ErrorType::File,
            metadata: HashMap::new(),
        }
    }

    /// Create a filesystem error from a host path with metadata.
    pub fn new_file_error(
        path: &Path,
        msg: impl Into<String>,
        metadata: HashMap<CompilerErrorMetadataKey, String>,
    ) -> CompilerError {
        CompilerError {
            msg: msg.into(),
            source_span: None,
            host_path: Some(path.to_path_buf()),
            error_type: ErrorType::File,
            metadata,
        }
    }
}

// Classifies the source of an infrastructure failure.
//
// Filesystem failures retain their host path separately from source-owned spans.
#[derive(PartialEq, Debug, Clone)]
pub enum ErrorType {
    File,
    Compiler,
    DevServer,
    HirTransformation,
    Backend(crate::backends::error_types::BackendErrorType),
}

/// Return a filesystem infrastructure error.
///
/// Usage: `return_file_error!(path, "message", { metadata })`;
#[macro_export]
macro_rules! return_file_error {
    // Metadata usage for direct infrastructure rendering.
    ($path:expr, $msg:expr, { $( $key:ident => $value:expr ),* $(,)? }) => {{
        return Err($crate::compiler_frontend::compiler_errors::CompilerError::new_file_error(
            $path,
            $msg,
            {
                let mut map = std::collections::HashMap::new();
                $( map.insert($crate::compiler_frontend::compiler_errors::CompilerErrorMetadataKey::$key, $value.into()); )*
                map
            },
        ));
    }};
    // Usage without guidance metadata.
    ($path:expr, $msg:expr) => {{
        return Err($crate::compiler_frontend::compiler_errors::CompilerError::file_error(
            $path,
            $msg,
        ));
    }};
}

/// Returns a new `CompilerError` for internal compiler_frontend bugs.
///
/// Compiler errors indicate bugs in the compiler_frontend itself, not user code issues.
#[macro_export]
macro_rules! return_compiler_error {
    // Variant with format string, arguments, and metadata (with semicolon separator)
    ($fmt:expr, $($arg:expr),+ ; { $( $key:ident => $value:expr ),* $(,)? }) => {{
        let mut error = $crate::compiler_frontend::compiler_errors::CompilerError::compiler_error(
            format!($fmt, $($arg),+)
        );
        $(
            error.new_metadata_entry(
                $crate::compiler_frontend::compiler_errors::CompilerErrorMetadataKey::$key,
                $value.into(),
            );
        )*
        return Err(error);
    }};
    // Variant with format string and arguments (no metadata)
    ($fmt:expr, $($arg:expr),+ $(,)?) => {{
        return Err($crate::compiler_frontend::compiler_errors::CompilerError::compiler_error(
            format!($fmt, $($arg),+)
        ));
    }};
    // Variant with message and metadata (with semicolon separator)
    ($msg:expr ; { $( $key:ident => $value:expr ),* $(,)? }) => {{
        let mut error = $crate::compiler_frontend::compiler_errors::CompilerError::compiler_error(
            $msg
        );
        $(
            error.new_metadata_entry(
                $crate::compiler_frontend::compiler_errors::CompilerErrorMetadataKey::$key,
                $value.into(),
            );
        )*
        return Err(error);
    }};
    // Simple variant with just a message (no metadata)
    ($msg:expr) => {{
        return Err($crate::compiler_frontend::compiler_errors::CompilerError::compiler_error(
            $msg
        ));
    }};
}

/// Returns a new CompilerError for HIR transformation failures.
///
/// HIR transformation errors indicate failures during AST to HIR conversion.
/// These are typically compiler_frontend bugs where the HIR infrastructure is missing
/// or incomplete for a particular language feature.
///
/// Usage: `return_hir_transformation_error!("Function '{}' transformation not yet implemented", func_name, source_span, {})`;
#[macro_export]
macro_rules! return_hir_transformation_error {
    // HIR failures may carry metadata for direct infrastructure rendering.
    ($msg:expr, $source_span:expr, { $( $key:ident => $value:expr ),* $(,)? }) => {
        let mut error = $crate::compiler_frontend::compiler_errors::CompilerError::new(
            $msg,
            $source_span,
            $crate::compiler_frontend::compiler_errors::ErrorType::HirTransformation,
        );
        $(
            error.new_metadata_entry(
                $crate::compiler_frontend::compiler_errors::CompilerErrorMetadataKey::$key,
                $value.into(),
            );
        )*
        return Err(error.into())
    };
    ($msg:expr, $source_span:expr) => {
        return Err($crate::compiler_frontend::compiler_errors::CompilerError::new(
            $msg,
            $source_span,
            $crate::compiler_frontend::compiler_errors::ErrorType::HirTransformation,
        ).into())
    };
}
