//! External package registry and lookup APIs.
//!
//! WHAT: owns all registered virtual packages and provides resolution from package-scoped
//! paths to stable IDs, and from stable IDs to definitions.
//! WHY: the frontend needs one canonical source for external symbol metadata. Keeping
//! registration and lookup in one place ensures consistency between the package surface maps,
//! the ID-indexed maps, and the prelude.
use crate::compiler_frontend::instrumentation::{FrontendCounter, increment_frontend_counter};

use super::abi::{ExternalAccessKind, ExternalReturnAlias};
use super::definitions::{
    ExternalConstantDef, ExternalFunctionDef, ExternalFunctionSpec, ExternalPackage,
    ExternalTypeDef, ExternalTypeSpec,
};
use super::ids::{
    CanonicalBindingSymbolIdentity, ExternalConstantId, ExternalFunctionId, ExternalPackageId,
    ExternalSymbolId, ExternalTypeId,
};
use super::{ExternalSymbolPath, ExternalSymbolPathError, InvalidExternalSymbolComponent};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::semantic_identity::StablePackageIdentity;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::return_compiler_error;
use rustc_hash::FxHashMap;

/// Atomic reverse-identity record for one registered external symbol.
///
/// WHAT: carries the build-local owning package handle plus the owned structured symbol path
/// for a single `ExternalTypeId`. The stable identity exposed to callers remains the package
/// path plus symbol path; this record never surfaces the build-local `ExternalPackageId`.
/// WHY: a single atomic record makes duplicate-ExternalTypeId rejection and clone consistency
/// one ownership decision instead of two parallel maps (`type_package_by_id` and
/// `type_symbol_path_by_id`) that could disagree if one insert succeeded and the other failed.
/// `resolve_type_package_and_symbol_path` reads through this record and the package table in O(1)
/// without scanning the package surface map.
#[derive(Debug, Clone)]
struct ExternalSymbolReverseIdentity {
    package_id: ExternalPackageId,
    symbol_path: ExternalSymbolPath,
}

/// Match between a dependency path and the longest registered external package prefix.
///
/// WHAT: records the package that matched plus how many dependency-path components belong to it.
/// WHY: dependency clauses, namespace bindings, and Stage 0 discovery all need the same
///      package-prefix rule before they decide how to handle any remaining symbol components.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ExternalPackagePathMatch {
    pub(crate) package_path: String,
    pub(crate) package_id: ExternalPackageId,
    pub(crate) matched_component_count: usize,
}

#[derive(Debug, Default)]
pub struct ExternalPackageRegistry {
    packages: FxHashMap<ExternalPackageId, ExternalPackage>,
    /// Path-to-ID index so package lookup by readable dependency path still works.
    package_id_by_path: FxHashMap<String, ExternalPackageId>,
    functions_by_id: FxHashMap<ExternalFunctionId, ExternalFunctionDef>,
    types_by_id: FxHashMap<ExternalTypeId, ExternalTypeDef>,
    constants_by_id: FxHashMap<ExternalConstantId, ExternalConstantDef>,
    /// Atomic reverse identities for functions, types and constants.
    function_reverse_identity_by_id: FxHashMap<ExternalFunctionId, ExternalSymbolReverseIdentity>,
    /// Atomic reverse-identity lookup: type ID -> owning package handle plus symbol path.
    ///
    /// WHAT: one record keyed by `ExternalTypeId` carrying the build-local package handle and
    /// the owned structured symbol path. The stable identity exposed to callers is derived from
    /// this record and the package table, never from the build-local IDs directly.
    /// WHY: `ExternalTypeDef` stores its package ID, but the canonical identity must not embed
    /// the build-local `ExternalPackageId` or `ExternalTypeId`. One atomic record keeps the
    /// registry as the single owner of type reverse identity, makes duplicate-ExternalTypeId
    /// rejection mutation-free and keeps clone consistency in one place.
    type_reverse_identity_by_id: FxHashMap<ExternalTypeId, ExternalSymbolReverseIdentity>,
    constant_reverse_identity_by_id: FxHashMap<ExternalConstantId, ExternalSymbolReverseIdentity>,
    /// Prelude symbols that are implicitly bound into every module.
    /// Bare-name lookup is only valid for the prelude.
    prelude_symbols_by_name: FxHashMap<&'static str, ExternalSymbolId>,
    /// Prelude namespace aliases that expose an external package surface under a
    /// bare name without an explicit dependency clause.
    ///
    /// WHAT: maps the local alias name to the target external package path (e.g.
    ///       `io` -> `@core/io`). The binding environment resolves collisions with
    ///       same-file declarations and explicit dependencies before injecting the record.
    /// WHY: keeps the prelude namespace model in the registry alongside the
    ///      existing prelude symbol model, so header preparation owns both.
    prelude_namespace_aliases_by_name: FxHashMap<&'static str, &'static str>,
    /// Counter for package IDs.
    next_package_id: u32,
    /// Counter for dynamically assigned synthetic IDs.
    next_synthetic_id: u32,
}

impl Clone for ExternalPackageRegistry {
    fn clone(&self) -> Self {
        increment_frontend_counter(FrontendCounter::ExternalPackageRegistryCloneCount);
        Self {
            packages: self.packages.clone(),
            package_id_by_path: self.package_id_by_path.clone(),
            functions_by_id: self.functions_by_id.clone(),
            types_by_id: self.types_by_id.clone(),
            constants_by_id: self.constants_by_id.clone(),
            function_reverse_identity_by_id: self.function_reverse_identity_by_id.clone(),
            type_reverse_identity_by_id: self.type_reverse_identity_by_id.clone(),
            constant_reverse_identity_by_id: self.constant_reverse_identity_by_id.clone(),
            prelude_symbols_by_name: self.prelude_symbols_by_name.clone(),
            prelude_namespace_aliases_by_name: self.prelude_namespace_aliases_by_name.clone(),
            next_package_id: self.next_package_id,
            next_synthetic_id: self.next_synthetic_id,
        }
    }
}

