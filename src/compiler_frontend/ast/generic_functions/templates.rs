//! Generic function template records.
//!
//! WHAT: stores the original generic function body plus its resolved signature.
//! WHY: concrete instance emission reparses the body under inferred type substitutions while
//! keeping the original source locations for diagnostics.

use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::datatypes::ids::GenericParameterListId;
use crate::compiler_frontend::semantic_identity::GeneratedDeclarationIdentity;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, SourceLocation};

#[derive(Clone, Debug)]
pub(crate) struct GenericFunctionTemplate {
    pub(crate) function_path: InternedPath,
    pub(crate) source_file: InternedPath,
    /// Imported contracts already know their stable declaration origin. Local templates receive
    /// it from the public/private identity join after AST construction.
    pub(crate) declaration_identity: Option<GeneratedDeclarationIdentity>,
    pub(crate) generic_parameter_list_id: GenericParameterListId,
    pub(crate) signature: FunctionSignature,
    /// Only the declaring module retains body syntax. A requester imports the semantic contract
    /// and emits a generated-sidecar request without copying this payload.
    pub(crate) body_tokens: Option<FileTokens>,
    pub(crate) declaration_location: SourceLocation,
}
