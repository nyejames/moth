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
//!   -> CompilerDiagnostic { kind, severity, primary_span, labels, payload }
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
//! - **Config** — configuration file issues.
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
//! CompilerMessages (ordered diagnostics + frozen identity rows + transitional StringTable)
//! ```

use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, DiagnosticSeverity};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::source::{
    FrozenIdentityContext, SourceDatabase, SourceSpan, SpanCapacityError, SpanCapacityReason,
};
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

    /// Outer infrastructure failure carried beside the diagnostic stream.
    ///
    /// WHAT: holds the single typed `CompilerError` for internal/tooling/filesystem failures
    ///       without converting it into a user-facing diagnostic payload.
    /// WHY: an infrastructure failure aborts the owning compilation yet must stay observable at
    ///      existing command/render/test boundaries. Storing the rare, large failure behind a
    ///      pointer keeps this transitional boundary compact without boxing common diagnostics.
    pub(crate) infrastructure_error: Option<Box<CompilerError>>,

    /// Interned diagnostic strings owned by this boundary.
    ///
    /// The table is separately allocated because the message vessel crosses many `Result`
    /// boundaries and must remain below the large-error threshold without boxing individual
    /// `CompilerDiagnostic` values.
    pub string_table: Box<StringTable>,

    /// Per-diagnostic frozen identity snapshots used by diagnostic renderers.
    ///
    /// WHAT: owns one or many immutable identity contexts with range-aligned lookup. Each row
    /// pairs an `Arc<FrozenIdentityContext>` with the diagnostic range produced against it.
    /// WHY: the frozen context is authoritative for retained source spans and path/string
    /// resolution; ranges shift alongside diagnostics during aggregation with first-match
    /// precedence. Frozen IDs are already final and must never be remapped.
    pub(crate) render_frozen_contexts: Vec<RenderFrozenContext>,

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
}

const _: () = assert!(std::mem::size_of::<CompilerMessages>() <= 128);
#[derive(Debug, Clone)]
pub(crate) struct RenderTypeContext {
    pub(crate) diagnostic_range: Range<usize>,
    pub(crate) type_environment: TypeEnvironment,
}
#[derive(Debug, Clone)]
pub(crate) struct RenderFrozenContext {
    pub(crate) diagnostic_range: Range<usize>,
    pub(crate) identity: Arc<FrozenIdentityContext>,
}
#[derive(Debug, Clone)]
pub(crate) struct RenderSourceContext {
    pub(crate) diagnostic_range: Range<usize>,
    pub(crate) source_database: Arc<SourceDatabase>,
}

impl CompilerMessages {
    pub fn empty(string_table: StringTable) -> Self {
        Self {
            diagnostics: Vec::new(),
            infrastructure_error: None,
            string_table: Box::new(string_table),
            render_frozen_contexts: Vec::new(),
            render_source_contexts: Vec::new(),
            render_type_contexts: Vec::new(),
        }
    }

    pub(crate) fn from_diagnostics(
        diagnostics: Vec<CompilerDiagnostic>,
        string_table: StringTable,
    ) -> Self {
        Self {
            diagnostics,
            infrastructure_error: None,
            string_table: Box::new(string_table),
            render_frozen_contexts: Vec::new(),
            render_source_contexts: Vec::new(),
            render_type_contexts: Vec::new(),
        }
    }

    /// Whether this boundary carries the legacy outer infrastructure failure.
    pub(crate) fn has_infrastructure_error(&self) -> bool {
        self.infrastructure_error.is_some()
    }

    /// Borrow the legacy outer infrastructure failure, if present.
    ///
    /// WHAT: exposes the typed `CompilerError` without converting it into a diagnostic.
    /// WHY: renderers, tests and status summaries inspect the outer failure's message,
    ///      source span or host path, type and metadata directly.
    pub(crate) fn infrastructure_error(&self) -> Option<&CompilerError> {
        self.infrastructure_error.as_deref()
    }

