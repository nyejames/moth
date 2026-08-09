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
//!   -> CompilerDiagnostic { kind, severity, primary_location, labels, payload }
//!   -> DiagnosticBag accumulates one or many diagnostics locally
//!   -> CompilerMessages owns ordered diagnostics + StringTable + range-bound render type contexts
//!      at stage/build boundaries
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
//! ### Shared StringTable Context
//! Diagnostics preserve interned path scopes, so top-level renderers and file-adjacent helpers
//! resolve paths through the shared `StringTable` for the current build or parse lifecycle.
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
//! CompilerMessages (ordered diagnostics + StringTable)
//! ```

pub use crate::compiler_frontend::compiler_messages::source_location::SourceLocation;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticKind, DiagnosticPayload, DiagnosticSeverity,
    InfrastructureDiagnosticKind,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::symbols::interned_path::{InternedPath, NonUtf8PathComponent};
use crate::compiler_frontend::symbols::string_interning::{StringIdRemap, StringTable};
use std::collections::HashMap;
use std::ops::Range;
use std::path::Path;

// -------------------------
//  Compiler Message Set
// -------------------------

#[derive(Debug, Clone)]
pub struct CompilerMessages {
    /// Ordered diagnostics at a build/render boundary.
    ///
    /// WHAT: stores errors and warnings in the order the compiler produced them.
    /// WHY: renderers, tests, dev-server summaries, and CLI output all consume this one sequence
    /// instead of consulting parallel message stores.
    pub(crate) diagnostics: Vec<CompilerDiagnostic>,

    pub string_table: StringTable,

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

#[derive(Debug, Clone)]
pub(crate) struct RenderTypeContext {
    pub(crate) diagnostic_range: Range<usize>,
    pub(crate) type_environment: TypeEnvironment,
}

impl CompilerMessages {
    pub fn empty(string_table: StringTable) -> Self {
        Self {
            diagnostics: Vec::new(),
            string_table,
            render_type_contexts: Vec::new(),
        }
    }

