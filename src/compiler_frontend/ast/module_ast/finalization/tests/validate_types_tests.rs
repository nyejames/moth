//! Tests for final AST type-boundary validation of template expression payloads.

use super::*;
use crate::compiler_frontend::ast::ast_nodes::DeclarationValue;
use crate::compiler_frontend::ast::ast_nodes::{
    AstNode, Declaration, LoopBindings, NodeKind, RangeEndKind, RangeLoopSpec,
};
use crate::compiler_frontend::ast::expressions::expression::CollectionExpressionType;
use crate::compiler_frontend::ast::expressions::expression_store::ExpressionOp;
use crate::compiler_frontend::ast::expressions::expression_store::SuccessShape;
use crate::compiler_frontend::ast::expressions::expression_types::ConstRecordState;
use crate::compiler_frontend::ast::statements::match_patterns::MatchPattern;
use crate::compiler_frontend::ast::statements::value_production::types::ValueCatchBlock;
use crate::compiler_frontend::ast::templates::template::Template;
use crate::compiler_frontend::ast::templates::template::{
    Style, TemplateSegmentOrigin, TemplateType,
};
use crate::compiler_frontend::ast::templates::template_control_flow::{
    TemplateBranchSelector, TemplateLoopHeader,
};
use crate::compiler_frontend::ast::templates::template_slots::{
    RuntimeSlotContributionSourceId, RuntimeSlotSiteId,
};
use crate::compiler_frontend::ast::templates::tir::{
    ExpressionSiteId, TemplateIr, TemplateIrBuilder, TemplateIrNode, TemplateIrStore,
    TemplateIrSummary, TemplateLoopHeaderExpressionSites, TemplateTirPhase, TemplateTirReference,
    TemplateViewContext, TirExpressionOverlay,
};
use crate::compiler_frontend::ast::templates::{
    OwnedRuntimeSlotApplicationHandoff, OwnedRuntimeSlotContributionSource, OwnedRuntimeSlotSite,
    OwnedRuntimeTemplateBody, OwnedRuntimeTemplateHandoff, OwnedRuntimeTemplateNode,
};
use crate::compiler_frontend::compiler_errors::ErrorType;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::{TypeId, builtin_type_ids};
use crate::compiler_frontend::datatypes::numeric_scalar::NumericScalar;
use crate::compiler_frontend::source::{ExtendedSpanBuilder, LocalSpan, SourceId, SourceSpan};
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::value_mode::ValueMode;
use moth_lexical::numeric::fixed_scalar::FixedScalar;

/// Runs the production walk from one statement and returns the first error in source order.
fn validate_node(node: &AstNode, context: &TypeValidationContext) -> Result<(), CompilerError> {
    validate_roots(
        *context,
        |children| {
            push_nodes(std::slice::from_ref(node), context, children);
            Ok(())
        },
        &mut || {},
    )
}

fn validate_expression(
    expression: &ExprId,
    context: &TypeValidationContext,
) -> Result<(), CompilerError> {
    validate_expression_counted(expression, context, &mut || {})
}

/// Walks one expression root and reports each expansion through `count`.
fn validate_expression_counted(
    expression: &ExprId,
    context: &TypeValidationContext,
    count: &mut impl FnMut(),
) -> Result<(), CompilerError> {
    validate_roots(
        *context,
        |children| {
            push_expression(*expression, context, children);
            Ok(())
        },
        count,
    )
}

fn validate_owned_runtime_template_handoff(
    handoff: &OwnedRuntimeTemplateHandoff,
    context: &TypeValidationContext,
) -> Result<(), CompilerError> {
    validate_roots(
        *context,
        |children| expand_owned_runtime_template_handoff(handoff, context, children),
        &mut || {},
    )
}

fn validate_owned_runtime_slot_application_handoff(
    handoff: &OwnedRuntimeSlotApplicationHandoff,
    context: &TypeValidationContext,
) -> Result<(), CompilerError> {
    validate_roots(
        *context,
        |children| expand_owned_runtime_slot_application_handoff(handoff, context, children),
        &mut || {},
    )
}

fn validate_template_expression_payloads(
    template: &Template,
    context: &TypeValidationContext,
) -> Result<(), CompilerError> {
    validate_roots(
        *context,
        |children| expand_template(template, context, children),
        &mut || {},
    )
}

fn invalid_string_expression(
    builder: &mut AstExpressionBuilder,
    span: Option<SourceSpan>,
) -> ExprId {
    builder
        .reference_with_type_id(
            PathId::ROOT,
            TypeId(9999),
            span,
            ValueMode::ImmutableOwned,
            ConstRecordState::RuntimeValue,
        )
        .expect("fixture row")
        .id()
}

