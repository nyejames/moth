//! Template body parsing.
//!
//! WHAT: Parses the body section of a template — string tokens, nested child
//! templates, slot definitions, and newlines — in source order.
//!
//! WHY: Separates body token consumption from head parsing and composition,
//! keeping each parsing phase focused and testable.

use crate::ast_log;
use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::cursor::AstCursor;
use crate::compiler_frontend::ast::templates::create_template_node::TemplatePathTables;
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::template::{
    CommentDirectiveKind, SlotPlaceholder, Style, Template, TemplateParsingMode,
    TemplateSegmentOrigin, TemplateType,
};
use crate::compiler_frontend::ast::templates::template_build_state::TemplateBuildState;
use crate::compiler_frontend::ast::templates::template_control_flow::{
    TemplateBodyParseMode, TemplateControlFlowValidationMode, TemplateIfBodyParseInput,
    TemplateLoopBodyParseInput,
};
use crate::compiler_frontend::ast::templates::tir::{
    TemplateConstructionContext, TemplateIrNodeId, TemplatePreparationMode, TemplateTirPhase,
    TemplateWrapperReference,
};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{CompilerDiagnostic, DiagnosticToken};
use crate::compiler_frontend::instrumentation::{AstCounter, add_ast_counter};
use crate::compiler_frontend::keywords::token_tag_for_source_word;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::tokenizer::tokens::TokenTag;
use crate::compiler_frontend::utilities::token_scan::TemplateBalance;
use moth_lexical::words::SourceWord;

/// Template-body parsing owns recursive template construction, so it carries the template error
/// boundary rather than reducing an inner retained-data failure to a user diagnostic.
type BodyParseResult<T> = Result<T, TemplateError>;

// -------------------------
//  Body Parser Entry
// -------------------------

/// How an outer template body ends when its donor stream has no physical close token.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TemplateBodyEndPolicy {
    RequireClose,
    DonorExhaustionIsClose,
}

/// Parses the body section of a template, consuming tokens until the explicit
/// closing delimiter. Nested child templates are recursively parsed.
///
/// Truncated source is reported as a user-facing EOF diagnostic.
pub(crate) fn parse_template_body(
    token_stream: &mut AstCursor<'_>,
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
        string_table,
        default_style,
        source_path,
        path_fork,
        end_policy,
    } = input;
    let outer_end_policy = end_policy;

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
        source_path,
        path_fork,
        end_policy: outer_end_policy,
    };

    match body_mode {
        TemplateBodyParseMode::Normal => {
            let parse_input = BodyParseInput {
                context,
                build_state,
                inherited_wrappers: InheritedChildWrapperPolicy::Apply,
            };
            parser.parse_content(parse_input, construction_context, parser.end_policy)
        }

        TemplateBodyParseMode::If(input) => {
            parser.parse_if_body(build_state, construction_context, *input)
        }

        TemplateBodyParseMode::Loop(input) => {
            parser.parse_loop_body(build_state, construction_context, *input)
        }
    }
}

/// Shared input bundle for one template body parse.
///
/// WHAT: carries the context and parser services needed to consume one body.
/// WHY: keeps body parsing's mutable dependencies explicit at its entry point.
pub(crate) struct TemplateBodyParseRequest<'a, 'types> {
    pub(crate) context: &'a ScopeContext,
    pub(crate) end_policy: TemplateBodyEndPolicy,
    pub(crate) type_interner: &'a mut AstTypeInterner<'types>,
    pub(crate) body_mode: TemplateBodyParseMode,
    pub(crate) direct_child_wrappers: &'a [TemplateWrapperReference],
    pub(crate) control_flow_validation: TemplateControlFlowValidationMode,
    pub(crate) string_table: &'a mut StringTable,
    /// Source-kind policy applied to child templates without an explicit formatter.
    pub(crate) default_style: Option<Style>,
    pub(crate) source_path: PathId,
    pub(crate) path_fork: &'a mut PathInternerFork,
}

