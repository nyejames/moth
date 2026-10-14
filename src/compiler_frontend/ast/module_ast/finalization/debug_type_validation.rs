//! Debug-only canonical type checks at the AST/HIR boundary.
//!
//! Executable roots use the required validator so debug and release readers consume the same
//! owner-bound graph and completed facts. Choice definitions remain a separate nominal root.

use super::validate_types::validate_debug_roots;
use crate::compiler_frontend::ast::AstChoiceDefinition;
use crate::compiler_frontend::ast::ast_nodes::AstNode;
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::expressions::expression_store::AstExpressionBuilder;
use crate::compiler_frontend::ast::templates::tir::TemplateIrStore;
use crate::compiler_frontend::datatypes::definitions::{
    ChoiceVariantPayloadDefinition, TypeDefinition,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;

pub(super) fn debug_validate_type_ids_for_hir(
    nodes: &[AstNode],
    const_values: &ConstValueStore,
    choice_definitions: &[AstChoiceDefinition],
    type_environment: &TypeEnvironment,
    template_ir_store: &TemplateIrStore,
    expression_builder: &AstExpressionBuilder,
) {
    let result = validate_debug_roots(
        nodes,
        const_values,
        type_environment,
        template_ir_store,
        expression_builder,
    );
    debug_assert!(
        result.is_ok(),
        "AST boundary validation failed: {:?}",
        result.err()
    );
    // Choice definitions are collected separately from the AST node tree.
    // Validate that their variant payload fields also carry resolved TypeIds.
    for choice_definition in choice_definitions {
        let Some(nominal_id) =
            type_environment.nominal_id_for_path(&choice_definition.nominal_path)
        else {
            debug_assert!(
                false,
                "AST choice definition path is missing from TypeEnvironment"
            );
            continue;
        };

        let Some(type_id) = type_environment.type_id_for_nominal_id(nominal_id) else {
            debug_assert!(
                false,
                "AST choice definition nominal id is missing a canonical TypeId"
            );
            continue;
        };

        let Some(variants) = type_environment.variants_for(type_id) else {
            debug_assert!(
                false,
                "AST choice definition TypeId is missing variant metadata"
            );
            continue;
        };

        for variant in variants {
            if let ChoiceVariantPayloadDefinition::Record { fields } = &variant.payload {
                for field in fields {
                    debug_validate_type_id(field.type_id, type_environment, "choice field");
                }
            }
        }
    }
}

fn debug_validate_type_id(type_id: TypeId, type_environment: &TypeEnvironment, owner: &str) {
    let definition = type_environment.get(type_id);

    debug_assert!(
        definition.is_some(),
        "AST {owner} carried orphan TypeId({}) not registered in the final TypeEnvironment",
        type_id.0,
    );

    // Generic parameters must be fully resolved to concrete types before HIR.
    // Carrying an unresolved generic parameter across the boundary indicates a
    // frontend type-resolution bug.
    debug_assert!(
        !matches!(definition, Some(TypeDefinition::GenericParameter(..))),
        "AST {owner} carried unresolved generic parameter TypeId({}) to the HIR boundary",
        type_id.0,
    );
}
