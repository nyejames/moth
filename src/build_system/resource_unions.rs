//! Build-owned exact resource-origin unions.
//!
//! WHAT: owns the ordered, duplicate-free stable-origin set used by entry and package planning.
//! WHY: resource uses remain on their existing executable and metadata owners; this module only
//! materialises the live semantic-origin union after those owners have selected their uses.
//!
//! The helpers in this module read retained link facts and owned folded values. They never inspect
//! HIR directly, resolve paths, read bytes or assign output URLs. A generated sidecar is passed as
//! its own [`Module`], so its module-local `ResourceId` values are always resolved through the
//! sidecar's paired resource table.

use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::folded_value::{
    OwnedFoldedString, OwnedFoldedStringPiece, PublicConstTemplate, PublicConstTemplatePiece,
    PublicConstTemplateSlot, PublicFoldedField, PublicFoldedValue,
};
use crate::compiler_frontend::hir::reachability::HirReachability;
use crate::compiler_frontend::module_compilation::Module;
use crate::compiler_frontend::paths::resource_identity::StableResourceOriginId;
use crate::compiler_frontend::public_interface::{
    PublicConstantSemantics, PublicDeclarationRecord, PublicDeclarationSemantics,
    PublicSemanticInterface,
};
use rustc_hash::FxHashSet;

/// Ordered, duplicate-free live resource origins for one build target.
///
/// Origins are retained in first-seen order. The set stores no use locations or owner metadata:
/// executable uses stay on `HirReachability`, while compile-time and exported-value uses stay on
/// their metadata/public-value owners.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ResourceOriginUnion {
    origins: Vec<StableResourceOriginId>,
    seen: FxHashSet<StableResourceOriginId>,
}

impl ResourceOriginUnion {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Add one live origin, returning whether this was its first observation.
    pub(crate) fn insert(&mut self, origin: StableResourceOriginId) -> bool {
        if !self.seen.insert(origin.clone()) {
            return false;
        }
        self.origins.push(origin);
        true
    }

    #[cfg(test)]
    pub(crate) fn origins(&self) -> &[StableResourceOriginId] {
        &self.origins
    }

    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.origins.is_empty()
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &StableResourceOriginId> {
        self.origins.iter()
    }
}

/// Append executable resource uses selected by one module reachability result.
///
/// The supplied module is the exact owner of the supplied reachability facts. This is deliberately
/// a fallible operation: crossing a module or generated-sidecar resource-table boundary is
/// compiler corruption and must not silently drop a resource.
pub(crate) fn append_reachable_resource_uses(
    union: &mut ResourceOriginUnion,
    module: &Module,
    reachability: &HirReachability,
) -> Result<(), CompilerError> {
    for resource_use in &reachability.reachable_resource_uses {
        let origin = module
            .executable
            .resource_table
            .try_origin(resource_use.resource_id)?
            .origin
            .clone();
        union.insert(origin);
    }
    Ok(())
}

/// Append non-HIR resource pieces owned by one module's compile-time fragments.
pub(crate) fn append_const_fragment_resources(union: &mut ResourceOriginUnion, module: &Module) {
    for fragment in &module.metadata.const_top_level_fragments {
        append_owned_folded_string(union, &fragment.value);
    }
}

/// Append one module's exact entry-owned resource union in liveness order.
///
/// Reachable executable uses are observed before compile-time fragment metadata, matching the
/// entry-planning contract and preserving first-seen order across both owners.
pub(crate) fn append_entry_module_resources(
    union: &mut ResourceOriginUnion,
    module: &Module,
    reachability: &HirReachability,
) -> Result<(), CompilerError> {
    append_reachable_resource_uses(union, module, reachability)?;
    append_const_fragment_resources(union, module);
    Ok(())
}

/// Append every resource-bearing folded value selected by one public interface's exports.
///
/// The interface's `declarations` vector is a closed semantic surface and may contain provider
/// records that are not themselves selected by this interface. Walking `export_bindings` first is
/// therefore essential: private/unselected closed records must not enter a package union.
/// Missing declarations indicate compiler-owned interface corruption and return `CompilerError`.
pub(crate) fn append_exported_interface_resources(
    union: &mut ResourceOriginUnion,
    interface: &PublicSemanticInterface,
) -> Result<(), CompilerError> {
    for binding in &interface.export_bindings {
        let origin = binding.origin();
        let declaration = interface.declaration(origin).ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "export binding {origin:?} has no matching public declaration"
            ))
        })?;
        append_exported_declaration_resources(union, declaration);
    }

    Ok(())
}