    pub(crate) fn from_diagnostics(
        diagnostics: Vec<CompilerDiagnostic>,
        string_table: StringTable,
    ) -> Self {
        Self {
            diagnostics,
            string_table,
            render_type_contexts: Vec::new(),
        }
    }

    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|d| d.severity == DiagnosticSeverity::Error)
    }

    pub fn has_warnings(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|d| d.severity == DiagnosticSeverity::Warning)
    }

    /// Count diagnostics with `Error` severity.
    pub fn error_count(&self) -> usize {
        self.diagnostics
            .iter()
            .filter(|d| d.severity == DiagnosticSeverity::Error)
            .count()
    }

    /// Count diagnostics with `Warning` severity.
    pub fn warning_count(&self) -> usize {
        self.diagnostics
            .iter()
            .filter(|d| d.severity == DiagnosticSeverity::Warning)
            .count()
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

    #[cfg(test)]
    pub(crate) fn first_infrastructure_error_for_tests(
        &self,
    ) -> Option<(&ErrorType, &str, &SourceLocation)> {
        let diagnostic = self.first_error()?;
        let DiagnosticPayload::InfrastructureError {
            msg, error_type, ..
        } = &diagnostic.payload
        else {
            return None;
        };

        Some((error_type, msg.as_str(), &diagnostic.primary_location))
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
            string_table,
            render_type_contexts: Vec::new(),
        }
    }

    /// Wrap a single `CompilerDiagnostic` while cloning the caller's active `StringTable`.
    pub fn from_diagnostic_ref(diagnostic: CompilerDiagnostic, string_table: &StringTable) -> Self {
        Self::from_diagnostic(diagnostic, string_table.clone())
    }

    /// Wrap a single `CompilerError` into a `CompilerMessages` container with no warnings.
    ///
    /// WHY: Several build/backend modules need to convert a `CompilerError` into the richer
    /// `CompilerMessages` type at a boundary. Centralising this avoids repeated inline struct
    /// literals scattered across callers.
    ///
    /// When the error carries an attached render-identity context (set at a module semantic
    /// boundary), the context's `StringTable` is merged into the supplied target table and the
    /// error's interned location is remapped into that target exactly once. This keeps the
    /// location resolvable in the returned message set even when the error's path IDs were issued
    /// by a module-local table that no longer exists. Errors without an attached context keep the
    /// original behavior: the diagnostic borrows the location unchanged against the target table.
    pub fn from_error(mut error: CompilerError, mut string_table: StringTable) -> Self {
        if let Some(render_context) = error.take_render_context() {
            let remap = string_table.merge_from(&render_context);
            error.remap_string_ids(&remap);
        }
        let diagnostic = compiler_error_to_diagnostic(&error);
        Self {
            diagnostics: vec![diagnostic],
            string_table,
            render_type_contexts: Vec::new(),
        }
    }

    /// Wrap one error while cloning the caller's active `StringTable`.
    ///
    /// WHAT: snapshots the current table state into the returned message container.
    /// WHY: frontend/build boundaries often only borrow the shared table, but diagnostics still
    /// need the full interned-path context accumulated so far.
    pub fn from_error_ref(error: CompilerError, string_table: &StringTable) -> Self {
        Self::from_error(error, string_table.clone())
    }

    /// Wrap already-collected warnings plus one infrastructure error while preserving table context.
    ///
    /// WHAT: carries forward the caller's warning set, appends the boundary failure, and clones
    /// the current `StringTable`.
    /// WHY: these helpers receive warnings that were produced before the failure. Keeping that
    /// order makes `CompilerMessages` a true production-order diagnostic stream.
    pub fn from_error_with_warnings(
        mut error: CompilerError,
        warning_diagnostics: Vec<CompilerDiagnostic>,
        string_table: &StringTable,
    ) -> Self {
        // Merge an attached render-identity context into a clone of the caller's table before
        // building the diagnostic, mirroring `from_error`. The warnings were produced against the
        // caller's table, so merging only adds the error's local strings and leaves their IDs
        // valid in the resulting table.
        let mut merged_table = string_table.clone();
        if let Some(render_context) = error.take_render_context() {
            let remap = merged_table.merge_from(&render_context);
            error.remap_string_ids(&remap);
        }
        let mut diagnostics = warning_diagnostics;
        diagnostics.push(compiler_error_to_diagnostic(&error));
        Self {
            diagnostics,
            string_table: merged_table,
            render_type_contexts: Vec::new(),
        }
    }

    /// Wrap already-collected warnings plus one typed diagnostic while preserving table context.
    ///
    /// WHAT: carries forward the caller's warning set and a clone of the current `StringTable`,
    /// then stores the typed boundary diagnostic directly in `diagnostics`.
    /// WHY: frontend stages that emit `CompilerDiagnostic` need to preserve structured payloads
    /// so that boundary renderers can resolve `StringId` values through the shared `StringTable`.
    /// The warning set was emitted before the failure, so it stays first.
    pub fn from_diagnostic_with_warnings(
        diagnostic: CompilerDiagnostic,
        warning_diagnostics: Vec<CompilerDiagnostic>,
        string_table: &StringTable,
    ) -> Self {
        let mut diagnostics = warning_diagnostics;
        diagnostics.push(diagnostic);
        Self {
            diagnostics,
            string_table: string_table.clone(),
            render_type_contexts: Vec::new(),
        }
    }

    /// Build a single file-scoped message set while preserving the caller's existing table state.
    ///
    /// WHAT: clones the current table, interns the failing path into that clone, and returns a
    /// message set that owns the resulting diagnostic context.
    /// WHY: file-system errors often arise after the current build already interned many other
    /// paths, so the returned diagnostics must preserve those older interned IDs as well.
    pub fn file_error(path: &Path, msg: impl Into<String>, string_table: &StringTable) -> Self {
        let mut error_string_table = string_table.clone();
        let error = CompilerError::file_error(path, msg, &mut error_string_table);
        Self::from_error(error, error_string_table)
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
    /// WHAT: shifts every stored type-context range forward by the prepended length.
    /// WHY: frontend/build aggregation often carries warnings from earlier stages into a later
    /// failure. Those warnings must stay before the failure without disconnecting the failure's
    /// diagnostics from their render type table.
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

        for type_context in &mut self.render_type_contexts {
            type_context.diagnostic_range.start += shift;
            type_context.diagnostic_range.end += shift;
        }
    }

    /// Append another boundary message set while preserving its diagnostic type-context ranges.
    ///
    /// WHAT: moves diagnostics and render contexts from `messages` into this set and offsets the
    /// appended ranges by the current diagnostic count.
    /// WHY: directory builds aggregate failed modules into one ordered build failure. Each module
    /// may have its own `TypeEnvironment`, so the render boundary must preserve all of them.
    pub(crate) fn append_messages_preserving_context(&mut self, mut messages: CompilerMessages) {
        let shift = self.diagnostics.len();
        self.diagnostics.append(&mut messages.diagnostics);

        for mut type_context in messages.render_type_contexts {
            type_context.diagnostic_range.start += shift;
            type_context.diagnostic_range.end += shift;
            self.render_type_contexts.push(type_context);
        }
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
        crate::compiler_frontend::compiler_messages::render::DiagnosticRenderContext::new(
            &self.string_table,
        )
        .with_optional_type_environment(self.type_environment_for_diagnostic(diagnostic_index))
    }

    #[cfg(test)]
    pub(crate) fn render_type_contexts(&self) -> &[RenderTypeContext] {
        &self.render_type_contexts
    }

    pub fn remap_string_ids(&mut self, remap: &StringIdRemap) {
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

    // Stores the interned source scope for this diagnostic. Header-local scopes may include a
    // synthetic `.header` suffix and are resolved back to real file paths only at render time.
    pub location: SourceLocation,
    pub error_type: ErrorType,

    // Structured guidance for internal/tooling failures. User-facing diagnostics carry typed
    // payload facts on `CompilerDiagnostic` instead of using this string map.
    pub metadata: HashMap<CompilerErrorMetadataKey, String>,

    // Optional self-contained render-identity context for this error's interned `location`.
    //
    // WHAT: carries the `StringTable` that issued the `SourceLocation`'s interned path IDs, so a
    //       later ownership boundary can merge those IDs into its own table and remap the location
    //       exactly once instead of resolving it against a mismatched or empty table.
    // WHY: an infrastructure error recovered at a module semantic boundary can carry a location
    //      whose interned path IDs are only valid in the module-local table that produced them.
    //      Without this context a consumer that supplies a different table cannot resolve or
    //      remap the location, so the path would render incorrectly or point at the wrong string.
    //      The context is attached only at ownership boundaries that need it; every ordinary
    //      constructor leaves it `None` so cheap construction and cloning are unchanged.
    render_context: Option<StringTable>,
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
    messages.string_table = string_table.clone();
    messages
}

