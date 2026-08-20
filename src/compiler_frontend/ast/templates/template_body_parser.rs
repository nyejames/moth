//! Template body parsing.
//!
//! WHAT: Parses the body section of a template — string tokens, nested child
//! templates, slot definitions, and newlines — in source order.
//!
//! WHY: Separates body token consumption from head parsing and composition,
//! keeping each parsing phase focused and testable.

use crate::ast_log;
use crate::compiler_frontend::ast::statements::if_headers::{ParsedIfHeader, parse_if_header};
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::template::Template;
use crate::compiler_frontend::ast::templates::template::{
    CommentDirectiveKind, SlotPlaceholder, Style, TemplateParsingMode, TemplateSegmentOrigin,
    TemplateType,
};
use crate::compiler_frontend::ast::templates::template_body_sentinels::{
    BodySentinelTarget, DirectLoopControlMarker, ElseSentinelPolicy, TemplateBodyBoundary,
    TemplateBodyControlContext, classify_direct_else_marker, classify_direct_loop_control_marker,
    ensure_else_boundary_after_sentinel, ensure_loop_control_boundary_after_sentinel,
    ensure_loop_control_boundary_before_sentinel, first_line_has_meaningful_text,
    handle_direct_else_marker, inline_else_diagnostic, loop_control_marker_close_index,
    loop_control_marker_location, malformed_loop_control_reason, orphan_loop_control_diagnostic,
    remap_else_if_inline_diagnostic,
};
use crate::compiler_frontend::ast::templates::template_build_state::TemplateBuildState;
use crate::compiler_frontend::ast::templates::template_control_flow::{
    TemplateBodyParseMode, TemplateBranchSelector, TemplateControlFlowValidationMode,
    TemplateIfBodyParseInput, TemplateLoopBodyParseInput, TemplateLoopControlKind,
    inline_source_consts_for_const_required_if_condition,
};
use crate::compiler_frontend::ast::templates::tir::{
    TemplateConstructionContext, TemplateIrBranch, TemplateIrNodeId, TemplateIrNodeKind,
    TemplatePreparationMode, TemplateTirPhase, TemplateWrapperReference,
};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::ast::{ContextKind, ScopeContext};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidTemplateStructureReason,
};
use crate::compiler_frontend::instrumentation::{AstCounter, add_ast_counter};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, SourceLocation, TokenKind};
use crate::compiler_frontend::utilities::token_scan::consume_balanced_template_region;

/// Template-body parsing owns recursive template construction, so it carries the template error
/// boundary rather than reducing an inner retained-data failure to a user diagnostic.
type BodyParseResult<T> = Result<T, TemplateError>;

// -------------------------
//  Body Parser Entry
// -------------------------

/// Parses the body section of a template, consuming tokens until the explicit
/// closing delimiter. Nested child templates are recursively parsed.
///
/// Truncated source is reported as a user-facing EOF diagnostic.
pub(crate) fn parse_template_body(
    token_stream: &mut FileTokens,
    build_state: &mut TemplateBuildState,
    construction_context: &mut TemplateConstructionContext,
    input: TemplateBodyParseRequest<'_, '_>,
) -> BodyParseResult<()> {
    let TemplateBodyParseRequest {
        context,
        type_interner,
        body_mode,
        direct_child_wrappers,
        control_flow_validation,
        control_context,
        string_table,
        default_style,
    } = input;

    // Pre-intern common single-character literals used on every newline and
    // bracket token. These IDs are stable for the lifetime of the string table,
    // so caching them once per body parse avoids repeated hash lookups.
    let newline_id = string_table.intern("\n");
    let open_bracket_id = string_table.intern("[");
    let close_bracket_id = string_table.intern("]");

    let mut parser = TemplateBodyParser {
        token_stream,
        type_interner,
        direct_child_wrappers,
        control_flow_validation,
        string_table,
        newline_id,
        open_bracket_id,
        close_bracket_id,
        default_style,
    };

    match body_mode {
        TemplateBodyParseMode::Normal => {
            let parse_input = BodyParseInput {
                context,
                build_state,
                control_context: control_context.with_else_policy(ElseSentinelPolicy::Orphan),
                inherited_wrappers: InheritedChildWrapperPolicy::Apply,
            };
            parser
                .parse_content(parse_input, construction_context)
                .map(|_| ())
        }

        TemplateBodyParseMode::If(input) => {
            parser.parse_if_body(build_state, construction_context, *input, control_context)
        }

        TemplateBodyParseMode::Loop(input) => {
            parser.parse_loop_body(build_state, construction_context, *input, control_context)
        }
    }
}