fn orphan_bool_expression(builder: &mut AstExpressionBuilder) -> ExprId {
    builder
        .reference_with_type_id(
            PathId::ROOT,
            TypeId(9999),
            None,
            ValueMode::ImmutableOwned,
            ConstRecordState::RuntimeValue,
        )
        .expect("fixture row")
        .id()
}

fn owned_render_handoff(node: OwnedRuntimeTemplateNode) -> OwnedRuntimeTemplateHandoff {
    OwnedRuntimeTemplateHandoff {
        body: OwnedRuntimeTemplateBody::Render(node),
    }
}

fn validate_owned_handoff_with_orphan_type_id(
    node: OwnedRuntimeTemplateNode,
    expressions: &AstExpressionBuilder,
    constants: &ConstValueStore,
) {
    let handoff = owned_render_handoff(node);
    let type_environment = TypeEnvironment::new();
    let store = TemplateIrStore::new();
    let context = TypeValidationContext {
        expression_builder: expressions,
        const_values: constants,
        recovery_allowed: false,
        type_environment: &type_environment,
        template_ir_store: &store,
    };

    let error = validate_owned_runtime_template_handoff(&handoff, &context)
        .expect_err("owned runtime handoff payloads must retain TypeId validation");
    assert!(error.msg.contains("9999"));
}

#[test]
fn expression_type_validation_walks_all_success_slots_and_the_error_channel() {
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let constants = ConstValueStore::default();
    let type_environment = TypeEnvironment::new();
    let store = TemplateIrStore::new();
    let call = expressions
        .function_call_with_typed_arguments(PathId::ROOT, vec![], vec![], None)
        .expect("call")
        .id();
    let context = TypeValidationContext {
        expression_builder: &expressions,
        const_values: &constants,
        recovery_allowed: false,
        type_environment: &type_environment,
        template_ir_store: &store,
    };
    validate_expression(&call, &context).expect("zero results need no sentinel TypeId");
    let multiple = expressions
        .function_call_with_typed_arguments(
            PathId::ROOT,
            vec![],
            vec![builtin_type_ids::BOOL, TypeId(9999)],
            None,
        )
        .expect("call")
        .id();
    let fallible = expressions
        .with_typed_error_producer(expressions.own(call), TypeId(9999), None)
        .expect("typed producer")
        .id();
    let context = TypeValidationContext {
        expression_builder: &expressions,
        const_values: &constants,
        recovery_allowed: false,
        type_environment: &type_environment,
        template_ir_store: &store,
    };
    for root in [multiple, fallible] {
        let error = validate_expression(&root, &context)
            .expect_err("every result and error channel must resolve");
        assert!(error.msg.contains("9999"));
    }
}

#[test]
fn owned_runtime_conditional_selector_type_ids_are_validated_before_inactive_elision() {
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let mut strings = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let selectors = vec![
        TemplateBranchSelector::Bool(orphan_bool_expression(&mut expressions)),
        TemplateBranchSelector::OptionPresentCapture {
            scrutinee: expressions
                .bool(&mut constants, true, None, ValueMode::ImmutableOwned)
                .expect("fixture literal")
                .id(),
            pattern: Box::new(MatchPattern::OptionPresentCapture {
                name: strings.intern("value"),
                binding_path: PathId::ROOT,
                inner_type_id: TypeId(9999),
                span: None,
                binding_span: None,
            }),
        },
    ];

    for selector in selectors {
        validate_owned_handoff_with_orphan_type_id(
            OwnedRuntimeTemplateNode::Conditional {
                selector: Box::new(selector),
                body: Box::new(OwnedRuntimeTemplateNode::Sequence {
                    children: Vec::new(),
                    span: None,
                }),
                span: None,
            },
            &expressions,
            &constants,
        );
    }
}

fn orphan_declaration(expressions: &mut AstExpressionBuilder) -> Declaration {
    {
        let value = orphan_bool_expression(expressions);
        Declaration {
            id: PathId::ROOT,
            type_id: TypeId(9999),
            type_syntax: None,
            value_mode: ValueMode::ImmutableOwned,
            value: DeclarationValue::Initializer(value),
            binding_span: None,
            config_qualifier: None,
        }
    }
}

fn empty_loop_body() -> Box<OwnedRuntimeTemplateNode> {
    Box::new(OwnedRuntimeTemplateNode::Sequence {
        children: Vec::new(),
        span: None,
    })
}

