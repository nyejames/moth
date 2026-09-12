use super::*;
use crate::compiler_frontend::ast::const_values::resolver::classify_template_from_effective_tir;
use crate::compiler_frontend::ast::expressions::expression_types::ConstRecordState;
use crate::compiler_frontend::ast::statements::match_patterns::MatchPattern;
use crate::compiler_frontend::ast::templates::template::{
    SlotKey, TemplateConstValueKind, TemplateSegmentOrigin, TemplateType,
};
use crate::compiler_frontend::ast::templates::template_body_parser::{
    NestedTemplateParseOptions, TemplateBodyParseRequest, parse_template_body,
};
use crate::compiler_frontend::ast::templates::template_body_sentinels::TemplateBodyControlContext;
use crate::compiler_frontend::ast::templates::template_build_state::TemplateBuildState;
use crate::compiler_frontend::ast::templates::template_control_flow::{
    TemplateBranchSelector, TemplateControlFlowValidationMode,
    validate_runtime_template_control_flow_slot_artifacts,
};
#[cfg(feature = "benchmark_counters")]
use crate::compiler_frontend::ast::templates::template_folding::TemplateFoldResult;
use crate::compiler_frontend::ast::templates::template_head_parser::{
    TemplateHeadParseRequest, parse_template_head,
};
use crate::compiler_frontend::ast::templates::tir::TirExpressionOverlayId;
use crate::compiler_frontend::ast::templates::tir::refs::TemplateTirChildReference;
use crate::compiler_frontend::ast::templates::tir::{
    ExpressionSiteId, SlotOccurrenceId, TemplateConstructionContext, TemplateIrBranch,
    TemplateIrBuilder, TemplateIrId, TemplateIrNodeId, TemplateIrNodeKind, TemplateIrStore,
    TemplateIrSummary, TemplateLoopHeaderExpressionSites, TemplateTirPhase, TemplateTirReference,
    TemplateViewContext, TirExpressionOverlay, TirSlotResolution, TirSlotResolutionOverlay,
};
#[cfg(feature = "benchmark_counters")]
use crate::compiler_frontend::ast::templates::tir::{
    TemplatePreparationOutcome, TirView, fold_prepared_template,
};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::ast::{
    FileValueResolutionServices, ScopeContext, Stage0ResolutionFacts, TopLevelDeclarationTable,
};
use crate::compiler_frontend::compiler_messages::{
    DiagnosticPayload, InvalidExpressionReason, InvalidTemplateStructureReason, NameNamespace,
};
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::definitions::{FieldDefinition, StructTypeDefinition};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::{NominalTypeId, TypeId, builtin_type_ids};
use crate::compiler_frontend::headers::binding_environment::FileVisibility;
use crate::compiler_frontend::headers::synthetic_content_header::content_constant_path;
use crate::compiler_frontend::paths::file_references::{
    PreparedFileReferenceClass, ResolvedFileReference, ResolvedFileReferenceOutcome,
    ResolvedFileReferenceTable, ResolvedFileReferenceTarget,
};
use crate::compiler_frontend::paths::module_resources::ModuleResourceTable;
use crate::compiler_frontend::source::FrozenIdentityHandle;
use crate::compiler_frontend::source::SourceDatabase;
use crate::compiler_frontend::source::{SourceId, SourceSpan};
use crate::compiler_frontend::style_directives::{
    StyleDirectiveHandlerSpec, StyleDirectiveRegistry, StyleDirectiveSpec,
    TemplateHeadCompatibility, TemplateHeadTag,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, TokenKind};