/// Shared input bundle for one template body parse.
///
/// WHAT: carries the mutable AST/body parser services used by every recursive
/// body mode.
/// WHY: control-flow body parsing needs the same token/type/string-table state
/// across normal, if, and loop paths without threading long argument lists.
pub(crate) struct TemplateBodyParseRequest<'a, 'types> {
    pub(crate) context: &'a ScopeContext,
    pub(crate) type_interner: &'a mut AstTypeInterner<'types>,
    pub(crate) body_mode: TemplateBodyParseMode,
    pub(crate) direct_child_wrappers: &'a [TemplateWrapperReference],
    pub(crate) control_flow_validation: TemplateControlFlowValidationMode,
    pub(crate) control_context: TemplateBodyControlContext,
    pub(crate) string_table: &'a mut StringTable,
    /// Source-kind policy applied to child templates without an explicit formatter.
    pub(crate) default_style: Option<Style>,
}

/// Options that stay stable for one template node while its head and body are parsed.
///
/// WHAT: groups doc-comment mode, runtime/const validation mode, and inherited
/// body-control state for recursive template construction.
/// WHY: nested template parsing needs these three values together, and grouping
/// them keeps `Template::new_nested_template` from becoming a long argument list.
#[derive(Clone)]
pub(crate) struct NestedTemplateParseOptions {
    pub(crate) parsing_mode: TemplateParsingMode,
    pub(crate) control_flow_validation: TemplateControlFlowValidationMode,
    /// Preparation mode for this template node only. Nested nodes stay in value mode so a
    /// const-required parent performs the single authoritative const traversal over its complete
    /// composed view.
    pub(crate) preparation_mode: TemplatePreparationMode,
    pub(crate) control_context: TemplateBodyControlContext,
    pub(crate) default_style: Option<Style>,
    /// Allows a nested template whose sole content is a stored `$insert(...)`
    /// helper to be flattened into its immediate parent contribution stream.
    ///
    /// A standalone escaped insert remains invalid; only the body parser can
    /// opt into this carrier form because it owns the immediate parent.
    pub(crate) allow_stored_insert_carrier: bool,
}

impl NestedTemplateParseOptions {
    pub(crate) fn runtime_capable() -> Self {
        Self {
            parsing_mode: TemplateParsingMode::Standard,
            control_flow_validation: TemplateControlFlowValidationMode::RuntimeCapable,
            preparation_mode: TemplatePreparationMode::Value,
            control_context: TemplateBodyControlContext::normal(),
            default_style: None,
            allow_stored_insert_carrier: false,
        }
    }

    pub(crate) fn const_required() -> Self {
        Self {
            parsing_mode: TemplateParsingMode::Standard,
            control_flow_validation: TemplateControlFlowValidationMode::ConstRequired,
            preparation_mode: TemplatePreparationMode::ConstRequired,
            control_context: TemplateBodyControlContext::normal(),
            default_style: None,
            allow_stored_insert_carrier: false,
        }
    }

    pub(crate) fn with_default_style(mut self, default_style: Option<Style>) -> Self {
        self.default_style = default_style;
        self
    }
}

#[derive(Clone, Copy)]
struct BodyParseInput<'context, 'build> {
    context: &'context ScopeContext,
    build_state: &'build TemplateBuildState,
    control_context: TemplateBodyControlContext,
    inherited_wrappers: InheritedChildWrapperPolicy,
}

struct TemplateBodyParser<'a, 'types> {
    token_stream: &'a mut FileTokens,
    type_interner: &'a mut AstTypeInterner<'types>,
    direct_child_wrappers: &'a [TemplateWrapperReference],
    control_flow_validation: TemplateControlFlowValidationMode,
    string_table: &'a mut StringTable,
    // Cached interned IDs for common single-character literals that appear on
    // every newline and bracket token. Interning once per body parse avoids
    // repeated hash lookups in the hot parsing loop.
    newline_id: StringId,
    open_bracket_id: StringId,
    close_bracket_id: StringId,
    default_style: Option<Style>,
}