#[test]
fn owned_runtime_slot_handoff_validates_all_expression_payload_routes() {
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let slot_handoffs = vec![
        OwnedRuntimeSlotApplicationHandoff {
            wrapper: OwnedRuntimeTemplateNode::DynamicExpression {
                expression: orphan_bool_expression(&mut expressions),
                span: None,
            },
            contribution_sources: Vec::new(),
            slot_sites: Vec::new(),
        },
        OwnedRuntimeSlotApplicationHandoff {
            wrapper: OwnedRuntimeTemplateNode::Sequence {
                children: Vec::new(),
                span: None,
            },
            contribution_sources: vec![OwnedRuntimeSlotContributionSource {
                source: RuntimeSlotContributionSourceId(0),
                render_root: OwnedRuntimeTemplateNode::Conditional {
                    selector: Box::new(TemplateBranchSelector::Bool(orphan_bool_expression(
                        &mut expressions,
                    ))),
                    body: Box::new(OwnedRuntimeTemplateNode::Sequence {
                        children: Vec::new(),
                        span: None,
                    }),
                    span: None,
                },
                renders_wrapper_unconditionally: false,
                span: None,
            }],
            slot_sites: Vec::new(),
        },
        OwnedRuntimeSlotApplicationHandoff {
            wrapper: OwnedRuntimeTemplateNode::Sequence {
                children: Vec::new(),
                span: None,
            },
            contribution_sources: Vec::new(),
            slot_sites: vec![OwnedRuntimeSlotSite {
                site: RuntimeSlotSiteId(0),
                render_root: OwnedRuntimeTemplateNode::Loop {
                    header: TemplateLoopHeader::Range {
                        bindings: Box::new(LoopBindings {
                            item: Some(orphan_declaration(&mut expressions)),
                            index: None,
                        }),
                        range: Box::new(RangeLoopSpec {
                            start: expressions
                                .bool(&mut constants, true, None, ValueMode::ImmutableOwned)
                                .expect("fixture literal")
                                .id(),
                            end: expressions
                                .bool(&mut constants, true, None, ValueMode::ImmutableOwned)
                                .expect("fixture literal")
                                .id(),
                            end_kind: RangeEndKind::Exclusive,
                            step: None,
                            domain: NumericScalar::Fixed(FixedScalar::I32),
                        }),
                    },
                    body: empty_loop_body(),
                    aggregate_wrapper: None,
                    span: None,
                },
                span: None,
            }],
        },
    ];

    for handoff in slot_handoffs {
        let type_environment = TypeEnvironment::new();
        let store = TemplateIrStore::new();
        let context = TypeValidationContext {
            expression_builder: &expressions,
            const_values: &constants,
            recovery_allowed: false,
            type_environment: &type_environment,
            template_ir_store: &store,
        };
        let error = validate_owned_runtime_slot_application_handoff(&handoff, &context)
            .expect_err("slot handoff payloads must retain TypeId validation");
        assert!(error.msg.contains("9999"));
    }
}

#[test]
fn static_true_assertion_owned_handoff_is_validated_before_message_elision() {
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let handoff = owned_render_handoff(OwnedRuntimeTemplateNode::Conditional {
        selector: Box::new(TemplateBranchSelector::Bool(orphan_bool_expression(
            &mut expressions,
        ))),
        body: Box::new(OwnedRuntimeTemplateNode::Sequence {
            children: Vec::new(),
            span: None,
        }),
        span: None,
    });
    let runtime_template = expressions
        .append_runtime_template(handoff, None)
        .expect("handoff");
    let node = AstNode {
        kind: NodeKind::Assert {
            condition: expressions
                .bool(&mut constants, true, None, ValueMode::ImmutableOwned)
                .expect("fixture literal")
                .id(),
            message: expressions
                .append_expression(
                    ExpressionOp::RuntimeTemplate(runtime_template),
                    None,
                    builtin_type_ids::STRING,
                    ValueMode::ImmutableOwned,
                )
                .expect("fixture operation")
                .id(),
        },
        span: None,
        scope: PathId::ROOT,
    };
    let type_environment = TypeEnvironment::new();
    let store = TemplateIrStore::new();
    let context = TypeValidationContext {
        expression_builder: &expressions,
        const_values: &constants,
        recovery_allowed: false,
        type_environment: &type_environment,
        template_ir_store: &store,
    };

    let error = validate_node(&node, &context)
        .expect_err("static-true owned message payload must be validated before elision");
    assert!(error.msg.contains("9999"));
}

#[test]
fn static_true_assertion_slot_handoff_is_validated_before_message_elision() {
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let slot_handoff = OwnedRuntimeSlotApplicationHandoff {
        wrapper: OwnedRuntimeTemplateNode::DynamicExpression {
            expression: orphan_bool_expression(&mut expressions),
            span: None,
        },
        contribution_sources: Vec::new(),
        slot_sites: Vec::new(),
    };
    let runtime_slot_application = expressions
        .append_runtime_slot_application(slot_handoff, None)
        .expect("handoff");
    let node = AstNode {
        kind: NodeKind::Assert {
            condition: expressions
                .bool(&mut constants, true, None, ValueMode::ImmutableOwned)
                .expect("fixture literal")
                .id(),
            message: expressions
                .append_expression(
                    ExpressionOp::RuntimeSlotApplication(runtime_slot_application),
                    None,
                    builtin_type_ids::STRING,
                    ValueMode::ImmutableOwned,
                )
                .expect("fixture operation")
                .id(),
        },
        span: None,
        scope: PathId::ROOT,
    };
    let type_environment = TypeEnvironment::new();
    let store = TemplateIrStore::new();
    let context = TypeValidationContext {
        expression_builder: &expressions,
        const_values: &constants,
        recovery_allowed: false,
        type_environment: &type_environment,
        template_ir_store: &store,
    };

    let error = validate_node(&node, &context)
        .expect_err("static-true slot message payload must be validated before elision");
    assert!(error.msg.contains("9999"));
}