fn append_exported_declaration_resources(
    union: &mut ResourceOriginUnion,
    declaration: &PublicDeclarationRecord,
) {
    match &declaration.semantics {
        PublicDeclarationSemantics::Function(function) => {
            for parameter in &function.parameters {
                if let Some(default) = &parameter.folded_default {
                    append_public_folded_value(union, default);
                }
            }
        }
        PublicDeclarationSemantics::Struct(structure) => {
            for field in &structure.fields {
                if let Some(default) = &field.folded_default {
                    append_public_folded_value(union, default);
                }
            }
            for method in &structure.receiver_methods {
                for parameter in &method.parameters {
                    if let Some(default) = &parameter.folded_default {
                        append_public_folded_value(union, default);
                    }
                }
            }
        }
        PublicDeclarationSemantics::Choice(choice) => {
            for variant in &choice.variants {
                for field in &variant.payload_fields {
                    if let Some(default) = &field.folded_default {
                        append_public_folded_value(union, default);
                    }
                }
            }
            for method in &choice.receiver_methods {
                for parameter in &method.parameters {
                    if let Some(default) = &parameter.folded_default {
                        append_public_folded_value(union, default);
                    }
                }
            }
        }
        PublicDeclarationSemantics::Constant(PublicConstantSemantics { folded_value, .. }) => {
            append_public_folded_value(union, folded_value);
        }
        PublicDeclarationSemantics::TransparentAlias(_) | PublicDeclarationSemantics::Trait(_) => {}
    }
}

/// Append all resource origins nested in one owned folded value.
pub(crate) fn append_public_folded_value(
    union: &mut ResourceOriginUnion,
    value: &PublicFoldedValue,
) {
    match value {
        PublicFoldedValue::String(string) => append_owned_folded_string(union, string),
        PublicFoldedValue::ConstTemplate(template) => append_public_const_template(union, template),
        PublicFoldedValue::Collection(values) => {
            for value in values {
                append_public_folded_value(union, value);
            }
        }
        PublicFoldedValue::Record(fields) => {
            for PublicFoldedField { value, .. } in fields {
                append_public_folded_value(union, value);
            }
        }
        PublicFoldedValue::Choice { fields, .. } => {
            for PublicFoldedField { value, .. } in fields {
                append_public_folded_value(union, value);
            }
        }
        PublicFoldedValue::Range { start, end } => {
            append_public_folded_value(union, start);
            append_public_folded_value(union, end);
        }
        PublicFoldedValue::OptionSome(value) => append_public_folded_value(union, value),
        PublicFoldedValue::Int(_)
        | PublicFoldedValue::Float(_)
        | PublicFoldedValue::Bool(_)
        | PublicFoldedValue::Char(_)
        | PublicFoldedValue::OptionNone => {}
    }
}

fn append_owned_folded_string(union: &mut ResourceOriginUnion, string: &OwnedFoldedString) {
    let OwnedFoldedString::Pieces(pieces) = string else {
        return;
    };
    for piece in pieces {
        if let OwnedFoldedStringPiece::Resource(origin) = piece {
            union.insert(origin.clone());
        }
    }
}

fn append_public_const_template(union: &mut ResourceOriginUnion, template: &PublicConstTemplate) {
    for piece in &template.pieces {
        match piece {
            PublicConstTemplatePiece::Text(string) => append_owned_folded_string(union, string),
            PublicConstTemplatePiece::Slot(slot) => append_public_const_template_slot(union, slot),
        }
    }
    for wrapper in &template.conditional_child_wrappers {
        append_public_const_template(union, wrapper);
    }
}

fn append_public_const_template_slot(
    union: &mut ResourceOriginUnion,
    slot: &PublicConstTemplateSlot,
) {
    for wrapper in &slot.applied_child_wrappers {
        append_public_const_template(union, wrapper);
    }
    for wrapper in &slot.child_wrappers {
        append_public_const_template(union, wrapper);
    }
}

#[cfg(test)]
#[path = "tests/resource_union_tests.rs"]
mod tests;