impl<'a, 'types> TemplateBodyParser<'a, 'types> {
    /// Parses body tokens into parser TIR.
    ///
    /// All body content — literal text, newlines, nested templates, and slot
    /// definitions — is emitted exclusively into parser TIR through
    /// `TemplateConstructionContext`. `$doc` suppresses nested template parsing,
    /// so balanced brackets in documentation bodies remain literal text.
    fn parse_content(
        &mut self,
        input: BodyParseInput<'_, '_>,
        construction_context: &mut TemplateConstructionContext,
    ) -> BodyParseResult<TemplateBodyBoundary> {
        // The tokenizer only allows for strings, templates or slots inside the template body.
        let mut last_known_location = self.token_stream.current_location();
        while self.token_stream.index < self.token_stream.tokens.len() {
            add_ast_counter(AstCounter::TemplateBodyTokenVisits, 1);
            last_known_location = self.token_stream.current_location();

            // Match by reference to avoid cloning the token kind on every iteration.
            // Only the error fallback arm needs an owned clone for the diagnostic payload.
            match self.token_stream.current_token_kind() {
                TokenKind::Eof => {
                    return Err(CompilerDiagnostic::unexpected_end_of_file(
                        Some(self.close_bracket_id),
                        self.token_stream.current_location(),
                    )
                    .into());
                }

                TokenKind::TemplateClose => {
                    ast_log!("Breaking out of template body. Found a template close.");
                    // Consume the closing bracket so the caller resumes after the template body.
                    self.token_stream.advance();
                    return Ok(TemplateBodyBoundary::TemplateClose);
                }

                TokenKind::TemplateHead => {
                    if let Some(else_marker) = classify_direct_else_marker(self.token_stream) {
                        let sentinel_target = body_sentinel_target(
                            construction_context,
                            input.build_state.style.suppress_child_templates,
                        );
                        return Ok(handle_direct_else_marker(
                            self.token_stream,
                            else_marker,
                            input.control_context.else_policy,
                            sentinel_target,
                            self.string_table,
                        )?);
                    }

                    if let Some(loop_marker) =
                        classify_direct_loop_control_marker(self.token_stream)
                    {
                        self.handle_loop_control_marker(input, construction_context, &loop_marker)?;
                        continue;
                    }

                    // When child templates are suppressed (e.g. `$doc`), brackets are
                    // treated as balanced literal text rather than parsed as nested templates.
                    if input.build_state.style.suppress_child_templates {
                        consume_balanced_brackets_as_literal_text(
                            self.token_stream,
                            construction_context,
                            self.string_table,
                            LiteralTemplateTextIds {
                                newline_id: self.newline_id,
                                open_bracket_id: self.open_bracket_id,
                                close_bracket_id: self.close_bracket_id,
                            },
                        );
                        continue;
                    }

                    self.parse_nested_template(input, construction_context)?;
                    continue;
                }

                TokenKind::RawStringLiteral(content) | TokenKind::StringSliceLiteral(content) => {
                    let byte_len = self.string_table.resolve(*content).len();
                    #[cfg(feature = "detailed_timers")]
                    {
                        add_ast_counter(AstCounter::TemplateTextBytesParsed, byte_len);
                    }
                    let location = self.token_stream.current_location();
                    construction_context.record_text(*content, byte_len, location);
                }

                TokenKind::Newline => {
                    add_ast_counter(AstCounter::TemplateTextBytesParsed, 1);
                    let location = self.token_stream.current_location();
                    construction_context.record_text(self.newline_id, 1, location);
                }

                found => {
                    return Err(CompilerDiagnostic::unexpected_token(
                        found.clone(),
                        self.token_stream.current_location(),
                    )
                    .into());
                }
            }

            self.token_stream.advance();
        }

        Err(CompilerDiagnostic::unexpected_end_of_file(
            Some(self.close_bracket_id),
            last_known_location,
        )
        .into())
    }