/// Options that stay stable for one template node while its head and body are parsed.
///
/// WHAT: groups parsing mode, control-flow validation, preparation and style defaults for
/// recursive template construction.
/// WHY: nested template construction preserves source-kind defaults and the owning validation
/// boundary without inheriting body-parser state.
#[derive(Clone)]
pub(crate) struct NestedTemplateParseOptions {
    pub(crate) parsing_mode: TemplateParsingMode,
    pub(crate) control_flow_validation: TemplateControlFlowValidationMode,
    /// Preparation mode for this template node only. Nested nodes stay in value mode so a
    /// const-required parent performs the single authoritative const traversal over its complete
    /// composed view.
    pub(crate) preparation_mode: TemplatePreparationMode,
    pub(crate) default_style: Option<Style>,
}

impl NestedTemplateParseOptions {
    pub(crate) fn runtime_capable() -> Self {
        Self {
            parsing_mode: TemplateParsingMode::Standard,
            control_flow_validation: TemplateControlFlowValidationMode::RuntimeCapable,
            preparation_mode: TemplatePreparationMode::Value,
            default_style: None,
        }
    }

    pub(crate) fn const_required() -> Self {
        Self {
            parsing_mode: TemplateParsingMode::Standard,
            control_flow_validation: TemplateControlFlowValidationMode::ConstRequired,
            preparation_mode: TemplatePreparationMode::ConstRequired,
            default_style: None,
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
    inherited_wrappers: InheritedChildWrapperPolicy,
}

struct TemplateBodyParser<'a, 'cursor, 'types> {
    token_stream: &'a mut AstCursor<'cursor>,
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
    source_path: PathId,
    path_fork: &'a mut PathInternerFork,
    end_policy: TemplateBodyEndPolicy,
}

impl<'a, 'cursor, 'types> TemplateBodyParser<'a, 'cursor, 'types> {
    /// Parses body tokens into parser TIR.
    ///
    /// All body content — literal text, newlines, nested templates, and slots
    /// is emitted exclusively into parser TIR through `TemplateConstructionContext`.
    /// `$doc` suppresses nested template parsing, so balanced brackets in documentation bodies
    /// remain literal text. A body ends only at its own template close or its owner's donor
    /// boundary.
    fn parse_content(
        &mut self,
        input: BodyParseInput<'_, '_>,
        construction_context: &mut TemplateConstructionContext,
        end_policy: TemplateBodyEndPolicy,
    ) -> BodyParseResult<()> {
        // The tokenizer only allows for strings, templates or slots inside the template body.
        let mut last_known_span = current_token_source_span(self.token_stream);
        while self.token_stream.position() < self.token_stream.length() {
            add_ast_counter(AstCounter::TemplateBodyTokenVisits, 1);
            last_known_span = current_token_source_span(self.token_stream);

            match self.token_stream.current_tag() {
                TokenTag::EOF
                    if matches!(end_policy, TemplateBodyEndPolicy::DonorExhaustionIsClose) =>
                {
                    return Ok(());
                }
                TokenTag::EOF => {
                    return Err(CompilerDiagnostic::unexpected_end_of_file(
                        Some(self.close_bracket_id),
                        last_known_span,
                    )
                    .into());
                }
                TokenTag::TEMPLATE_CLOSE => {
                    ast_log!("Breaking out of template body. Found a template close.");
                    // Consume the closing bracket so the caller resumes after the template body.
                    self.token_stream.advance();
                    return Ok(());
                }

                TokenTag::TEMPLATE_HEAD => {
                    // When child templates are suppressed, brackets are treated as balanced
                    // literal text rather than parsed as nested templates.
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
                        )?;
                        continue;
                    }

                    self.parse_nested_template(input, construction_context)?;
                    continue;
                }

                TokenTag::RAW_STRING_LITERAL | TokenTag::STRING_SLICE_LITERAL => {
                    let content = self
                        .token_stream
                        .current_string_id_in(self.string_table)?
                        .ok_or_else(|| {
                            CompilerError::compiler_error(
                                "template-body string token had no string payload",
                            )
                        })?;
                    let byte_len = self.string_table.resolve(content).len();
                    #[cfg(feature = "detailed_timers")]
                    {
                        add_ast_counter(AstCounter::TemplateTextBytesParsed, byte_len);
                    }
                    construction_context.record_text(content, byte_len, last_known_span);
                }

                TokenTag::NEWLINE => {
                    add_ast_counter(AstCounter::TemplateTextBytesParsed, 1);
                    construction_context.record_text(self.newline_id, 1, last_known_span);
                }

                _ => {
                    let found_token = self
                        .token_stream
                        .current_diagnostic_token(self.string_table)
                        .map_err(|error| {
                            CompilerDiagnostic::token_view_invariant_error(
                                error,
                                "template-body unexpected-token diagnostic",
                            )
                        })?
                        .unwrap_or_else(|| {
                            DiagnosticToken::from_static_tag(self.token_stream.current_tag())
                        });
                    let mut diagnostic =
                        CompilerDiagnostic::unexpected_token_from_tag(found_token, last_known_span);
                    diagnostic.primary_span = last_known_span;
                    return Err(diagnostic.into());
                }
            }

