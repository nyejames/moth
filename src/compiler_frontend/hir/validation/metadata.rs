//! Module-level HIR metadata validation.
//!
//! WHAT: checks folded module constant payloads after lowering.
//! WHY: these values are consumed by builders outside the executable CFG, so they need explicit
//! validation instead of relying on statement or expression walks.
//!
//! Documentation-metadata validation moved to the module compilation boundary
//! (`HirLoweringMetadata::validate`) because documentation fragments are not executable HIR state.

use super::HirValidator;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::hir::constants::HirConstValue;

impl<'a> HirValidator<'a> {
    // -------------------------
    //  Metadata Validation
    // -------------------------

    pub(super) fn validate_module_constants(&self) -> Result<(), CompilerError> {
        for module_constant in &self.module.module_constants {
            if module_constant.name.trim().is_empty() {
                return Err(self.error_with_hir(
                    format!(
                        "Module constant {:?} has an empty constant name",
                        module_constant.id
                    ),
                    None,
                ));
            }

            self.require_type_id(module_constant.ty, None)?;
            if let HirConstValue::Number(value) = &module_constant.value
                && self.type_environment.number_scale(module_constant.ty) != Some(value.scale())
            {
                return Err(self.error_with_hir(
                    format!(
                        "HIR Dec constant scale {} does not match module constant type {:?}",
                        value.scale(),
                        module_constant.ty
                    ),
                    None,
                ));
            }
            self.validate_module_const_value(&module_constant.value)?;
        }

        Ok(())
    }

    pub(super) fn validate_module_const_value(
        &self,
        value: &HirConstValue,
    ) -> Result<(), CompilerError> {
        match value {
            HirConstValue::Collection(values) => {
                for value in values {
                    self.validate_module_const_value(value)?;
                }
            }
            HirConstValue::Record(fields) => {
                for field in fields {
                    if field.name.trim().is_empty() {
                        return Err(self.error_with_hir(
                            "Module constant record contains an empty field name",
                            None,
                        ));
                    }
                    self.validate_module_const_value(&field.value)?;
                }
            }
            HirConstValue::Range(start, end) => {
                self.validate_module_const_value(start)?;
                self.validate_module_const_value(end)?;
            }
            HirConstValue::OptionSome(value) => {
                self.validate_module_const_value(value)?;
            }
            HirConstValue::OptionNone => {}
            HirConstValue::Choice { fields, .. } => {
                for field in fields {
                    if field.name.trim().is_empty() {
                        return Err(self.error_with_hir(
                            "Module constant choice contains an empty field name",
                            None,
                        ));
                    }
                    self.validate_module_const_value(&field.value)?;
                }
            }
            // A structural string is a string-typed constant: `Text` and `Resource` handles
            // and the site-root mark carry nothing this validation inspects (no empty-name,
            // type or location check reads final characters), so no check needs the resolved
            // text and pieces must not be flattened to provide one.
            HirConstValue::Uint(_)
            | HirConstValue::Int(_)
            | HirConstValue::Float(_)
            | HirConstValue::FixedScalar(_)
            | HirConstValue::Number(_)
            | HirConstValue::Bool(_)
            | HirConstValue::Char(_)
            | HirConstValue::String(_)
            | HirConstValue::StructuralString { .. } => {}
        }

        Ok(())
    }
}