    /// Parses an `[if]` body and any `[else if]` / `[else]` followers into a
    /// branch-chain control-flow node.
    ///
    /// WHAT: each branch body is parsed into a fresh TIR construction context.
    ///       Branch selectors, fallback bodies and whitespace trimming are
    ///       collected in source order.
    /// WHY: branch bodies are TIR-only and must not inherit the parent wrapper
    ///      policy directly; composition attaches wrappers to the whole chain.
    fn parse_if_body(
        &mut self,
        build_state: &mut TemplateBuildState,
        construction_context: &mut TemplateConstructionContext,
        input: TemplateIfBodyParseInput,
        control_context: TemplateBodyControlContext,
    ) -> BodyParseResult<()> {
        let mut branch_tir_branches = Vec::new();
        let mut branch_selector = input.selector;
        let mut branch_context = input.then_context;
        let mut branch_location = input.location.clone();
        let mut branch_starts_after_else_if = false;
        let mut fallback_tir_body = None;

        // -------------------
        //  Parse branch bodies
        // -------------------

        let opening_location = construction_context.location().to_owned();
        loop {
            let mut branch_construction_context =
                tir_only_body_construction_context(&opening_location, &branch_context);
            let parse_input = BodyParseInput {
                context: &branch_context,
                build_state,
                control_context: control_context.with_else_policy(ElseSentinelPolicy::SplitIf),
                inherited_wrappers: InheritedChildWrapperPolicy::Skip,
            };

            let boundary = self.parse_content(parse_input, &mut branch_construction_context)?;

            // An `[else if]` sentinel ends on the same line as the previous branch's
            // closing bracket. Strip the leading whitespace that follows it so the
            // new branch body starts at the first meaningful line.
            if branch_starts_after_else_if {
                branch_construction_context.trim_leading_whitespace(self.string_table);
            }

            let branch_body_node_id = finalize_tir_body_builder(
                build_state.style.clone(),
                build_state.kind.clone(),
                branch_construction_context,
            )?;

            let selector_site_id = construction_context.next_expression_site_id();
            branch_tir_branches.push(TemplateIrBranch::new(
                branch_selector.clone(),
                branch_body_node_id,
                branch_location.clone(),
                selector_site_id,
            ));

            match boundary {
                TemplateBodyBoundary::ElseIf {
                    if_index,
                    close_index,
                    location,
                } => {
                    let parsed_else_if = self.parse_else_if_branch_header(
                        &input.else_context,
                        if_index,
                        close_index,
                        &location,
                    )?;
                    branch_selector = parsed_else_if.selector;
                    branch_context = parsed_else_if.branch_context;
                    branch_location = location;
                    branch_starts_after_else_if = true;
                }

                TemplateBodyBoundary::Else { location } => {
                    let fallback_branch = self.parse_fallback_branch(
                        build_state,
                        &opening_location,
                        &input.else_context,
                        control_context,
                        location,
                    )?;
                    fallback_tir_body = Some(fallback_branch.body_node_id);
                    break;
                }

                TemplateBodyBoundary::TemplateClose => {
                    break;
                }
            }
        }

        // -----------------------
        //  Finalize branch chain
        // -----------------------

        construction_context.record_branch_chain(
            branch_tir_branches,
            fallback_tir_body,
            input.location.clone(),
        );

        Ok(())
    }

    /// Parses the `[else]` fallback body of a branch chain.
    ///
    /// WHAT: validates the sentinel boundary, parses the fallback body as TIR-only,
    ///       trims leading whitespace, and finishes the construction context.
    /// WHY: fallback bodies share the same control-flow semantics as branch bodies
    ///      but start from a different sentinel and must begin on a fresh boundary.
    fn parse_fallback_branch(
        &mut self,
        build_state: &TemplateBuildState,
        opening_location: &SourceLocation,
        fallback_context: &ScopeContext,
        control_context: TemplateBodyControlContext,
        location: SourceLocation,
    ) -> BodyParseResult<ParsedFallbackBranch> {
        ensure_else_boundary_after_sentinel(self.token_stream, &location, self.string_table)?;

        let mut else_construction_context =
            tir_only_body_construction_context(opening_location, fallback_context);
        let parse_input = BodyParseInput {
            context: fallback_context,
            build_state,
            control_context: control_context.with_else_policy(ElseSentinelPolicy::Duplicate),
            inherited_wrappers: InheritedChildWrapperPolicy::Skip,
        };
        self.parse_content(parse_input, &mut else_construction_context)?;

        ensure_else_body_starts_on_new_boundary(
            &else_construction_context,
            &location,
            self.string_table,
        )?;
        else_construction_context.trim_leading_whitespace(self.string_table);

        let fallback_body_node_id = finalize_tir_body_builder(
            build_state.style.clone(),
            build_state.kind.clone(),
            else_construction_context,
        )?;

        Ok(ParsedFallbackBranch {
            body_node_id: fallback_body_node_id,
        })
    }