#[test]
fn owned_runtime_loop_header_type_ids_are_validated_before_inactive_elision() {
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let headers = [
        TemplateLoopHeader::Conditional {
            condition: orphan_bool_expression(&mut expressions),
        },
        TemplateLoopHeader::Range {
            bindings: Box::new(LoopBindings {
                item: None,
                index: None,
            }),
            range: Box::new(RangeLoopSpec {
                start: orphan_bool_expression(&mut expressions),
                end: expressions
                    .bool(&mut constants, true, None, ValueMode::ImmutableOwned)
                    .expect("fixture literal")
                    .id(),
                end_kind: RangeEndKind::Exclusive,
                step: None,
                domain: NumericScalar::Fixed(FixedScalar::I32),
            }),
        },
        TemplateLoopHeader::Range {
            bindings: Box::new(LoopBindings {
                item: None,
                index: None,
            }),
            range: Box::new(RangeLoopSpec {
                start: expressions
                    .bool(&mut constants, true, None, ValueMode::ImmutableOwned)
                    .expect("fixture literal")
                    .id(),
                end: orphan_bool_expression(&mut expressions),
                end_kind: RangeEndKind::Exclusive,
                step: None,
                domain: NumericScalar::Fixed(FixedScalar::I32),
            }),
        },
        TemplateLoopHeader::Range {
            bindings: Box::new(LoopBindings {
                item: None,
                index: None,
            }),
            range: Box::new(RangeLoopSpec {
                start: expressions
                    .bool(&mut constants, true, None, ValueMode::ImmutableOwned)
                    .expect("fixture literal")
                    .id(),
                end: expressions
                    .bool(&mut constants, true, None, ValueMode::ImmutableOwned)
                    .expect("fixture literal")
                    .id(),
                end_kind: RangeEndKind::Exclusive,
                step: Some(orphan_bool_expression(&mut expressions)),
                domain: NumericScalar::Fixed(FixedScalar::I32),
            }),
        },
        TemplateLoopHeader::Range {
            bindings: Box::new(LoopBindings {
                item: Some(orphan_declaration(&mut expressions)),
                index: None,
            }),
            range: Box::new(RangeLoopSpec {
                start: expressions
                    .bool(&mut constants, true, None, ValueMode::ImmutableOwned)
                    .expect("fixture literal")
                    .id(),
                end: expressions
                    .bool(&mut constants, true, None, ValueMode::ImmutableOwned)
                    .expect("fixture literal")
                    .id(),
                end_kind: RangeEndKind::Exclusive,
                step: None,
                domain: NumericScalar::Fixed(FixedScalar::I32),
            }),
        },
        TemplateLoopHeader::Collection {
            bindings: Box::new(LoopBindings {
                item: None,
                index: None,
            }),
            iterable: orphan_bool_expression(&mut expressions),
        },
        TemplateLoopHeader::Collection {
            bindings: Box::new(LoopBindings {
                item: None,
                index: Some(orphan_declaration(&mut expressions)),
            }),
            iterable: expressions
                .bool(&mut constants, true, None, ValueMode::ImmutableOwned)
                .expect("fixture literal")
                .id(),
        },
    ];

    for header in headers {
        validate_owned_handoff_with_orphan_type_id(
            OwnedRuntimeTemplateNode::Loop {
                header,
                body: Box::new(OwnedRuntimeTemplateNode::Sequence {
                    children: Vec::new(),
                    span: None,
                }),
                aggregate_wrapper: None,
                span: None,
            },
            &expressions,
            &constants,
        );
    }
}

