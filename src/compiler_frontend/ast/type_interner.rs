//! Restricted AST body-emission access to the canonical type environment.
//!
//! WHAT: exposes only the type interning and read-only queries needed while parsing function,
//! start, and template bodies.
//! WHY: nominal declaration registration belongs to AST environment construction. Body emission
//! may create derived types such as collections, results, functions, external handles, and
//! generic nominal instances, but it must not patch declaration-time nominal definitions.

// AstTypeInterner exposes a narrow API over TypeEnvironment for body-emission
// derived-type interning. Methods that are not used in current production paths
// are removed; they can be re-added when a genuine caller exists.

use crate::compiler_frontend::datatypes::environment::{BuiltinTypes, TypeEnvironment};
use crate::compiler_frontend::datatypes::ids::{NominalTypeId, TypeId};
use crate::compiler_frontend::type_coercion::compatibility::TypeCompatibilityCache;

/// Narrow façade over the module-local canonical `TypeEnvironment`.
pub(crate) struct AstTypeInterner<'types> {
    type_environment: &'types mut TypeEnvironment,
    compatibility_cache: &'types mut TypeCompatibilityCache,
}

/// Borrowed type-checking state for AST body-emission validation.
///
/// WHAT: call validation needs read-only type facts plus a mutable cache of pure
/// compatibility results.
/// WHY: keeping the cache beside the body-emission type façade scopes reuse to
/// one module compilation and prevents cross-module `TypeId` leakage.
pub(crate) struct AstTypeCheckContext<'a> {
    pub(crate) type_environment: &'a TypeEnvironment,
    pub(crate) compatibility_cache: &'a mut TypeCompatibilityCache,
}

impl<'types> AstTypeInterner<'types> {
    pub(crate) fn new(
        type_environment: &'types mut TypeEnvironment,
        compatibility_cache: &'types mut TypeCompatibilityCache,
    ) -> Self {
        Self {
            type_environment,
            compatibility_cache,
        }
    }

    pub(crate) fn environment(&self) -> &TypeEnvironment {
        self.type_environment
    }

    pub(crate) fn type_check_context(&mut self) -> AstTypeCheckContext<'_> {
        AstTypeCheckContext {
            type_environment: &*self.type_environment,
            compatibility_cache: &mut *self.compatibility_cache,
        }
    }

    /// Mutable access to the underlying `TypeEnvironment`.
    ///
    /// WHAT: allows body-emission code to intern derived types (collections, results,
    ///       functions, external handles, generic instances).
    /// WHY: body emission may create new type shapes that were not known at declaration
    ///      time, but it must never register or patch nominal declarations.
    pub(crate) fn environment_mut_for_derived_types(&mut self) -> &mut TypeEnvironment {
        self.type_environment
    }

    pub(crate) fn builtins(&self) -> &BuiltinTypes {
        self.type_environment.builtins()
    }

    pub(crate) fn intern_generic_instance(
        &mut self,
        base: NominalTypeId,
        arguments: Box<[TypeId]>,
    ) -> TypeId {
        self.type_environment
            .intern_generic_instance(base, arguments)
    }
}