    fn parse_else_if_branch_header(
        &mut self,
        base_context: &ScopeContext,
        if_index: usize,
        close_index: usize,
        location: &SourceLocation,
    ) -> BodyParseResult<ParsedElseIfBranch> {
        self.token_stream.index = if_index + 1;

        if next_meaningful_token_is_template_close(self.token_stream, close_index) {
            return Err(CompilerDiagnostic::invalid_template_structure(
                InvalidTemplateStructureReason::MissingTemplateElseIfCondition,
                location.clone(),
            )
            .into());
        }

        let parsed_header = parse_if_header(
            self.token_stream,
            base_context,
            self.type_interner,
            self.string_table,
        )?;

        if self.token_stream.index != close_index
            || !matches!(
                self.token_stream.current_token_kind(),
                TokenKind::TemplateClose
            )
        {
            return Err(CompilerDiagnostic::invalid_template_structure(
                InvalidTemplateStructureReason::MalformedTemplateElseIf,
                self.token_stream.current_location(),
            )
            .into());
        }

        self.token_stream.advance();
        ensure_else_boundary_after_sentinel(self.token_stream, location, self.string_table)
            .map_err(|diagnostic| remap_else_if_inline_diagnostic(diagnostic, location))?;

        let (mut selector, branch_context) =
            branch_selector_and_context_from_parsed_if_header(parsed_header, base_context, self)?;

        if self.control_flow_validation == TemplateControlFlowValidationMode::ConstRequired {
            selector = inline_source_consts_for_const_required_if_condition(
                selector,
                base_context,
                self.string_table,
            );
        }

        Ok(ParsedElseIfBranch {
            selector,
            branch_context,
        })
    }

    fn parse_loop_body(
        &mut self,
        build_state: &mut TemplateBuildState,
        construction_context: &mut TemplateConstructionContext,
        input: TemplateLoopBodyParseInput,
        control_context: TemplateBodyControlContext,
    ) -> BodyParseResult<()> {
        let loop_location_snapshot = construction_context.location().to_owned();
        let mut body_construction_context =
            tir_only_body_construction_context(&loop_location_snapshot, &input.body_context);
        let parse_input = BodyParseInput {
            context: &input.body_context,
            build_state,
            control_context: control_context.enter_template_loop(),
            inherited_wrappers: InheritedChildWrapperPolicy::Skip,
        };

        self.parse_content(parse_input, &mut body_construction_context)?;

        let body_node_id = finalize_tir_body_builder(
            build_state.style.clone(),
            build_state.kind.clone(),
            body_construction_context,
        )?;

        construction_context.record_loop(input.header, body_node_id, input.location);

        Ok(())
    }