            self.token_stream.advance();
        }

        if matches!(end_policy, TemplateBodyEndPolicy::DonorExhaustionIsClose) {
            Ok(())
        } else {
            Err(CompilerDiagnostic::unexpected_end_of_file(
                Some(self.close_bracket_id),
                last_known_span,
            )
            .into())
        }
    }

    /// Parses one selected body under the conditional's binding scope.
    ///
    /// The body does not inherit parent wrappers directly. Composition applies those wrappers
    /// conditionally to the complete child instead.
    fn parse_if_body(
        &mut self,
        build_state: &mut TemplateBuildState,
        construction_context: &mut TemplateConstructionContext,
        input: TemplateIfBodyParseInput,
    ) -> BodyParseResult<()> {
        let conditional_span = input.span;
        let mut body_construction_context =
            tir_only_body_construction_context(construction_context.span(), &input.then_context);
        let parse_input = BodyParseInput {
            context: &input.then_context,
            build_state,
            inherited_wrappers: InheritedChildWrapperPolicy::Skip,
        };
        self.parse_content(parse_input, &mut body_construction_context, self.end_policy)?;

        let body_node_id = finalize_tir_body_builder(
            build_state.style.clone(),
            build_state.kind.clone(),
            body_construction_context,
        )?;
        construction_context.record_conditional(input.selector, body_node_id, conditional_span);

        Ok(())
    }
    fn parse_loop_body(
        &mut self,
        build_state: &mut TemplateBuildState,
        construction_context: &mut TemplateConstructionContext,
        input: TemplateLoopBodyParseInput,
    ) -> BodyParseResult<()> {
        let mut body_construction_context =
            tir_only_body_construction_context(construction_context.span(), &input.body_context);
        let parse_input = BodyParseInput {
            context: &input.body_context,
            build_state,
            inherited_wrappers: InheritedChildWrapperPolicy::Skip,
        };

        self.parse_content(
            parse_input,
            &mut body_construction_context,
            TemplateBodyEndPolicy::RequireClose,
        )?;

        let body_node_id = finalize_tir_body_builder(
            build_state.style.clone(),
            build_state.kind.clone(),
            body_construction_context,
        )?;

        construction_context.record_loop(input.header, body_node_id, input.span);

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
            default_style: self.default_style.clone(),
        };

        let child_construction = Template::new_nested_template(
            self.token_stream,
            self.source_path,
            input.context,
            self.type_interner,
            nested_direct_child_wrappers,
            parse_options,
            TemplatePathTables {
                string_table: self.string_table,
                path_fork: self.path_fork,
            },
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
            for (template_id, span) in contributions {
                construction_context.record_insert_contribution(template_id, span);
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
                    child_template.span,
                );

                construction_context.record_slot(slot_placeholder, child_template.span)?;
                return Ok(());
            }
        }

        // Ordinary nested child templates (String, SlotInsert) are fully
        // TIR-owned: `record_parser_tir_child_template` above recorded the
        // child reference in parser TIR, and the TIR fold/format/handoff
        // pipeline owns composition, folding, and runtime handoff.
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
        child_template.span,
    );
}

