//! AST semantic environment construction.
//!
//! WHAT: resolves dependencies, aliases, nominal declarations, constants, signatures and receiver
//! methods into a stable declaration table before executable bodies are parsed.
//! WHY: body emission should read one complete semantic environment instead of relying on
//! partially valid accumulator fields.

pub(in crate::compiler_frontend::ast) mod builder;
pub(in crate::compiler_frontend::ast) mod constant_resolution;
mod declaration_semantics;
#[cfg(test)]
mod declaration_semantics_tests;
pub(in crate::compiler_frontend::ast) mod declaration_table;
mod function_signatures;
mod input;
mod lookups;
mod public_surface;
pub(in crate::compiler_frontend::ast) mod resolved_public_trait_roots;
pub(in crate::compiler_frontend::ast) mod resolved_public_type_roots;
mod traits;
mod type_aliases;
mod type_resolution;

// --------------------------
//  Re-exports
// --------------------------

pub(in crate::compiler_frontend::ast) use builder::AstModuleEnvironmentBuilder;
pub(crate) use declaration_semantics::{DeclarationSemanticKind, DeclarationSemanticTable};
pub(crate) use declaration_table::TopLevelDeclarationTable;
pub(in crate::compiler_frontend::ast) use input::AstEnvironmentInput;
pub(in crate::compiler_frontend::ast) use lookups::{AstModuleEnvironment, AstModuleLookups};
pub(crate) use resolved_public_trait_roots::{
    AstPublicInterfaceProjectionInput, ResolvedPublicTraitRoot, ResolvedTraitRequirementFact,
    TraitReceiverAccessKind, build_resolved_public_trait_roots,
};
#[cfg(test)]
pub(crate) use resolved_public_trait_roots::{
    ResolvedTraitParameterFact, ResolvedTraitReceiverFact, ResolvedTraitReturnFact,
};
pub(in crate::compiler_frontend::ast) use resolved_public_type_roots::{
    BuildResolvedPublicTypeRootsInput, build_resolved_public_type_roots,
};
pub(crate) use resolved_public_type_roots::{
    ResolvedPublicTypeRoot, ResolvedPublicTypeRootKind, ResolvedPublicTypeRootTable,
    ResolvedTraitSourceFact,
};