    /// Handles a nested `[...]` template token encountered inside a parent body.
    /// Recursively parses the child, then records it as a parser TIR
    /// child-template, slot, or insert-contribution node.
    fn parse_nested_template(
        &mut self,
        input: BodyParseInput<'_, '_>,
        construction_context: &mut TemplateConstructionContext,
    ) -> BodyParseResult<()> {
        add_ast_counter(AstCounter::TemplateNestedTemplateParses, 1);

        let nested_direct_child_wrappers = input.build_state.child_wrappers.to_owned();

        let parse_options = NestedTemplateParseOptions {
            parsing_mode: if matches!(
                input.build_state.kind,
                TemplateType::Comment(CommentDirectiveKind::Doc)
            ) {
                TemplateParsingMode::DocComment
            } else {
                TemplateParsingMode::Standard
            },
            control_flow_validation: self.control_flow_validation,
            preparation_mode: TemplatePreparationMode::Value,
            control_context: input.control_context,
            default_style: self.default_style.clone(),
            allow_stored_insert_carrier: true,
        };

        let child_construction = Template::new_nested_template(
            self.token_stream,
            input.context,
            self.type_interner,
            nested_direct_child_wrappers,
            self.string_table,
            parse_options,
        )?;
        let child_template = child_construction.template;

        // The child was just constructed in this context's store. Read its
        // authoritative kind before mutating the construction context again.
        let child_kind = {
            let store = construction_context.store();
            store
                .get_template(child_template.tir_reference.root)
                .map(|template_ir| template_ir.kind.clone())
                .ok_or_else(|| {
                    TemplateError::from(CompilerError::compiler_error(
                        "Nested template kind was missing from the parser's owning TIR store.",
                    ))
                })?
        };

        let stored_insert_contributions = {
            let store = construction_context.store();
            crate::compiler_frontend::ast::templates::tir::stored_insert_contribution_templates(
                &store,
                child_template.tir_reference.root,
            )
            .map_err(TemplateError::from)?
        };

        if let Some(contributions) = stored_insert_contributions {
            // A stored named insert is authored as a binding, then referenced
            // through a nested `[...]` body template. Flatten only the
            // insert-only carrier so the immediate wrapper owns routing.
            for (template_id, location) in contributions {
                construction_context.record_insert_contribution(template_id, location);
            }
        } else {
            match &child_kind {
                TemplateType::SlotInsert(_) => {
                    record_parser_tir_insert_contribution(construction_context, &child_template);
                }
                TemplateType::Comment(_) | TemplateType::SlotDefinition(_) => {}
                _ => {
                    record_parser_tir_child_template(construction_context, &child_template);
                }
            }
        }

        // Control-flow children are fully TIR-owned: their body roots carry the
        // branch/loop structure and the child template node is already recorded
        // above through `record_parser_tir_child_template`.
        let child_template_id = child_template.tir_reference.root;
        let has_control_flow_root = {
            let store = construction_context.store();
            store
                .control_flow_node_id_for_template(child_template_id)?
                .is_some()
        };
        if has_control_flow_root {
            return Ok(());
        }

        match &child_kind {
            TemplateType::Comment(_) => {
                return Ok(());
            }

            TemplateType::String | TemplateType::StringFunction | TemplateType::SlotInsert(_) => {}

            TemplateType::SlotDefinition(slot_key) => {
                let inherited_direct_child_wrappers = match input.inherited_wrappers {
                    InheritedChildWrapperPolicy::Apply => self.direct_child_wrappers.to_owned(),
                    InheritedChildWrapperPolicy::Skip => Vec::new(),
                };

                let slot_placeholder = SlotPlaceholder::with_wrappers(
                    slot_key.to_owned(),
                    inherited_direct_child_wrappers,
                    input.build_state.child_wrappers.to_owned(),
                    input.build_state.style.skip_parent_child_wrappers,
                );

                // Slot definitions are recorded exclusively in parser TIR
                // through `record_slot`.
                construction_context
                    .record_slot(slot_placeholder, child_template.location.clone())?;
                return Ok(());
            }
        }

        // Ordinary nested child templates (String, SlotInsert) are fully
        // TIR-owned: `record_parser_tir_child_template` above recorded the
        // child reference in parser TIR, and the TIR fold/format/handoff
        // pipeline owns composition, folding, and runtime handoff.
        Ok(())
    }

    fn handle_loop_control_marker(
        &mut self,
        input: BodyParseInput<'_, '_>,
        construction_context: &mut TemplateConstructionContext,
        marker: &DirectLoopControlMarker,
    ) -> BodyParseResult<()> {
        if input.build_state.style.suppress_child_templates {
            return Err(CompilerDiagnostic::invalid_template_structure(
                InvalidTemplateStructureReason::TemplateLoopControlInLiteralBody,
                loop_control_marker_location(marker).clone(),
            )
            .into());
        }

        if !input.control_context.accepts_loop_control() {
            return Err(orphan_loop_control_diagnostic(marker).into());
        }

        let Some(close_index) = loop_control_marker_close_index(marker) else {
            return Err(malformed_loop_control_reason(marker).into());
        };

        ensure_loop_control_boundary_before_sentinel(self.token_stream, marker, self.string_table)?;

        construction_context.trim_trailing_whitespace(self.string_table);

        let kind = loop_control_kind(marker);
        let location = loop_control_marker_location(marker).clone();
        construction_context.record_loop_control(kind, location.clone());

        self.token_stream.index = close_index;
        self.token_stream.advance();
        ensure_loop_control_boundary_after_sentinel(self.token_stream, marker, self.string_table)?;

        Ok(())
    }
}

fn record_parser_tir_child_template(
    construction_context: &mut TemplateConstructionContext,
    child_template: &Template,
) {
    let child_reference = &child_template.tir_reference;

    construction_context.record_child_template(
        child_reference,
        TemplateSegmentOrigin::Body,
        child_template.location.clone(),
    );
}

fn record_parser_tir_insert_contribution(
    construction_context: &mut TemplateConstructionContext,
    child_template: &Template,
) {
    let child_template_id = child_template.tir_reference.root;

    construction_context
        .record_insert_contribution(child_template_id, child_template.location.clone());
}

