//! # Infrastructure Failures and the Diagnostic Boundary
//!
//! This module owns typed infrastructure failures, ordered cause aggregation and the frozen
//! diagnostic boundary. User-facing source diagnostics are constructed as typed family drafts in
//! `diagnostics::families`.
//!
//! `InfrastructureFailure` carries a compact infrastructure kind, compiler stage, optional owned host
//! path, message and typed details. Infrastructure causes retain no source span or compact
//! compiler identity, so aggregation needs no source-owner pairing. `InfrastructureErrors`
//! preserves primary-then-additional cause order until the command boundary renders the event.
//!
//! User diagnostics travel through `DiagnosticBag`, `PreparedFailure`, report freezing and the
//! terminal, terse or dev-server renderer. Infrastructure failures remain separate from that
//! diagnostic storage and are presented by the shared infrastructure renderers.
//!
//! Proven compiler invariants use `compiler_bug!`. `InfrastructureFailure` is the operational
//! and unproven-internal lane: every constructor is stage-bearing, and failures carry a typed
//! kind, optional host path, message and details without source spans or compact identity.

use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::diagnostics::report::DiagnosticReportSet;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathTable};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use std::path::Path;

// -------------------------
//  Compilation result boundaries
// -------------------------

/// An owning compilation boundary has three disjoint lanes: a successful outcome with optional
/// warnings, a diagnosed outcome, or an outer `InfrastructureErrors` failure.
///
/// If diagnostics and infrastructure failures meet at a boundary, infrastructure wins: every
/// infrastructure cause is returned and the diagnostic set is discarded. Compiler bugs remain
/// panics. Warning reports are populated only by a command/report owner, after the project source
/// database's final use. Inner boundaries leave `warnings` as `None` and stage warning batches in
/// the command-owned accumulator.
pub type CompilationResult<T> = Result<CompilationOutcome<T>, InfrastructureErrors>;

#[derive(Debug)]
pub enum CompilationOutcome<T> {
    Success {
        value: T,
        warnings: Option<DiagnosticReportSet>,
    },
    Diagnosed(DiagnosticReportSet),
}
impl<T> CompilationOutcome<T> {
    /// Transform the success payload while preserving diagnosed outcomes unchanged.
    pub(crate) fn map_value<U>(self, map: impl FnOnce(T) -> U) -> CompilationOutcome<U> {
        match self {
            Self::Success { value, warnings } => CompilationOutcome::Success {
                value: map(value),
                warnings,
            },
            Self::Diagnosed(reports) => CompilationOutcome::Diagnosed(reports),
        }
    }
}

// -------------------------
//  Infrastructure failures
// -------------------------

/// Ordered infrastructure failures; the first cause remains primary.
#[derive(Debug)]
pub struct InfrastructureErrors {
    pub(crate) primary: InfrastructureFailure,
    pub(crate) additional: Vec<InfrastructureFailure>,
}

impl InfrastructureErrors {
    pub(crate) fn new(primary: InfrastructureFailure) -> Self {
        Self {
            primary,
            additional: Vec::new(),
        }
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &InfrastructureFailure> {
        std::iter::once(&self.primary).chain(self.additional.iter())
    }

    pub(crate) fn push(&mut self, error: InfrastructureFailure) {
        self.additional.push(error);
    }

    pub(crate) fn append(&mut self, other: Self) {
        self.additional.push(other.primary);
        self.additional.extend(other.additional);
    }
}
impl From<InfrastructureFailure> for InfrastructureErrors {
    fn from(error: InfrastructureFailure) -> Self {
        Self::new(error)
    }
}

impl From<Box<InfrastructureErrors>> for InfrastructureErrors {
    fn from(errors: Box<InfrastructureErrors>) -> Self {
        *errors
    }
}

impl From<InfrastructureFailure> for Box<InfrastructureErrors> {
    fn from(error: InfrastructureFailure) -> Self {
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

/// The rendered classification for an infrastructure failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InfrastructureClass {
    CompilerBug,
    Operational,
}

/// Stable infrastructure cause classification and presentation heading.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum InfrastructureFailureKind {
    Internal,
    HirTransformation,
    LirTransformation,
    WasmGeneration,
    Filesystem,
    DevelopmentServer,
}

impl InfrastructureFailureKind {
    pub(crate) const fn class(self) -> InfrastructureClass {
        match self {
            Self::Internal
            | Self::HirTransformation
            | Self::LirTransformation
            | Self::WasmGeneration => InfrastructureClass::CompilerBug,
            Self::Filesystem | Self::DevelopmentServer => InfrastructureClass::Operational,
        }
    }