#[test]
fn static_true_assertion_message_reaches_type_validation_before_elision() {
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let mut strings = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let structural = expressions
        .string_slice(
            &mut constants,
            strings.intern("structural"),
            None,
            ValueMode::ImmutableOwned,
        )
        .expect("fixture literal")
        .id();
    let mut store = TemplateIrStore::new();
    let template = template_with_dynamic_overlay(
        &mut store,
        structural,
        invalid_string_expression(&mut expressions, None),
        TemplateTirPhase::Finalized,
    );
    let template_expression = expressions
        .template(template, ValueMode::ImmutableOwned)
        .expect("template row")
        .id();

    let mut type_environment = TypeEnvironment::new();
    let option_string_type_id = type_environment.intern_option(builtin_type_ids::STRING);
    let message = expressions
        .append_expression(
            ExpressionOp::Coerce {
                value: expressions.own(template_expression),
                operation: CoercionOperation::ToType(option_string_type_id),
            },
            None,
            option_string_type_id,
            ValueMode::ImmutableOwned,
        )
        .expect("fixture operation")
        .id();
    let node = AstNode {
        kind: NodeKind::Assert {
            condition: expressions
                .bool(&mut constants, true, None, ValueMode::ImmutableOwned)
                .expect("fixture literal")
                .id(),
            message,
        },
        span: None,
        scope: PathId::ROOT,
    };
    let context = TypeValidationContext {
        expression_builder: &expressions,
        const_values: &constants,
        recovery_allowed: false,
        type_environment: &type_environment,
        template_ir_store: &store,
    };

    let error = validate_node(&node, &context)
        .expect_err("static-true message payloads must be validated before elision");
    assert!(error.msg.contains("9999"));
}

fn template_with_dynamic_overlay(
    store: &mut TemplateIrStore,
    structural: ExprId,
    overlay: ExprId,
    phase: TemplateTirPhase,
) -> Template {
    let site_id = store.next_expression_site_id();
    let node = store.push_node(TemplateIrNode::new(
        crate::compiler_frontend::ast::templates::tir::TemplateIrNodeKind::DynamicExpression {
            expression: structural,
            origin: TemplateSegmentOrigin::Body,
            site_id,
        },
        None,
    ));
    let root = store.push_template(TemplateIr::new(
        node,
        Style::default(),
        TemplateType::StringFunction,
        TemplateIrSummary::default(),
        None,
    ));
    let expression_overlay_id = store
        .allocate_expression_overlay(TirExpressionOverlay {
            overrides: vec![(site_id, overlay)],
        })
        .expect("test overlay allocation");
    let context = TemplateViewContext {
        expression_overlay: Some(expression_overlay_id),
        slot_resolution: None,
        wrapper_context: None,
    };
    Template {
        tir_reference: TemplateTirReference {
            root,
            phase,
            context,
        },
        span: None,
    }
}

#[test]
fn validation_checks_effective_dynamic_expression_overlay() {
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let mut strings = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let structural = expressions
        .string_slice(
            &mut constants,
            strings.intern("structural"),
            None,
            ValueMode::ImmutableOwned,
        )
        .expect("fixture literal")
        .id();
    let overlay = invalid_string_expression(&mut expressions, None);
    let mut store = TemplateIrStore::new();
    let template =
        template_with_dynamic_overlay(&mut store, structural, overlay, TemplateTirPhase::Finalized);
    let type_environment = TypeEnvironment::new();
    let store_borrow = store;
    let context = TypeValidationContext {
        expression_builder: &expressions,
        const_values: &constants,
        recovery_allowed: false,
        type_environment: &type_environment,
        template_ir_store: &store_borrow,
    };

    let error = validate_template_expression_payloads(&template, &context)
        .expect_err("orphan overlay type should be rejected");
    assert!(matches!(error.error_type, ErrorType::Compiler));
    assert!(error.msg.contains("9999"));
}

#[test]
fn validation_rejects_non_finalized_template_reference() {
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let mut strings = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let structural = expressions
        .string_slice(
            &mut constants,
            strings.intern("structural"),
            None,
            ValueMode::ImmutableOwned,
        )
        .expect("fixture literal")
        .id();
    let mut store = TemplateIrStore::new();
    let template = template_with_dynamic_overlay(
        &mut store,
        structural,
        structural,
        TemplateTirPhase::Composed,
    );
    let type_environment = TypeEnvironment::new();
    let context = TypeValidationContext {
        expression_builder: &expressions,
        const_values: &constants,
        recovery_allowed: false,
        type_environment: &type_environment,
        template_ir_store: &store,
    };

    let error = validate_template_expression_payloads(&template, &context)
        .expect_err("non-finalized template should be rejected");
    assert!(matches!(error.error_type, ErrorType::Compiler));
    assert!(error.msg.contains("Finalized"));
}

#[test]
fn validation_reports_missing_template_root() {
    let expressions = AstExpressionBuilder::new().unwrap();
    let constants = ConstValueStore::default();
    let store = TemplateIrStore::new();
    let template = Template {
        tir_reference: TemplateTirReference {
            root: crate::compiler_frontend::ast::templates::tir::TemplateIrId::new(99),
            phase: TemplateTirPhase::Finalized,
            context: TemplateViewContext::default(),
        },
        span: None,
    };
    let type_environment = TypeEnvironment::new();
    let context = TypeValidationContext {
        expression_builder: &expressions,
        const_values: &constants,
        recovery_allowed: false,
        type_environment: &type_environment,
        template_ir_store: &store,
    };

    let error = validate_template_expression_payloads(&template, &context)
        .expect_err("missing root should be rejected");
    assert!(error.msg.contains("root"));
}