fn loop_control_kind(marker: &DirectLoopControlMarker) -> TemplateLoopControlKind {
    match marker {
        DirectLoopControlMarker::Break { .. } => TemplateLoopControlKind::Break,
        DirectLoopControlMarker::Continue { .. } => TemplateLoopControlKind::Continue,
    }
}

struct ParsedElseIfBranch {
    selector: TemplateBranchSelector,
    branch_context: ScopeContext,
}

struct ParsedFallbackBranch {
    body_node_id: TemplateIrNodeId,
}

fn branch_selector_and_context_from_parsed_if_header(
    parsed_header: ParsedIfHeader,
    base_context: &ScopeContext,
    parser: &mut TemplateBodyParser<'_, '_>,
) -> BodyParseResult<(TemplateBranchSelector, ScopeContext)> {
    match parsed_header {
        ParsedIfHeader::BoolCondition { condition } => {
            let branch_context =
                base_context.new_child_control_flow(ContextKind::Branch, parser.string_table);

            Ok((TemplateBranchSelector::Bool(condition), branch_context))
        }

        ParsedIfHeader::OptionPresentCapture {
            scrutinee,
            pattern,
            then_context,
        } => {
            let branch_context =
                then_context.new_child_control_flow(ContextKind::Branch, parser.string_table);

            Ok((
                TemplateBranchSelector::OptionPresentCapture {
                    scrutinee,
                    pattern: Box::new(pattern),
                },
                branch_context,
            ))
        }

        ParsedIfHeader::MatchStyle { scrutinee } => {
            Err(CompilerDiagnostic::invalid_template_structure(
                InvalidTemplateStructureReason::TemplateMatchStyleControlFlowUnsupported,
                scrutinee.location,
            )
            .into())
        }
    }
}

fn next_meaningful_token_is_template_close(token_stream: &FileTokens, close_index: usize) -> bool {
    let mut index = token_stream.index;

    while index <= close_index && index < token_stream.length {
        match token_stream.tokens[index].kind {
            TokenKind::Newline => index += 1,
            TokenKind::TemplateClose => return true,
            _ => return false,
        }
    }

    true
}

#[derive(Clone, Copy)]
enum InheritedChildWrapperPolicy {
    // Normal template bodies apply wrappers inherited from their parent.
    Apply,
    // Control-flow branch bodies must not consume parent wrappers directly; the
    // composition pass attaches those wrappers to the control-flow child as a whole.
    Skip,
}

fn tir_only_body_construction_context(
    location: &SourceLocation,
    context: &ScopeContext,
) -> TemplateConstructionContext {
    TemplateConstructionContext::new(context.template_ir_store.clone(), location.to_owned())
}

fn body_sentinel_target<'a>(
    construction_context: &'a mut TemplateConstructionContext,
    suppress_child_templates: bool,
) -> BodySentinelTarget<'a> {
    BodySentinelTarget {
        construction_context,
        suppress_child_templates,
    }
}

/// Finalizes a control-flow body template's parser-emitted TIR and returns the
/// body root node ID.
///
/// WHAT: every branch/loop body shell starts a `TemplateConstructionContext`.
///       This helper finishes that context into a finalized `TemplateIr` and
///       reads the root node from the shared store.
/// WHY: control-flow bodies are emitted directly into TIR, so the body only
///      needs to be finished. The body root node ID is all render-unit
///      preparation needs to format, wrap and install the body.
fn finalize_tir_body_builder(
    style: Style,
    kind: TemplateType,
    construction_context: TemplateConstructionContext,
) -> Result<TemplateIrNodeId, TemplateError> {
    let store = construction_context.store_handle();
    let tir_reference = construction_context.finish(style, kind, TemplateTirPhase::Parsed)?;
    let store = store.borrow();
    let template_ir = store
        .get_template(tir_reference.root)
        .expect("a just-pushed control-flow body template must exist in the TIR store");
    Ok(template_ir.root)
}

