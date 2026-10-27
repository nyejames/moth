//! Stage names shared by compiler bug reports and infrastructure failures.

use std::fmt;

/// Compiler, build-system, or tooling boundary associated with a failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(
    dead_code,
    reason = "stage variants also name pipeline and host boundaries that do not currently raise compiler_bug!"
)]
pub(crate) enum CompilerStage {
    SourceLoading,
    Tokenization,
    HeaderParsing,
    DependencyResolution,
    AstConstruction,
    ConstantEvaluation,
    GenericMaterialisation,
    HirConstruction,
    BorrowChecking,
    DiagnosticReporting,
    ProjectBuildAssembly,
    BackendLowering,
    Timing,
    Output,
    DevelopmentServer,
    /// A foreign panic payload whose raising stage is not recorded.
    Unknown,
}

impl CompilerStage {
    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::SourceLoading => "Source loading",
            Self::Tokenization => "Tokenization",
            Self::HeaderParsing => "Header parsing",
            Self::DependencyResolution => "Dependency resolution",
            Self::AstConstruction => "AST construction",
            Self::ConstantEvaluation => "Constant evaluation",
            Self::GenericMaterialisation => "Generic materialisation",
            Self::HirConstruction => "HIR construction",
            Self::BorrowChecking => "Borrow checking",
            Self::DiagnosticReporting => "Diagnostic reporting",
            Self::ProjectBuildAssembly => "Project build/assembly",
            Self::BackendLowering => "Backend lowering",
            Self::Timing => "Timing",
            Self::Output => "Output",
            Self::DevelopmentServer => "Development server",
            Self::Unknown => "unknown",
        }
    }
}

impl fmt::Display for CompilerStage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}