    pub(crate) const fn heading(self) -> &'static str {
        match self {
            Self::Internal => "Compiler Bug",
            Self::HirTransformation => "HIR Transformation Bug",
            Self::LirTransformation => "LIR Transformation Bug",
            Self::WasmGeneration => "Wasm Generation Bug",
            Self::Filesystem => "Filesystem Failure",
            Self::DevelopmentServer => "Dev Server Failure",
        }
    }
}

/// Typed facts retained for the infrastructure renderers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum InfrastructureDetail {
    PrimarySuggestion(Box<str>),
    OutputRejectionReason(Box<str>),
}

// -------------------------
//  Infrastructure Failure
// -------------------------

/// An infrastructure failure: an operational failure, or an internal case not proven impossible.
/// `InfrastructureFailureKind::class` selects its operational or compiler-bug introduction inside
/// the infrastructure record. Violations of proven compiler invariants use `compiler_bug!` instead.
#[derive(Debug, Clone)]
pub struct InfrastructureFailure {
    pub(crate) kind: InfrastructureFailureKind,
    pub(crate) stage: CompilerStage,
    pub(crate) host_path: Option<Box<Path>>,
    pub(crate) message: Box<str>,
    pub(crate) details: Box<[InfrastructureDetail]>,
}

const _: () = assert!(std::mem::size_of::<InfrastructureFailure>() <= 56);

impl InfrastructureFailure {
    fn new(
        kind: InfrastructureFailureKind,
        stage: CompilerStage,
        host_path: Option<Box<Path>>,
        message: impl Into<Box<str>>,
    ) -> Self {
        Self {
            kind,
            stage,
            host_path,
            message: message.into(),
            details: Box::new([]),
        }
    }

    pub(crate) fn internal(stage: CompilerStage, message: impl Into<Box<str>>) -> Self {
        Self::new(InfrastructureFailureKind::Internal, stage, None, message)
    }

    pub(crate) fn hir_transformation(message: impl Into<Box<str>>) -> Self {
        Self::new(
            InfrastructureFailureKind::HirTransformation,
            CompilerStage::HirConstruction,
            None,
            message,
        )
    }

    pub(crate) fn lir_transformation(message: impl Into<Box<str>>) -> Self {
        Self::new(
            InfrastructureFailureKind::LirTransformation,
            CompilerStage::BackendLowering,
            None,
            message,
        )
    }

    pub(crate) fn wasm_generation(message: impl Into<Box<str>>) -> Self {
        Self::new(
            InfrastructureFailureKind::WasmGeneration,
            CompilerStage::BackendLowering,
            None,
            message,
        )
    }

    pub(crate) fn filesystem(
        stage: CompilerStage,
        path: &Path,
        message: impl Into<Box<str>>,
    ) -> Self {
        Self::new(
            InfrastructureFailureKind::Filesystem,
            stage,
            Some(path.to_path_buf().into_boxed_path()),
            message,
        )
    }
    /// Construct the typed filesystem failure used when a command entry path cannot cross the
    /// compiler's UTF-8 string interface.
    ///
    /// Command entry validation belongs to project assembly and retains the original host path.
    pub fn non_utf8_entry_path(path: &Path, command: &str) -> Self {
        Self::filesystem(
            CompilerStage::ProjectBuildAssembly,
            path,
            format!(
                "Could not pass non-UTF-8 absolute {command} path '{}' to the compiler.",
                path.display()
            ),
        )
    }

    pub(crate) fn development_server(path: &Path, message: impl Into<Box<str>>) -> Self {
        Self::new(
            InfrastructureFailureKind::DevelopmentServer,
            CompilerStage::DevelopmentServer,
            Some(path.into()),
            message,
        )
    }

    pub(crate) fn with_detail(mut self, detail: InfrastructureDetail) -> Self {
        let mut details = std::mem::take(&mut self.details).into_vec();
        details.push(detail);
        self.details = details.into_boxed_slice();
        self
    }

    /// The failure text, for host tools outside the compiler crate.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Preserve the exact context and payload while reporting a malformed token projection.
    pub(crate) fn malformed_token_projection(context: &str, error: impl std::fmt::Debug) -> Self {
        Self::internal(
            CompilerStage::Tokenization,
            format!("{context} token payload was malformed: {error:?}"),
        )
    }
}