use crate::compiler_frontend::type_coercion::compatibility::TypeCompatibilityCache;
use rustc_hash::{FxHashMap, FxHashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
fn synthetic_source_span(start: u32, end: u32) -> SourceSpan {
    let mut span_builder = ExtendedSpanBuilder::new();
    SourceSpan::new(
        SourceId::COMPILATION_ROOT,
        LocalSpan::exact(start, end - start, &mut span_builder)
            .expect("synthetic test span should fit"),
    )
}

fn assert_stale_template_directive_argument_is_infrastructure(source: &str) {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut token_stream =
        template_tokens_from_source(source, &mut string_table, &mut span_builder);
    let scope = token_stream.src_path.clone();
    let stale_name = string_table.intern("stale_template");
    let stale_template = Template {
        tir_reference: TemplateTirReference {
            root: TemplateIrId::new(99),
            phase: TemplateTirPhase::Parsed,
            context: TemplateViewContext::default(),
        },
        span: None,
    };
    let declaration = Declaration {
        id: path_fork
            .try_intern_child(scope, stale_name)
            .expect("stale declaration path should intern"),
        value: Expression::template(stale_template, ValueMode::ImmutableOwned),
        binding_span: None,
        config_qualifier: None,
    };
    let style_directives = frontend_test_style_directives();
    let context = with_test_path_context(
        ScopeContext::new_for_tests(
            ContextKind::Template,
            scope.clone(),
            Rc::new(TopLevelDeclarationTable::new(vec![declaration], &path_fork)),
            Arc::new(ExternalPackageRegistry::default()),
            vec![],
            0,
        ),
        &scope,
        &style_directives,
    );

    let error =
        Template::new(&mut token_stream, &context, vec![], &mut string_table, &mut path_fork)
        .expect_err("stale directive argument authority must fail during expression parsing");
    let TemplateError::Infrastructure(error) = error else {
        panic!("stale directive argument authority must remain an infrastructure failure");
    };
    assert!(
        error.msg.contains("missing same-store template"),
        "unexpected infrastructure error: {error:?}"
    );
}

fn imported_const_template_context(
    scope: &PathId,
    declaration: Declaration,
    visible_name: StringId,
) -> ScopeContext {
    let mut visible_declarations = FxHashSet::default();
    visible_declarations.insert(declaration.id.clone());

    let mut visible_bindings = FxHashMap::default();
    visible_bindings.insert(
        visible_name,
        crate::compiler_frontend::headers::binding_environment::SourceDeclarationTarget::Local(
            declaration.id.clone(),
        ),
    );

    // Production scopes install one header-built `FileVisibility` package rather than
    // assembling it field by field, so the fixture does the same.
    let file_visibility = FileVisibility {
        visible_declaration_paths: Arc::new(visible_declarations),
        visible_source_names: visible_bindings,
        ..FileVisibility::default()
    };

    constant_template_context(scope, &[declaration]).with_file_visibility(Arc::new(file_visibility))
}

/// Builds a const-required option-capture template fixture directly as a
/// module-local TIR branch-chain root in the context's module store.
///
/// WHAT: constructs the branch body (text "Hello " plus the capture reference)
///       and fallback body (text "Guest") as TIR nodes, wraps them in a
///       `BranchChain` node, finishes the template record, and returns a
///       `Template` whose `tir_reference` points at that root.
/// WHY: manual fixtures no longer need detached content or its TIR materializer.
///      Validation and folding already consume the authoritative
///      branch-chain root through the module-store `TirView`.
fn const_required_option_capture_template_with_direct_tir(
    scrutinee: Expression,
    capture_name: StringId,
    capture_path: PathId,
    inner_type_id: TypeId,
    context: &ScopeContext,
    string_table: &mut StringTable,
) -> Template {
    let span = None;

    let capture_reference = Expression::reference_with_type_id(
        capture_path.clone(),
        DataType::StringSlice,
        inner_type_id,
        span,
        ValueMode::ImmutableOwned,
        ConstRecordState::RuntimeValue,
    );

    let hello_id = string_table.intern("Hello ");
    let guest_id = string_table.intern("Guest");

    let store_handle = context.template_ir_store();

    let template_id = {
        let mut store = store_handle.borrow_mut();
        let mut builder = TemplateIrBuilder::new(&mut store);

        let hello_node =
            builder.push_text_node(hello_id, "Hello ".len(), TemplateSegmentOrigin::Body, span);
        let capture_node = builder.push_dynamic_expression_node(
            capture_reference,
            TemplateSegmentOrigin::Body,
            None,
            span,
        );
        let branch_body = builder.push_sequence_node(vec![hello_node, capture_node], span);

        let guest_node =
            builder.push_text_node(guest_id, "Guest".len(), TemplateSegmentOrigin::Body, span);
        let fallback_body = builder.push_sequence_node(vec![guest_node], span);

        let selector = TemplateBranchSelector::OptionPresentCapture {
            scrutinee,
            pattern: Box::new(MatchPattern::OptionPresentCapture {
                name: capture_name,
                binding_path: capture_path,
                inner_type_id,
                span,
                binding_span: None,
            }),
        };
        let branch = TemplateIrBranch::new(
            selector,
            branch_body,
            span,
            builder.store.next_expression_site_id(),
        );
        let branch_chain_root =
            builder.push_branch_chain_node(vec![branch], Some(fallback_body), None, span);

        let summary = TemplateIrSummary {
            estimated_output_bytes: "Hello ".len() + "Guest".len(),
            text_node_count: 2,
            text_byte_count: "Hello ".len() + "Guest".len(),
            dynamic_expression_count: 1,
            max_depth: 2,
            has_control_flow: true,
            ..TemplateIrSummary::default()
        };

        builder.finish_template(
            branch_chain_root,
            Style::default(),
            TemplateType::String,
            summary,
            span,
        )
    };

    let context = TemplateViewContext::default();

    Template {
        tir_reference: TemplateTirReference {
            root: template_id,
            phase: TemplateTirPhase::Composed,
            context,
        },
        span: None,
    }
}

fn parse_template_error(
    source: &str,
) -> crate::compiler_frontend::compiler_messages::CompilerDiagnostic {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut token_stream =
        template_tokens_from_source(source, &mut string_table, &mut span_builder);
    let context = new_constant_context(token_stream.src_path.clone());

    expect_template_diagnostic(
        Template::new(&mut token_stream, &context, vec![], &mut string_table, &mut path_fork)
            .expect_err("template source should fail"),
    )
}

fn parse_runtime_template(source: &str) -> (Template, ScopeContext, StringTable) {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut token_stream =
        template_tokens_from_source(source, &mut string_table, &mut span_builder);
    let context = new_constant_context(token_stream.src_path.clone());

    let template =
        Template::new(&mut token_stream, &context, vec![], &mut string_table, &mut path_fork)
        .expect("template source should parse");

    (template, context, string_table)
}

fn parse_control_flow_template_after_body_parse(
    source: &str,
) -> (Template, ScopeContext, StringTable) {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut token_stream =
        template_tokens_from_source(source, &mut string_table, &mut span_builder);
    let context = new_constant_context(token_stream.src_path.clone());

    let mut type_environment = TypeEnvironment::new();
    let mut compatibility_cache = TypeCompatibilityCache::new();
    let mut type_interner = AstTypeInterner::new(&mut type_environment, &mut compatibility_cache);

    let mut build_state = TemplateBuildState::new();

    let mut construction_context = TemplateConstructionContext::new(
        context.template_ir_store.clone(),
        Some(token_stream.current_span()),
    );

    let parsed_head = parse_template_head(
        &mut token_stream,
        TemplateHeadParseRequest {
            context: &context,
            type_interner: &mut type_interner,
            build_state: &mut build_state,
            construction_context: &mut construction_context,
            control_flow_validation: TemplateControlFlowValidationMode::RuntimeCapable,
            string_table: &mut string_table,
            path_fork: &mut path_fork,
        },
    )
    .expect("template head should parse");

    parse_template_body(
        &mut token_stream,
        &mut build_state,
        &mut construction_context,
        TemplateBodyParseRequest {
            context: &context,
            type_interner: &mut type_interner,
            body_mode: parsed_head.body_mode,
            direct_child_wrappers: &[],
            control_flow_validation: TemplateControlFlowValidationMode::RuntimeCapable,
            control_context: TemplateBodyControlContext::normal(),
            string_table: &mut string_table,
            path_fork: &mut path_fork,
            default_style: None,
        },
    )
    .expect("template body should parse");

    let span = construction_context.span();
    let tir_reference = construction_context
        .finish(
            build_state.style.clone(),
            build_state.kind.clone(),
            crate::compiler_frontend::ast::templates::tir::TemplateTirPhase::Parsed,
        )
        .expect("parsed template TIR is finite");

    let template = Template {
        tir_reference,
        span,
    };

    (template, context, string_table)
}

fn parse_control_flow_template_after_composition(
    source: &str,
) -> (Template, ScopeContext, StringTable) {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut token_stream =
        template_tokens_from_source(source, &mut string_table, &mut span_builder);
    let context = new_constant_context(token_stream.src_path.clone());

    let mut type_environment = TypeEnvironment::new();
    let mut compatibility_cache = TypeCompatibilityCache::new();
    let mut type_interner = AstTypeInterner::new(&mut type_environment, &mut compatibility_cache);

    let template = Template::new_nested_template(
        &mut token_stream,
        &context,
        &mut type_interner,
        Vec::new(),
        &mut string_table,
        NestedTemplateParseOptions::runtime_capable(),
        &mut path_fork,
    )
    .expect("control-flow template should parse through composition")
    .template;

    (template, context, string_table)
}

fn parse_control_flow_template_after_composition_error(
    source: &str,
) -> crate::compiler_frontend::compiler_messages::CompilerDiagnostic {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut token_stream =
        template_tokens_from_source(source, &mut string_table, &mut span_builder);
    let context = new_constant_context(token_stream.src_path.clone());

    let mut type_environment = TypeEnvironment::new();
    let mut compatibility_cache = TypeCompatibilityCache::new();
    let mut type_interner = AstTypeInterner::new(&mut type_environment, &mut compatibility_cache);

    expect_template_diagnostic(
        Template::new_nested_template(
            &mut token_stream,
            &context,
            &mut type_interner,
            Vec::new(),
            &mut string_table,
            NestedTemplateParseOptions::runtime_capable(),
            &mut path_fork,
        )
        .expect_err("control-flow template should fail during composition"),
    )
}

fn parse_runtime_template_without_validation(
    source: &str,
) -> (Template, ScopeContext, StringTable) {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut token_stream =
        template_tokens_from_source(source, &mut string_table, &mut span_builder);
    let context = new_constant_context(token_stream.src_path.clone());

    let mut type_environment = TypeEnvironment::new();
    let mut compatibility_cache = TypeCompatibilityCache::new();
    let mut type_interner = AstTypeInterner::new(&mut type_environment, &mut compatibility_cache);

    let mut build_state = TemplateBuildState::new();

    let mut construction_context = TemplateConstructionContext::new(
        context.template_ir_store.clone(),
        Some(token_stream.current_span()),
    );

    let parsed_head = parse_template_head(
        &mut token_stream,
        TemplateHeadParseRequest {
            context: &context,
            type_interner: &mut type_interner,
            build_state: &mut build_state,
            construction_context: &mut construction_context,
            control_flow_validation: TemplateControlFlowValidationMode::RuntimeCapable,
            string_table: &mut string_table,
            path_fork: &mut path_fork,
        },
    )
    .expect("template head should parse");

    parse_template_body(
        &mut token_stream,
        &mut build_state,
        &mut construction_context,
        TemplateBodyParseRequest {
            context: &context,
            type_interner: &mut type_interner,
            body_mode: parsed_head.body_mode,
            direct_child_wrappers: &[],
            control_flow_validation: TemplateControlFlowValidationMode::RuntimeCapable,
            control_context: TemplateBodyControlContext::normal(),
            string_table: &mut string_table,
            path_fork: &mut path_fork,
            default_style: None,
        },
    )
    .expect("template body should parse");

    // Finish the construction context to install a module-store `tir_reference`
    // without running render-unit preparation or runtime validation. This lets
    // focused tests call the view-based runtime validator directly.
    let style = build_state.style.to_owned();
    let kind = build_state.kind.to_owned();
    let span = construction_context.span();
    let tir_reference = construction_context
        .finish(style, kind, TemplateTirPhase::Parsed)
        .expect("parsed template TIR is finite");

    let template = Template {
        tir_reference,
        span,
    };

    (template, context, string_table)
}

/// Prepares an already-constructed template's const view the way construction does.
///
/// WHAT: builds the same Composed-or-later `TirView` from the template's durable reference that
///       `Template::new_nested_template` builds at its preparation stage, then runs the same
///       production `prepare_tir_view` in `ConstRequired` mode.
/// WHY:  production reaches const preparation exactly once, inside construction, and carries the
///       result into folding. A test that mutates a template after construction — installing an
///       expression overlay, or corrupting a root node — has no production entry point that will
///       re-classify it. This helper composes the two production functions rather than
///       duplicating their logic, and lives in the test module so no production module carries a
///       validation entry that production never calls.
fn prepare_const_required_view_directly(
    template: &Template,
    tir_store: &TemplateIrStore,
) -> Result<TemplatePreparation, TemplateError> {
    let reference = &template.tir_reference;
    let view = TirView::with_minimum_phase(
        tir_store,
        reference.root,
        reference.phase,
        TemplateTirPhase::Composed,
        reference.context,
    )
    .map_err(TemplateError::from)?;

    prepare_tir_view(&view, TemplatePreparationMode::ConstRequired)
}

/// Constructs a const-required template and keeps the preparation construction carried.
///
/// `parse_const_required_template` drops the preparation because most callers only need the
/// handle. A test whose subject is the const classification itself must keep it: it is the
/// value production passes to folding.
fn const_required_construction(source: &str) -> PreparedTemplateConstruction {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut token_stream =
        template_tokens_from_source(source, &mut string_table, &mut span_builder);
    let context = new_constant_context(token_stream.src_path.clone());

    Template::new_const_required(&mut token_stream, &context, vec![], &mut string_table, &mut path_fork)
        .expect("const-required template should parse")
}

fn parse_const_required_template(source: &str) -> (Template, ScopeContext, StringTable) {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut token_stream =
        template_tokens_from_source(source, &mut string_table, &mut span_builder);
    let context = new_constant_context(token_stream.src_path.clone());

    let template =
        Template::new_const_required(&mut token_stream, &context, vec![], &mut string_table, &mut path_fork)
            .expect("const-required template should parse")
            .template;

    (template, context, string_table)
}

fn parse_const_required_template_error(
    source: &str,
) -> crate::compiler_frontend::compiler_messages::CompilerDiagnostic {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut span_builder = ExtendedSpanBuilder::new();
    let mut token_stream =
        template_tokens_from_source(source, &mut string_table, &mut span_builder);
    let context = new_constant_context(token_stream.src_path.clone());

    expect_template_diagnostic(
        Template::new_const_required(&mut token_stream, &context, vec![], &mut string_table, &mut path_fork)
            .expect_err("const-required template source should fail"),
    )
}

fn assert_internal_template_error_contains(error: TemplateError, expected_message: &str) {
    let TemplateError::Infrastructure(error) = error else {
        panic!("malformed template authority should remain an infrastructure error");
    };
    assert!(
        error.msg.contains(expected_message),
        "error message should contain {expected_message:?}, got: {}",
        error.msg
    );
}

fn find_first_branch_selector_site_id(
    template: &Template,
    store: &TemplateIrStore,
) -> Option<ExpressionSiteId> {
    let reference = &template.tir_reference;
    let template_ir = store.get_template(reference.root)?;
    find_branch_selector_site_id_in_subtree(store, template_ir.root)
}

fn find_branch_selector_site_id_in_subtree(
    store: &TemplateIrStore,
    node_id: TemplateIrNodeId,
) -> Option<ExpressionSiteId> {
    let node = store.get_node(node_id)?;
    match &node.kind {
        TemplateIrNodeKind::BranchChain { branches, .. } => {
            branches.first().map(|branch| branch.selector_site_id)
        }
        TemplateIrNodeKind::Sequence { children } => children
            .iter()
            .find_map(|child| find_branch_selector_site_id_in_subtree(store, *child)),
        _ => None,
    }
}

fn find_first_branch_span(template: &Template, store: &TemplateIrStore) -> Option<SourceSpan> {
    let reference = &template.tir_reference;
    let template_ir = store.get_template(reference.root)?;
    find_branch_span_in_subtree(store, template_ir.root)
}

fn find_branch_span_in_subtree(
    store: &TemplateIrStore,
    node_id: TemplateIrNodeId,
) -> Option<SourceSpan> {
    let node = store.get_node(node_id)?;
    match &node.kind {
        TemplateIrNodeKind::BranchChain { branches, .. } => {
            branches.first().and_then(|branch| branch.span)
        }
        TemplateIrNodeKind::Sequence { children } => children
            .iter()
            .find_map(|child| find_branch_span_in_subtree(store, *child)),
        _ => None,
    }
}

fn find_first_loop_header_site_id(
    template: &Template,
    store: &TemplateIrStore,
) -> Option<ExpressionSiteId> {
    let reference = &template.tir_reference;
    let template_ir = store.get_template(reference.root)?;
    find_loop_header_site_id_in_subtree(store, template_ir.root)
}

fn find_loop_header_site_id_in_subtree(
    store: &TemplateIrStore,
    node_id: TemplateIrNodeId,
) -> Option<ExpressionSiteId> {
    let node = store.get_node(node_id)?;
    match &node.kind {
        TemplateIrNodeKind::Loop {
            header_sites: TemplateLoopHeaderExpressionSites::Conditional { condition },
            ..
        } => Some(*condition),
        TemplateIrNodeKind::Sequence { children } => children
            .iter()
            .find_map(|child| find_loop_header_site_id_in_subtree(store, *child)),
        _ => None,
    }
}

fn install_expression_overlay_on_template(
    template: &mut Template,
    store: &mut TemplateIrStore,
    site_id: ExpressionSiteId,
    expression: Expression,
) {
    let overlay_id = store
        .allocate_expression_overlay(TirExpressionOverlay {
            overrides: vec![(site_id, Box::new(expression))],
        })
        .expect("test overlay allocation");
    let context = TemplateViewContext {
        expression_overlay: Some(overlay_id),
        slot_resolution: None,
        wrapper_context: None,
    };

    {
        let reference = &mut template.tir_reference;
        reference.phase = TemplateTirPhase::Finalized;
        reference.context = context;
    }
}

fn find_first_slot_occurrence_id(
    template: &Template,
    store: &TemplateIrStore,
) -> Option<(SlotOccurrenceId, SlotKey)> {
    let reference = &template.tir_reference;
    let template_ir = store.get_template(reference.root)?;
    find_slot_occurrence_id_in_subtree(store, template_ir.root)
}

fn find_slot_occurrence_id_in_subtree(
    store: &TemplateIrStore,
    node_id: TemplateIrNodeId,
) -> Option<(SlotOccurrenceId, SlotKey)> {
    let node = store.get_node(node_id)?;
    match &node.kind {
        TemplateIrNodeKind::Slot { placeholder } => {
            Some((placeholder.occurrence_id, placeholder.key.clone()))
        }
        TemplateIrNodeKind::Sequence { children } => children
            .iter()
            .find_map(|child| find_slot_occurrence_id_in_subtree(store, *child)),
        TemplateIrNodeKind::BranchChain {
            branches, fallback, ..
        } => branches
            .iter()
            .find_map(|branch| find_slot_occurrence_id_in_subtree(store, branch.body))
            .or_else(|| {
                fallback.and_then(|fallback| find_slot_occurrence_id_in_subtree(store, fallback))
            }),
        TemplateIrNodeKind::Loop {
            body,
            aggregate_wrapper,
            ..
        } => find_slot_occurrence_id_in_subtree(store, *body).or_else(|| {
            aggregate_wrapper.and_then(|wrapper| find_slot_occurrence_id_in_subtree(store, wrapper))
        }),
        TemplateIrNodeKind::ChildTemplate { reference, .. } => {
            let template_id = reference.root;
            let template_ir = store.get_template(template_id)?;
            find_slot_occurrence_id_in_subtree(store, template_ir.root)
        }
        TemplateIrNodeKind::InsertContribution { template } => {
            let template_ir = store.get_template(*template)?;
            find_slot_occurrence_id_in_subtree(store, template_ir.root)
        }
        _ => None,
    }
}

fn install_slot_resolution_overlay_on_template(
    template: &mut Template,
    store: &mut TemplateIrStore,
    occurrence_id: SlotOccurrenceId,
    resolution: TirSlotResolution,
) {
    let overlay_id = store
        .allocate_slot_resolution_overlay(TirSlotResolutionOverlay {
            resolutions: vec![(occurrence_id, resolution)],
        })
        .expect("test overlay allocation");
    let context = TemplateViewContext {
        expression_overlay: None,
        slot_resolution: Some(overlay_id),
        wrapper_context: None,
    };

    {
        let reference = &mut template.tir_reference;
        reference.phase = TemplateTirPhase::Finalized;
        reference.context = context;
    }
}

fn body_node_static_text(
    body_node: TemplateIrNodeId,
    context: &ScopeContext,
    string_table: &StringTable,
) -> String {
    let store = context.template_ir_store.borrow();
    let mut rendered = String::new();
    collect_static_tir_fragments(body_node, &store, string_table, &mut rendered);
    rendered
}

fn collect_static_tir_fragments(
    node_id: crate::compiler_frontend::ast::templates::tir::TemplateIrNodeId,
    store: &TemplateIrStore,
    string_table: &StringTable,
    output: &mut String,
) {
    let Some(node) = store.get_node(node_id) else {
        return;
    };

    match &node.kind {
        TemplateIrNodeKind::Sequence { children } => {
            for child in children {
                collect_static_tir_fragments(*child, store, string_table, output);
            }
        }

        TemplateIrNodeKind::Text { text, .. } => output.push_str(string_table.resolve(*text)),

        TemplateIrNodeKind::DynamicExpression { expression, .. } => {
            if let ExpressionKind::StringSlice(value) = &expression.kind {
                output.push_str(string_table.resolve(*value));
            }
        }

        TemplateIrNodeKind::ChildTemplate { reference, .. } => {
            let child_id = reference.root;
            if let Some(template) = store.get_template(child_id) {
                collect_static_tir_fragments(template.root, store, string_table, output);
            }
        }
        TemplateIrNodeKind::InsertContribution { template } => {
            if let Some(template) = store.get_template(*template) {
                collect_static_tir_fragments(template.root, store, string_table, output);
            }
        }

        TemplateIrNodeKind::BranchChain {
            branches, fallback, ..
        } => {
            for branch in branches {
                collect_static_tir_fragments(branch.body, store, string_table, output);
            }
            if let Some(fallback) = fallback {
                collect_static_tir_fragments(*fallback, store, string_table, output);
            }
        }

        TemplateIrNodeKind::Loop {
            body,
            aggregate_wrapper,
            ..
        } => {
            collect_static_tir_fragments(*body, store, string_table, output);
            if let Some(aggregate_wrapper) = aggregate_wrapper {
                collect_static_tir_fragments(*aggregate_wrapper, store, string_table, output);
            }
        }

        TemplateIrNodeKind::Slot { .. }
        | TemplateIrNodeKind::AggregateOutput
        | TemplateIrNodeKind::LoopControl { .. }
        | TemplateIrNodeKind::RuntimeSlotSite { .. }
        | TemplateIrNodeKind::RuntimeSlotContributionSource { .. } => {}
    }
}

fn body_node_contains_unresolved_slots(
    body_node: TemplateIrNodeId,
    context: &ScopeContext,
) -> bool {
    let store = context.template_ir_store.borrow();
    tir_subtree_contains_slot(body_node, &store)
}

fn tir_subtree_contains_slot(
    node_id: crate::compiler_frontend::ast::templates::tir::TemplateIrNodeId,
    store: &TemplateIrStore,
) -> bool {
    let Some(node) = store.get_node(node_id) else {
        return false;
    };

    match &node.kind {
        TemplateIrNodeKind::Slot { .. } => true,

        TemplateIrNodeKind::Sequence { children } => children
            .iter()
            .any(|child| tir_subtree_contains_slot(*child, store)),

        TemplateIrNodeKind::ChildTemplate { reference, .. } => store
            .get_template(reference.root)
            .is_some_and(|template| tir_subtree_contains_slot(template.root, store)),
        TemplateIrNodeKind::InsertContribution { template } => store
            .get_template(*template)
            .is_some_and(|template| tir_subtree_contains_slot(template.root, store)),

        TemplateIrNodeKind::BranchChain {
            branches, fallback, ..
        } => {
            branches
                .iter()
                .any(|branch| tir_subtree_contains_slot(branch.body, store))
                || fallback.is_some_and(|fallback| tir_subtree_contains_slot(fallback, store))
        }

        TemplateIrNodeKind::Loop {
            body,
            aggregate_wrapper,
            ..
        } => {
            tir_subtree_contains_slot(*body, store)
                || aggregate_wrapper
                    .is_some_and(|wrapper| tir_subtree_contains_slot(wrapper, store))
        }

        TemplateIrNodeKind::Text { .. }
        | TemplateIrNodeKind::DynamicExpression { .. }
        | TemplateIrNodeKind::AggregateOutput
        | TemplateIrNodeKind::LoopControl { .. }
        | TemplateIrNodeKind::RuntimeSlotSite { .. }
        | TemplateIrNodeKind::RuntimeSlotContributionSource { .. } => false,
    }
}

fn body_node_loop_control_signal_count(
    body_node: TemplateIrNodeId,
    context: &ScopeContext,
) -> usize {
    let store = context.template_ir_store.borrow();
    count_tir_loop_control_signals(body_node, &store)
}

fn count_tir_loop_control_signals(
    node_id: crate::compiler_frontend::ast::templates::tir::TemplateIrNodeId,
    store: &TemplateIrStore,
) -> usize {
    let Some(node) = store.get_node(node_id) else {
        return 0;
    };

    match &node.kind {
        TemplateIrNodeKind::LoopControl { .. } => 1,

        TemplateIrNodeKind::Sequence { children } => children
            .iter()
            .map(|child| count_tir_loop_control_signals(*child, store))
            .sum(),

        TemplateIrNodeKind::ChildTemplate { reference, .. } => {
            store.get_template(reference.root).map_or(0, |template| {
                count_tir_loop_control_signals(template.root, store)
            })
        }
        TemplateIrNodeKind::InsertContribution { template } => {
            store.get_template(*template).map_or(0, |template| {
                count_tir_loop_control_signals(template.root, store)
            })
        }

        TemplateIrNodeKind::BranchChain {
            branches, fallback, ..
        } => {
            branches
                .iter()
                .map(|branch| count_tir_loop_control_signals(branch.body, store))
                .sum::<usize>()
                + fallback.map_or(0, |fallback| {
                    count_tir_loop_control_signals(fallback, store)
                })
        }

        TemplateIrNodeKind::Loop {
            body,
            aggregate_wrapper,
            ..
        } => {
            count_tir_loop_control_signals(*body, store)
                + aggregate_wrapper
                    .map_or(0, |wrapper| count_tir_loop_control_signals(wrapper, store))
        }

        TemplateIrNodeKind::Text { .. }
        | TemplateIrNodeKind::DynamicExpression { .. }
        | TemplateIrNodeKind::Slot { .. }
        | TemplateIrNodeKind::AggregateOutput
        | TemplateIrNodeKind::RuntimeSlotSite { .. }
        | TemplateIrNodeKind::RuntimeSlotContributionSource { .. } => 0,
    }
}

fn assert_body_node_static_contains(
    body_node: TemplateIrNodeId,
    context: &ScopeContext,
    string_table: &StringTable,
    expected: &str,
) {
    let rendered = body_node_static_text(body_node, context, string_table);
    assert!(
        rendered.contains(expected),
        "expected {rendered:?} to contain {expected:?}"
    );
}

fn assert_body_node_static_excludes(
    body_node: TemplateIrNodeId,
    context: &ScopeContext,
    string_table: &StringTable,
    unexpected: &str,
) {
    let rendered = body_node_static_text(body_node, context, string_table);
    assert!(
        !rendered.contains(unexpected),
        "expected {rendered:?} to exclude {unexpected:?}"
    );
}

fn assert_invalid_template_structure(
    diagnostic: &crate::compiler_frontend::compiler_messages::CompilerDiagnostic,
    expected_reason: InvalidTemplateStructureReason,
) {
    match &diagnostic.payload {
        DiagnosticPayload::InvalidTemplateStructure { reason } => {
            assert_eq!(*reason, expected_reason);
        }
        payload => panic!("expected invalid template structure payload, found {payload:?}"),
    }
}

fn expect_branch_chain_node(template: &Template, context: &ScopeContext) -> TemplateIrNodeId {
    let store = context.template_ir_store.borrow();
    let template_id = template.tir_reference.root;
    let control_flow_node_id = store
        .control_flow_node_id_for_template(template_id)
        .expect("control-flow lookup")
        .expect("template should contain a control-flow node");
    let node = store
        .get_node(control_flow_node_id)
        .expect("control-flow node should exist in the store");
    assert!(
        matches!(node.kind, TemplateIrNodeKind::BranchChain { .. }),
        "expected BranchChain control-flow node"
    );
    control_flow_node_id
}

fn expect_loop_node(template: &Template, context: &ScopeContext) -> TemplateIrNodeId {
    let store = context.template_ir_store.borrow();
    let template_id = template.tir_reference.root;
    let control_flow_node_id = store
        .control_flow_node_id_for_template(template_id)
        .expect("control-flow lookup")
        .expect("template should contain a control-flow node");
    let node = store
        .get_node(control_flow_node_id)
        .expect("control-flow node should exist in the store");
    assert!(
        matches!(node.kind, TemplateIrNodeKind::Loop { .. }),
        "expected Loop control-flow node"
    );
    control_flow_node_id
}

fn first_branch_body_node(
    branch_chain_node: TemplateIrNodeId,
    context: &ScopeContext,
) -> TemplateIrNodeId {
    branch_body_node(branch_chain_node, 0, context)
}

fn branch_body_node(
    branch_chain_node: TemplateIrNodeId,
    index: usize,
    context: &ScopeContext,
) -> TemplateIrNodeId {
    let store = context.template_ir_store.borrow();
    let node = store
        .get_node(branch_chain_node)
        .expect("branch chain node should exist");
    let TemplateIrNodeKind::BranchChain { branches, .. } = &node.kind else {
        panic!("expected BranchChain node");
    };
    branches
        .get(index)
        .unwrap_or_else(|| panic!("branch chain should contain branch {index}"))
        .body
}

fn fallback_body_node(
    branch_chain_node: TemplateIrNodeId,
    context: &ScopeContext,
) -> TemplateIrNodeId {
    let store = context.template_ir_store.borrow();
    let node = store
        .get_node(branch_chain_node)
        .expect("branch chain node should exist");
    let TemplateIrNodeKind::BranchChain { fallback, .. } = &node.kind else {
        panic!("expected BranchChain node");
    };
    fallback
        .as_ref()
        .copied()
        .expect("branch chain should contain fallback")
}

fn loop_body_node(loop_node: TemplateIrNodeId, context: &ScopeContext) -> TemplateIrNodeId {
    let store = context.template_ir_store.borrow();
    let node = store.get_node(loop_node).expect("loop node should exist");
    let TemplateIrNodeKind::Loop { body, .. } = &node.kind else {
        panic!("expected Loop node");
    };
    *body
}

fn branch_count(branch_chain_node: TemplateIrNodeId, context: &ScopeContext) -> usize {
    let store = context.template_ir_store.borrow();
    let node = store
        .get_node(branch_chain_node)
        .expect("branch chain node should exist");
    let TemplateIrNodeKind::BranchChain { branches, .. } = &node.kind else {
        panic!("expected BranchChain node");
    };
    branches.len()
}

fn loop_aggregate_wrapper_node(
    loop_node: TemplateIrNodeId,
    context: &ScopeContext,
) -> TemplateIrNodeId {
    let store = context.template_ir_store.borrow();
    let node = store.get_node(loop_node).expect("loop node should exist");
    let TemplateIrNodeKind::Loop {
        aggregate_wrapper, ..
    } = &node.kind
    else {
        panic!("expected Loop node");
    };
    aggregate_wrapper
        .as_ref()
        .copied()
        .expect("loop should have an aggregate wrapper installed")
}

/// Returns true when the TIR subtree rooted at `node_id` contains a
/// `BranchChain` or `Loop` node (i.e. a control-flow child template).
fn tir_subtree_contains_control_flow(node_id: TemplateIrNodeId, store: &TemplateIrStore) -> bool {
    let Some(node) = store.get_node(node_id) else {
        return false;
    };
    match &node.kind {
        TemplateIrNodeKind::BranchChain { .. } | TemplateIrNodeKind::Loop { .. } => true,
        TemplateIrNodeKind::Sequence { children } => children
            .iter()
            .any(|child| tir_subtree_contains_control_flow(*child, store)),
        TemplateIrNodeKind::ChildTemplate { reference, .. } => store
            .get_template(reference.root)
            .is_some_and(|child_ir| tir_subtree_contains_control_flow(child_ir.root, store)),
        _ => false,
    }
}

/// Returns true when the template's TIR root contains a `ChildTemplate` node
/// whose referenced child template has control flow.
fn tir_root_has_control_flow_child(template: &Template, store: &TemplateIrStore) -> bool {
    let reference = &template.tir_reference;
    let Some(tir_template) = store.get_template(reference.root) else {
        return false;
    };
    tir_subtree_contains_control_flow(tir_template.root, store)
}

#[path = "template_head_tests.rs"]
mod template_head_tests;

#[path = "reactive_tests.rs"]
mod reactive_tests;

#[path = "handler_runtime_tests.rs"]
mod handler_runtime_tests;

#[path = "control_flow_tests.rs"]
mod control_flow_tests;