impl CompilerError {
    pub fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        // An attached render context is only authoritative for the location's original ID space.
        // Once an external caller remaps the location into another table, that table becomes the
        // authority and the attached context would describe a stale identity. Drop it so no
        // double-authority can survive the conversion. `CompilerMessages::from_error` takes the
        // context before remapping, so this drop only affects other external remap callers.
        self.render_context = None;
        self.location.remap_string_ids(remap);
    }

    pub fn new(
        msg: impl Into<String>,
        location: SourceLocation,
        error_type: ErrorType,
    ) -> CompilerError {
        CompilerError {
            msg: msg.into(),
            location,
            error_type,
            metadata: HashMap::new(),
            render_context: None,
        }
    }

    /// Attach structured guidance metadata to this error and return it for chaining.
    ///
    /// WHAT: replaces the metadata map wholesale.
    /// WHY: the module semantic boundary recovers an infrastructure error's metadata from its
    ///      structured payload alongside the message, location and error type, so a single
    ///      builder keeps that reconstruction readable without repeated `new_metadata_entry` calls.
    pub fn with_metadata(mut self, metadata: HashMap<CompilerErrorMetadataKey, String>) -> Self {
        self.metadata = metadata;
        self
    }

    /// Attach the render-identity `StringTable` that issued this error's interned location.
    ///
    /// WHAT: stores the table so a later `CompilerMessages::from_error` boundary can merge the
    ///       location's IDs into its own table and remap the location exactly once.
    /// WHY: only ownership boundaries that move an error across a string-table scope need this.
    ///      Ordinary constructors leave the context `None`, so this builder is the single attach
    ///      point and keeps normal construction cheap.
    pub fn with_render_context(mut self, string_table: StringTable) -> Self {
        self.render_context = Some(string_table);
        self
    }

    /// Take the attached render-identity context, leaving `None`.
    ///
    /// WHAT: moves the optional `StringTable` out of this error.
    /// WHY: `CompilerMessages::from_error` consumes the context once to merge and remap the
    ///      location, so the error never keeps a stale authority after the boundary conversion.
    pub(crate) fn take_render_context(&mut self) -> Option<StringTable> {
        self.render_context.take()
    }

    /// Replace only the location scope path while preserving the existing span positions.
    ///
    /// WHAT: rewrites the interned path for a diagnostic without touching its line/column data.
    /// WHY: some helpers need to attach a resolved file path after building a precise span-based
    /// error, and downgrading that span to a file-level location would lose useful diagnostics.
    pub fn with_scope_path(mut self, file_path: &Path, string_table: &mut StringTable) -> Self {
        self.location.scope = match InternedPath::try_from_filesystem_path(file_path, string_table)
        {
            Ok(interned) => interned,
            Err(NonUtf8PathComponent { .. }) => {
                InternedPath::from_single_str(&format!("{file_path:?}"), string_table)
            }
        };
        self
    }

    pub fn with_error_type(mut self, error_type: ErrorType) -> Self {
        self.error_type = error_type;
        self
    }

    pub fn new_metadata_entry(&mut self, key: CompilerErrorMetadataKey, value: String) {
        self.metadata.insert(key, value);
    }

    /// Create a thread panic error (internal compiler_frontend issue)
    pub fn new_thread_panic(msg: impl Into<String>) -> Self {
        CompilerError {
            msg: msg.into(),
            location: SourceLocation::default(),
            error_type: ErrorType::Compiler,
            metadata: HashMap::new(),
            render_context: None,
        }
    }

    /// Create a compiler_frontend error (internal bug, not user's fault)
    // Existing backend and frontend invariant checks use `CompilerError::compiler_error(...)`
    // as the direct constructor for infrastructure diagnostics.
    #[allow(clippy::self_named_constructors)]
    pub fn compiler_error(msg: impl Into<String>) -> Self {
        CompilerError {
            msg: msg.into(),
            location: SourceLocation::default(),
            error_type: ErrorType::Compiler,
            metadata: HashMap::new(),
            render_context: None,
        }
    }

    /// Create a file system error from a Path
    pub fn file_error(path: &Path, msg: impl Into<String>, string_table: &mut StringTable) -> Self {
        CompilerError {
            msg: msg.into(),
            location: SourceLocation::from_path(path, string_table),
            error_type: ErrorType::File,
            metadata: HashMap::new(),
            render_context: None,
        }
    }

    /// Create a file system error from Path with metadata
    pub fn new_file_error(
        path: &Path,
        msg: impl Into<String>,
        metadata: HashMap<CompilerErrorMetadataKey, String>,
        string_table: &mut StringTable,
    ) -> CompilerError {
        CompilerError {
            msg: msg.into(),
            location: SourceLocation::from_path(path, string_table),
            error_type: ErrorType::File,
            metadata,
            render_context: None,
        }
    }
}

