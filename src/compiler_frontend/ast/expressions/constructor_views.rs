//! AST-owned constructor parameter semantic views.
//!
//! WHAT: replaces fake `Declaration` values used for struct and choice constructor
//! validation with lightweight views that carry canonical `TypeId`s and optional
//! default expressions, and names the one parse result struct and choice
//! constructors share.
//! WHY: semantic `FieldDefinition` and `ChoiceVariantDefinition` carry resolved types
//! but not defaults; constructor validation needs both without reconstructing
//! synthetic AST declarations with placeholder types.

use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::expressions::expression_rpn::ExpressionRpnItem;
use crate::compiler_frontend::datatypes::definitions::FieldDefinition;
use crate::compiler_frontend::datatypes::ids::TypeId;
#[cfg(feature = "benchmark_counters")]
use crate::compiler_frontend::instrumentation::{
    FrontendCounter, add_frontend_counter, increment_frontend_counter,
};
use crate::compiler_frontend::symbols::path_interner::PathId;

/// One parsed nominal constructor.
///
/// WHAT: a constructor that finished at its authored position, or one retained call-shaped
///       operand whose numeric-dependent arguments resume at the body root.
/// WHY: struct and choice constructors share the same retained-argument lane, so one result type
///      keeps the identifier dispatch's two constructor arms identical and stops either surface
///      from fabricating a value for a constructor whose arguments have no selected type yet.
pub(crate) enum ParsedConstructorExpression {
    /// The constructor completed at its authored position.
    Value(Expression),
    /// At least one argument was retained; the constructor resumes exactly once at the body root.
    RetainedOperand(ExpressionRpnItem),
}

/// Lightweight view of one constructor parameter for call validation.
///
/// WHAT: carries everything `resolve_call_arguments` needs to build
/// `ParameterExpectation` for struct and choice payload constructors.
/// WHY: avoids creating fake `Declaration` values with placeholder types and
/// `ExpressionKind::NoValue` just to feed the shared call-validation pipeline.
#[derive(Debug, Clone)]
pub(crate) struct ConstructorField {
    pub name: PathId,
    pub type_id: TypeId,
    pub access_mode: ConstructorFieldAccessMode,
    pub default_value: Option<Expression>,
}

/// Access mode expected when initializing a constructor field.
///
/// Constructor calls initialize fresh nominal values, so current struct and choice
/// fields are shared at the call boundary. The enum is kept on the view rather
/// than baked into call validation so future constructor surfaces do not have to
/// recover this information from declarations again.
#[derive(Debug, Clone, Copy)]
pub(crate) enum ConstructorFieldAccessMode {
    Shared,

    #[allow(dead_code)]
    // Planned only if constructor fields gain source-visible mutable initialization.
    Mutable,
}

impl ConstructorField {
    /// Build views from real AST struct-field declarations (base, non-generic case).
    pub(crate) fn from_struct_declarations(declarations: &[Declaration]) -> Vec<ConstructorField> {
        // One view per FIELD, not per constructor definition or call.
        #[cfg(feature = "benchmark_counters")]
        add_frontend_counter(
            FrontendCounter::CensusConstructorFieldViewsCreated,
            declarations.len(),
        );
        declarations
            .iter()
            .map(|declaration| ConstructorField {
                name: declaration.id,
                type_id: declaration.value.type_id,
                access_mode: ConstructorFieldAccessMode::Shared,
                default_value: extract_default_value(&declaration.value),
            })
            .collect()
    }

    /// Build views from choice payload field definitions (no defaults).
    pub(crate) fn from_choice_payload_fields(
        field_definitions: &[FieldDefinition],
    ) -> Vec<ConstructorField> {
        #[cfg(feature = "benchmark_counters")]
        add_frontend_counter(
            FrontendCounter::CensusConstructorFieldViewsCreated,
            field_definitions.len(),
        );
        field_definitions
            .iter()
            .map(|field_definition| ConstructorField {
                name: field_definition.name,
                type_id: field_definition.type_id,
                access_mode: ConstructorFieldAccessMode::Shared,
                default_value: None,
            })
            .collect()
    }
}

/// Extract an optional default expression, treating `ExpressionKind::NoValue` as absent.
///
/// WHAT: struct-field declarations use `NoValue` when no default is written;
/// every other expression kind is a real default that must be preserved.
fn extract_default_value(expression: &Expression) -> Option<Expression> {
    match &expression.kind {
        ExpressionKind::NoValue => None,
        _ => {
            #[cfg(feature = "benchmark_counters")]
            increment_frontend_counter(FrontendCounter::CensusConstructorDefaultViewClones);
            Some(expression.clone())
        }
    }
}