fn authored_span(start: u32) -> SourceSpan {
    let mut builder = ExtendedSpanBuilder::new();
    let local = LocalSpan::exact(start, 1, &mut builder).expect("test span should fit");
    SourceSpan::new(SourceId::COMPILATION_ROOT, local)
}

/// Builds a finalized `Template` over `root` with one expression overlay replacing
/// the expression at `site_id` with `overlay_expression`.
fn finalized_template_with_site_overlay(
    store: &mut TemplateIrStore,
    root: crate::compiler_frontend::ast::templates::tir::TemplateIrId,
    site_id: ExpressionSiteId,
    overlay_expression: ExprId,
) -> Template {
    let expression_overlay_id = store
        .allocate_expression_overlay(TirExpressionOverlay {
            overrides: vec![(site_id, overlay_expression)],
        })
        .expect("test overlay allocation");
    let context = TemplateViewContext {
        expression_overlay: Some(expression_overlay_id),
        slot_resolution: None,
        wrapper_context: None,
    };
    Template {
        tir_reference: TemplateTirReference {
            root,
            phase: TemplateTirPhase::Finalized,
            context,
        },
        span: None,
    }
}

fn invalid_bool_expression(
    builder: &mut AstExpressionBuilder,
    _value: bool,
    span: Option<SourceSpan>,
) -> ExprId {
    builder
        .reference_with_type_id(
            PathId::ROOT,
            TypeId(9999),
            span,
            ValueMode::ImmutableOwned,
            ConstRecordState::RuntimeValue,
        )
        .expect("fixture row")
        .id()
}

#[test]
fn finalized_tir_view_conditional_selector_payload_validates_effective_overlay_span() {
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let type_environment = TypeEnvironment::new();
    let mut store = TemplateIrStore::new();

    let structural_span = Some(authored_span(10));
    let structural_selector = expressions
        .bool(
            &mut constants,
            true,
            structural_span,
            ValueMode::ImmutableOwned,
        )
        .expect("fixture literal")
        .id();

    let overlay_span = Some(authored_span(20));
    let overlay_selector = invalid_bool_expression(&mut expressions, true, overlay_span);

    let (template_id, selector_site_id) = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let body = builder.push_sequence_node(Vec::new(), None);
        let conditional_node_id = builder.push_conditional_node(
            TemplateBranchSelector::Bool(structural_selector),
            body,
            structural_span,
        );
        let template_id = builder.finish_template(
            conditional_node_id,
            Style::default(),
            TemplateType::StringFunction,
            TemplateIrSummary::default(),
            None,
        );
        let selector_site_id = match &store
            .get_node(conditional_node_id)
            .expect("conditional node should exist")
            .kind
        {
            crate::compiler_frontend::ast::templates::tir::TemplateIrNodeKind::Conditional {
                selector_site_id,
                ..
            } => *selector_site_id,
            other => panic!("expected conditional node, got {other:?}"),
        };
        (template_id, selector_site_id)
    };

    let template = finalized_template_with_site_overlay(
        &mut store,
        template_id,
        selector_site_id,
        overlay_selector,
    );
    let context = TypeValidationContext {
        expression_builder: &expressions,
        const_values: &constants,
        recovery_allowed: false,
        type_environment: &type_environment,
        template_ir_store: &store,
    };

    let error = validate_template_expression_payloads(&template, &context).expect_err(
        "finalized TirView path should detect orphan TypeId on effective overlay selector",
    );

    assert_eq!(
        error.source_span, overlay_span,
        "error span must point to the effective overlay selector, not the structural selector"
    );
    assert!(error.msg.contains("9999"));
}

