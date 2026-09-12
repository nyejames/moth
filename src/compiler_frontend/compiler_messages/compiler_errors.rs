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
use crate::compiler_frontend::semantic_identity::StablePackageIdentity;
use crate::compiler_frontend::source::{
    FrozenIdentityContext, FrozenIdentityHandle, SourceDatabase, SourceSpan, SpanCapacityError,
};
use crate::compiler_frontend::symbols::path_interner::{PathIdRemap, PathTable};
use crate::compiler_frontend::symbols::string_interning::{StringIdRemap, StringTable};
use std::collections::{HashMap, HashSet};
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
    /// WHY: the frozen context is authoritative for every retained fact in its range, including
    /// compact string IDs. Ranges shift alongside diagnostics during aggregation with
    /// first-match precedence; facts covered by a row must move with that owner and never be
    /// remapped through a mutable table.
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
    /// Per-diagnostic path-table snapshots used by transitional diagnostic renderers.
    ///
    /// Each row retains the path identity domain that issued the diagnostics in its range. The
    /// table is separate from source/frozen contexts because diagnosed lanes may finish before a
    /// source database or boundary path builder exists.
    pub(crate) render_path_contexts: Option<Box<Vec<RenderPathContext>>>,
}

const _: () = assert!(std::mem::size_of::<CompilerMessages>() <= 128);
#[derive(Debug, Clone)]
pub(crate) struct RenderTypeContext {
    /// Range whose `TypeId` payloads resolve against this environment. When the range is covered
    /// by a frozen identity row, the environment's `StringId`s belong to that row's immutable
    /// string table and must move unchanged with it.
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
    pub(crate) domain: Option<StablePackageIdentity>,
}
#[derive(Debug, Clone)]
pub(crate) struct RenderPathContext {
    pub(crate) diagnostic_range: Range<usize>,
    pub(crate) path_table: Arc<PathTable>,
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
            render_path_contexts: None,
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
            render_path_contexts: None,
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
    /// Fill missing diagnostic span owners without replacing mixed-domain provenance.
    pub(crate) fn set_frozen_identity_handle_if_missing(
        &mut self,
        frozen_identity_handle: FrozenIdentityHandle,
    ) {
        for diagnostic in &mut self.diagnostics {
            diagnostic.attach_frozen_identity_handle_if_missing(frozen_identity_handle.clone());
        }
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
            infrastructure_error: Some(Box::new(error)),
            string_table: Box::new(string_table),
            render_frozen_contexts: Vec::new(),
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
            infrastructure_error: Some(Box::new(error)),
            string_table: Box::new(string_table.clone()),
            render_frozen_contexts: Vec::new(),
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
            infrastructure_error: None,
            string_table: Box::new(string_table.clone()),
            render_frozen_contexts: Vec::new(),
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
                domain: None,
            });
        } else if self.infrastructure_error.is_some() {
            self.render_source_contexts.push(RenderSourceContext {
                diagnostic_range: 0..1,
                source_database,
                domain: None,
            });
        }
    }

    /// Extend the domain-less project source row across diagnostics appended after build output.
    ///
    /// Late output-plan/write failures are produced after the warning rows were collected. They
    /// still belong to the project source database, so keep that fallback range aligned through
    /// the final combined freeze without widening package-domain rows.
    pub(crate) fn extend_project_source_context_to_diagnostics(&mut self) {
        let diagnostic_end = self.diagnostics.len();
        for source_context in &mut self.render_source_contexts {
            if source_context.domain.is_none() && !source_context.diagnostic_range.is_empty() {
                source_context.diagnostic_range.end =
                    source_context.diagnostic_range.end.max(diagnostic_end);
            }
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
    /// Convert remaining transitional source rows into frozen identity rows without copying source
    /// or diagnostic storage, and without discarding frozen rows already present.
    ///
    /// Existing frozen identity rows stay at the front so first-match lookup still prefers the
    /// earlier owner. The first newly frozen source row consumes the aggregate string table;
    /// subsequent rows share that frozen string allocation while moving only their own source
    /// database. Empty package rows are retained only when an explicitly owned diagnostic span
    /// needs their domain. They supply the corresponding frozen handle but never become default
    /// render ranges.
    pub(crate) fn freeze_source_contexts(mut self) -> Result<Self, CompilerError> {
        let mut required_handle_domains = HashSet::new();
        for diagnostic in &self.diagnostics {
            if let Some(handle) = diagnostic.primary_frozen_identity_handle.as_ref()
                && let Some(domain) = handle.domain()
            {
                required_handle_domains.insert(domain.clone());
            }
            for label in &diagnostic.labels {
                if let Some(handle) = label.frozen_identity_handle.as_ref()
                    && let Some(domain) = handle.domain()
                {
                    required_handle_domains.insert(domain.clone());
                }
            }
        }

        let source_contexts = std::mem::take(&mut self.render_source_contexts)
            .into_iter()
            .filter(|context| {
                !context.diagnostic_range.is_empty()
                    || context
                        .domain
                        .as_ref()
                        .is_some_and(|domain| required_handle_domains.contains(domain))
            })
            .collect::<Vec<_>>();
        if source_contexts.is_empty() {
            self.ensure_frozen_identity_handles_installed()?;
            return Ok(self);
        }

        let mut database_owners = HashMap::<*const SourceDatabase, Arc<SourceDatabase>>::new();
        let mut context_rows = Vec::with_capacity(source_contexts.len());
        for source_context in source_contexts {
            let RenderSourceContext {
                diagnostic_range,
                source_database,
                domain,
            } = source_context;
            let database_key = Arc::as_ptr(&source_database);
            database_owners
                .entry(database_key)
                .or_insert(source_database);
            context_rows.push((diagnostic_range, domain, database_key));
        }

        let mut identity_by_database =
            HashMap::<*const SourceDatabase, Arc<FrozenIdentityContext>>::new();
        let mut frozen_root: Option<Arc<FrozenIdentityContext>> = None;
        let mut frozen_contexts = Vec::with_capacity(context_rows.len());
        for (diagnostic_range, domain, database_key) in context_rows {
            let identity = if let Some(identity) = identity_by_database.get(&database_key) {
                Arc::clone(identity)
            } else {
                let source_database = database_owners.remove(&database_key).ok_or_else(|| {
                    CompilerError::compiler_error(
                        "source database owner disappeared at the frozen render handoff",
                    )
                })?;
                let database = Arc::try_unwrap(source_database).map_err(|_| {
                    CompilerError::compiler_error(
                        "source database was unexpectedly shared at the frozen render handoff",
                    )
                })?;
                let identity = match frozen_root.as_ref() {
                    Some(root) => Arc::new(FrozenIdentityContext::from_shared_strings(
                        root.shared_strings(),
                        database,
                    )),
                    None => {
                        let identity = Arc::new(FrozenIdentityContext::from_parts(
                            *std::mem::take(&mut self.string_table),
                            database,
                        ));
                        frozen_root = Some(Arc::clone(&identity));
                        identity
                    }
                };
                identity_by_database.insert(database_key, Arc::clone(&identity));
                identity
            };
            frozen_contexts.push((diagnostic_range, domain, identity));
        }

        for (diagnostic_range, domain, identity) in &frozen_contexts {
            if domain.is_some() {
                // Package domains identify one source owner even when the owned span is a
                // related label on a diagnostic whose default range belongs to the project.
                for diagnostic in &self.diagnostics {
                    if let Some(handle) = diagnostic.primary_frozen_identity_handle.as_ref()
                        && handle.get().is_none()
                        && handle.domain() == domain.as_ref()
                    {
                        handle.install(Arc::clone(identity))?;
                    }
                    for label in &diagnostic.labels {
                        if let Some(handle) = label.frozen_identity_handle.as_ref()
                            && handle.get().is_none()
                            && handle.domain() == domain.as_ref()
                        {
                            handle.install(Arc::clone(identity))?;
                        }
                    }
                }
            } else {
                // Domain-less handles remain tied to their genuine default source range. In
                // particular, a project row cannot satisfy a foreign package handle.
                for diagnostic in self
                    .diagnostics
                    .iter()
                    .skip(diagnostic_range.start)
                    .take(diagnostic_range.end.saturating_sub(diagnostic_range.start))
                {
                    if let Some(handle) = diagnostic.primary_frozen_identity_handle.as_ref()
                        && handle.get().is_none()
                        && handle.domain() == domain.as_ref()
                    {
                        handle.install(Arc::clone(identity))?;
                    }
                    for label in &diagnostic.labels {
                        if let Some(handle) = label.frozen_identity_handle.as_ref()
                            && handle.get().is_none()
                            && handle.domain() == domain.as_ref()
                        {
                            handle.install(Arc::clone(identity))?;
                        }
                    }
                }
            }
        }

        self.render_frozen_contexts
            .extend(
                frozen_contexts
                    .into_iter()
                    .filter_map(|(diagnostic_range, _, identity)| {
                        (!diagnostic_range.is_empty()).then_some(RenderFrozenContext {
                            diagnostic_range,
                            identity,
                        })
                    }),
            );
        self.ensure_frozen_identity_handles_installed()?;
        Ok(self)
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
    // The compiler-source-token-and-diagnostic-data-layout plan retains direct-template warning
    // contexts. Test-gated template pipelines and diagnostic aggregation tests consume this method.
    #[cfg_attr(not(test), allow(dead_code))]
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
        if let Some(path_contexts) = self.render_path_contexts.as_mut() {
            for path_context in path_contexts.iter_mut() {
                path_context.diagnostic_range.start += shift;
                path_context.diagnostic_range.end += shift;
            }
        }
    }
    /// Append another boundary message set while preserving its diagnostic render contexts.
    ///
    /// WHAT: moves diagnostics and all frozen/source/type render-context ranges from `messages`
    ///       into this set, offsets the appended ranges by the current diagnostic count, and
    ///       remaps only premerge facts into this set's table.
    /// WHY: final aggregation consumes module-local message owners without cloning their
    pub(crate) fn append_messages_preserving_context(&mut self, messages: CompilerMessages) {
        let CompilerMessages {
            mut diagnostics,
            infrastructure_error,
            string_table,
            mut render_frozen_contexts,
            mut render_source_contexts,
            mut render_type_contexts,
            mut render_path_contexts,
        } = messages;
        let remap = self.string_table.merge_from(string_table.as_ref());
        remap_diagnostics_preserving_frozen_context(
            &mut diagnostics,
            &remap,
            &render_frozen_contexts,
        );
        remap_type_contexts_preserving_frozen_context(
            &mut render_type_contexts,
            &remap,
            &render_frozen_contexts,
        );
        if let Some(path_contexts) = render_path_contexts.as_mut() {
            for path_context in path_contexts.iter_mut() {
                Arc::make_mut(&mut path_context.path_table).remap_string_ids(&remap);
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

        if let Some(mut path_contexts) = render_path_contexts {
            for path_context in path_contexts.iter_mut() {
                path_context.diagnostic_range.start += shift;
                path_context.diagnostic_range.end += shift;
            }
            self.render_path_contexts
                .get_or_insert_with(|| Box::new(Vec::new()))
                .extend(*path_contexts);
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
    /// Remap premerge facts into another mutable string-table domain.
    ///
    /// Facts covered by a [`RenderFrozenContext`] already belong to that row's immutable identity
    /// owner. They must remain byte-for-byte in the owner's ID domain while any premerge
    /// diagnostic or type-context facts continue through the supplied mutable-table remap.
    pub fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        remap_diagnostics_preserving_frozen_context(
            &mut self.diagnostics,
            remap,
            &self.render_frozen_contexts,
        );
        remap_type_contexts_preserving_frozen_context(
            &mut self.render_type_contexts,
            remap,
            &self.render_frozen_contexts,
        );
        if let Some(path_contexts) = self.render_path_contexts.as_mut() {
            for path_context in path_contexts.iter_mut() {
                Arc::make_mut(&mut path_context.path_table).remap_string_ids(remap);
            }
        }
    }

    /// Remap complete logical paths for diagnostics and renderer-owned type contexts.
    ///
    /// Diagnostics covered by a frozen identity row already use the merged domain and must stay
    /// unchanged. Premerge diagnostics and type environments belong to the worker fork and are
    /// rewritten through the same `PathIdRemap` used by the rest of the module facts.
    pub fn remap_path_ids(&mut self, remap: &PathIdRemap) {
        if remap.is_identity() {
            return;
        }
        for (diagnostic_index, diagnostic) in self.diagnostics.iter_mut().enumerate() {
            if !diagnostic_is_frozen(diagnostic_index, &self.render_frozen_contexts) {
                diagnostic.remap_path_ids(remap);
            }
        }
        for context in &mut self.render_type_contexts {
            if !diagnostic_is_frozen(context.diagnostic_range.start, &self.render_frozen_contexts) {
                context.type_environment.remap_path_ids(remap);
            }
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
    pub(crate) fn path_table_for_diagnostic(
        &self,
        diagnostic_index: usize,
    ) -> Option<&PathTable> {
        self.render_path_contexts
            .as_deref()
            .and_then(|contexts| {
                contexts
                    .iter()
                    .find(|context| context.diagnostic_range.contains(&diagnostic_index))
            })
            .map(|context| context.path_table.as_ref())
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
        .with_optional_path_table(self.path_table_for_diagnostic(diagnostic_index))
    }

    /// Install late-bound generic donor owners for one stable package domain.
    ///
    /// A diagnosed module can fail before its declaring module's materialisation context reaches
    /// the successful-boundary installation walk. Labels still carry the donor's package identity,
    /// so the final render tail installs the exact frozen context without guessing from a colliding
    /// `SourceId` or the diagnostic's requester domain.
    pub(crate) fn install_frozen_identity_handles_for_domain(
        &self,
        domain: &StablePackageIdentity,
        identity: &Arc<FrozenIdentityContext>,
    ) -> Result<(), CompilerError> {
        for diagnostic in &self.diagnostics {
            if let Some(handle) = diagnostic.primary_frozen_identity_handle.as_ref()
                && handle.domain() == Some(domain)
            {
                handle.install(Arc::clone(identity))?;
            }
            for label in &diagnostic.labels {
                let Some(handle) = label.frozen_identity_handle.as_ref() else {
                    continue;
                };
                if handle.domain() == Some(domain) {
                    handle.install(Arc::clone(identity))?;
                }
            }
        }
        Ok(())
    }

    /// Reject any donor label that reached the final boundary without an owning identity.
    ///
    /// Unknown domains are compiler bugs, not permission to reinterpret a donor span through the
    /// requester or project-root context.
    pub(crate) fn ensure_frozen_identity_handles_installed(&self) -> Result<(), CompilerError> {
        for diagnostic in &self.diagnostics {
            if diagnostic
                .primary_frozen_identity_handle
                .as_ref()
                .is_some_and(|handle| handle.domain().is_some() && handle.get().is_none())
            {
                return Err(CompilerError::compiler_error(
                    "diagnostic primary reached the frozen render boundary without its donor identity",
                ));
            }
            for label in &diagnostic.labels {
                let Some(handle) = label.frozen_identity_handle.as_ref() else {
                    continue;
                };
                if handle.domain().is_some() && handle.get().is_none() {
                    return Err(CompilerError::compiler_error(
                        "diagnostic label reached the frozen render boundary without its donor identity",
                    ));
                }
            }
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn render_type_contexts(&self) -> &[RenderTypeContext] {
        &self.render_type_contexts
    }
}
fn diagnostic_is_frozen(diagnostic_index: usize, frozen_contexts: &[RenderFrozenContext]) -> bool {
    frozen_contexts
        .iter()
        .any(|context| context.diagnostic_range.contains(&diagnostic_index))
}

fn remap_diagnostics_preserving_frozen_context(
    diagnostics: &mut [CompilerDiagnostic],
    remap: &StringIdRemap,
    frozen_contexts: &[RenderFrozenContext],
) {
    if remap.is_identity() {
        return;
    }

    for (diagnostic_index, diagnostic) in diagnostics.iter_mut().enumerate() {
        if !diagnostic_is_frozen(diagnostic_index, frozen_contexts) {
            diagnostic.remap_string_ids(remap);
        }
    }
}

/// Split a type-context range at frozen-owner boundaries.
///
/// A producer normally gives one type environment to one owner range. Splitting the uncommon
/// mixed range keeps that invariant true even when a caller composes frozen and premerge
/// diagnostics into one incoming vessel: frozen segments retain the original environment while
/// premerge segments receive a remapped clone.
fn type_context_segments(
    range: &Range<usize>,
    frozen_contexts: &[RenderFrozenContext],
) -> Vec<(Range<usize>, bool)> {
    if range.is_empty() {
        return vec![(range.clone(), false)];
    }

    let mut boundaries = vec![range.start, range.end];
    for frozen_context in frozen_contexts {
        let start = range.start.max(frozen_context.diagnostic_range.start);
        let end = range.end.min(frozen_context.diagnostic_range.end);
        if start < end {
            boundaries.push(start);
            boundaries.push(end);
        }
    }
    boundaries.sort_unstable();
    boundaries.dedup();

    let mut segments: Vec<(Range<usize>, bool)> =
        Vec::with_capacity(boundaries.len().saturating_sub(1));
    for window in boundaries.windows(2) {
        let segment = window[0]..window[1];
        if segment.is_empty() {
            continue;
        }
        let is_frozen = diagnostic_is_frozen(segment.start, frozen_contexts);
        if let Some((previous, previous_is_frozen)) = segments.last_mut()
            && *previous_is_frozen == is_frozen
            && previous.end == segment.start
        {
            previous.end = segment.end;
        } else {
            segments.push((segment, is_frozen));
        }
    }
    segments
}

fn remap_type_contexts_preserving_frozen_context(
    type_contexts: &mut Vec<RenderTypeContext>,
    remap: &StringIdRemap,
    frozen_contexts: &[RenderFrozenContext],
) {
    if remap.is_identity() {
        return;
    }

    let incoming = std::mem::take(type_contexts);
    let mut remapped = Vec::with_capacity(incoming.len());
    for context in incoming {
        let segments = type_context_segments(&context.diagnostic_range, frozen_contexts);
        let segment_count = segments.len();
        let mut environment = Some(context.type_environment);

        for (segment_index, (diagnostic_range, is_frozen)) in segments.into_iter().enumerate() {
            // Keep one owned environment for the final segment and clone only when a mixed range
            // must expose both owner domains. Homogeneous premerge/frozen rows stay move-only.
            let mut type_environment = if segment_index + 1 == segment_count {
                environment
                    .take()
                    .expect("type-context environment must have one owner")
            } else {
                environment
                    .as_ref()
                    .expect("type-context environment must have one owner")
                    .clone()
            };
            if !is_frozen {
                type_environment.remap_string_ids(remap);
            }
            remapped.push(RenderTypeContext {
                diagnostic_range,
                type_environment,
            });
        }
    }
    *type_contexts = remapped;
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
