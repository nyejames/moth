//! Generic parameter declarations and scope validation.
//!
//! WHAT: owns parsed generic parameter IDs/lists and validates their declaration-local names.
//! WHY: header parsing and AST validation need the same parameter rules, while semantic
//!      type identity stays in `TypeEnvironment` as canonical `GenericParameterId`s.

use crate::compiler_frontend::builtins::error_type::is_reserved_builtin_symbol;
use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, InvalidDeclarationReason};
use crate::compiler_frontend::datatypes::ids::{GenericParameterId, TypeId};
use crate::compiler_frontend::symbols::identifier_policy::is_camel_case_type_name;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringIdRemap, StringTable};
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use rustc_hash::{FxHashMap, FxHashSet};

/// Boxed diagnostic result for generic-parameter scope validation.
///
/// WHAT: keeps `from_parameter_list` on a small error boundary so the large
///       `CompilerDiagnostic` value does not inflate every successful scope build.
/// WHY: both declaration-syntax parsing and AST type-resolution already box
///      their generic-parameter diagnostic boundaries; this owner should match.
type GenericParameterScopeResult<T> = Result<T, Box<CompilerDiagnostic>>;

// -----------------------------------------------------------
//  Generic Parameter Declarations
// -----------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TypeParameterId(pub u32);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GenericParameter {
    pub id: TypeParameterId,
    pub name: StringId,
    pub location: SourceLocation,
    pub trait_bounds: Vec<GenericTraitBound>,
}

/// Parsed declaration-site trait bound on one generic parameter.
///
/// WHAT: records `type T is TRAIT` before AST resolves `TRAIT` to a stable `TraitId`.
/// WHY: bounds belong to generic parameter metadata from the start; later stages should not
///      recover them from raw tokens or a parallel declaration table.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GenericTraitBound {
    pub trait_name: StringId,
    pub location: SourceLocation,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GenericParameterList {
    pub parameters: Vec<GenericParameter>,
}

impl GenericParameter {
    /// Remap the parameter name and source location into a merged string table.
    ///
    // Called by per-file frontend output remapping before module-wide dependency sorting.
    pub fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        self.name = remap.get(self.name);
        self.location.remap_string_ids(remap);
        for trait_bound in &mut self.trait_bounds {
            trait_bound.remap_string_ids(remap);
        }
    }

    pub fn rebind_source_identity(&mut self, logical_path: &InternedPath) {
        self.location.rebind_source_identity(logical_path);
        for trait_bound in &mut self.trait_bounds {
            trait_bound.rebind_source_identity(logical_path);
        }
    }
}

impl GenericParameterList {
    /// Remap every generic parameter in this list.
    ///
    // Called by per-file frontend output remapping before module-wide dependency sorting.
    pub fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        for parameter in &mut self.parameters {
            parameter.remap_string_ids(remap);
        }
    }

    pub fn rebind_source_identity(&mut self, logical_path: &InternedPath) {
        for parameter in &mut self.parameters {
            parameter.rebind_source_identity(logical_path);
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.parameters.is_empty()
    }

    pub(crate) fn len(&self) -> usize {
        self.parameters.len()
    }

    pub(crate) fn contains_name(&self, name: StringId) -> bool {
        self.parameters
            .iter()
            .any(|parameter| parameter.name == name)
    }
}

impl GenericTraitBound {
    pub fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        self.trait_name = remap.get(self.trait_name);
        self.location.remap_string_ids(remap);
    }

    pub fn rebind_source_identity(&mut self, logical_path: &InternedPath) {
        self.location.rebind_source_identity(logical_path);
    }
}

// -----------------------------------------------------------
//  Scope Validation
// -----------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct GenericParameterScope {
    parameters_by_name: FxHashMap<StringId, ScopedGenericParameter>,
}

/// Generic type state visible while parsing one executable generic body.
///
/// WHAT: pairs the canonical parameter names visible in source annotations with
/// optional concrete substitutions for an emitted generic function instance.
/// WHY: template validation should resolve `T` to its generic parameter `TypeId`,
/// while concrete instance emission must resolve the same annotation to the
/// inferred concrete `TypeId`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ActiveGenericTypeContext {
    pub(crate) parameter_scope: GenericParameterScope,
    pub(crate) substitutions: Option<FxHashMap<GenericParameterId, TypeId>>,
    pub(crate) source_parameter_by_rebased_path: FxHashMap<InternedPath, GenericParameterId>,
}