    /// Store the outer infrastructure failure without converting it into a user diagnostic.
    ///
    /// WHAT: chains a second failure behind the first while preserving the first available source
    ///       span and host path.
    /// WHY: aggregation can observe a deterministic double-failure tail. `CompilerError` owns
    ///      only its typed provenance, so no diagnostic string table or render context is merged
    ///      here.
    pub(crate) fn set_infrastructure_error(&mut self, error: CompilerError) {
        match &mut self.infrastructure_error {
            Some(existing) => {
                existing.msg = format!(
                    "{existing_msg}; {incoming_msg}",
                    existing_msg = existing.msg,
                    incoming_msg = error.msg,
                );
                if existing.source_span.is_none() {
                    existing.source_span = error.source_span;
                }
                if existing.host_path.is_none() {
                    existing.host_path = error.host_path;
                }
                for (key, value) in error.metadata {
                    existing.metadata.entry(key).or_insert(value);
                }
            }
            None => {
                self.infrastructure_error = Some(Box::new(error));
            }
        }
    }

    pub fn has_errors(&self) -> bool {
        self.infrastructure_error.is_some()
            || self
                .diagnostics
                .iter()
                .any(|d| d.severity == DiagnosticSeverity::Error)
    }

    /// Count diagnostics with `Error` severity.
    pub fn error_count(&self) -> usize {
        self.diagnostics
            .iter()
            .filter(|d| d.severity == DiagnosticSeverity::Error)
            .count()
            + usize::from(self.infrastructure_error.is_some())
    }

    /// Count diagnostics with `Warning` severity.
    pub fn warning_count(&self) -> usize {
        self.diagnostics
            .iter()
            .filter(|d| d.severity == DiagnosticSeverity::Warning)
            .count()
    }

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

    /// Return the original diagnostic indexes in display order.
    ///
    /// WHAT: sorts diagnostics by severity bucket (`Error`, then `Warning`, then `Note`) while
    /// keeping the original compiler production order within each bucket.
    /// WHY: some aggregation paths prepend warnings before errors, but users expect to see errors
    /// first. This is a render-time policy, not a mutation of `diagnostics`.
    ///
    /// Renderers must keep using the returned original index with
    /// `diagnostic_render_context(index)` so that type-context lookups stay aligned with the
    /// stored diagnostic positions.
    pub(crate) fn diagnostic_display_order(&self) -> Vec<usize> {
        let mut errors = Vec::new();
        let mut warnings = Vec::new();
        let mut notes = Vec::new();

        for (index, diagnostic) in self.diagnostics.iter().enumerate() {
            match diagnostic.severity {
                DiagnosticSeverity::Error => errors.push(index),
                DiagnosticSeverity::Warning => warnings.push(index),
                DiagnosticSeverity::Note => notes.push(index),
            }
        }

        errors.into_iter().chain(warnings).chain(notes).collect()
    }

    /// Append already-structured diagnostics while preserving current order.
    pub(crate) fn extend_diagnostics(
        &mut self,
        diagnostics: impl IntoIterator<Item = CompilerDiagnostic>,
    ) {
        self.diagnostics.extend(diagnostics);
    }

