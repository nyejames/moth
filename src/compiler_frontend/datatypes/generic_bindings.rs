//! TypeId-native generic parameter binding helper.
//!
//! WHAT: records inferred `GenericParameterId -> TypeId` substitutions.
//! WHY: constructor inference and generic function inference must use canonical
//!      semantic type identity rather than parse-local `TypeParameterId`s or `DataType`.

use super::environment::TypeEnvironment;
use super::ids::{GenericParameterId, GenericParameterListId, TypeId};
use rustc_hash::FxHashMap;

// -----------------------------------------------------------
//  Binding Conflicts
// -----------------------------------------------------------

/// Conflict produced when one generic parameter is inferred as two different concrete types.
///
/// WHAT: records the canonical parameter and TypeIds involved in the conflict.
/// WHY: generic inference should operate on semantic `TypeId`s and leave display
///      rendering to diagnostic boundaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BindingConflict {
    pub(crate) parameter_id: GenericParameterId,
    pub(crate) existing_type_id: TypeId,
    pub(crate) replacement_type_id: TypeId,
}

// -----------------------------------------------------------
//  Generic Type Bindings
// -----------------------------------------------------------

/// TypeId-native generic parameter bindings.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct GenericTypeBindings {
    replacements: FxHashMap<GenericParameterId, TypeId>,
}

impl GenericTypeBindings {
    pub(crate) fn new() -> Self {
        Self {
            replacements: FxHashMap::default(),
        }
    }

    pub(crate) fn insert_consistent(
        &mut self,
        parameter_id: GenericParameterId,
        concrete_type_id: TypeId,
    ) -> Result<(), BindingConflict> {
        if let Some(existing_type_id) = self.replacements.get(&parameter_id).copied() {
            if existing_type_id == concrete_type_id {
                return Ok(());
            }

            return Err(BindingConflict {
                parameter_id,
                existing_type_id,
                replacement_type_id: concrete_type_id,
            });
        }

        self.replacements.insert(parameter_id, concrete_type_id);
        Ok(())
    }

    pub(crate) fn get(&self, parameter_id: GenericParameterId) -> Option<TypeId> {
        self.replacements.get(&parameter_id).copied()
    }

    pub(crate) fn concrete_arguments_for(
        &self,
        parameter_list_id: GenericParameterListId,
        type_environment: &TypeEnvironment,
    ) -> Option<Box<[TypeId]>> {
        let parameter_list = type_environment.generic_parameters(parameter_list_id)?;
        parameter_list
            .parameters
            .iter()
            .map(|parameter| self.get(parameter.id))
            .collect::<Option<Vec<_>>>()
            .map(Vec::into_boxed_slice)
    }
}
