//! Tests for AST template normalization at the HIR boundary.

use super::super::public_const_templates::{
    const_template_value_from_projection, project_const_template_value,
};
use super::super::template_helpers::{
    FinalizedTemplateValue, TemplateValueFinalizationInputs, finalize_template_value,
};
use super::*;
use crate::compiler_frontend::ast::const_values::store::ConstStringPiece;
use crate::compiler_frontend::ast::const_values::store::{
    ConstStringValue, ConstTemplateValue, ConstValueStore, ConstValueStoreError,
};
use crate::compiler_frontend::ast::expressions::call_argument::{CallAccessMode, CallArgument};
use crate::compiler_frontend::ast::expressions::expression::{
    Expression, ExpressionKind, ReactiveSource, ReactiveSourceKind,
};
use crate::compiler_frontend::ast::expressions::expression_types::ConstRecordState;
use crate::compiler_frontend::ast::expressions::expression_types::ConstValueKind;
use crate::compiler_frontend::ast::module_ast::environment::builder::import_projection::values::{
    FoldedValueMaterialiser, materialize_public_const_template,
};
use crate::compiler_frontend::ast::statements::functions::{ReturnChannel, ReturnSlot};
use crate::compiler_frontend::ast::templates::template::TemplateConstValueKind;
use crate::compiler_frontend::ast::templates::template::{
    ReactiveSubscription, SlotKey, Style, TemplateSegmentOrigin, TemplateType,
};
use crate::compiler_frontend::ast::templates::template_control_flow::{
    TemplateBranchSelector, TemplateLoopHeader,
};
use crate::compiler_frontend::ast::templates::tir::SlotOccurrenceId;
use crate::compiler_frontend::ast::templates::tir::TirExpressionOverlayId;
use crate::compiler_frontend::ast::templates::tir::refs::TemplateTirChildReference;
use crate::compiler_frontend::ast::templates::tir::{
    MalformedTirStore, TemplateIr, TemplateIrBranch, TemplateIrBuilder, TemplateIrNode,
    TemplateIrNodeKind, TemplateIrStore, TemplateIrSummary, TemplateLoopHeaderExpressionSites,
    TemplateSlotPlan, TemplateTirPhase, TemplateTirReference, TemplateWrapperReference,
    TemplateWrapperSet, TirView,
};
use crate::compiler_frontend::ast::templates::tir::{
    TemplatePreparationMode, TemplatePreparationOutcome, prepare_tir_view,
};
use crate::compiler_frontend::ast::templates::tir::{
    TemplateViewContext, TirExpressionOverlay, TirSlotResolution, TirSlotResolutionOverlay,
    TirWrapperContext, TirWrapperContextOverlay,
};
use crate::compiler_frontend::ast::templates::{
    OwnedRuntimeTemplateBody, OwnedRuntimeTemplateHandoff, OwnedRuntimeTemplateNode,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::DiagnosticPayload;
use crate::compiler_frontend::compiler_messages::InvalidTemplateStructureReason;
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::ReceiverKey;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::folded_value::{
    OwnedFoldedString, OwnedFoldedStringPiece, PublicConstTemplatePiece,
};
use crate::compiler_frontend::module_compilation::DEFAULT_TEMPLATE_CONST_LOOP_ITERATIONS;
use crate::compiler_frontend::paths::module_resources::{ModuleResourceTable, ResourceId};
use crate::compiler_frontend::paths::resource_identity::{
    PortableResourcePath, StableResourceOriginId,
};
use crate::compiler_frontend::semantic_identity::{
    ModuleRootRole, StableModuleOriginIdentity, StablePackageIdentity,
};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::synthetic_interface_provenance::{
    SyntheticInterfaceClass, SyntheticInterfaceMemberIdentity, SyntheticInterfaceProvenance,
};
use crate::compiler_frontend::tests::ast_fixture_support::{
    function_node as fixture_function_node, node, test_if_branch_metadata,
};
use crate::compiler_frontend::value_mode::ValueMode;
use std::path::Path;

/// Consumer-local materializer used to verify that projected const-template strings rebuild as
struct ConstTemplateProjectionMaterializer {
    type_environment: TypeEnvironment,
    module_resources: ModuleResourceTable,
    template_ir_store: Rc<RefCell<TemplateIrStore>>,
    path_fork: PathInternerFork,
}
impl ConstTemplateProjectionMaterializer {
    fn new() -> Self {
        Self {
            type_environment: TypeEnvironment::new(),
            module_resources: ModuleResourceTable::new(),
            template_ir_store: Rc::new(RefCell::new(TemplateIrStore::new())),
            path_fork: PathInternerFork::empty(),
        }
    }
}
impl FoldedValueMaterialiser for ConstTemplateProjectionMaterializer {
    fn path_fork(&mut self) -> &mut PathInternerFork {
        &mut self.path_fork
    }
    fn intern_resource_origin(
        &mut self,
        origin: &StableResourceOriginId,
        span: Option<SourceSpan>,
    ) -> Result<ResourceId, CompilerError> {
        Ok(self.module_resources.intern_origin(origin.clone(), span))
    }

    fn intern_canonical_type(
        &mut self,
        _identity: &crate::compiler_frontend::canonical_type_identity::CanonicalTypeIdentity,
        _string_table: &mut StringTable,
    ) -> Result<crate::compiler_frontend::datatypes::ids::TypeId, CompilerError> {
        Err(CompilerError::compiler_error(
            "const-template structural-string test materializer does not intern canonical types",
        ))
    }

    fn type_environment(&self) -> &TypeEnvironment {
        &self.type_environment
    }

    fn template_ir_store(&self) -> Rc<RefCell<TemplateIrStore>> {
        Rc::clone(&self.template_ir_store)
    }
}
#[cfg(feature = "benchmark_counters")]
use crate::compiler_frontend::instrumentation::ast_counters::{
    reset_ast_counters, test_read_ast_counter,
};
use std::cell::RefCell;
use std::rc::Rc;

fn finalized_folded(value: FinalizedTemplateValue) -> StringId {
    // This helper owns text-only assertions; provenance is covered by the focused fold tests.
    let FinalizedTemplateValue::Folded(ConstStringValue::Text(value), _) = value else {
        panic!("test expected a folded text finalization value");
    };
    value
}

/// Constructs a `Template` directly from a real module-local TIR reference.
fn template_with_reference(reference: TemplateTirReference, span: Option<SourceSpan>) -> Template {
    Template {
        tir_reference: reference,
        span,
    }
}

/// Builds a `Template` carrying a registered TIR root with a single text node,
/// matching the production shape parser-created const text templates carry
/// before finalization normalizes their enclosing payload.
fn registered_text_template(
    text: crate::compiler_frontend::symbols::string_interning::StringId,
    context: TemplateViewContext,
    template_ir_store: &Rc<RefCell<TemplateIrStore>>,
    string_table: &StringTable,
) -> Template {
    let byte_len = string_table.resolve(text).len();
    let template_id = {
        let mut store = template_ir_store.borrow_mut();
        let mut builder = TemplateIrBuilder::new(&mut store);
        let text_node = builder.push_text_node(text, byte_len, TemplateSegmentOrigin::Body, None);
        let root = builder.push_sequence_node(vec![text_node], None);
        builder.finish_template(
            root,
            Style::default(),
            TemplateType::String,
            TemplateIrSummary::default(),
            None,
        )
    };
    template_with_reference(
        TemplateTirReference {
            root: template_id,
            phase: TemplateTirPhase::Composed,
            context,
        },
        None,
    )
}

/// Builds a nested wrapper-context graph whose wrapper references carry their
/// own exact overlay views. The unsafe variant places a runtime slot plan only
/// on the nested wrapper reached through the outer wrapper's overlay.
fn nested_wrapper_finalization_fixture(
    string_table: &mut StringTable,
    unsafe_nested_wrapper: bool,
) -> (Template, Rc<RefCell<TemplateIrStore>>) {
    let template_ir_store = Rc::new(RefCell::new(TemplateIrStore::new()));
    let empty_context = TemplateViewContext::default();

    let (
        parent_template_id,
        parent_occurrence_id,
        outer_wrapper_set_id,
        nested_occurrence_id,
        outer_expression_site_id,
        inner_wrapper_set_id,
    ) = {
        let mut store = template_ir_store.borrow_mut();

        let child_template_id = {
            let mut builder = TemplateIrBuilder::new(&mut store);
            let text = string_table.intern("parent");
            let text_node =
                builder.push_text_node(text, "parent".len(), TemplateSegmentOrigin::Body, None);
            let root = builder.push_sequence_node(vec![text_node], None);
            builder.finish_template(
                root,
                Style::default(),
                TemplateType::String,
                TemplateIrSummary::default(),
                None,
            )
        };

        let nested_child_template_id = {
            let mut builder = TemplateIrBuilder::new(&mut store);
            let text = string_table.intern("nested");
            let text_node =
                builder.push_text_node(text, "nested".len(), TemplateSegmentOrigin::Body, None);
            let root = builder.push_sequence_node(vec![text_node], None);
            builder.finish_template(
                root,
                Style::default(),
                TemplateType::String,
                TemplateIrSummary::default(),
                None,
            )
        };

        let inner_wrapper_template_id = {
            let mut builder = TemplateIrBuilder::new(&mut store);
            let before = string_table.intern("inner-before");
            let after = string_table.intern("inner-after");
            let before_node = builder.push_text_node(
                before,
                "inner-before".len(),
                TemplateSegmentOrigin::Body,
                None,
            );
            let slot_node = builder.push_slot_node(SlotKey::Default, None);
            let after_node = builder.push_text_node(
                after,
                "inner-after".len(),
                TemplateSegmentOrigin::Body,
                None,
            );
            let root = builder.push_sequence_node(vec![before_node, slot_node, after_node], None);
            builder.finish_template(
                root,
                Style::default(),
                TemplateType::String,
                TemplateIrSummary::default(),
                None,
            )
        };

        if unsafe_nested_wrapper {
            let runtime_slot_plan_id = store.push_slot_plan(TemplateSlotPlan {
                contribution_sources: Vec::new(),
                slot_sites: Vec::new(),
                span: None,
            });
            store
                .attach_runtime_slot_plan(inner_wrapper_template_id, runtime_slot_plan_id)
                .expect("inner wrapper should accept the committed slot plan");
        }

        let inner_wrapper_reference = TemplateWrapperReference::new(
            inner_wrapper_template_id,
            TemplateTirPhase::Finalized,
            empty_context,
        );
        let inner_wrapper_set_id = store.push_wrapper_set(TemplateWrapperSet {
            wrappers: vec![inner_wrapper_reference],
        });

        let (outer_wrapper_template_id, nested_child_node, outer_dynamic_node) = {
            let mut builder = TemplateIrBuilder::new(&mut store);
            let outer_dynamic_text = string_table.intern("outer-structural");
            let outer_dynamic_node = builder.push_dynamic_expression_node(
                Expression::string_slice(outer_dynamic_text, None, ValueMode::ImmutableOwned),
                TemplateSegmentOrigin::Body,
                None,
                None,
            );
            let nested_child_node = builder.push_child_template_node_with_reference(
                TemplateTirChildReference::new(
                    nested_child_template_id,
                    TemplateTirPhase::Composed,
                    empty_context,
                ),
                None,
            );
            let slot_node = builder.push_slot_node(SlotKey::Default, None);
            let after = string_table.intern("outer-after");
            let after_node = builder.push_text_node(
                after,
                "outer-after".len(),
                TemplateSegmentOrigin::Body,
                None,
            );
            let root = builder.push_sequence_node(
                vec![outer_dynamic_node, nested_child_node, slot_node, after_node],
                None,
            );
            let template_id = builder.finish_template(
                root,
                Style::default(),
                TemplateType::String,
                TemplateIrSummary::default(),
                None,
            );
            (template_id, nested_child_node, outer_dynamic_node)
        };
        let nested_occurrence_id = match &store
            .get_node(nested_child_node)
            .expect("nested child node should exist")
            .kind
        {
            TemplateIrNodeKind::ChildTemplate { occurrence_id, .. } => *occurrence_id,
            _ => panic!("expected nested child-template node"),
        };
        let outer_expression_site_id = match &store
            .get_node(outer_dynamic_node)
            .expect("outer dynamic node should exist")
            .kind
        {
            TemplateIrNodeKind::DynamicExpression { site_id, .. } => *site_id,
            _ => panic!("expected outer dynamic-expression node"),
        };
        let outer_wrapper_reference = TemplateWrapperReference::new(
            outer_wrapper_template_id,
            TemplateTirPhase::Finalized,
            empty_context,
        );
        let outer_wrapper_set_id = store.push_wrapper_set(TemplateWrapperSet {
            wrappers: vec![outer_wrapper_reference],
        });

        let parent_child_node = {
            let mut builder = TemplateIrBuilder::new(&mut store);
            builder.push_child_template_node_with_reference(
                TemplateTirChildReference::new(
                    child_template_id,
                    TemplateTirPhase::Composed,
                    empty_context,
                ),
                None,
            )
        };
        let parent_occurrence_id = match &store
            .get_node(parent_child_node)
            .expect("parent child node should exist")
            .kind
        {
            TemplateIrNodeKind::ChildTemplate { occurrence_id, .. } => *occurrence_id,
            _ => panic!("expected parent child-template node"),
        };
        let parent_template_id = {
            let mut builder = TemplateIrBuilder::new(&mut store);
            let root = builder.push_sequence_node(vec![parent_child_node], None);
            builder.finish_template(
                root,
                Style::default(),
                TemplateType::String,
                TemplateIrSummary::default(),
                None,
            )
        };

        (
            parent_template_id,
            parent_occurrence_id,
            outer_wrapper_set_id,
            nested_occurrence_id,
            outer_expression_site_id,
            inner_wrapper_set_id,
        )
    };

    let nested_context_overlay_id = template_ir_store
        .borrow_mut()
        .allocate_wrapper_context_overlay(TirWrapperContextOverlay {
            contexts: vec![(
                nested_occurrence_id,
                TirWrapperContext {
                    inherited_wrapper_set: Some(inner_wrapper_set_id),
                    ..TirWrapperContext::default()
                },
            )],
        })
        .expect("test overlay allocation");
    let outer_expression_overlay_id = template_ir_store
        .borrow_mut()
        .allocate_expression_overlay(TirExpressionOverlay {
            overrides: vec![(
                outer_expression_site_id,
                Box::new(Expression::string_slice(
                    string_table.intern("outer-overlay"),
                    None,
                    ValueMode::ImmutableOwned,
                )),
            )],
        })
        .expect("test overlay allocation");
    let outer_context = TemplateViewContext {
        expression_overlay: Some(outer_expression_overlay_id),
        slot_resolution: None,
        wrapper_context: Some(nested_context_overlay_id),
    };
    MalformedTirStore::new(&mut template_ir_store.borrow_mut()).set_wrapper_reference_context(
        outer_wrapper_set_id,
        0,
        outer_context,
    );

    let parent_context_overlay_id = template_ir_store
        .borrow_mut()
        .allocate_wrapper_context_overlay(TirWrapperContextOverlay {
            contexts: vec![(
                parent_occurrence_id,
                TirWrapperContext {
                    inherited_wrapper_set: Some(outer_wrapper_set_id),
                    ..TirWrapperContext::default()
                },
            )],
        })
        .expect("test overlay allocation");
    let parent_context = TemplateViewContext {
        expression_overlay: None,
        slot_resolution: None,
        wrapper_context: Some(parent_context_overlay_id),
    };
    let template = template_with_reference(
        TemplateTirReference {
            root: parent_template_id,
            phase: TemplateTirPhase::Finalized,
            context: parent_context,
        },
        None,
    );

    (template, template_ir_store)
}

#[path = "normalize_ast_finalization_tests.rs"]
mod normalize_ast_finalization_tests;

#[path = "normalize_ast_runtime_tests.rs"]
mod normalize_ast_runtime_tests;

#[path = "normalize_ast_synchronize_tests.rs"]
mod normalize_ast_synchronize_tests;

#[path = "normalize_ast_slot_tests.rs"]
mod normalize_ast_slot_tests;