/// Ensures an `[else]` fallback body starts on a new boundary line.
///
/// WHAT: after the `[else]` sentinel is consumed, the first meaningful content
///       in the fallback body must not share the sentinel's line.
/// WHY: control-flow bodies are TIR-only, so boundary validation reads the
///      in-progress construction context.
fn ensure_else_body_starts_on_new_boundary(
    construction_context: &TemplateConstructionContext,
    sentinel_location: &SourceLocation,
    string_table: &StringTable,
) -> BodyParseResult<()> {
    let store = construction_context.store();
    let Some(first_child_id) = construction_context.root_children().first() else {
        return Ok(());
    };

    let node = store
        .get_node(*first_child_id)
        .expect("control-flow body TIR builder child should exist in the store");

    if let TemplateIrNodeKind::Text { text, .. } = &node.kind
        && first_line_has_meaningful_text(string_table.resolve(*text))
    {
        return Err(inline_else_diagnostic(sentinel_location).into());
    }

    Ok(())
}

// -------------------------
//  Literal Content
// -------------------------

/// Consumes a `[...]` bracketed region directly into parser TIR as literal text
/// when child templates are suppressed (e.g. in `$doc` bodies). Tracks bracket
/// nesting depth so balanced brackets are included in the literal output.
///
/// Accepts pre-interned `StringId`s for newline and bracket literals so the
/// caller can reuse cached IDs rather than re-interning on every token.
#[derive(Clone, Copy)]
struct LiteralTemplateTextIds {
    newline_id: StringId,
    open_bracket_id: StringId,
    close_bracket_id: StringId,
}

fn consume_balanced_brackets_as_literal_text(
    token_stream: &mut FileTokens,
    construction_context: &mut TemplateConstructionContext,
    string_table: &mut StringTable,
    text_ids: LiteralTemplateTextIds,
) {
    // Emit the opening bracket as literal text.
    add_ast_counter(AstCounter::TemplateTextBytesParsed, 1);
    let location = token_stream.current_location();
    construction_context.record_text(text_ids.open_bracket_id, 1, location);
    token_stream.advance();

    let _ = consume_balanced_template_region(
        token_stream,
        |token, token_kind| match token_kind {
            TokenKind::TemplateHead => {
                add_ast_counter(AstCounter::TemplateTextBytesParsed, 1);
                construction_context.record_text(
                    text_ids.open_bracket_id,
                    1,
                    token.location.clone(),
                );
            }

            TokenKind::TemplateClose => {
                add_ast_counter(AstCounter::TemplateTextBytesParsed, 1);
                construction_context.record_text(
                    text_ids.close_bracket_id,
                    1,
                    token.location.clone(),
                );
            }

            TokenKind::RawStringLiteral(content) | TokenKind::StringSliceLiteral(content) => {
                let byte_len = string_table.resolve(*content).len();
                #[cfg(feature = "detailed_timers")]
                {
                    add_ast_counter(AstCounter::TemplateTextBytesParsed, byte_len);
                }
                construction_context.record_text(*content, byte_len, token.location.clone());
            }

            TokenKind::Newline => {
                add_ast_counter(AstCounter::TemplateTextBytesParsed, 1);
                construction_context.record_text(text_ids.newline_id, 1, token.location.clone());
            }

            TokenKind::Symbol(id) | TokenKind::StyleDirective(id) => {
                let prefix = if matches!(token_kind, TokenKind::StyleDirective(_)) {
                    "$"
                } else {
                    ""
                };
                let name = string_table.resolve(*id).to_owned();
                let literal = format!("{prefix}{name}");
                add_ast_counter(AstCounter::TemplateTextBytesParsed, literal.len());
                let literal_id = string_table.intern(&literal);
                construction_context.record_text(literal_id, literal.len(), token.location.clone());
            }

            TokenKind::StartTemplateBody | TokenKind::Colon => {
                add_ast_counter(AstCounter::TemplateTextBytesParsed, 1);
                let colon_id = string_table.intern(":");
                construction_context.record_text(colon_id, 1, token.location.clone());
            }

            TokenKind::Comma => {
                add_ast_counter(AstCounter::TemplateTextBytesParsed, 1);
                let comma_id = string_table.intern(",");
                construction_context.record_text(comma_id, 1, token.location.clone());
            }

            TokenKind::OpenParenthesis => {
                add_ast_counter(AstCounter::TemplateTextBytesParsed, 1);
                let paren_id = string_table.intern("(");
                construction_context.record_text(paren_id, 1, token.location.clone());
            }

            TokenKind::CloseParenthesis => {
                add_ast_counter(AstCounter::TemplateTextBytesParsed, 1);
                let paren_id = string_table.intern(")");
                construction_context.record_text(paren_id, 1, token.location.clone());
            }

            _ => {}
        },
        |_location| (),
    );
}

// -------------------------
//  Internal Helpers
