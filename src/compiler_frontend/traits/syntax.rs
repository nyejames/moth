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
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringIdRemap};
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;

/// Parsed trait declaration shell: `TRAIT must: requirements ;`
#[derive(Clone, Debug)]
pub struct TraitDeclarationSyntax {
    pub name: StringId,
    pub name_location: SourceLocation,
    pub requirements: Vec<TraitRequirementSyntax>,
    pub location: SourceLocation,
}

/// One method requirement inside a trait block.
#[derive(Clone, Debug)]
pub struct TraitRequirementSyntax {
    pub name: StringId,
    pub name_location: SourceLocation,
    // Retained with the shell as diagnostic metadata for the parsed requirement receiver.
    #[allow(dead_code)]
    pub this_usage: TraitThisUsage,
    pub signature: FunctionSignatureSyntax,
    pub location: SourceLocation,
}

/// Classification of `This` usage in a trait requirement receiver.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TraitThisUsage {
    Immutable,
    Mutable,
}

/// Reference to a trait name in a conformance list.
#[derive(Clone, Debug)]
pub struct TraitReferenceSyntax {
    pub name: StringId,
    pub location: SourceLocation,
}

/// Target type in a conformance declaration.
#[derive(Clone, Debug)]
pub struct ConformanceTargetSyntax {
    pub name: StringId,
    pub kind: ConformanceTargetKind,
    pub location: SourceLocation,
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
    pub location: SourceLocation,
}

/// Parsed trait incompatibility declaration shell: `TRAIT must not TRAIT, TRAIT`
///
/// WHAT: records a source-authored mutual exclusion between the subject trait and one or more
///      other traits. No concrete type may explicitly conform to both sides of the relation.
/// WHY: incompatibility declarations are bodyless top-level metadata discovered at the header
///      stage; semantic resolution and conflict recording happen during AST environment
///      construction after all trait definitions are registered.
#[derive(Clone, Debug)]
pub struct TraitIncompatibilitySyntax {
    pub subject: TraitReferenceSyntax,
    pub incompatible_traits: Vec<TraitReferenceSyntax>,
    pub location: SourceLocation,
}

impl TraitDeclarationSyntax {
    /// Remap every interned string owned by this trait declaration into the merged global string table.
    // Called when merging per-file frontend outputs into the module-wide compilation.
    pub fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        self.name = remap.get(self.name);
        self.name_location.remap_string_ids(remap);
        for requirement in &mut self.requirements {
            requirement.remap_string_ids(remap);
        }
        self.location.remap_string_ids(remap);
    }

    pub fn validate_required_source_prefixes(
        &self,
        provisional_source_file: &InternedPath,
    ) -> Result<(), CompilerError> {
        for requirement in &self.requirements {
            requirement.validate_required_source_prefixes(provisional_source_file)?;
        }
        Ok(())
    }

    pub fn rebind_source_identity(
        &mut self,
        logical_path: &InternedPath,
        provisional_source_file: &InternedPath,
    ) -> Result<(), CompilerError> {
        self.name_location.rebind_source_identity(logical_path);
        for requirement in &mut self.requirements {
            requirement.rebind_source_identity(logical_path, provisional_source_file)?;
        }
        self.location.rebind_source_identity(logical_path);
        Ok(())
    }
}

impl TraitRequirementSyntax {
    /// Remap every interned string owned by this requirement into the merged global string table.
    // Called when merging per-file frontend outputs into the module-wide compilation.
    pub fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        self.name = remap.get(self.name);
        self.name_location.remap_string_ids(remap);
        self.signature.remap_string_ids(remap);
        self.location.remap_string_ids(remap);
    }

    pub fn validate_required_source_prefixes(
        &self,
        provisional_source_file: &InternedPath,
    ) -> Result<(), CompilerError> {
        self.signature
            .validate_required_source_prefixes(provisional_source_file)
    }

    pub fn rebind_source_identity(
        &mut self,
        logical_path: &InternedPath,
        provisional_source_file: &InternedPath,
    ) -> Result<(), CompilerError> {
        self.name_location.rebind_source_identity(logical_path);
        self.signature
            .rebind_source_identity(logical_path, provisional_source_file)?;
        self.location.rebind_source_identity(logical_path);
        Ok(())
    }
}

impl TraitReferenceSyntax {
    /// Remap the trait reference name into the merged global string table.
    // Called when merging per-file frontend outputs into the module-wide compilation.
    pub fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        self.name = remap.get(self.name);
        self.location.remap_string_ids(remap);
    }

    pub fn rebind_source_identity(&mut self, logical_path: &InternedPath) {
        self.location.rebind_source_identity(logical_path);
    }
}

impl ConformanceTargetSyntax {
    /// Remap the target type name into the merged global string table.
    // Called when merging per-file frontend outputs into the module-wide compilation.
    pub fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        self.name = remap.get(self.name);
        self.location.remap_string_ids(remap);
    }

    pub fn rebind_source_identity(&mut self, logical_path: &InternedPath) {
        self.location.rebind_source_identity(logical_path);
    }
}

impl TraitConformanceSyntax {
    /// Remap every interned string owned by this conformance into the merged global string table.
    // Called when merging per-file frontend outputs into the module-wide compilation.
    pub fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        self.target.remap_string_ids(remap);
        for trait_ref in &mut self.traits {
            trait_ref.remap_string_ids(remap);
        }
        self.location.remap_string_ids(remap);
    }

    pub fn rebind_source_identity(&mut self, logical_path: &InternedPath) {
        self.target.rebind_source_identity(logical_path);
        for trait_ref in &mut self.traits {
            trait_ref.rebind_source_identity(logical_path);
        }
        self.location.rebind_source_identity(logical_path);
    }
}

impl TraitIncompatibilitySyntax {
    /// Remap every interned string owned by this incompatibility declaration into the merged
    /// global string table.
    // Called when merging per-file frontend outputs into the module-wide compilation.
    pub fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        self.subject.remap_string_ids(remap);
        for trait_ref in &mut self.incompatible_traits {
            trait_ref.remap_string_ids(remap);
        }
        self.location.remap_string_ids(remap);
    }

    pub fn rebind_source_identity(&mut self, logical_path: &InternedPath) {
        self.subject.rebind_source_identity(logical_path);
        for trait_ref in &mut self.incompatible_traits {
            trait_ref.rebind_source_identity(logical_path);
        }
        self.location.rebind_source_identity(logical_path);
    }
}