fn record_parser_tir_insert_contribution(
    construction_context: &mut TemplateConstructionContext,
    child_template: &Template,
) {
    let child_template_id = child_template.tir_reference.root;

    construction_context.record_insert_contribution(child_template_id, child_template.span);
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
    span: Option<SourceSpan>,
    context: &ScopeContext,
) -> TemplateConstructionContext {
    TemplateConstructionContext::new(context.template_ir_store.clone(), span)
}
fn current_token_source_span(token_stream: &AstCursor) -> Option<SourceSpan> {
    Some(token_stream.current_span())
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
    token_stream: &mut AstCursor,
    construction_context: &mut TemplateConstructionContext,
    string_table: &mut StringTable,
    text_ids: LiteralTemplateTextIds,
) -> BodyParseResult<()> {
    // Emit the opening bracket as literal text.
    add_ast_counter(AstCounter::TemplateTextBytesParsed, 1);
    let span = current_token_source_span(token_stream);
    construction_context.record_text(text_ids.open_bracket_id, 1, span);
    token_stream.advance();

    let mut balance = TemplateBalance::with_opening_template();
    while balance.has_unclosed_templates() {
        let tag = token_stream.current_tag();
        if tag == TokenTag::EOF {
            return Ok(());
        }
        let span = current_token_source_span(token_stream);
        balance.step_tag(tag);
        match tag {
            TokenTag::TEMPLATE_HEAD => {
                add_ast_counter(AstCounter::TemplateTextBytesParsed, 1);
                construction_context.record_text(text_ids.open_bracket_id, 1, span);
            }

            TokenTag::TEMPLATE_CLOSE => {
                add_ast_counter(AstCounter::TemplateTextBytesParsed, 1);
                construction_context.record_text(text_ids.close_bracket_id, 1, span);
            }

            TokenTag::RAW_STRING_LITERAL | TokenTag::STRING_SLICE_LITERAL => {
                let content = token_stream
                    .current_string_id_in(string_table)?
                    .ok_or_else(|| {
                        CompilerError::compiler_error(
                            "literal template body string token had no string payload",
                        )
                    })?;
                let byte_len = string_table.resolve(content).len();
                #[cfg(feature = "detailed_timers")]
                {
                    add_ast_counter(AstCounter::TemplateTextBytesParsed, byte_len);
                }
                construction_context.record_text(content, byte_len, span);
            }

            TokenTag::NEWLINE => {
                add_ast_counter(AstCounter::TemplateTextBytesParsed, 1);
                construction_context.record_text(text_ids.newline_id, 1, span);
            }

            TokenTag::SYMBOL | TokenTag::STYLE_DIRECTIVE => {
                let id = token_stream
                    .current_string_id_in(string_table)?
                    .ok_or_else(|| {
                        CompilerError::compiler_error(
                            "literal template body symbol token had no string payload",
                        )
                    })?;
                let prefix = if tag == TokenTag::STYLE_DIRECTIVE {
                    "$"
                } else {
                    ""
                };
                let name = string_table.resolve(id);
                let literal = format!("{prefix}{name}");
                add_ast_counter(AstCounter::TemplateTextBytesParsed, literal.len());
                let literal_id = string_table.intern(&literal);
                construction_context.record_text(literal_id, literal.len(), span);
            }

            TokenTag::START_TEMPLATE_BODY | TokenTag::COLON => {
                add_ast_counter(AstCounter::TemplateTextBytesParsed, 1);
                let colon_id = string_table.intern(":");
                construction_context.record_text(colon_id, 1, span);
            }

            TokenTag::COMMA => {
                add_ast_counter(AstCounter::TemplateTextBytesParsed, 1);
                let comma_id = string_table.intern(",");
                construction_context.record_text(comma_id, 1, span);
            }

            TokenTag::OPEN_PARENTHESIS => {
                add_ast_counter(AstCounter::TemplateTextBytesParsed, 1);
                let paren_id = string_table.intern("(");
                construction_context.record_text(paren_id, 1, span);
            }

            TokenTag::CLOSE_PARENTHESIS => {
                add_ast_counter(AstCounter::TemplateTextBytesParsed, 1);
                let paren_id = string_table.intern(")");
                construction_context.record_text(paren_id, 1, span);
            }

            // Static source words have no retained spelling payload; the boolean tag is
            // ambiguous between `true` and `false`, so leave that payload-bearing token alone.
            _ if tag != TokenTag::BOOL_LITERAL => {
                if let Some(source_word) = SourceWord::ALL
                    .iter()
                    .copied()
                    .find(|word| token_tag_for_source_word(*word) == tag)
                {
                    let spelling = source_word.spelling();
                    let spelling_id = string_table.intern(spelling);
                    add_ast_counter(AstCounter::TemplateTextBytesParsed, spelling.len());
                    construction_context.record_text(spelling_id, spelling.len(), span);
                }
            }

            _ => {}
        }
        token_stream.advance();
    }
    Ok(())
}

// -------------------------
//  Internal Helpers