impl ExternalPackageRegistry {
    /// Builds the builtin external package registry used by normal frontend compilation.
    pub fn new() -> Self {
        super::build_builtin_registry()
    }

    /// Attaches test packages to this registry for integration-test coverage.
    pub fn with_test_packages_for_integration(mut self) -> Self {
        super::packages::test_packages::register_test_packages_for_integration(&mut self);
        self
    }

    // ------------------------------------------------------------------
    // Package registration
    // ------------------------------------------------------------------

    /// Registers a new virtual package in the registry, assigning a stable package ID.
    ///
    /// WHAT: creates the owned package identity and path-to-ID index entry.
    /// The registry always constructs `PackageMetadata::binding(origin)` so a
    /// `MothSource`-backed package is unrepresentable through this API.
    /// WHY: built-in packages and later dynamic provider results must flow through the
    /// same identity path.
    pub fn register_package(
        &mut self,
        path: impl Into<String>,
        origin: crate::builder_surface::PackageOrigin,
    ) -> Result<ExternalPackageId, CompilerError> {
        let path = path.into();
        let Some(path_without_introducer) = path.strip_prefix('@') else {
            return Err(CompilerError::compiler_error(format!(
                "Host package '{path}' must start with '@'."
            )));
        };
        if let Err(reason) = crate::builder_surface::package_metadata::validate_package_path(
            path_without_introducer,
            origin,
        ) {
            return Err(CompilerError::compiler_error(format!(
                "Host package '{path}' {reason}."
            )));
        }
        if self.package_id_by_path.contains_key(&path) {
            return_compiler_error!("Host package '{}' is already registered.", path);
        }
        let id = ExternalPackageId(self.next_package_id);
        self.next_package_id += 1;
        let metadata = crate::builder_surface::PackageMetadata::binding(origin);
        let package = ExternalPackage::new(id, path.clone(), metadata);
        self.packages.insert(id, package);
        self.package_id_by_path.insert(path, id);
        Ok(id)
    }

    // ------------------------------------------------------------------
    // Function registration
    // ------------------------------------------------------------------