    /// Consume the boundary container and return its ordered diagnostics.
    pub(crate) fn into_diagnostics(self) -> Vec<CompilerDiagnostic> {
        self.diagnostics
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

    /// Iterate over diagnostics with `Warning` severity.
    pub(crate) fn warnings(&self) -> impl Iterator<Item = &CompilerDiagnostic> {
        self.diagnostics
            .iter()
            .filter(|d| d.severity == DiagnosticSeverity::Warning)
    }

    /// Wrap a single `CompilerDiagnostic` into a `CompilerMessages` container with no warnings.
    ///
    /// WHY: frontend stages emit `CompilerDiagnostic` values directly and need a clean boundary
    /// conversion into the message container expected by build-system callers.
    pub fn from_diagnostic(diagnostic: CompilerDiagnostic, string_table: StringTable) -> Self {
        Self {
            diagnostics: vec![diagnostic],
            infrastructure_error: None,
            string_table: Box::new(string_table),
            render_frozen_contexts: Vec::new(),
            render_source_contexts: Vec::new(),
            render_type_contexts: Vec::new(),
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
            infrastructure_error: Some(Box::new(error)),
            string_table: Box::new(string_table),
            render_frozen_contexts: Vec::new(),
            render_source_contexts: Vec::new(),
            render_type_contexts: Vec::new(),
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
            infrastructure_error: Some(Box::new(error)),
            string_table: Box::new(string_table.clone()),
            render_frozen_contexts: Vec::new(),
            render_source_contexts: Vec::new(),
            render_type_contexts: Vec::new(),
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
            infrastructure_error: None,
            string_table: Box::new(string_table.clone()),
            render_frozen_contexts: Vec::new(),
            render_source_contexts: Vec::new(),
            render_type_contexts: Vec::new(),
        }
    }

    /// Carry a filesystem failure while retaining the caller's table for any surrounding
    /// diagnostics.
    pub fn file_error(path: &Path, msg: impl Into<String>, string_table: &StringTable) -> Self {
        Self::from_error(CompilerError::file_error(path, msg), string_table.clone())
    }

    /// Associate the current diagnostics with the source snapshot database that produced them.
    ///
    /// The range is deliberately captured at the time of attachment: a single message set may
    /// later aggregate diagnostics from several independent project/package databases.
    ///
    /// Infrastructure-only failures have no diagnostic row, but their source owner is still useful
    /// to boundary callers inspecting retained source slots. Reserve index zero as a source-context
    /// sentinel for that shape; ordinary diagnostic ranges continue to cover exactly the current
    /// stream.
    pub(crate) fn set_source_database(&mut self, source_database: Arc<SourceDatabase>) {
        let diagnostic_end = self.diagnostics.len();
        if diagnostic_end != 0 {
            self.render_source_contexts.push(RenderSourceContext {
                diagnostic_range: 0..diagnostic_end,
                source_database,
            });
        } else if self.infrastructure_error.is_some() {
            self.render_source_contexts.push(RenderSourceContext {
                diagnostic_range: 0..1,
                source_database,
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

    /// Install precomputed frozen identity contexts for diagnostics appended at `diagnostic_offset`.
    ///
    /// WHAT: offsets each incoming frozen range by the diagnostic offset, mirroring
    /// `install_source_contexts`.
    /// WHY: frozen rows are range-aligned exactly like the transitional source rows, so the same
    /// offset keeps each identity row tied to its diagnostics whether warnings or failures are
    /// appended around them.
    pub(crate) fn install_frozen_identity_contexts(
        &mut self,
        frozen_contexts: impl IntoIterator<Item = RenderFrozenContext>,
        diagnostic_offset: usize,
    ) {
        self.render_frozen_contexts
            .extend(frozen_contexts.into_iter().map(|mut frozen_context| {
                frozen_context.diagnostic_range.start += diagnostic_offset;
                frozen_context.diagnostic_range.end += diagnostic_offset;
                frozen_context
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
    /// WHAT: shifts every stored frozen/source/type-context range forward by the prepended length.
    /// WHY: frontend/build aggregation often carries warnings from earlier stages into a later
    /// failure. Those warnings must stay before the failure without disconnecting diagnostics from
    /// their frozen identity snapshot, retained source snapshot or render type table. Frozen IDs
    /// are already final and are only shifted, never remapped.
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

        for frozen_context in &mut self.render_frozen_contexts {
            frozen_context.diagnostic_range.start += shift;
            frozen_context.diagnostic_range.end += shift;
        }

        for source_context in &mut self.render_source_contexts {
            source_context.diagnostic_range.start += shift;
            source_context.diagnostic_range.end += shift;
        }

        for type_context in &mut self.render_type_contexts {
            type_context.diagnostic_range.start += shift;
            type_context.diagnostic_range.end += shift;
        }
    }
    /// Append another boundary message set while preserving its diagnostic render contexts.
    ///
    /// WHAT: moves diagnostics and all frozen/source/type render-context ranges from `messages`
    ///       into this set, offsets the appended ranges by the current diagnostic count, and
    ///       remaps diagnostic-owned IDs into this set's table.
    /// WHY: final aggregation consumes module-local message owners without cloning their
    ///       diagnostics or string tables. Each incoming table is merged exactly once before its
    ///       diagnostics and type environments are appended; the outer infrastructure error keeps
    ///       its own typed source span and host path.
    pub(crate) fn append_messages_preserving_context(&mut self, messages: CompilerMessages) {
        let CompilerMessages {
            mut diagnostics,
            infrastructure_error,
            string_table,
            mut render_frozen_contexts,
            mut render_source_contexts,
            mut render_type_contexts,
        } = messages;
        let remap = self.string_table.merge_from(string_table.as_ref());
        if !remap.is_identity() {
            for diagnostic in &mut diagnostics {
                diagnostic.remap_string_ids(&remap);
            }
            for type_context in &mut render_type_contexts {
                type_context.type_environment.remap_string_ids(&remap);
            }
        }

        let shift = self.diagnostics.len();
        self.diagnostics.append(&mut diagnostics);

        for mut frozen_context in render_frozen_contexts.drain(..) {
            frozen_context.diagnostic_range.start += shift;
            frozen_context.diagnostic_range.end += shift;
            self.render_frozen_contexts.push(frozen_context);
        }

        for mut source_context in render_source_contexts.drain(..) {
            source_context.diagnostic_range.start += shift;
            source_context.diagnostic_range.end += shift;
            self.render_source_contexts.push(source_context);
        }

        for mut type_context in render_type_contexts {
            type_context.diagnostic_range.start += shift;
            type_context.diagnostic_range.end += shift;
            self.render_type_contexts.push(type_context);
        }

        if let Some(error) = infrastructure_error {
            self.set_infrastructure_error(*error);
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

    /// Resolve the frozen identity context that produced one diagnostic.
    ///
    /// The first matching association wins, matching `source_database_for_diagnostic` semantics:
    /// a package's own association must already be present to take precedence over a
    /// whole-set fallback attached later.
    pub(crate) fn frozen_identity_context_for_diagnostic(
        &self,
        diagnostic_index: usize,
    ) -> Option<&FrozenIdentityContext> {
        self.render_frozen_contexts
            .iter()
            .find(|frozen_context| frozen_context.diagnostic_range.contains(&diagnostic_index))
            .map(|frozen_context| frozen_context.identity.as_ref())
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

    pub(crate) fn diagnostic_render_context(
        &self,
        diagnostic_index: usize,
    ) -> crate::compiler_frontend::compiler_messages::render::DiagnosticRenderContext<'_> {
        let frozen_identity = self.frozen_identity_context_for_diagnostic(diagnostic_index);
        crate::compiler_frontend::compiler_messages::render::DiagnosticRenderContext::new(
            self.string_table.as_ref(),
        )
        .with_optional_type_environment(self.type_environment_for_diagnostic(diagnostic_index))
        .with_optional_source_database(self.source_database_for_diagnostic(diagnostic_index))
        .with_optional_frozen_identity(frozen_identity)
    }

    #[cfg(test)]
    pub(crate) fn render_type_contexts(&self) -> &[RenderTypeContext] {
        &self.render_type_contexts
    }

    pub fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        // Frozen identity rows are intentionally left untouched: frozen IDs are already final.
        for diagnostic in self.diagnostics.iter_mut() {
            diagnostic.remap_string_ids(remap);
        }
        for type_context in &mut self.render_type_contexts {
            type_context.type_environment.remap_string_ids(remap);
        }
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

/// Merge warnings produced by an earlier frontend stage into a later message set.
///
/// WHAT: preserves diagnostic order and render type-context ranges while attaching the caller's
///       current string table to the returned boundary container.
/// WHY: stage orchestration and HIR-derived convergence share this diagnostic boundary, so the
///      neutral compiler-message owner keeps warning composition out of either coordinator.
pub(crate) fn merge_stage_messages(
    messages: CompilerMessages,
    warnings: &[CompilerDiagnostic],
    string_table: &StringTable,
) -> CompilerMessages {
    let mut messages = messages;
    messages.prepend_diagnostics_preserving_context(warnings.iter().cloned());
    messages.string_table = Box::new(string_table.clone());
    messages
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

    /// Span exhaustion shares the existing source-size failure lane until Phase 5 reclassifies
    /// it. The optional source span preserves any already-encoded provenance without requiring
    /// callers to fabricate a span when packing failed before one could be created.
    pub(crate) fn source_span_capacity(
        error: SpanCapacityError,
        source_span: Option<SourceSpan>,
    ) -> Self {
        let (message, error_type) = match error.reason() {
            SpanCapacityReason::ExtendedTableFull => (
                format!(
                    "this source needs an exact span at byte offset {} with length {}, but \
                     its span table cannot hold another long or late range",
                    error.start(),
                    error.length(),
                ),
                ErrorType::File,
            ),
            SpanCapacityReason::EndUnrepresentable => (
                format!(
                    "span at byte offset {} with length {} ends past u32::MAX in a source \
                     that was accepted as addressable; this is a compiler bug",
                    error.start(),
                    error.length(),
                ),
                ErrorType::Compiler,
            ),
        };
        Self::new(message, source_span, error_type)
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
    Config,
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
        return Err(error)
    };
    ($msg:expr, $source_span:expr) => {
        return Err($crate::compiler_frontend::compiler_errors::CompilerError::new(
            $msg,
            $source_span,
            $crate::compiler_frontend::compiler_errors::ErrorType::HirTransformation,
        ))
    };
}