/// A generic parameter visible while resolving one declaration.
///
/// WHAT: pairs the parsed declaration-local ID with the canonical semantic ID
/// allocated by `TypeEnvironment`.
/// WHY: unused-parameter validation stays parse-local, while every semantic
/// `TypeId` must use declaration-unique canonical generic parameter identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScopedGenericParameter {
    pub(crate) local_id: TypeParameterId,
    pub(crate) canonical_id: Option<GenericParameterId>,
    pub(crate) name: StringId,
    pub(crate) location: SourceLocation,
}

impl GenericParameterScope {
    pub(crate) fn empty() -> Self {
        Self {
            parameters_by_name: FxHashMap::default(),
        }
    }

    pub(crate) fn from_parameter_list(
        parameter_list: &GenericParameterList,
        canonical_by_local: Option<&FxHashMap<TypeParameterId, GenericParameterId>>,
        forbidden_names: &FxHashSet<StringId>,
        string_table: &StringTable,
        _compilation_stage: &str,
    ) -> GenericParameterScopeResult<Self> {
        let mut scope = Self::empty();

        for parameter in &parameter_list.parameters {
            if scope.parameters_by_name.contains_key(&parameter.name) {
                return Err(Box::new(CompilerDiagnostic::invalid_declaration(
                    InvalidDeclarationReason::DuplicateGenericParameter {
                        parameter_name: parameter.name,
                    },
                    None,
                    parameter.location.to_owned(),
                )));
            }

            if forbidden_names.contains(&parameter.name) {
                return Err(Box::new(CompilerDiagnostic::invalid_declaration(
                    InvalidDeclarationReason::GenericParameterNameCollision {
                        parameter_name: parameter.name,
                    },
                    None,
                    parameter.location.to_owned(),
                )));
            }

            let parameter_name = string_table.resolve(parameter.name);
            if is_reserved_generic_parameter_name(parameter_name) {
                return Err(Box::new(CompilerDiagnostic::invalid_declaration(
                    InvalidDeclarationReason::ReservedGenericParameterName {
                        parameter_name: parameter.name,
                    },
                    None,
                    parameter.location.to_owned(),
                )));
            }

            if !is_generic_parameter_name(parameter_name) {
                return Err(Box::new(CompilerDiagnostic::invalid_declaration(
                    InvalidDeclarationReason::InvalidGenericParameterName {
                        parameter_name: parameter.name,
                    },
                    None,
                    parameter.location.to_owned(),
                )));
            }

            scope.parameters_by_name.insert(
                parameter.name,
                scoped_parameter(parameter, canonical_by_local),
            );
        }

        Ok(scope)
    }

    /// Builds a source-name lookup from a canonical `TypeEnvironment` parameter list.
    ///
    /// WHAT: recreates the body-resolution view after signatures have already
    /// been canonicalized and stored in `TypeEnvironment`.
    /// WHY: generic body parsing no longer has the parsed local parameter list at
    /// hand, but local annotations still need to resolve names such as `T`.
    pub(crate) fn from_canonical_parameter_list(
        parameter_list: &crate::compiler_frontend::datatypes::environment::GenericParameterList,
    ) -> Self {
        let mut scope = Self::empty();

        for parameter in &parameter_list.parameters {
            scope.parameters_by_name.insert(
                parameter.name,
                ScopedGenericParameter {
                    // The parsed local ID is no longer semantically relevant at
                    // body-emission time. Keep a stable diagnostic placeholder
                    // while canonical_id remains the source of type identity.
                    local_id: TypeParameterId(parameter.id.0),
                    canonical_id: Some(parameter.id),
                    name: parameter.name,
                    location: SourceLocation::default(),
                },
            );
        }

        scope
    }

    pub(crate) fn resolve(&self, name: StringId) -> Option<&ScopedGenericParameter> {
        self.parameters_by_name.get(&name)
    }

    pub(crate) fn contains_name(&self, name: StringId) -> bool {
        self.parameters_by_name.contains_key(&name)
    }
}

fn scoped_parameter(
    parameter: &GenericParameter,
    canonical_by_local: Option<&FxHashMap<TypeParameterId, GenericParameterId>>,
) -> ScopedGenericParameter {
    ScopedGenericParameter {
        local_id: parameter.id,
        canonical_id: canonical_by_local.and_then(|mapping| mapping.get(&parameter.id).copied()),
        name: parameter.name,
        location: parameter.location.clone(),
    }
}

// -----------------------------------------------------------
//  Naming Invariants
// -----------------------------------------------------------

fn is_generic_parameter_name(name: &str) -> bool {
    if name.len() == 1 {
        return name
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_uppercase());
    }

    is_camel_case_type_name(name)
}

fn is_reserved_generic_parameter_name(name: &str) -> bool {
    matches!(name, "Int" | "Float" | "Bool" | "String" | "Char") || is_reserved_builtin_symbol(name)
}