    /// Registers an external function at a structured path within a specific package.
    ///
    /// WHAT: the canonical path-aware registration entry point. The definition is stored by ID,
    /// and the package surface map records `path -> id`.
    /// WHY: nested namespace symbols such as `io.input.new` need a full path identity, while
    /// one-component callers use `register_function_in_package` as a convenience wrapper.
    pub fn register_function_at_path(
        &mut self,
        package_id: ExternalPackageId,
        path: ExternalSymbolPath,
        id: ExternalFunctionId,
        mut function: ExternalFunctionDef,
    ) -> Result<(), CompilerError> {
        let package = self.packages.get(&package_id).ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "Cannot register function '{}' in unknown package '{:?}'.",
                path.display_text(),
                package_id
            ))
        })?;
        let package_path_str = package.path.clone();

        if let Some(op) = function.const_eval {
            let valid_registration = package.metadata.origin
                == crate::builder_surface::PackageOrigin::Core
                && function
                    .parameters
                    .iter()
                    .all(|parameter| parameter.access_kind == ExternalAccessKind::Shared)
                && function
                    .returns
                    .iter()
                    .all(|slot| matches!(&slot.alias, ExternalReturnAlias::Fresh))
                && function.error_return_type.is_none()
                && op.matches_signature(&function.parameters, &function.returns);
            if !valid_registration {
                return Err(CompilerError::compiler_error(format!(
                    "Invalid trusted external constant-evaluation registration for '{}' in package '{}': {:?} does not match its Core signature (compiler bug).",
                    path.display_text(),
                    package_path_str,
                    op
                )));
            }
        }

        if let Err(reason) = validate_signature_slots(&function) {
            return Err(CompilerError::compiler_error(format!(
                "External function '{}' in package '{}' {reason}.",
                path.display_text(),
                package_path_str
            )));
        }

        function.name = path.leaf().to_owned();

        self.reject_duplicate_path(
            &package_path_str,
            package_id,
            &path,
            path.leaf(),
            "host function",
        )?;

        if let Some(existing_identity) = self.function_reverse_identity_by_id.get(&id) {
            return Err(self.duplicate_symbol_id_error(
                "function",
                id,
                existing_identity,
                &package_path_str,
                &path,
            ));
        }

        let package = self
            .packages
            .get_mut(&package_id)
            .expect("package that was just looked up disappeared during function registration");
        package.function_ids.insert(path.clone(), id);
        self.functions_by_id.insert(id, function);
        self.function_reverse_identity_by_id.insert(
            id,
            ExternalSymbolReverseIdentity {
                package_id,
                symbol_path: path,
            },
        );
        Ok(())
    }

    /// Registers an external function within a specific package using its leaf name as a
    /// one-component path.
    ///
    /// WHAT: convenience wrapper for the common case where the package symbol has no namespace.
    /// WHY: keeps existing core package and provider registrations readable.
    pub fn register_function_in_package(
        &mut self,
        package_id: ExternalPackageId,
        id: ExternalFunctionId,
        function: ExternalFunctionDef,
    ) -> Result<(), CompilerError> {
        let path = one_component_symbol_path("host function", &function.name)?;
        self.register_function_at_path(package_id, path, id, function)
    }

    // ------------------------------------------------------------------
    // Type registration
    // ------------------------------------------------------------------

    /// Registers an external type at a structured path within a specific package.
    pub fn register_type_at_path(
        &mut self,
        package_id: ExternalPackageId,
        path: ExternalSymbolPath,
        id: ExternalTypeId,
        mut type_def: ExternalTypeDef,
    ) -> Result<(), CompilerError> {
        let package_path_str = self
            .packages
            .get(&package_id)
            .ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "Cannot register type '{}' in unknown package '{:?}'.",
                    path.display_text(),
                    package_id
                ))
            })?
            .path
            .clone();

        // Ensure the definition's leaf name matches the path leaf. This keeps the ID-indexed
        // definition consistent with the package surface map.
        type_def.name = path.leaf().to_owned();
        type_def.package_id = package_id;

        self.reject_duplicate_path(&package_path_str, package_id, &path, &type_def.name, "type")?;

        // Reject a duplicate ExternalTypeId before mutating any registry state. A type already
        // registered at one path must not be re-registered at another; the failure leaves the
        // original type definition, package surface and reverse identity untouched.
        if let Some(existing_identity) = self.type_reverse_identity_by_id.get(&id) {
            let existing_package_path = self
                .packages
                .get(&existing_identity.package_id)
                .ok_or_else(|| {
                    CompilerError::compiler_error(format!(
                        "External type id {id:?} has reverse identity for missing package {:?}.",
                        existing_identity.package_id
                    ))
                })?
                .path
                .as_str();
            return_compiler_error!(
                "External type id {:?} is already registered as '{}.{}'; it cannot be reused as \
                 '{}.{}'.",
                id,
                existing_package_path,
                existing_identity.symbol_path,
                package_path_str,
                path
            );
        }

        let package = self
            .packages
            .get_mut(&package_id)
            .expect("package that was just looked up disappeared during type registration");
        package.type_ids.insert(path.clone(), id);
        self.types_by_id.insert(id, type_def);
        self.type_reverse_identity_by_id.insert(
            id,
            ExternalSymbolReverseIdentity {
                package_id,
                symbol_path: path,
            },
        );
        Ok(())
    }

    /// Registers an external type within a specific package using its leaf name as a
    /// one-component path.
    pub fn register_type_in_package(
        &mut self,
        package_id: ExternalPackageId,
        id: ExternalTypeId,
        type_def: ExternalTypeDef,
    ) -> Result<(), CompilerError> {
        let path = one_component_symbol_path("type", &type_def.name)?;
        self.register_type_at_path(package_id, path, id, type_def)
    }

    // ------------------------------------------------------------------
    // Constant registration
    // ------------------------------------------------------------------

    /// Registers an external constant at a structured path within a specific package.
    pub fn register_constant_at_path(
        &mut self,
        package_id: ExternalPackageId,
        path: ExternalSymbolPath,
        id: ExternalConstantId,
        mut constant: ExternalConstantDef,
    ) -> Result<(), CompilerError> {
        let package_path_str = self
            .packages
            .get(&package_id)
            .ok_or_else(|| {
                CompilerError::compiler_error(format!(
                    "Cannot register constant '{}' in unknown package '{:?}'.",
                    path.display_text(),
                    package_id
                ))
            })?
            .path
            .clone();

        constant.name = path.leaf().to_owned();

        if !constant.value.matches_signature_type(&constant.data_type) {
            return_compiler_error!(
                "External constant '{}' declares semantic type {:?} incompatible with payload {:?}.",
                constant.name,
                constant.data_type,
                constant.value
            );
        }

        self.reject_duplicate_path(
            &package_path_str,
            package_id,
            &path,
            &constant.name,
            "constant",
        )?;

        if let Some(existing_identity) = self.constant_reverse_identity_by_id.get(&id) {
            return Err(self.duplicate_symbol_id_error(
                "constant",
                id,
                existing_identity,
                &package_path_str,
                &path,
            ));
        }

        let package = self
            .packages
            .get_mut(&package_id)
            .expect("package that was just looked up disappeared during constant registration");
        package.constant_ids.insert(path.clone(), id);
        self.constants_by_id.insert(id, constant);
        self.constant_reverse_identity_by_id.insert(
            id,
            ExternalSymbolReverseIdentity {
                package_id,
                symbol_path: path,
            },
        );
        Ok(())
    }

    /// Registers an external constant within a specific package using its leaf name as a
    /// one-component path.
    pub fn register_constant_in_package(
        &mut self,
        package_id: ExternalPackageId,
        id: ExternalConstantId,
        constant: ExternalConstantDef,
    ) -> Result<(), CompilerError> {
        let path = one_component_symbol_path("constant", &constant.name)?;
        self.register_constant_at_path(package_id, path, id, constant)
    }

    // ------------------------------------------------------------------
    // Cross-kind duplicate detection
    // ------------------------------------------------------------------

    /// Rejects a symbol path that is already occupied by any kind of symbol in the package.
    ///
    /// WHAT: a single namespace slot cannot hold a function, type, and constant with the same
    /// path, because namespace records would be ambiguous.
    /// WHY: pushing this check into the registry keeps the package surface consistent and lets
    /// tests and later phases rely on the invariant.
    fn reject_duplicate_path(
        &self,
        package_path: &str,
        package_id: ExternalPackageId,
        path: &ExternalSymbolPath,
        symbol_name: &str,
        kind: &str,
    ) -> Result<(), CompilerError> {
        if self.has_symbol_at_path(package_id, path) {
            return_compiler_error!(
                "The {} '{}' at path '{}' in package '{}' collides with another symbol at the same namespace slot.",
                kind,
                symbol_name,
                path.display_text(),
                package_path
            );
        }
        Ok(())
    }

    fn duplicate_symbol_id_error(
        &self,
        category: &str,
        id: impl std::fmt::Debug,
        existing_identity: &ExternalSymbolReverseIdentity,
        new_package_path: &str,
        new_symbol_path: &ExternalSymbolPath,
    ) -> CompilerError {
        let existing_package_path = self
            .packages
            .get(&existing_identity.package_id)
            .map(|package| package.path.as_str())
            .unwrap_or("<missing-package>");

        CompilerError::compiler_error(format!(
            "External {category} id {id:?} is already registered as \
             '{existing_package_path}.{}'; it cannot be reused as \
             '{new_package_path}.{new_symbol_path}'.",
            existing_identity.symbol_path,
        ))
    }

    /// Returns true if any symbol (function, type, or constant) is registered at the path.
    pub fn has_symbol_at_path(
        &self,
        package_id: ExternalPackageId,
        path: &ExternalSymbolPath,
    ) -> bool {
        self.packages.get(&package_id).is_some_and(|package| {
            package.function_ids.contains_key(path)
                || package.type_ids.contains_key(path)
                || package.constant_ids.contains_key(path)
        })
    }

    // ------------------------------------------------------------------
    // Prelude
    // ------------------------------------------------------------------

    /// Registers a prelude symbol that is implicitly bound into every module.
    // Kept covered by registry tests while the current builtin prelude only exposes namespace
    // aliases; the binding environment still owns both prelude-symbol and prelude-namespace paths.
    #[allow(dead_code)]
    pub(crate) fn register_prelude_symbol(
        &mut self,
        public_name: &'static str,
        symbol_id: ExternalSymbolId,
    ) -> Result<(), CompilerError> {
        if self.prelude_symbols_by_name.contains_key(public_name) {
            return_compiler_error!("Prelude symbol '{}' is already registered.", public_name);
        }
        if self
            .prelude_namespace_aliases_by_name
            .contains_key(public_name)
        {
            return_compiler_error!(
                "Prelude symbol '{}' collides with an existing prelude namespace alias.",
                public_name
            );
        }
        self.prelude_symbols_by_name.insert(public_name, symbol_id);
        Ok(())
    }

    /// Registers a prelude namespace alias that exposes an external package under a
    /// bare name without an explicit dependency clause.
    ///
    /// WHAT: adds `local_name` to every module's visible namespace records, backed by
    ///       the same recursive external package record as `@package`.
    /// WHY: keeps the registry as the single owner of prelude surface metadata.
    ///
    pub(crate) fn register_prelude_namespace_alias(
        &mut self,
        local_name: &'static str,
        package_path: &'static str,
    ) -> Result<(), CompilerError> {
        if self
            .prelude_namespace_aliases_by_name
            .contains_key(local_name)
        {
            return_compiler_error!(
                "Prelude namespace alias '{}' is already registered.",
                local_name
            );
        }
        if self.prelude_symbols_by_name.contains_key(local_name) {
            return_compiler_error!(
                "Prelude namespace alias '{}' collides with an existing prelude symbol.",
                local_name
            );
        }
        self.prelude_namespace_aliases_by_name
            .insert(local_name, package_path);
        Ok(())
    }

    // ------------------------------------------------------------------
    // Lookup by stable ID (always safe, no visibility involved)
    // ------------------------------------------------------------------

    /// Looks up an external function by its stable ID.
    pub fn get_function_by_id(&self, id: ExternalFunctionId) -> Option<&ExternalFunctionDef> {
        self.functions_by_id.get(&id)
    }

    /// Iterates registered functions as `(id, definition)` pairs.
    pub(crate) fn functions(
        &self,
    ) -> impl Iterator<Item = (ExternalFunctionId, &ExternalFunctionDef)> {
        self.functions_by_id
            .iter()
            .map(|(id, definition)| (*id, definition))
    }

    /// Looks up an external type by its stable ID.
    pub fn get_type_by_id(&self, id: ExternalTypeId) -> Option<&ExternalTypeDef> {
        self.types_by_id.get(&id)
    }

    /// Looks up an external constant by its stable ID.
    pub fn get_constant_by_id(&self, id: ExternalConstantId) -> Option<&ExternalConstantDef> {
        self.constants_by_id.get(&id)
    }

    // ------------------------------------------------------------------
    // Dynamic registration (alpha: assigns Synthetic IDs)
    // ------------------------------------------------------------------

    /// Registers an external function at a path, assigning the next available
    /// synthetic ID automatically.
    pub fn register_external_function_at_path(
        &mut self,
        package_id: ExternalPackageId,
        path: ExternalSymbolPath,
        spec: ExternalFunctionSpec,
    ) -> Result<ExternalFunctionId, CompilerError> {
        let id = ExternalFunctionId::Synthetic(self.next_synthetic_id);
        self.next_synthetic_id += 1;
        let mut function: ExternalFunctionDef = spec.into();
        function.name = path.leaf().to_owned();
        self.register_function_at_path(package_id, path, id, function)?;
        Ok(id)
    }

    /// Registers an external function in a package, assigning the next available
    /// synthetic ID automatically and using the spec name as a one-component path.
    ///
    /// WHAT: builder-friendly entry point that does not require hardcoding an
    /// `ExternalFunctionId` enum variant.
    /// WHY: alpha short-cut until the backend supports fully dynamic host imports.
    pub fn register_external_function(
        &mut self,
        package_id: ExternalPackageId,
        spec: ExternalFunctionSpec,
    ) -> Result<ExternalFunctionId, CompilerError> {
        let path = one_component_symbol_path("host function", &spec.name)?;
        self.register_external_function_at_path(package_id, path, spec)
    }

    /// Registers an external type at a path, assigning the next available
    /// dynamic ID automatically.
    pub fn register_external_type_at_path(
        &mut self,
        package_id: ExternalPackageId,
        path: ExternalSymbolPath,
        spec: ExternalTypeSpec,
    ) -> Result<ExternalTypeId, CompilerError> {
        let id = ExternalTypeId(self.next_synthetic_id);
        self.next_synthetic_id += 1;
        self.register_type_at_path(
            package_id,
            path,
            id,
            ExternalTypeDef {
                name: spec.name,
                package_id,
                abi_type: spec.abi_type,
            },
        )?;
        Ok(id)
    }

    /// Registers an external type in a package, assigning the next available
    /// dynamic ID automatically and using the spec name as a one-component path.
    pub fn register_external_type(
        &mut self,
        package_id: ExternalPackageId,
        spec: ExternalTypeSpec,
    ) -> Result<ExternalTypeId, CompilerError> {
        let path = one_component_symbol_path("type", &spec.name)?;
        self.register_external_type_at_path(package_id, path, spec)
    }

    /// Registers an external constant at a path, assigning the next available
    /// dynamic ID automatically.
    pub fn register_external_constant_at_path(
        &mut self,
        package_id: ExternalPackageId,
        path: ExternalSymbolPath,
        constant: ExternalConstantDef,
    ) -> Result<ExternalConstantId, CompilerError> {
        let id = ExternalConstantId(self.next_synthetic_id);
        self.next_synthetic_id += 1;
        self.register_constant_at_path(package_id, path, id, constant)?;
        Ok(id)
    }

    /// Registers an external constant in a package, assigning the next available
    /// dynamic ID automatically and using the constant name as a one-component path.
    pub fn register_external_constant(
        &mut self,
        package_id: ExternalPackageId,
        constant: ExternalConstantDef,
    ) -> Result<ExternalConstantId, CompilerError> {
        let path = one_component_symbol_path("constant", &constant.name)?;
        self.register_external_constant_at_path(package_id, path, constant)
    }

    // ------------------------------------------------------------------
    // Test-only registration
    // ------------------------------------------------------------------

    /// Registers a synthetic external function for test-only lowering and borrow-check scenarios.
    #[cfg(test)]
    pub fn register_function(
        &mut self,
        function: ExternalFunctionDef,
    ) -> Result<ExternalFunctionId, CompilerError> {
        let test_package_path = "@test/default";
        let package_id = match self.package_id_by_path.get(test_package_path).copied() {
            Some(id) => id,
            None => self.register_package(
                test_package_path,
                crate::builder_surface::PackageOrigin::Builder,
            )?,
        };

        let path = one_component_symbol_path("host function", &function.name)?;
        let id = ExternalFunctionId::Synthetic(self.next_synthetic_id);
        self.next_synthetic_id += 1;
        self.register_function_at_path(package_id, path, id, function)?;
        Ok(id)
    }

    // ------------------------------------------------------------------
    // Package-scoped resolution (used by dependency binding)
    // ------------------------------------------------------------------

    /// Resolves a package path to its stable ID.
    pub fn resolve_package_id(&self, path: &str) -> Option<ExternalPackageId> {
        self.package_id_by_path.get(path).copied()
    }

    /// Looks up a specific package by path.
    pub fn get_package(&self, path: &str) -> Option<&ExternalPackage> {
        let id = self.package_id_by_path.get(path)?;
        self.packages.get(id)
    }

    /// Looks up a specific package by its stable ID.
    pub fn get_package_by_id(&self, id: ExternalPackageId) -> Option<&ExternalPackage> {
        self.packages.get(&id)
    }

    /// Resolves any symbol (function, type, or constant) at a structured path within a
    /// specific package.
    pub fn resolve_package_symbol_by_path(
        &self,
        package_path: &str,
        path: &ExternalSymbolPath,
    ) -> Option<ExternalSymbolId> {
        self.resolve_package_function_by_path(package_path, path)
            .map(|(id, _)| ExternalSymbolId::Function(id))
            .or_else(|| {
                self.resolve_package_type_by_path(package_path, path)
                    .map(|(id, _)| ExternalSymbolId::Type(id))
            })
            .or_else(|| {
                self.resolve_package_constant_by_path(package_path, path)
                    .map(|(id, _)| ExternalSymbolId::Constant(id))
            })
    }

    /// Resolves a build-local symbol ID to its canonical binding identity.
    ///
    /// WHAT: returns the declaring package path and structured package-local symbol path.
    /// WHY: public source interfaces must not retain `ExternalSymbolId`, whose dynamically
    /// assigned variants are meaningful only inside one registry construction.
    pub(crate) fn canonical_symbol_identity(
        &self,
        symbol_id: ExternalSymbolId,
    ) -> Option<CanonicalBindingSymbolIdentity> {
        let reverse_identity = match symbol_id {
            ExternalSymbolId::Function(id) => self.function_reverse_identity_by_id.get(&id)?,
            ExternalSymbolId::Type(id) => self.type_reverse_identity_by_id.get(&id)?,
            ExternalSymbolId::Constant(id) => self.constant_reverse_identity_by_id.get(&id)?,
        };
        let package = self.packages.get(&reverse_identity.package_id)?;

        Some(CanonicalBindingSymbolIdentity {
            package: StablePackageIdentity::binding(package.metadata.origin, &package.path),
            symbol_path: reverse_identity.symbol_path.clone(),
            category: symbol_id.category(),
        })
    }

    /// Resolves a canonical binding identity through this registry's local IDs.
    pub(crate) fn resolve_canonical_symbol(
        &self,
        identity: &CanonicalBindingSymbolIdentity,
    ) -> Option<ExternalSymbolId> {
        let package = self.get_package(identity.package.name())?;
        if package.metadata.origin != identity.package.origin() {
            return None;
        }

        let symbol_id =
            self.resolve_package_symbol_by_path(identity.package.name(), &identity.symbol_path)?;
        (symbol_id.category() == identity.category).then_some(symbol_id)
    }

    /// Resolves any symbol at a one-component path within a specific package.
    pub fn resolve_package_symbol(
        &self,
        package_path: &str,
        symbol_name: &str,
    ) -> Option<ExternalSymbolId> {
        let path = ExternalSymbolPath::try_from_single(symbol_name).ok()?;
        self.resolve_package_symbol_by_path(package_path, &path)
    }

    /// Resolves a function symbol at a structured path within a specific package,
    /// returning its ID and definition.
    pub fn resolve_package_function_by_path(
        &self,
        package_path: &str,
        path: &ExternalSymbolPath,
    ) -> Option<(ExternalFunctionId, &ExternalFunctionDef)> {
        let package_id = self.resolve_package_id(package_path)?;
        let id = *self.packages.get(&package_id)?.function_ids.get(path)?;
        let def = self.functions_by_id.get(&id)?;
        Some((id, def))
    }

    /// Resolves a function symbol within a specific package, returning its ID and definition.
    pub fn resolve_package_function(
        &self,
        package_path: &str,
        symbol_name: &str,
    ) -> Option<(ExternalFunctionId, &ExternalFunctionDef)> {
        let path = ExternalSymbolPath::try_from_single(symbol_name).ok()?;
        self.resolve_package_function_by_path(package_path, &path)
    }

    /// Resolves a type symbol at a structured path within a specific package,
    /// returning its ID and definition.
    pub fn resolve_package_type_by_path(
        &self,
        package_path: &str,
        path: &ExternalSymbolPath,
    ) -> Option<(ExternalTypeId, &ExternalTypeDef)> {
        let package_id = self.resolve_package_id(package_path)?;
        let id = *self.packages.get(&package_id)?.type_ids.get(path)?;
        let def = self.types_by_id.get(&id)?;
        Some((id, def))
    }

    /// Resolves a type symbol within a specific package, returning its ID and definition.
    pub fn resolve_package_type(
        &self,
        package_path: &str,
        type_name: &str,
    ) -> Option<(ExternalTypeId, &ExternalTypeDef)> {
        let path = ExternalSymbolPath::try_from_single(type_name).ok()?;
        self.resolve_package_type_by_path(package_path, &path)
    }

    /// Resolves a constant symbol at a structured path within a specific package,
    /// returning its ID and definition.
    pub fn resolve_package_constant_by_path(
        &self,
        package_path: &str,
        path: &ExternalSymbolPath,
    ) -> Option<(ExternalConstantId, &ExternalConstantDef)> {
        let package_id = self.resolve_package_id(package_path)?;
        let id = *self.packages.get(&package_id)?.constant_ids.get(path)?;
        let def = self.constants_by_id.get(&id)?;
        Some((id, def))
    }

    /// Resolves a constant symbol within a specific package, returning its ID and definition.
    pub fn resolve_package_constant(
        &self,
        package_path: &str,
        constant_name: &str,
    ) -> Option<(ExternalConstantId, &ExternalConstantDef)> {
        let path = ExternalSymbolPath::try_from_single(constant_name).ok()?;
        self.resolve_package_constant_by_path(package_path, &path)
    }

    /// Returns true if the registry contains a package with the given path.
    pub fn has_package(&self, path: &str) -> bool {
        self.package_id_by_path.contains_key(path)
    }

    /// Iterate the registered package paths.
    ///
    /// Callers that require deterministic order must collect into an ordered set; the registry's
    /// hash index remains optimized for lookup and is the sole owner of package registration.
    pub(crate) fn package_paths(&self) -> impl Iterator<Item = &str> {
        self.package_id_by_path.keys().map(String::as_str)
    }

    /// Finds the longest registered external package prefix for a dependency path.
    ///
    /// WHAT: for a dependency such as `@core/math/sin`, checks `@core/math/sin`,
    ///      then `@core/math`, then `@core` and returns the first registered package.
    /// WHY: virtual packages share the same `@` syntax as source dependencies. Keeping the
    ///      longest-prefix rule here prevents Stage 0, namespace bindings, and direct-selection
    ///      dependencies from reimplementing subtly different package matching.
    pub(crate) fn longest_package_prefix_for_dependency(
        &self,
        import_path: PathId,
        path_fork: &PathInternerFork,
        string_table: &StringTable,
    ) -> Option<ExternalPackagePathMatch> {
        let mut components = Vec::new();
        let components = path_fork.resolve_components(import_path, &mut components);
        if components.is_empty() {
            return None;
        }

        for package_len in (1..=components.len()).rev() {
            let package_path =
                external_package_path_from_components(&components[..package_len], string_table);

            if let Some(package_id) = self.package_id_by_path.get(&package_path).copied() {
                return Some(ExternalPackagePathMatch {
                    package_path,
                    package_id,
                    matched_component_count: package_len,
                });
            }
        }

        None
    }

    /// Returns the package path that owns the given external function ID.
    ///
    /// WHAT: reverse lookup from stable function ID to its declaring package path.
    /// WHY: diagnostics need to name the package when a backend does not support a function.
    pub fn resolve_function_package(&self, id: ExternalFunctionId) -> Option<&str> {
        let package_id = &self.function_reverse_identity_by_id.get(&id)?.package_id;
        self.packages
            .get(package_id)
            .map(|package| package.path.as_str())
    }

    /// Returns the package-local symbol path for a registered external function.
    ///
    /// WHAT: reverse lookup from stable function ID to the structured symbol path used in the
    /// package surface map.
    /// WHY: diagnostics should name nested external functions as `input.new`, not only by their
    /// leaf name, while HIR and backends still carry stable function IDs rather than source syntax.
    pub fn resolve_function_symbol_path(
        &self,
        id: ExternalFunctionId,
    ) -> Option<&ExternalSymbolPath> {
        self.function_reverse_identity_by_id
            .get(&id)
            .map(|identity| &identity.symbol_path)
    }

    /// Returns the package ID that owns the given external function ID.
    ///
    /// WHAT: reverse lookup from stable function ID to its declaring package ID.
    /// WHY: backend glue generation needs the package ID to find runtime assets and to
    ///      construct deterministic wrapper names.
    pub fn resolve_function_package_id(&self, id: ExternalFunctionId) -> Option<ExternalPackageId> {
        self.function_reverse_identity_by_id
            .get(&id)
            .map(|identity| identity.package_id)
    }

    /// Returns the owned stable external type identity for a build-local `ExternalTypeId`.
    ///
    /// WHAT: O(1) reverse lookup from a stable type ID to the owning stable package identity and
    /// structured package-local symbol path. The caller can construct an owned canonical identity
    /// without embedding `ExternalPackageId` or `ExternalTypeId`.
    /// WHY: package path alone cannot distinguish identically named packages from different
    /// origins. A missing entry means the type was never registered through the single
    /// registration path, which is an inconsistent-registry invariant, not a user failure.
    pub(crate) fn resolve_type_package_and_symbol_path(
        &self,
        id: ExternalTypeId,
    ) -> Option<(StablePackageIdentity, &ExternalSymbolPath)> {
        let reverse_identity = self.type_reverse_identity_by_id.get(&id)?;
        let package = self.packages.get(&reverse_identity.package_id)?;
        Some((
            StablePackageIdentity::binding(package.metadata.origin, &package.path),
            &reverse_identity.symbol_path,
        ))
    }

    /// Resolves an opaque type through its exact stable package identity.
    ///
    /// The origin check prevents an independently constructed registry from accepting a
    /// same-spelling package supplied by a different owner.
    pub(crate) fn resolve_canonical_package_type_by_path(
        &self,
        package_identity: &StablePackageIdentity,
        path: &ExternalSymbolPath,
    ) -> Option<(ExternalTypeId, &ExternalTypeDef)> {
        let package = self.get_package(package_identity.name())?;
        if package.metadata.origin != package_identity.origin() {
            return None;
        }

        self.resolve_package_type_by_path(package_identity.name(), path)
    }

    /// Checks whether a dependency path should be treated as a virtual package dependency
    /// rather than a filesystem dependency.
    ///
    /// WHAT: tries progressively shorter prefixes of the dependency path against known packages.
    pub fn is_virtual_package_dependency(
        &self,
        import_path: PathId,
        path_fork: &PathInternerFork,
        string_table: &StringTable,
    ) -> bool {
        self.longest_package_prefix_for_dependency(import_path, path_fork, string_table)
            .is_some()
    }

    /// Returns a known optional package path when a dependency targets a package this builder
    /// did not expose.
    ///
    /// WHAT: recognizes compiler-known optional core package prefixes before path resolution falls
    /// back to filesystem dependencies.
    /// WHY: `@core/text` missing from a builder is a package-surface error, not a confusing
    /// missing source file.
    pub fn unsupported_known_package_dependency(
        &self,
        import_path: PathId,
        path_fork: &PathInternerFork,
        string_table: &StringTable,
    ) -> Option<&'static str> {
        let mut components = Vec::new();
        let components = path_fork.resolve_components(import_path, &mut components);
        for package_path in crate::builder_surface::core_packages::OPTIONAL_CORE_PACKAGE_PATHS {
            if self.has_package(package_path) {
                continue;
            }

            let package_components = package_path
                .strip_prefix('@')
                .unwrap_or(package_path)
                .split('/')
                .collect::<Vec<_>>();
            if components.len() < package_components.len() {
                continue;
            }

            let matches_prefix = package_components
                .iter()
                .enumerate()
                .all(|(index, expected)| string_table.resolve(components[index]) == *expected);
            if matches_prefix {
                return Some(*package_path);
            }
        }

        None
    }

    // ------------------------------------------------------------------
    // Prelude
    // ------------------------------------------------------------------

    /// Returns the prelude symbol map.
    /// Bare-name lookup is only valid for the prelude.
    pub fn prelude_symbols_by_name(&self) -> &FxHashMap<&'static str, ExternalSymbolId> {
        &self.prelude_symbols_by_name
    }

    /// Returns the prelude namespace alias map.
    pub fn prelude_namespace_aliases_by_name(&self) -> &FxHashMap<&'static str, &'static str> {
        &self.prelude_namespace_aliases_by_name
    }

    /// Returns true if the given name is a prelude function.
    pub fn is_prelude_function(&self, name: &str) -> bool {
        self.prelude_symbols_by_name
            .get(name)
            .is_some_and(|symbol_id| matches!(symbol_id, ExternalSymbolId::Function(_)))
    }

    /// Returns true if the given name is a prelude type.
    pub fn is_prelude_type(&self, name: &str) -> bool {
        self.prelude_symbols_by_name
            .get(name)
            .is_some_and(|symbol_id| matches!(symbol_id, ExternalSymbolId::Type(_)))
    }
}

