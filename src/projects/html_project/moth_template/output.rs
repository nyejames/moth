//! Output types for the direct Moth template API.
//!
//! WHAT: keeps the public result surface limited to compiled strings, source paths, relative
//! directory metadata, deferred resource outputs, the request's physical resource registry,
//! warnings, and the finalized source contexts needed to render those warnings.
//! WHY: callers should not depend on AST constants, interned paths, folded-value internals, HIR
//! or builder artifact policy. The registry and the deferred outputs travel with the result
//! because emission is the caller's policy, not this lane's; warning contexts travel only when
//! warning diagnostics retain source-backed data.
use crate::build_system::build::DeferredResourceOutput;
use crate::build_system::create_project_modules::resource_inputs::ResourceInputRegistry;
use crate::compiler_frontend::compiler_messages::prepared_batch::PreparedDiagnosticBatch;
use crate::compiler_frontend::source::SourceDatabase;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Debug)]
pub(crate) struct MothTemplateCompileOutput {
    pub(crate) documents: Vec<CompiledMothTemplateDocument>,
    pub(crate) resources: Vec<DeferredResourceOutput>,
    pub(crate) resource_inputs: ResourceInputRegistry,
    pub(crate) warnings: Vec<PreparedTemplateWarning>,
}

#[derive(Debug)]
pub(crate) struct PreparedTemplateWarning {
    pub(crate) batch: PreparedDiagnosticBatch,
    pub(crate) source_database: Arc<SourceDatabase>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CompiledMothTemplateDocument {
    pub(crate) source_path: PathBuf,
    pub(crate) relative_path: Option<PathBuf>,
    pub(crate) content: String,
}