#[test]
fn finalized_tir_view_loop_header_payload_validates_effective_overlay_span() {
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let type_environment = TypeEnvironment::new();
    let mut store = TemplateIrStore::new();

    let structural_span = Some(authored_span(10));
    let structural_condition = expressions
        .bool(
            &mut constants,
            false,
            structural_span,
            ValueMode::ImmutableOwned,
        )
        .expect("fixture literal")
        .id();

    let overlay_span = Some(authored_span(30));
    let overlay_condition = invalid_bool_expression(&mut expressions, false, overlay_span);

    let (template_id, condition_site_id) = {
        let mut builder = TemplateIrBuilder::new(&mut store);
        let loop_body = builder.push_sequence_node(Vec::new(), None);
        let header = TemplateLoopHeader::Conditional {
            condition: structural_condition,
        };
        let loop_node_id = builder.push_loop_node(header, loop_body, None, structural_span);
        let template_id = builder.finish_template(
            loop_node_id,
            Style::default(),
            TemplateType::StringFunction,
            TemplateIrSummary::default(),
            None,
        );
        let condition_site_id = match &store
            .get_node(loop_node_id)
            .expect("loop node should exist")
            .kind
        {
            crate::compiler_frontend::ast::templates::tir::TemplateIrNodeKind::Loop {
                header_sites,
                ..
            } => match header_sites {
                TemplateLoopHeaderExpressionSites::Conditional { condition } => *condition,
                other => panic!("expected conditional loop header sites, got {other:?}"),
            },
            other => panic!("expected loop node, got {other:?}"),
        };
        (template_id, condition_site_id)
    };

    let template = finalized_template_with_site_overlay(
        &mut store,
        template_id,
        condition_site_id,
        overlay_condition,
    );
    let context = TypeValidationContext {
        expression_builder: &expressions,
        const_values: &constants,
        recovery_allowed: false,
        type_environment: &type_environment,
        template_ir_store: &store,
    };

    let error = validate_template_expression_payloads(&template, &context).expect_err(
        "finalized TirView path should detect orphan TypeId on effective overlay loop header",
    );

    assert_eq!(
        error.source_span, overlay_span,
        "error span must point to the effective overlay loop header, not the structural header"
    );
    assert!(error.msg.contains("9999"));
}

fn bool_reference(expressions: &mut AstExpressionBuilder) -> ExprId {
    expressions
        .reference_with_type_id(
            PathId::ROOT,
            builtin_type_ids::BOOL,
            None,
            ValueMode::ImmutableOwned,
            ConstRecordState::RuntimeValue,
        )
        .expect("fixture leaf")
        .id()
}

fn bool_collection(
    expressions: &mut AstExpressionBuilder,
    type_environment: &mut TypeEnvironment,
    items: Vec<ExprId>,
) -> ExprId {
    let items = expressions.own_slice(&items);
    expressions
        .collection_with_type_id(
            items,
            CollectionExpressionType {
                element_type_id: builtin_type_ids::BOOL,
                fixed_capacity: None,
                collection_type_id: None,
            },
            type_environment,
            None,
            ValueMode::ImmutableOwned,
        )
        .expect("fixture collection")
        .id()
}

/// Builds a `depth`-deep DAG above `leaf`. Each collection row references the row below it twice.
fn shared_chain(
    expressions: &mut AstExpressionBuilder,
    type_environment: &mut TypeEnvironment,
    leaf: ExprId,
    depth: usize,
) -> ExprId {
    (0..depth).fold(leaf, |below, _| {
        bool_collection(expressions, type_environment, vec![below, below])
    })
}

/// A recovering call that `catch` protects. It returns `BOOL` and its error channel is `STRING`.
fn recovering_call(expressions: &mut AstExpressionBuilder) -> ExprId {
    expressions
        .handled_fallible_function_call_with_typed_arguments(
            PathId::ROOT,
            vec![],
            vec![builtin_type_ids::BOOL],
            builtin_type_ids::STRING,
            FallibleExpressionHandling::Recover,
            None,
        )
        .expect("recovering call fixture")
        .id()
}

/// A value-producing `catch` over `handled`, with an empty handler body.
fn catch_over(expressions: &mut AstExpressionBuilder, handled: ExprId) -> ExprId {
    expressions
        .value_block(
            ValueBlock::Catch(ValueCatchBlock {
                handled_value: handled,
                handler: FallibleHandling::Handler {
                    error: None,
                    body: vec![],
                },
                result_type_ids: vec![builtin_type_ids::BOOL],
            }),
            None,
            SuccessShape::One(builtin_type_ids::BOOL),
            ValueMode::ImmutableOwned,
        )
        .expect("catch fixture")
        .id()
}

#[test]
fn shared_expression_dag_expands_each_row_once() {
    const DEPTH: usize = 40;
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let constants = ConstValueStore::default();
    let mut type_environment = TypeEnvironment::new();
    let store = TemplateIrStore::new();
    let leaf = bool_reference(&mut expressions);
    let root = shared_chain(&mut expressions, &mut type_environment, leaf, DEPTH);
    let context = TypeValidationContext {
        expression_builder: &expressions,
        const_values: &constants,
        recovery_allowed: false,
        type_environment: &type_environment,
        template_ir_store: &store,
    };

    // Path recursion would expand 2^DEPTH paths. One expansion per row gives the leaf plus DEPTH rows.
    let mut expansions = 0usize;
    validate_expression_counted(&root, &context, &mut || expansions += 1)
        .expect("shared collection DAG is well typed");
    assert_eq!(expansions, DEPTH + 1);
}

