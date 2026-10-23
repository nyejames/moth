//! # Compiler Error Handling System
//!
//! This module owns `CompilerError`, infrastructure causes, and the frozen boundary failure.
//! User-facing source diagnostics live in `compiler_diagnostic.rs` as `CompilerDiagnostic`.
//!
//! ## Architecture
//!
//! ```text
//! Frontend/compiler stages
//!   -> CompilerDiagnostic { kind, severity, primary_span, primary owner, labels, payload }
//!   -> DiagnosticBag accumulates one or many diagnostics locally
//!   -> PreparedFailure carries prepared diagnostics and infrastructure failures before freeze
//!   -> BoundaryFailure owns the frozen reports returned to renderers
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
//! - Pre-freeze transport: `PreparedFailure`.
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
//! PreparedFailure -> BoundaryAccumulator -> frozen BoundaryFailure
//! ```

use crate::compiler_frontend::compiler_messages::diagnostics::report::DiagnosticReportSet;
use crate::compiler_frontend::source::{
    FrozenIdentityContext, SourceDatabase, SourceSpan, SpanCapacityError,
};
use crate::compiler_frontend::symbols::path_interner::{PathId, PathTable};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use std::collections::HashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;

// -------------------------
//  Infrastructure and Boundary Failure
// -------------------------

/// Rare failure causes and their explicitly paired source owners. Cause ranges, unlike diagnostic
/// ranges, remain meaningful when a failed compilation produced no diagnostic at all.
#[derive(Debug)]
pub struct InfrastructureErrors {
    pub(crate) primary: CompilerError,
    pub(crate) additional: Vec<CompilerError>,
    source_contexts: Vec<InfrastructureSourceContext>,
}

/// Boundary failure carrying frozen user diagnostics and infrastructure causes.
#[derive(Debug)]
pub struct BoundaryFailure {
    pub(crate) reports: DiagnosticReportSet,
    /// Infrastructure lane retained for callers that still distinguish infrastructure failures.
    pub(crate) infrastructure: Option<Box<InfrastructureErrors>>,
}

const _: () = assert!(std::mem::size_of::<BoundaryFailure>() <= 56);

impl BoundaryFailure {
    /// Count error records plus one for the infrastructure cause.
    pub(crate) fn error_count(&self) -> usize {
        let record_errors = usize::try_from(self.reports.counts().errors)
            .expect("a report set's error count fits in usize");
        record_errors + usize::from(self.has_infrastructure_error())
    }

    /// Count warning records in the frozen report set.
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
    #[cfg(any(test, feature = "data_layout_memory_probe"))]
    Frozen(Arc<FrozenIdentityContext>),
    #[cfg(not(any(test, feature = "data_layout_memory_probe")))]
    Frozen,
}

impl InfrastructureSourceOwner {
    fn frozen(identity: Arc<FrozenIdentityContext>) -> Self {
        #[cfg(any(test, feature = "data_layout_memory_probe"))]
        {
            Self::Frozen(identity)
        }
        #[cfg(not(any(test, feature = "data_layout_memory_probe")))]
        {
            let _ = identity;
            Self::Frozen
        }
    }

    fn is_frozen(&self) -> bool {
        match self {
            Self::Source(_) => false,
            #[cfg(any(test, feature = "data_layout_memory_probe"))]
            Self::Frozen(_) => true,
            #[cfg(not(any(test, feature = "data_layout_memory_probe")))]
            Self::Frozen => true,
        }
    }
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
                    context.cause_range.contains(&index) && context.owner.is_frozen()
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

    /// Pair currently unowned causes with the source that produced them.
    pub(crate) fn pair_unowned_causes(&mut self, source_database: Arc<SourceDatabase>) {
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
            .map(|context| {
                if context.owner.is_frozen() {
                    panic!("pre-frozen infrastructure source owner reached the boundary converter");
                }
                let InfrastructureSourceOwner::Source(database) = context.owner else {
                    unreachable!("unfrozen infrastructure source owner must be a database")
                };
                (context.cause_range, database)
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
                owner: InfrastructureSourceOwner::frozen(identity),
            })
            .collect();
    }
}

impl From<CompilerError> for Box<InfrastructureErrors> {
    fn from(error: CompilerError) -> Self {
        Box::new(InfrastructureErrors::new(error))
    }
}

/// Find the first defect that makes retaining a path table unsafe.
///
/// This checks required payload paths, their ancestors, and every non-root node that a retained
/// table remaps. Foreign domains are handled by the provider/materialisation boundary, which
/// retains each table with its frozen string table and rebases by spelling.
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
