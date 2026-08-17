//! Template node construction orchestrator.
//!
//! WHAT: Provides `Template::new()` — the main entry point for creating a
//! template AST node from a token stream. Delegates to focused submodules
//! for head parsing, body parsing, composition, formatting, and folding.
//!
//! WHY: Template construction crosses several tightly ordered responsibilities. Keeping the
//! orchestration here and the implementation details in sibling modules makes the stage boundary
//! explicit without rebuilding template state in later frontend phases.
//!
//! ## Runtime metadata ownership
//!
//! `Template::new()` is the authoritative owner of final runtime template metadata.
//! It finalizes the parser-owned TIR root and writes the classified kind to the
//! owning TIR entry before returning. AST finalization consumes that TIR reference
//! rather than rebuilding parser structure.

use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::styles::markdown::markdown_formatter;
use crate::compiler_frontend::ast::templates::template::Template;
use crate::compiler_frontend::ast::templates::template::{
    BodyWhitespacePolicy, CommentDirectiveKind, Style, TemplateParsingMode, TemplateType,
};
use crate::compiler_frontend::ast::templates::template_body_parser::{
    NestedTemplateParseOptions, TemplateBodyParseRequest, parse_template_body,
};
use crate::compiler_frontend::ast::templates::template_build_state::TemplateBuildState;
use crate::compiler_frontend::ast::templates::template_control_flow::{
    TemplateControlFlowValidationMode, validate_const_required_template_control_flow,
    validate_runtime_template_control_flow_slot_artifacts,
};
use crate::compiler_frontend::ast::templates::template_head_parser::{
    ParsedTemplateHead, TemplateHeadParseRequest, apply_doc_comment_defaults, parse_template_head,
};
use crate::compiler_frontend::ast::templates::template_render_units::{
    ControlFlowRenderUnitRequest, install_formatted_tir_reference_for_linear_template,
    prepare_control_flow_render_units,
};
use crate::compiler_frontend::ast::templates::tir::{
    TemplateConstructionContext, TemplatePreparation, TemplatePreparationMode, TemplateTirPhase,
    TemplateTirReference, TemplateWrapperReference, TirView, attach_wrapper_context_overlay,
    compose_tir_head_chain_from_root, prepare_tir_view,
};