#[test]
fn shared_expression_dag_still_reports_an_invalid_leaf() {
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let constants = ConstValueStore::default();
    let mut type_environment = TypeEnvironment::new();
    let store = TemplateIrStore::new();
    let leaf = invalid_string_expression(&mut expressions, None);
    let root = shared_chain(&mut expressions, &mut type_environment, leaf, 40);
    let context = TypeValidationContext {
        expression_builder: &expressions,
        const_values: &constants,
        recovery_allowed: false,
        type_environment: &type_environment,
        template_ir_store: &store,
    };

    let error =
        validate_expression(&root, &context).expect_err("the shared invalid leaf must be rejected");
    assert!(error.msg.contains("9999"));
}

#[test]
fn catch_protected_subexpression_shared_by_two_catches_is_accepted() {
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let constants = ConstValueStore::default();
    let mut type_environment = TypeEnvironment::new();
    let store = TemplateIrStore::new();
    let recovering = recovering_call(&mut expressions);
    let first = catch_over(&mut expressions, recovering);
    let second = catch_over(&mut expressions, recovering);
    let root = bool_collection(&mut expressions, &mut type_environment, vec![first, second]);
    let context = TypeValidationContext {
        expression_builder: &expressions,
        const_values: &constants,
        recovery_allowed: false,
        type_environment: &type_environment,
        template_ir_store: &store,
    };

    // Root, both catches, and the shared subject once under recovery. The second catch reuses it.
    let mut expansions = 0usize;
    validate_expression_counted(&root, &context, &mut || expansions += 1)
        .expect("a recovering subject may be shared by catches");
    assert_eq!(expansions, 4);
}

#[test]
fn recovering_subject_reached_outside_its_catch_is_rejected_after_the_catch_visit() {
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let constants = ConstValueStore::default();
    let mut type_environment = TypeEnvironment::new();
    let store = TemplateIrStore::new();
    let recovering = recovering_call(&mut expressions);
    let protected = catch_over(&mut expressions, recovering);
    // The catch visit validates `recovering` under recovery. The plain reference must still fail,
    // whichever order the two appear in.
    let roots = [
        bool_collection(
            &mut expressions,
            &mut type_environment,
            vec![protected, recovering],
        ),
        bool_collection(
            &mut expressions,
            &mut type_environment,
            vec![recovering, protected],
        ),
    ];
    let context = TypeValidationContext {
        expression_builder: &expressions,
        const_values: &constants,
        recovery_allowed: false,
        type_environment: &type_environment,
        template_ir_store: &store,
    };

    for root in roots {
        let error = validate_expression(&root, &context)
            .expect_err("a plain use of a recovering subject must be rejected");
        assert!(
            error
                .msg
                .contains("Recovering expression reached finalization outside ValueBlock::Catch")
        );
    }
}

#[test]
fn shared_template_overlay_dag_validates_on_the_tir_path() {
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let mut strings = StringTable::new();
    let mut type_environment = TypeEnvironment::new();
    let structural = expressions
        .string_slice(
            &mut constants,
            strings.intern("structural"),
            None,
            ValueMode::ImmutableOwned,
        )
        .expect("fixture literal")
        .id();
    let leaf = bool_reference(&mut expressions);
    let overlay = shared_chain(&mut expressions, &mut type_environment, leaf, 30);
    let mut store = TemplateIrStore::new();
    let template =
        template_with_dynamic_overlay(&mut store, structural, overlay, TemplateTirPhase::Finalized);
    let context = TypeValidationContext {
        expression_builder: &expressions,
        const_values: &constants,
        recovery_allowed: false,
        type_environment: &type_environment,
        template_ir_store: &store,
    };

    validate_template_expression_payloads(&template, &context)
        .expect("a shared well-typed overlay DAG validates");
}

#[test]
fn shared_template_overlay_dag_still_reports_an_invalid_leaf() {
    let mut expressions = AstExpressionBuilder::new().unwrap();
    let mut constants = ConstValueStore::default();
    let mut strings = StringTable::new();
    let mut type_environment = TypeEnvironment::new();
    let structural = expressions
        .string_slice(
            &mut constants,
            strings.intern("structural"),
            None,
            ValueMode::ImmutableOwned,
        )
        .expect("fixture literal")
        .id();
    let leaf = invalid_string_expression(&mut expressions, None);
    let overlay = shared_chain(&mut expressions, &mut type_environment, leaf, 30);
    let mut store = TemplateIrStore::new();
    let template =
        template_with_dynamic_overlay(&mut store, structural, overlay, TemplateTirPhase::Finalized);
    let context = TypeValidationContext {
        expression_builder: &expressions,
        const_values: &constants,
        recovery_allowed: false,
        type_environment: &type_environment,
        template_ir_store: &store,
    };

    let error = validate_template_expression_payloads(&template, &context)
        .expect_err("the shared invalid overlay leaf must be rejected");
    assert!(error.msg.contains("9999"));
}
