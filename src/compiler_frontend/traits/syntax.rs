//! Parse-only syntax shells for trait declarations and conformances.
//!
//! WHAT: data structures produced by header parsing when it encounters trait-related syntax.
//! WHY: header parsing owns top-level declaration discovery; these shells preserve the parsed
//!      shape so that later phases (AST, type resolution, evidence validation) can consume it.
//!
//! These shells intentionally stay parse-only; semantic trait identity belongs to AST environment
//! construction after dependency bindings, visibility and type metadata are available.

use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::declaration_syntax::signature_members::FunctionSignatureSyntax;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathIdRemap, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringIdRemap};

/// Parsed trait declaration shell: `TRAIT must: requirements ;`
#[derive(Clone, Debug)]
pub struct TraitDeclarationSyntax {
    pub name: StringId,
    pub name_span: SourceSpan,
    /// Token-stream order of the declaration name inside its owning source file.
    ///
    /// This preserves the source-order rule for same-file incompatibility references without
    /// retaining line/column coordinates in the semantic trait record.
    pub source_order: usize,
    pub requirements: Vec<TraitRequirementSyntax>,
    pub span: SourceSpan,
}
/// One method requirement inside a trait block.
#[derive(Clone, Debug)]
pub struct TraitRequirementSyntax {
    pub name: StringId,
    #[allow(dead_code)] // Retained for deferred trait requirement diagnostics.
    pub name_span: SourceSpan,
    pub signature: FunctionSignatureSyntax,
    pub span: SourceSpan,
}

/// Reference to a trait name in a conformance list.
#[derive(Clone, Debug)]
pub struct TraitReferenceSyntax {
    pub name: StringId,
    pub span: SourceSpan,
}

/// Target type in a conformance declaration.
#[derive(Clone, Debug)]
pub struct ConformanceTargetSyntax {
    pub name: StringId,
    pub kind: ConformanceTargetKind,
    pub span: SourceSpan,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConformanceTargetKind {
    Named,
    SpecializedGenericInstance,
}

/// Parsed conformance declaration shell: `Type must TRAIT, TRAIT`
#[derive(Clone, Debug)]
pub struct TraitConformanceSyntax {
    pub target: ConformanceTargetSyntax,
    pub traits: Vec<TraitReferenceSyntax>,
}

/// Parsed trait incompatibility declaration shell: `TRAIT must not TRAIT, TRAIT`
///
/// WHAT: records a source-authored mutual exclusion between the subject trait and one or more
///      other traits. No concrete type may explicitly conform to both sides of the relation.
/// WHY: incompatibility declarations are top-level metadata discovered at the header stage; semantic
///      resolution and conflict recording happen during AST environment construction after all trait
///      definitions are registered.
#[derive(Clone, Debug)]
pub struct TraitIncompatibilitySyntax {
    pub subject: TraitReferenceSyntax,
    /// Token-stream order of the relation's subject inside its owning source file.
    pub source_order: usize,
    pub incompatible_traits: Vec<TraitReferenceSyntax>,
}

impl TraitDeclarationSyntax {
    /// Remap every interned string owned by this trait declaration.
    // Called when merging per-file frontend outputs into the module-wide compilation.
    pub fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        self.name = remap.get(self.name);
        for requirement in &mut self.requirements {
            requirement.remap_string_ids(remap);
        }
    }
    pub fn remap_path_ids(&mut self, remap: &PathIdRemap) {
        for requirement in &mut self.requirements {
            requirement.signature.remap_path_ids(remap);
        }
    }

    pub fn validate_required_source_prefixes(
        &self,
        provisional_source_file: PathId,
        path_fork: &PathInternerFork,
    ) -> Result<(), CompilerError> {
        for requirement in &self.requirements {
            requirement.validate_required_source_prefixes(provisional_source_file, path_fork)?;
        }
        Ok(())
    }
}

impl TraitRequirementSyntax {
    /// Remap every interned string owned by this requirement.
    // Called when merging per-file frontend outputs into the module-wide compilation.
    pub fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        self.name = remap.get(self.name);
        self.signature.remap_string_ids(remap);
    }

    pub fn validate_required_source_prefixes(
        &self,
        provisional_source_file: PathId,
        path_fork: &PathInternerFork,
    ) -> Result<(), CompilerError> {
        self.signature
            .validate_required_source_prefixes(provisional_source_file, path_fork)
    }
}

impl TraitReferenceSyntax {
    /// Remap the trait reference name into the merged global string table.
    // Called when merging per-file frontend outputs before module-wide dependency sorting.
    pub fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        self.name = remap.get(self.name);
    }
}

impl ConformanceTargetSyntax {
    /// Remap the target type name into the merged global string table.
    // Called when merging per-file frontend outputs before module-wide dependency sorting.
    pub fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        self.name = remap.get(self.name);
    }
}

impl TraitConformanceSyntax {
    /// Remap every interned string owned by this conformance into the merged global string table.
    // Called when merging per-file frontend outputs before module-wide dependency sorting.
    pub fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        self.target.remap_string_ids(remap);
        for trait_ref in &mut self.traits {
            trait_ref.remap_string_ids(remap);
        }
    }
}

impl TraitIncompatibilitySyntax {
    /// Remap every interned string owned by this incompatibility declaration into the merged
    /// global string table.
    // Called when merging per-file frontend outputs before module-wide dependency sorting.
    pub fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        self.subject.remap_string_ids(remap);
        for trait_ref in &mut self.incompatible_traits {
            trait_ref.remap_string_ids(remap);
        }
    }
}