// Adds more information to the CompilerError
// So it knows the file path (possible specific part of the line soon)
// And the type of error
#[derive(PartialEq, Debug, Clone)]
pub enum ErrorType {
    File,
    Config,
    Compiler,
    DevServer,
    HirTransformation,
    Backend(crate::backends::error_types::BackendErrorType),
}

/// Convert a direct `CompilerError` into the boundary diagnostic sequence.
///
/// This exists only for infrastructure/tooling paths that still return `CompilerError`.
pub(crate) fn compiler_error_to_diagnostic(error: &CompilerError) -> CompilerDiagnostic {
    CompilerDiagnostic::with_severity(
        DiagnosticKind::Infrastructure(InfrastructureDiagnosticKind::InfrastructureFailure),
        DiagnosticSeverity::Error,
        error.location.clone(),
        DiagnosticPayload::InfrastructureError {
            msg: error.msg.clone(),
            error_type: error.error_type.clone(),
            metadata: error.metadata.clone(),
        },
    )
}

/// Return a filesystem infrastructure error.
///
/// Usage: `return_file_error!(path, "message", { metadata })`;
#[macro_export]
macro_rules! return_file_error {
    // Metadata usage for direct infrastructure rendering.
    ($string_table:expr, $path:expr, $msg:expr, { $( $key:ident => $value:expr ),* $(,)? }) => {{
        return Err($crate::compiler_frontend::compiler_errors::CompilerError::new_file_error(
            $path,
            $msg,
            {
                let mut map = std::collections::HashMap::new();
                $( map.insert($crate::compiler_frontend::compiler_errors::CompilerErrorMetadataKey::$key, $value.into()); )*
                map
            },
            $string_table,
        ));
    }};
    // Usage without guidance metadata.
    ($string_table:expr, $path:expr, $msg:expr) => {{
        return Err($crate::compiler_frontend::compiler_errors::CompilerError::file_error(
            $path,
            $msg,
            $string_table,
        ));
    }};
}

/// Returns a new CompilerError for internal compiler_frontend bugs.
///
/// Compiler errors indicate bugs in the compiler_frontend itself, not user code issues.
/// These provide the location of the bug in the compiler_frontend source code
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
/// Usage: `return_hir_transformation_error!("Function '{}' transformation not yet implemented", func_name, location, {})`;
#[macro_export]
macro_rules! return_hir_transformation_error {
    // HIR failures may carry metadata for direct infrastructure rendering.
    ($msg:expr, $location:expr, { $( $key:ident => $value:expr ),* $(,)? }) => {
        let mut error = $crate::compiler_frontend::compiler_errors::CompilerError::new(
            $msg,
            $location,
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
    ($msg:expr, $location:expr) => {
        return Err($crate::compiler_frontend::compiler_errors::CompilerError::new(
            $msg,
            $location,
            $crate::compiler_frontend::compiler_errors::ErrorType::HirTransformation,
        ))
    };
}