fn external_package_path_from_components(
    components: &[StringId],
    string_table: &StringTable,
) -> String {
    let mut package_path = String::from("@");

    for (index, component) in components.iter().enumerate() {
        if index > 0 {
            package_path.push('/');
        }
        package_path.push_str(string_table.resolve(*component));
    }

    package_path
}

fn one_component_symbol_path(kind: &str, name: &str) -> Result<ExternalSymbolPath, CompilerError> {
    ExternalSymbolPath::try_from_single(name)
        .map_err(|error| invalid_external_symbol_path_error(kind, name, error))
}

fn invalid_external_symbol_path_error(
    kind: &str,
    name: &str,
    error: ExternalSymbolPathError,
) -> CompilerError {
    let reason = match error {
        ExternalSymbolPathError::EmptyPath => "must have at least one component",
        ExternalSymbolPathError::InvalidComponent { reason, .. } => match reason {
            InvalidExternalSymbolComponent::Empty => "must not be empty",
            InvalidExternalSymbolComponent::ContainsPathSeparator => "must not contain '/'",
            InvalidExternalSymbolComponent::ContainsNamespaceSeparator => "must not contain '.'",
        },
    };
    CompilerError::compiler_error(format!("Invalid {kind} name '{name}': {reason}."))
}

/// Rejects signature slots that cannot carry a Moth value.
///
/// WHAT: every parameter and success slot names a Moth value, and every alias names an existing
/// parameter.
/// WHY: later projections skip slots without a `TypeId`, which would misalign ordered success
/// slots with their aliases and skip parameter compatibility checks.
fn validate_signature_slots(function: &ExternalFunctionDef) -> Result<(), &'static str> {
    if function
        .parameters
        .iter()
        .any(|parameter| !parameter.language_type.represents_moth_value())
    {
        return Err("declares a parameter without a Moth value");
    }
    for slot in &function.returns {
        if !slot.value_type.represents_moth_value() {
            return Err("declares a success slot without a Moth value");
        }
        if let ExternalReturnAlias::AliasArgs(indices) = &slot.alias
            && indices
                .iter()
                .any(|&index| index >= function.parameters.len())
        {
            return Err("aliases a parameter it does not declare");
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::super::abi::{
        ExternalAbiType, ExternalAccessKind, ExternalParameter, ExternalReturnAlias,
        ExternalSignatureType,
    };
    use super::super::definitions::{
        ExternalFunctionDef, ExternalFunctionLowerings, external_success_returns,
    };
    use super::super::ids::ExternalFunctionId;
    use super::CompilerError;
    use super::ExternalPackageRegistry;
    use moth_lexical::numeric::fixed_scalar::FixedScalar;

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub enum TestExternalReturnType {
        Fixed(FixedScalar),
        Void,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    pub enum TestExternalAccessKind {
        Shared,
        Mutable,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Hash)]
    pub enum TestExternalReturnAlias {
        Fresh,
        AliasArgs(Vec<usize>),
    }

    impl From<TestExternalReturnType> for ExternalSignatureType {
        fn from(value: TestExternalReturnType) -> Self {
            match value {
                TestExternalReturnType::Fixed(scalar) => {
                    ExternalSignatureType::Abi(ExternalAbiType::Fixed(scalar))
                }
                TestExternalReturnType::Void => ExternalSignatureType::Abi(ExternalAbiType::Void),
            }
        }
    }

    impl From<TestExternalAccessKind> for ExternalAccessKind {
        fn from(value: TestExternalAccessKind) -> Self {
            match value {
                TestExternalAccessKind::Shared => ExternalAccessKind::Shared,
                TestExternalAccessKind::Mutable => ExternalAccessKind::Mutable,
            }
        }
    }

    impl From<TestExternalReturnAlias> for ExternalReturnAlias {
        fn from(value: TestExternalReturnAlias) -> Self {
            match value {
                TestExternalReturnAlias::Fresh => ExternalReturnAlias::Fresh,
                TestExternalReturnAlias::AliasArgs(indices) => {
                    ExternalReturnAlias::AliasArgs(indices)
                }
            }
        }
    }

    /// Registers a synthetic external function using test-local metadata wrappers.
    pub fn register_test_external_function(
        registry: &mut ExternalPackageRegistry,
        name: impl Into<String>,
        parameters: Vec<(ExternalSignatureType, TestExternalAccessKind)>,
        return_alias: TestExternalReturnAlias,
        return_type: TestExternalReturnType,
    ) -> Result<ExternalFunctionId, CompilerError> {
        registry.register_function(ExternalFunctionDef {
            name: name.into(),
            parameters: parameters
                .into_iter()
                .map(|(language_type, access_kind)| ExternalParameter {
                    language_type,
                    access_kind: access_kind.into(),
                })
                .collect(),
            returns: external_success_returns(return_type, return_alias.into()),
            error_return_type: None,
            const_eval: None,
            lowerings: ExternalFunctionLowerings::default(),
        })
    }
}