use crate::builder_surface::SourceFileKind;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidTemplateSlotReason, InvalidTemplateStructureReason,
};
use crate::compiler_frontend::instrumentation::{
    AstCounter, FrontendCounter, add_ast_counter, increment_frontend_counter,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::FileTokens;
#[cfg(test)]
use crate::compiler_frontend::{
    datatypes::environment::TypeEnvironment, type_coercion::compatibility::TypeCompatibilityCache,
};

const SYNTHETIC_CONTENT_CONSTANT_NAME: &str = "content";

/// Template construction is the durable local error boundary for head/body/TIR work. It carries
/// source diagnostics and internal TIR or retained-syntax failures separately until the AST
/// caller selects its reporting lane.
type TemplateConstructionResult = Result<Template, TemplateError>;

/// The immediate result of const-required construction.
///
/// `Template` remains the durable two-field handle. The preparation is carried
/// only across the construction-to-fold boundary because it proves the exact
/// view that construction just validated; storing it on the handle would make
/// preparation part of durable template identity.
#[derive(Debug)]
pub(crate) struct ConstRequiredTemplateConstruction {
    pub(crate) template: Template,
    pub(crate) preparation: TemplatePreparation,
}

type ConstRequiredTemplateConstructionResult =
    Result<ConstRequiredTemplateConstruction, TemplateError>;

// -------------------------
//  Template Construction
// -------------------------

impl Template {
    /// Creates a new template node by parsing the token stream.
    ///
    /// This is the main public entry point. It delegates to:
    /// 1. `parse_template_head` — head directives, expressions, style config
    /// 2. `parse_template_body` — body string tokens, nested templates, slots
    /// 3. Composition — child wrapper application, head-chain resolution
    /// 4. Formatting — style-directed body formatting
    /// 5. Validation — directive-owned warnings and slot insertion checks
    pub(crate) fn new_with_type_interner(
        token_stream: &mut FileTokens,
        context: &ScopeContext,
        type_interner: &mut AstTypeInterner<'_>,
        direct_child_wrappers: Vec<TemplateWrapperReference>,
        string_table: &mut StringTable,
    ) -> TemplateConstructionResult {
        let default_style = default_nested_style_for_source_path(token_stream, string_table);
        Self::new_nested_template(
            token_stream,
            context,
            type_interner,
            direct_child_wrappers,
            string_table,
            NestedTemplateParseOptions::runtime_capable().with_default_style(default_style),
        )
    }

    /// Creates a template for a context that must fold during AST construction.
    ///
    /// Const-required callers need the structured control-flow template so AST
    /// folding can select branches and produce source diagnostics before the
    /// template reaches runtime lowering.
    pub(crate) fn new_const_required_with_type_interner(
        token_stream: &mut FileTokens,
        context: &ScopeContext,
        type_interner: &mut AstTypeInterner<'_>,
        direct_child_wrappers: Vec<TemplateWrapperReference>,
        string_table: &mut StringTable,
    ) -> ConstRequiredTemplateConstructionResult {
        let default_style = default_nested_style_for_source_path(token_stream, string_table);
        let template = Self::new_nested_template(
            token_stream,
            context,
            type_interner,
            direct_child_wrappers,
            string_table,
            NestedTemplateParseOptions::const_required().with_default_style(default_style),
        )?;

        let preparation = validate_const_required_template_control_flow(
            &template,
            &context.template_ir_store.borrow(),
        )?;

        Ok(ConstRequiredTemplateConstruction {
            template,
            preparation,
        })
    }

    #[cfg(test)]
    pub(crate) fn new(
        token_stream: &mut FileTokens,
        context: &ScopeContext,
        templates_inherited: Vec<TemplateWrapperReference>,
        string_table: &mut StringTable,
    ) -> TemplateConstructionResult {
        let mut type_environment = TypeEnvironment::new();
        let mut compatibility_cache = TypeCompatibilityCache::new();
        let mut type_interner =
            AstTypeInterner::new(&mut type_environment, &mut compatibility_cache);
        Self::new_with_type_interner(
            token_stream,
            context,
            &mut type_interner,
            templates_inherited,
            string_table,
        )
    }

    #[cfg(test)]
    pub(crate) fn new_const_required(
        token_stream: &mut FileTokens,
        context: &ScopeContext,
        templates_inherited: Vec<TemplateWrapperReference>,
        string_table: &mut StringTable,
    ) -> ConstRequiredTemplateConstructionResult {
        let mut type_environment = TypeEnvironment::new();
        let mut compatibility_cache = TypeCompatibilityCache::new();
        let mut type_interner =
            AstTypeInterner::new(&mut type_environment, &mut compatibility_cache);
        Self::new_const_required_with_type_interner(
            token_stream,
            context,
            &mut type_interner,
            templates_inherited,
            string_table,
        )
    }

    /// Internal constructor that supports doc comment context propagation.
    /// Called recursively for nested templates in the body parser.
    pub(crate) fn new_nested_template(
        token_stream: &mut FileTokens,
        context: &ScopeContext,
        type_interner: &mut AstTypeInterner<'_>,
        direct_child_wrappers: Vec<TemplateWrapperReference>,
        string_table: &mut StringTable,
        parse_options: NestedTemplateParseOptions,
    ) -> TemplateConstructionResult {
        let NestedTemplateParseOptions {
            parsing_mode,
            control_flow_validation,
            control_context,
            default_style,
            allow_stored_insert_carrier,
        } = parse_options;

        // The parser-local build state accumulates head/body metadata while
        // parsing. The durable `Template` is constructed once after
        // authoritative TIR identity exists, not mutated throughout parsing.
        let mut build_state = TemplateBuildState::new();

        // Capture the opening token location on the construction context; it
        // remains the sole location owner so style/directive errors still point
        // at the template even if parsing later advances deeply.
        let mut construction_context = TemplateConstructionContext::new(
            context.template_ir_store.clone(),
            token_stream.current_location(),
        );

        // ---------------------
        //  Parse template head
        // ---------------------
        //
        // Directives, expressions, and style config.
        let parsed_head = parse_template_head(
            token_stream,
            TemplateHeadParseRequest {
                context,
                type_interner,
                build_state: &mut build_state,
                construction_context: &mut construction_context,
                control_flow_validation,
                string_table,
            },
        )?;

        apply_default_style_if_needed(&mut build_state, &parsed_head, default_style.as_ref());

        let body_mode = parsed_head.body_mode;

        if parsing_mode == TemplateParsingMode::DocComment {
            apply_doc_comment_defaults(&mut build_state);
        }

        // Stage 2: Parse the template body (strings, nested templates, slots)
        parse_template_body(
            token_stream,
            &mut build_state,
            &mut construction_context,
            TemplateBodyParseRequest {
                context,
                type_interner,
                body_mode,
                direct_child_wrappers: &direct_child_wrappers,
                control_flow_validation,
                control_context,
                string_table,
                default_style: default_style.clone(),
            },
        )?;

        // Stage 3-5: render-unit shaping.
        //
        // Linear templates always install a TIR-formatted root. Control-flow
        // templates keep branch/body units structured so later folding/lowering
        // can stay lazy.
        let style = build_state.style.to_owned();
        let has_control_flow = construction_context.control_flow_node_id().is_some();
        if has_control_flow {
            prepare_control_flow_render_units(
                &mut construction_context,
                ControlFlowRenderUnitRequest {
                    style: &style,
                    context,
                    string_table,
                },
            )?;
        }

        // Finish parser-emitted TIR with a provisional kind. The kind is
        // updated after classification once the TIR-native composition block
        // below has produced the final post-composition reference.
        //
        // Prepared control-flow owner roots are at Formatted phase because
        // render-unit preparation has installed formatted body content. Linear
        // templates start at Parsed; linear formatting installs the formatted
        // reference below.
        let owner_phase = if has_control_flow {
            TemplateTirPhase::Formatted
        } else {
            TemplateTirPhase::Parsed
        };
        let construction_location = construction_context.location().to_owned();
        let mut tir_reference = construction_context.finish(
            build_state.style.to_owned(),
            build_state.kind.to_owned(),
            owner_phase,
        )?;
        let style = build_state.style.to_owned();
        install_formatted_tir_reference_for_linear_template(
            &mut tir_reference,
            has_control_flow,
            &style,
            context,
            string_table,
        )?;

        {
            // Head-chain composition materializes slot routing as needed, while
            // `$children(..)` direct-child wrappers are represented as
            // wrapper-context overlays. Both passes update the parser-owned TIR
            // reference directly. There is no second template representation to
            // reconstruct here.
            //
            // Wrapper-context overlays are attached after head-chain composition
            // so they use the final child occurrence IDs. Slot-resolution
            // context lives on each composed child reference.

            let template_id = tir_reference.root;

            // --- Phase 1: head-chain composition ---

            add_ast_counter(AstCounter::TemplateTirHeadChainCompositionCalls, 1);

            let original_root = {
                let store = context.template_ir_store.borrow();
                store
                    .get_template(template_id)
                    .map(|t| t.root)
                    .ok_or_else(|| {
                        TemplateError::from(CompilerError::compiler_error(
                            "Template head-chain composition started from a missing TIR root.",
                        ))
                    })?
            };

            let composed_root = compose_tir_head_chain_from_root(
                &mut context.template_ir_store.borrow_mut(),
                original_root,
                string_table,
                matches!(
                    control_flow_validation,
                    TemplateControlFlowValidationMode::RuntimeCapable
                ),
            )?;

            if composed_root != original_root {
                add_ast_counter(AstCounter::TemplateTirHeadChainCompositionHits, 1);

                let mut template_ir_store = context.template_ir_store.borrow_mut();
                let composed_template_id = template_ir_store.push_structurally_derived_template(
                    template_id,
                    composed_root,
                    crate::compiler_frontend::ast::templates::tir::DerivedTemplateMetadata::preserve_source(),
                )?;

                let phase = if tir_reference.phase.is_at_least(TemplateTirPhase::Formatted) {
                    TemplateTirPhase::Formatted
                } else {
                    TemplateTirPhase::Composed
                };

                tir_reference = TemplateTirReference {
                    root: composed_template_id,
                    phase,
                    context: tir_reference.context,
                };
            }

            let wrapper_context_owns_direct_children = !build_state.child_wrappers.is_empty();

            // --- Phase 2: wrapper-context overlay ---
            //
            // Record `$fresh` suppression and inherited wrapper-set context on
            // the final authoritative root after head-chain composition so the
            // occurrence keys match the structural root consumed downstream.
            if wrapper_context_owns_direct_children {
                attach_wrapper_context_overlay(
                    &mut tir_reference,
                    &build_state.child_wrappers,
                    &context.template_ir_store,
                )
                .map_err(TemplateError::from)?;
            }
        }

        // Stage 6: Preparation from the effective TirView of the final
        // reference (post-composition).
        //
        // The reference is now either a composed root with slots expanded and
        // inserts consumed, or a formatted linear root. Preparation reads
        // that authoritative view — preserving exact root, phase and view context
        // identity — without a separate TIR allocation.
        let template_preparation = {
            let store = context.template_ir_store.borrow();
            let view = TirView::with_minimum_phase(
                &store,
                tir_reference.root,
                tir_reference.phase,
                TemplateTirPhase::Composed,
                tir_reference.context,
            )
            .map_err(TemplateError::from)?;
            prepare_tir_view(&view, TemplatePreparationMode::Value)?
        };

        build_state.refresh_kind_from_preparation(&template_preparation.facts);

        // Post-parse validation
        if matches!(
            build_state.kind,
            TemplateType::Comment(CommentDirectiveKind::Doc)
        ) && !template_preparation.facts.is_const_evaluable_shape
        {
            return Err(CompilerDiagnostic::invalid_template_structure(
                InvalidTemplateStructureReason::NonFoldableDocComment,
                construction_location.clone(),
            )
            .into());
        }

        // `$insert(...)` helpers are allowed to survive while a template still has
        // unresolved `$slot` markers, because that template may later compose into
        // an immediate parent and contribute upward. Once a template has no slots
        // left, any remaining `$insert(...)` is out of scope and must error.
        //
        // Composed templates are exempt: head-chain composition routes insert
        // contributions into the receiving wrapper's slots, leaving
        // `InsertContribution` nodes in the composed tree. These are not
        // orphaned — they were consumed by composition — so the check must not
        // fire on a composed reference.
        let is_stored_insert_carrier = allow_stored_insert_carrier
            && crate::compiler_frontend::ast::templates::tir::stored_insert_contribution_templates(
                &context.template_ir_store.borrow(),
                tir_reference.root,
            )
            .map_err(TemplateError::from)?
            .is_some();
        if !matches!(build_state.kind, TemplateType::SlotInsert(_))
            && !template_preparation.facts.has_unresolved_slot_occurrences
            && template_preparation.facts.has_escaped_insert_helpers
            && !tir_reference.phase.is_at_least(TemplateTirPhase::Composed)
            && !is_stored_insert_carrier
        {
            return Err(CompilerDiagnostic::invalid_template_slot(
                InvalidTemplateSlotReason::InsertOutsideParentSlot,
                None,
                construction_location.clone(),
            )
            .into());
        }

        // Write the parser-local classification through the store owner before
        // constructing the durable handle. All later consumers read this TIR entry.
        let template_id = tir_reference.root;
        let mut template_ir_store = context.template_ir_store.borrow_mut();
        template_ir_store.set_template_kind(template_id, build_state.kind.to_owned())?;
        drop(template_ir_store);

        // Construct the durable `Template` only after its authoritative TIR
        // entry has received the final parser classification.
        let template = Template {
            tir_reference,
            location: construction_location.clone(),
        };

        if matches!(
            control_flow_validation,
            TemplateControlFlowValidationMode::RuntimeCapable
        ) {
            let store = context.template_ir_store.borrow();

            validate_runtime_template_control_flow_slot_artifacts(&template, &store)?;
        }

        increment_frontend_counter(FrontendCounter::TemplateCount);
        match control_flow_validation {
            TemplateControlFlowValidationMode::ConstRequired => {
                increment_frontend_counter(FrontendCounter::ConstTemplateCount);
            }
            TemplateControlFlowValidationMode::RuntimeCapable => {
                increment_frontend_counter(FrontendCounter::RuntimeTemplateCount);
            }
        }

        Ok(template)
    }
}

fn default_nested_style_for_source_path(
    token_stream: &FileTokens,
    string_table: &StringTable,
) -> Option<Style> {
    if !is_moth_template_content_constant_path(token_stream, string_table) {
        return None;
    }

    Some(markdown_default_style())
}

fn is_moth_template_content_constant_path(
    token_stream: &FileTokens,
    string_table: &StringTable,
) -> bool {
    if token_stream.src_path.name_str(string_table) != Some(SYNTHETIC_CONTENT_CONSTANT_NAME) {
        return false;
    }

    token_stream
        .src_path
        .parent()
        .and_then(|parent| parent.name_str(string_table).map(str::to_owned))
        .is_some_and(|source_name| {
            source_name.ends_with(SourceFileKind::MothTemplate.extension_suffix())
        })
}

fn markdown_default_style() -> Style {
    let mut style = Style::default();
    style.id = "markdown";
    style.formatter = Some(markdown_formatter());
    style.body_whitespace_policy = BodyWhitespacePolicy::StyleDirectiveControlled;
    style
}

fn apply_default_style_if_needed(
    build_state: &mut TemplateBuildState,
    parsed_head: &ParsedTemplateHead,
    default_style: Option<&Style>,
) {
    if parsed_head.has_explicit_template_directive {
        return;
    }

    if !matches!(
        build_state.kind,
        TemplateType::String | TemplateType::StringFunction
    ) {
        return;
    }

    if let Some(default_style) = default_style {
        build_state.style = default_style.to_owned();
    }
}

#[cfg(test)]
#[path = "tests/create_template_node/mod.rs"]
mod create_template_node_tests;
