//! Body-root numeric completion driver.
//!
//! WHAT: resolves the body-owned numeric origins from every permitted receiving fact, materialises
//!       each retained literal recipe exactly once at its selected destination, upgrades the
//!       pending frame slots in place, resumes every retained continuation in authored order and
//!       releases the construction tables before the body is published.
//! WHY: a binding's single static type is selected from every use in its body, so numeric-dependent
//!      statements cannot complete where they were parsed. Completion happens at the independent
//!      body root (function, root runtime body or concrete generic body), which is also the only
//!      place that owns the construction state.
//!
//! Nested bodies share the enclosing body's draft and domain, so their statements resume through
//! the same ordered continuation list. No statement is re-parsed, no argument routing is rebuilt
//! from tokens, and no pending ID or recipe can leave this module.

use std::cell::RefCell;
use std::rc::Rc;

use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::ast_nodes::{
    AstNode, Declaration, DeclarationValue, IfBranchMetadata, LoopBindings, MultiBindTarget,
    NodeKind, RangeLoopSpec,
};
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::expressions::assertion_message_effects::assertion_condition_is_statically_true;
use crate::compiler_frontend::ast::expressions::call_argument::CallArgument;
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::eval_expression::evaluate_expression;
use crate::compiler_frontend::ast::expressions::expression_rpn::{
    ExpressionRpnItem, single_operand_mut,
};
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExprId, ExpressionOp, OwnedExpr, PlaceId,
};
use crate::compiler_frontend::ast::expressions::expression_types::{
    ConstRecordState, FallibleExpressionHandling, FallibleHandling,
};
use crate::compiler_frontend::ast::expressions::mutation::{
    CompoundAssignmentReadyInput, evaluate_compound_assignment_with_ready_rhs,
};
use crate::compiler_frontend::ast::expressions::parse_expression::validate_completed_typed_errors;
use crate::compiler_frontend::ast::expressions::parse_expression_places::{
    place_expression_from_expression, place_expression_is_mutable,
};
use crate::compiler_frontend::ast::generic_functions::{
    GenericRequestRange, IfGenericRequestRanges,
};
use crate::compiler_frontend::ast::numeric_construction::{
    AssignmentContinuation, BindingContinuation, BodyDraft, BodyEntry,
    CompoundAssignmentContinuation, DeferredArmPattern, DeferredAssignmentTarget, DeferredBody,
    DeferredCall, DeferredCallArgument, DeferredCallHandling, DeferredConstruct,
    DeferredExpression, DeferredExpressionCompletion, DeferredExpressionServices,
    DeferredExpressionSuffix, DeferredIf, DeferredLoop, DeferredLoopHeader, DeferredMatch,
    DeferredMultiBindTarget, DeferredMultiBindValue, DeferredNumericExpression,
    DeferredRangeHeader, DeferredRangeSpec, DeferredSourceCall, ExpressionStatementContinuation,
    ForwardProducerId, MultiBindContinuation, NumericBindingId, NumericConstructionState,
    NumericContinuation, NumericContinuationId, NumericRecipe, PendingBindingRow,
    PendingMultiBindProducer, PendingMultiBindProducerKind, ReturnContinuation,
    StatementExpression, accept_forward_binding_result, binding_annotation_type,
    binding_known_shape, directed_annotation_destination, merged_receiver_type,
    numeric_witness_type, resolve_raw_binding, union_alias_classes,
};
use crate::compiler_frontend::ast::numeric_known_shape::{
    KnownShape, KnownShapeId, materialise_known_shape, numeric_class_type, pin_numeric_leaves,
    register_ordered_shape_witness,
};
use crate::compiler_frontend::ast::statements::body_expr_stmt::validate_resumed_statement_expression;
use crate::compiler_frontend::ast::statements::condition_validation::{
    ensure_boolean_condition, ensure_loop_condition,
};
use crate::compiler_frontend::ast::statements::fallible_handling::HandledFallibleCall;
use crate::compiler_frontend::ast::statements::match_patterns::{
    MatchPattern, ensure_pattern_compatible_with_subject, finish_pending_option_pattern,
};
use crate::compiler_frontend::ast::templates::template_head_parser::validate_template_head_value_type;
use crate::compiler_frontend::ast::templates::tir::{
    NumericTemplateIntakePosition, RangeOperandSlot, TemplateIrNodeId,
};
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::builtins::casts::resolution::{
    CastResolutionInput, resolve_cast_expression,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidAssignmentTargetReason, TypeMismatchContext, ValueReceiverContext,
};
use crate::compiler_frontend::datatypes::diagnostic_type_spelling;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::type_coercion::contextual::coerce_expression_to_explicit_type_boundary;
use crate::compiler_frontend::type_coercion::parse_context::ExpectedType;
use crate::compiler_frontend::value_mode::ValueMode;

/// One resolved pending binding: its selected type, the metadata later uses need, and the
/// completed declaration payload until it moves into its single authored AST position.
///
/// WHY: later copies, places and substituted operands need the binding's identity, mode and
/// const-record classification even after its declaration moves. The completed initializer
/// remains an immutable store ID; the declaration itself is taken once at its authored position.
struct ResolvedBinding {
    type_id: TypeId,
    /// The completed declaration; taken exactly once by its owning AST position.
    declaration: Option<Declaration>,
    /// `declaration.id`, stable after the payload moves.
    declaration_id: PathId,
    /// The authored `=`/`~=` value mode of the binding.
    value_mode: ValueMode,
    /// Const-record classification of the completed initializer.
    const_record_state: ConstRecordState,
    /// The authored binding-name span, for target diagnostics.
    binding_span: Option<SourceSpan>,
}

impl ResolvedBinding {
    fn new(
        type_id: TypeId,
        declaration: Declaration,
        expressions: &AstExpressionBuilder,
    ) -> Result<Self, ExpressionParseError> {
        let const_record_state = match declaration.value.initializer() {
            Some(value) => expressions.view(value)?.classification().const_record_state,
            None => ConstRecordState::RuntimeValue,
        };
        Ok(Self {
            declaration_id: declaration.id,
            value_mode: declaration.value_mode,
            const_record_state,
            binding_span: declaration.binding_span,
            type_id,
            declaration: Some(declaration),
        })
    }
}

/// Lifecycle of one numeric binding's completion.
///
/// WHAT: pending until its recipe is taken, resolving while that recipe completes, then resolved
///       with its installed entry.
/// WHY: a forward producer that depends on the binding it updates can demand that binding while it
///      resolves. That demand must read the seeded storage facts, and the recipe must never be taken
///      twice.
// Boxing each resolved entry would add one heap allocation per numeric binding, and the slots
// live in one dense vector, so the size gap between variants is accepted here.
#[allow(clippy::large_enum_variant)]
enum BindingSlot {
    Pending,
    Resolving(ResolvingBinding),
    Resolved(ResolvedBinding),
}

/// The in-flight facts of a binding whose recipe is still completing.
struct ResolvingBinding {
    /// The known shape the recipe owned, captured before the recipe moved out.
    shape: Option<KnownShapeId>,
    /// The storage type a cyclic use seeded; completion must install exactly this type.
    seed: Option<TypeId>,
}

/// The facts one use of a numeric binding reads from its owner.
struct BindingUseFacts {
    type_id: TypeId,
    declaration_id: PathId,
    value_mode: ValueMode,
    const_record_state: ConstRecordState,
}

/// Resolution state of one retained continuation.
///
/// WHAT: explicit unresolved/resolving/completed/consumed states instead of an optional node
///       vector.
/// WHY: a nested draft (retained value block or catch handler) splices a continuation's nodes
///      into its authored position; the dense sweep that follows must not mistake that drained
///      slot for unresolved work, and each completed payload must move to exactly one AST
///      position.
enum ContinuationResolution {
    Unresolved,
    Resolving,
    Completed(Vec<AstNode>),
    Consumed,
}

/// Numeric completion session for one body root.
struct Completion<'a, 'env> {
    context: &'a ScopeContext,
    type_interner: &'a mut AstTypeInterner<'env>,
    expression_builder: &'a mut AstExpressionBuilder,
    const_values: &'a mut ConstValueStore,
    string_table: &'a mut StringTable,
    path_fork: &'a mut PathInternerFork,
    state: Rc<RefCell<NumericConstructionState>>,
    bindings: Vec<BindingSlot>,
    resumed: Vec<ContinuationResolution>,
    /// The completed range spec each retained loop counter produced during this pass.
    range_specs: Vec<Option<RangeLoopSpec>>,
    /// Set while one catch-protected fragment completes: nested producers stay open until the
    /// enclosing catch attaches, and the recovered expression is validated at its own boundary.
    protected_typed_errors: bool,
    /// The resolved produced-value expectations of the value-producing site whose nested body is
    /// currently completing (innermost last); see `resolved_value_slot_expectations`.
    resolved_value_slots: Vec<Option<Vec<TypeId>>>,
}

/// Complete every numeric-dependent statement of one body root.
///
/// WHAT: the caller is the body root: a function body, the implicit root runtime body, or one
///       concrete generic/materialised body. Child blocks never call this.
/// WHY: completion is the single owner that may release the body's construction tables, so it must
///      run exactly once, after the whole body has been parsed.
pub(crate) fn complete_numeric_body(
    draft: BodyDraft,
    context: &ScopeContext,
    type_interner: &mut AstTypeInterner<'_>,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> Result<Vec<AstNode>, ExpressionParseError> {
    // The session runs for every root: a domain can hold taken template intakes even when no
    // binding row survived, so no entry-list shortcut may bypass intake completion.
    let state = context.numeric_construction_handle();
    let binding_count = state.borrow().bindings().len();
    let continuation_count = state.borrow().continuation_count();
    let mut completion = Completion {
        context,
        type_interner,
        expression_builder,
        const_values,
        string_table,
        path_fork,
        state,
        bindings: (0..binding_count).map(|_| BindingSlot::Pending).collect(),
        range_specs: (0..binding_count).map(|_| None).collect(),
        resumed: (0..continuation_count)
            .map(|_| ContinuationResolution::Unresolved)
            .collect(),
        protected_typed_errors: false,
        resolved_value_slots: Vec::new(),
    };
    let result = completion.run(draft);
    // No origin, recipe, pending ID, forward producer or suspended check may outlive the body
    // boundary, on success or on failure.
    completion.state.borrow_mut().release();
    result
}

impl Completion<'_, '_> {
    fn run(&mut self, draft: BodyDraft) -> Result<Vec<AstNode>, ExpressionParseError> {
        self.link_alias_classes()?;
        self.resolve_bindings()?;
        self.complete_template_intakes()?;
        self.resolve_draft(draft)
    }

    /// Splice the root body's draft in authored order, pulling each retained position on demand.
    fn resolve_draft(&mut self, draft: BodyDraft) -> Result<Vec<AstNode>, ExpressionParseError> {
        let mut nodes = Vec::with_capacity(draft.entries.len());
        for entry in draft.entries {
            match entry {
                BodyEntry::Node(node) => nodes.push(node),
                BodyEntry::Pending(id) => {
                    // Nested constructs pull their own drafts, and each live dependency is resolved
                    // before its owning authored position is emitted.
                    self.resolve_continuation(id)?;
                    let state = self.resumed.get_mut(id.as_usize()).ok_or_else(|| {
                        CompilerError::compiler_error(
                            "numeric construction completion addressed a continuation \
                                 outside its own dense table",
                        )
                    })?;
                    let ContinuationResolution::Completed(resumed) =
                        std::mem::replace(state, ContinuationResolution::Consumed)
                    else {
                        return Err(CompilerError::compiler_error(
                            "numeric construction completion reached a continuation whose nodes \
                             were already spliced into another AST position",
                        )
                        .into());
                    };
                    nodes.extend(resumed);
                }
            }
        }
        Ok(nodes)
    }

    /// Complete every numeric-dependent dynamic expression a template retained.
    ///
    /// WHAT: takes each registered template intake from the module-shared TIR store, completes
    ///       its retained fragment in the authored receiving context and installs the completed
    ///       value back into the recorded authored position. Range-loop operands complete as one
    ///       header per recorded node and install through the range-header entry point.
    /// WHY: a formatting consumer does not constrain its operand, so a template payload keeps its
    ///      pending recipe until the body root has selected the binding's single static type.
    ///      Installation must happen after origins resolve and before construction state is
    ///      released, so no pending recipe survives into a published template. Grouping happens
    ///      before completion and installation so no store borrow is held across fragment work.
    ///
    /// Nested statement blocks share this body's domain, while each independent function body
    /// owns its own. The module-shared TIR store therefore drains only this root's issuing domain.
    fn complete_template_intakes(&mut self) -> Result<(), ExpressionParseError> {
        let domain = self.context.numeric_domain_id();
        let intakes = self
            .context
            .template_ir_store
            .borrow_mut()
            .take_pending_numeric_intakes(domain);

        // Dense body-local groups arrive in first-operand registration order. Their store row
        // handles let installation address exactly this header and its at-most-three intake sites.
        let mut range_headers = Vec::new();

        for intake in intakes {
            if let Some(slot) = intake.position.range_operand_slot() {
                // Range slots are registered with their containing header and group.
                let (header, group) = intake.range_header.expect("retained range operand handle");
                let value = self.complete_items_with_suffix(
                    intake.items,
                    intake.expected,
                    &intake.value_mode,
                    intake.suffix,
                )?;
                validate_completed_typed_errors(self.expression_builder.view_owned(value)?)?;
                if group == range_headers.len() {
                    range_headers.push((header, [None, None, None]));
                }
                let operands = &mut range_headers[group].1;
                let operand = match slot {
                    RangeOperandSlot::Start => &mut operands[0],
                    RangeOperandSlot::End => &mut operands[1],
                    RangeOperandSlot::Step => &mut operands[2],
                };
                *operand = Some(value.id());
                continue;
            }
            // A retained collection header's pending item installs before its iterable completes,
            // so the header's bindings are final when the collection completion takes them.
            self.install_pending_collection_item(intake.position.node_id())?;
            let value = self.complete_items_with_suffix(
                intake.items,
                intake.expected,
                &intake.value_mode,
                intake.suffix,
            )?;
            validate_completed_typed_errors(self.expression_builder.view_owned(value)?)?;
            if let NumericTemplateIntakePosition::HeadExpression(node_id) = intake.position {
                // A head is a formatting consumer: its completed value must be renderable text.
                // Template values take the head's own template path, not this scalar check.
                if !matches!(
                    self.expression_builder.view_owned(value)?.operation_id(),
                    ExpressionOp::Template(_)
                ) {
                    let span = self
                        .context
                        .template_ir_store
                        .borrow()
                        .get_node(node_id)
                        .and_then(|node| node.span);
                    validate_template_head_value_type(
                        self.expression_builder.view_owned(value)?,
                        span,
                        self.type_interner.environment(),
                    )
                    .map_err(ExpressionParseError::from)?;
                }
            }
            // An option-present selector finishes its deferred capture pattern against the
            // completed subject's type here, outside the store borrow, because finishing a
            // retained payload resolves through this body root.
            let pending_selector_pattern = self
                .context
                .template_ir_store
                .borrow_mut()
                .take_pending_selector_pattern(intake.position.node_id());
            let selector_pattern = match pending_selector_pattern {
                Some(pattern) => Some(finish_retained_selector_pattern(
                    *pattern,
                    self.expression_builder
                        .view_owned(value)?
                        .require_single(ValueReceiverContext::MatchScrutinee)?,
                    self,
                )?),
                None => None,
            };
            self.context
                .template_ir_store
                .borrow_mut()
                .install_completed_numeric_intake(intake.site, value.id(), selector_pattern)?;
        }

        for (header, operands) in range_headers {
            self.context
                .template_ir_store
                .borrow_mut()
                .install_completed_numeric_range_header(
                    header,
                    operands,
                    self.context,
                    self.type_interner,
                    self.expression_builder,
                    self.const_values,
                    self.string_table,
                    self.path_fork,
                )
                .map_err(ExpressionParseError::from)?;
        }
        Ok(())
    }

    /// Merge every copy/equality relationship into one origin class before any selection runs.
    fn link_alias_classes(&mut self) -> Result<(), ExpressionParseError> {
        let binding_count = self.state.borrow().bindings().len();
        for index in 0..binding_count {
            let binding = NumericBindingId::from_dense_index(index)?;
            let mut state = self.state.borrow_mut();
            let alias = match &state.binding(binding).recipe {
                Some(NumericRecipe::Alias { source, .. }) => Some(*source),
                _ => None,
            };
            if let Some(source) = alias {
                union_alias_classes(&mut state, binding, source);
            }
            let producer_count = match &state.binding(binding).recipe {
                Some(NumericRecipe::MultiBindSlot { producers, .. }) => producers.len(),
                _ => 0,
            };
            for producer_index in 0..producer_count {
                let source = match &state.binding(binding).recipe {
                    Some(NumericRecipe::MultiBindSlot { producers, .. }) => {
                        match producers[producer_index].kind {
                            PendingMultiBindProducerKind::Binding(source) => Some(source),
                            PendingMultiBindProducerKind::Raw(_)
                            | PendingMultiBindProducerKind::Forward => None,
                        }
                    }
                    _ => None,
                };
                if let Some(source) = source {
                    union_alias_classes(&mut state, binding, source);
                }
            }
        }
        Ok(())
    }

    fn resolve_bindings(&mut self) -> Result<(), ExpressionParseError> {
        let binding_count = self.state.borrow().bindings().len();
        for index in 0..binding_count {
            self.ensure_binding_resolved(NumericBindingId::from_dense_index(index)?)?;
        }
        Ok(())
    }

    /// Resolve one pending binding exactly once, on first demand.
    ///
    /// WHAT: a binding resolves from its retained recipe the first time anything needs its type:
    /// the body's own loop, an alias or element source, a forward operand, or a multi-bind slot.
    /// WHY: a dependency's type must be fixed before a dependent reads it, and an installed
    /// binding must never resolve twice.
    fn ensure_binding_resolved(
        &mut self,
        binding: NumericBindingId,
    ) -> Result<(), ExpressionParseError> {
        let pending = self
            .bindings
            .get(binding.as_usize())
            .is_some_and(|slot| matches!(slot, BindingSlot::Pending));
        if pending {
            // Move the retained recipe out once: every path below consumes it, and nothing may
            // probe or re-resolve it afterwards. The slot enters `Resolving` first, so a cyclic
            // demand reads the seeded facts instead of taking the recipe again.
            let shape = binding_known_shape(&self.state.borrow(), binding);
            self.bindings[binding.as_usize()] =
                BindingSlot::Resolving(ResolvingBinding { shape, seed: None });
            let recipe = self.state.borrow_mut().take_binding_recipe(binding)?;
            let row = self.state.borrow().binding(binding).clone();
            let resolved = match recipe {
                NumericRecipe::Raw { items, family } => {
                    let recipe_span = items
                        .first()
                        .map(|item| item.source_span(self.expression_builder))
                        .transpose()?
                        .flatten();
                    // A numeric receiver types the raw leaves; without one the authored family
                    // default does. The literal destination is never the binding's final type:
                    // a comparison completes into `Bool` and integer division into `F64` even
                    // when the leaves materialised in `I32`, so the computed result is recorded
                    // and the receivers are validated against it.
                    // A fragment whose own operators fix the result (comparison, whole division) is
                    // not typed by a fixed literal destination: it resolves through the ordinary
                    // peer machinery, where the completed division result supplies the eligible
                    // local peer that types its adjacent raw literal, so `1 / 2 / 3` and
                    // `1 / 2 + 1` stay F64 instead of mixing a completed F64 result with an I32
                    // leaf. A receiver still never types those leaves; it validates the result.
                    let fixed_result = crate::compiler_frontend::ast::numeric_construction::raw_fragment_result_is_independently_fixed(
                        &items,
                        family,
                    );
                    let receiver_destination = {
                        let mut state = self.state.borrow_mut();
                        resolve_raw_binding(
                            &mut state,
                            binding,
                            family,
                            recipe_span,
                            self.type_interner.environment(),
                            fixed_result,
                        )?
                    };
                    let destination = match receiver_destination {
                        Some(receiver_type_id) => ExpectedType::NumericLiteral(receiver_type_id),
                        None if fixed_result => ExpectedType::Infer,
                        None => ExpectedType::NumericLiteral(family.default_type_id()),
                    };
                    let value = self.complete_items(items, destination, &row.value_mode)?;
                    // Binding initializers and template intakes are closed receiving sites: an enclosing
                    // catch never handles an independent initializer, so the completed value is validated
                    // before it is accepted or installed.
                    validate_completed_typed_errors(self.expression_builder.view_owned(value)?)?;
                    let computed = self
                        .expression_builder
                        .view_owned(value)?
                        .require_single(ValueReceiverContext::Declaration)?;
                    {
                        let mut state = self.state.borrow_mut();
                        accept_forward_binding_result(&mut state, binding, computed, recipe_span)?;
                    }
                    let value = self.coerce_to_binding_annotation(binding, value)?;
                    let declaration = declaration_for(
                        &row,
                        value,
                        self.expression_builder,
                        self.type_interner.environment(),
                    )?;
                    ResolvedBinding::new(
                        self.expression_builder.view_owned(value)?.value_type_id()?,
                        declaration,
                        self.expression_builder,
                    )?
                }
                NumericRecipe::Directed { items, .. } => {
                    let recipe_span = items
                        .first()
                        .map(|item| item.source_span(self.expression_builder))
                        .transpose()?
                        .flatten();
                    // Only the initializer's own annotation is an immediate receiver. A later class
                    // witness never types these raw leaves; the acceptance check below validates
                    // the completed value against it.
                    let destination = directed_annotation_destination(
                        &self.state.borrow(),
                        self.type_interner.environment(),
                        binding,
                    )
                    .map_or(ExpectedType::Infer, ExpectedType::NumericLiteral);
                    let value = self.complete_items(items, destination, &row.value_mode)?;
                    validate_completed_typed_errors(self.expression_builder.view_owned(value)?)?;
                    let computed = self
                        .expression_builder
                        .view_owned(value)?
                        .require_single(ValueReceiverContext::Declaration)?;
                    {
                        let mut state = self.state.borrow_mut();
                        accept_forward_binding_result(&mut state, binding, computed, recipe_span)?;
                    }
                    let value = self.coerce_to_binding_annotation(binding, value)?;
                    let declaration = declaration_for(
                        &row,
                        value,
                        self.expression_builder,
                        self.type_interner.environment(),
                    )?;
                    ResolvedBinding::new(
                        self.expression_builder.view_owned(value)?.value_type_id()?,
                        declaration,
                        self.expression_builder,
                    )?
                }
                NumericRecipe::RangeCounter { range } => {
                    // The counter's type is the iteration domain selected by its own completed
                    // operands. The shared range policy selected it while building the spec, so it
                    // resolves here in the binding pass, before any later row that reads it, and
                    // never by resuming the loop construct (its body may read the counter).
                    let recipe_span = range.span.or(row.binding_span);
                    let spec = self.complete_range_spec(*range)?;
                    let computed =
                        crate::compiler_frontend::ast::statements::loop_headers::range_binding_type(
                            &spec,
                            self.type_interner.environment(),
                        );
                    {
                        let mut state = self.state.borrow_mut();
                        accept_forward_binding_result(&mut state, binding, computed, recipe_span)?;
                    }
                    self.range_specs[binding.as_usize()] = Some(spec);
                    let resolved = ResolvedBinding::new(
                        computed,
                        absent_declaration(&row, computed, self.type_interner.environment()),
                        self.expression_builder,
                    )?;
                    // A template range header bound to this counter installs its finished spec
                    // right here: its operands live in this recipe, which already ran the shared
                    // range policy exactly once.
                    self.install_bound_template_range_headers(binding, &resolved)?;
                    resolved
                }
                NumericRecipe::MultiBindSlot { producers, .. } => {
                    let type_id = self.resolve_multi_bind_slot(binding, producers)?;
                    ResolvedBinding::new(
                        type_id,
                        absent_declaration(&row, type_id, self.type_interner.environment()),
                        self.expression_builder,
                    )?
                }
                NumericRecipe::Alias { source, is_copy } => {
                    let source_facts = self.binding_use_facts(source)?;
                    let type_id = source_facts.type_id;
                    let value = {
                        let expressions = &mut *self.expression_builder;
                        alias_value(
                            &row,
                            source_facts.declaration_id,
                            &source_facts.value_mode,
                            source_facts.const_record_state,
                            type_id,
                            is_copy,
                            expressions,
                        )?
                    };
                    let value = self.coerce_to_binding_annotation(binding, value)?;
                    let declaration = declaration_for(
                        &row,
                        value,
                        self.expression_builder,
                        self.type_interner.environment(),
                    )?;
                    ResolvedBinding::new(
                        self.expression_builder.view_owned(value)?.value_type_id()?,
                        declaration,
                        self.expression_builder,
                    )?
                }
                NumericRecipe::CollectionElement { element } => {
                    // A collection-element loop item has no initializer of its own; its loop
                    // supplies the value. Its type is the source element shape materialised to a
                    // `TypeId`; a numeric element shares its leaf class with the source literal.
                    self.settle_forward_producers(element)?;
                    let type_id = {
                        let mut state = self.state.borrow_mut();
                        materialise_known_shape(
                            &mut state,
                            self.type_interner.environment_mut_for_derived_types(),
                            element,
                        )?
                    };
                    ResolvedBinding::new(
                        type_id,
                        absent_declaration(&row, type_id, self.type_interner.environment()),
                        self.expression_builder,
                    )?
                }
                NumericRecipe::CaughtError { shape } | NumericRecipe::PatternPayload { shape } => {
                    // A pending handler's error or pattern payload capture has no initializer: the
                    // protected call or the matched subject supplies the value. Its type is the shape
                    // materialised once its forward producers settle, so the binding reads the
                    // producer's concrete type exactly once.
                    self.settle_forward_producers(shape)?;
                    let type_id = {
                        let mut state = self.state.borrow_mut();
                        materialise_known_shape(
                            &mut state,
                            self.type_interner.environment_mut_for_derived_types(),
                            shape,
                        )?
                    };
                    ResolvedBinding::new(
                        type_id,
                        absent_declaration(&row, type_id, self.type_interner.environment()),
                        self.expression_builder,
                    )?
                }
            };
            self.install_binding(binding, resolved)?;
        }
        Ok(())
    }

    /// Apply a pending binding's declared annotation to its completed initializer.
    ///
    /// WHAT: coerces the completed value once through the ordinary explicit-type boundary to the
    ///       binding's annotated type, such as `T?` around a constructed `T`. A binding without a
    ///       recorded annotation, or whose value already has that type, keeps its value unchanged.
    /// WHY: reads of an annotated binding already present the annotated type, so the stored
    ///      declaration must hold it too. The constructor keeps its own type and the receiving
    ///      context performs the one wrap; aggregates are never rewritten recursively.
    fn coerce_to_binding_annotation(
        &mut self,
        binding: NumericBindingId,
        value: OwnedExpr,
    ) -> Result<OwnedExpr, ExpressionParseError> {
        let Some(annotated) = binding_annotation_type(&self.state.borrow(), binding) else {
            return Ok(value);
        };
        let value = coerce_expression_to_explicit_type_boundary(
            value,
            annotated,
            self.type_interner.environment(),
            TypeMismatchContext::Declaration,
            self.expression_builder,
        )?;
        Ok(value)
    }

    /// Select an inferred multi-bind slot from its real retained producers.
    ///
    /// WHAT: a direct binding shares the numeric equality class, a raw origin uses its receiver or
    ///       authored family, and a forward expression completes naturally: no later receiver
    ///       supplies its literal destination, and its result is validated against the slot.
    /// WHY: later reads need the slot in the ordinary pending frame, while every producer's result
    ///      remains owned by its expression and moves into its authored branch exactly once.
    fn resolve_multi_bind_slot(
        &mut self,
        binding: NumericBindingId,
        producers: Vec<PendingMultiBindProducer>,
    ) -> Result<TypeId, ExpressionParseError> {
        let direct_source = producers.iter().find_map(|producer| match producer.kind {
            PendingMultiBindProducerKind::Binding(source) => Some((source, producer.span)),
            PendingMultiBindProducerKind::Raw(_) | PendingMultiBindProducerKind::Forward => None,
        });
        if let Some((source, span)) = direct_source {
            let type_id = self.binding_use_type(source)?;
            {
                let mut state = self.state.borrow_mut();
                accept_forward_binding_result(&mut state, binding, type_id, span)?;
            }
            for producer in producers
                .iter()
                .copied()
                .filter(|producer| matches!(producer.kind, PendingMultiBindProducerKind::Forward))
            {
                self.complete_multi_bind_forward_producer(producer, ExpectedType::Infer)?;
            }
            self.apply_multi_bind_raw_receivers(&producers, type_id)?;
            return Ok(type_id);
        }

        if producers
            .iter()
            .any(|producer| matches!(producer.kind, PendingMultiBindProducerKind::Forward))
        {
            let mut selected_type = None;
            for producer in producers
                .iter()
                .copied()
                .filter(|producer| matches!(producer.kind, PendingMultiBindProducerKind::Forward))
            {
                let type_id =
                    self.complete_multi_bind_forward_producer(producer, ExpectedType::Infer)?;
                if selected_type.is_none() {
                    selected_type = Some((type_id, producer.span));
                }
            }
            let (type_id, span) = selected_type.ok_or_else(|| {
                CompilerError::compiler_error(
                    "a retained multi-bind forward producer disappeared during slot resolution",
                )
            })?;
            {
                let mut state = self.state.borrow_mut();
                accept_forward_binding_result(&mut state, binding, type_id, span)?;
            }
            self.apply_multi_bind_raw_receivers(&producers, type_id)?;
            return Ok(type_id);
        }

        let Some(producer) = producers.first().copied() else {
            return Err(CompilerError::compiler_error(
                "an inferred multi-bind target had no retained producer route",
            )
            .into());
        };
        let PendingMultiBindProducerKind::Raw(family) = producer.kind else {
            return Err(CompilerError::compiler_error(
                "a direct multi-bind producer was not selected as its slot origin",
            )
            .into());
        };
        let receiver_destination = {
            let mut state = self.state.borrow_mut();
            resolve_raw_binding(
                &mut state,
                binding,
                family,
                producer.span,
                self.type_interner.environment(),
                false,
            )?
        };
        let type_id = receiver_destination.unwrap_or_else(|| family.default_type_id());
        {
            let mut state = self.state.borrow_mut();
            accept_forward_binding_result(&mut state, binding, type_id, producer.span)?;
        }
        if receiver_destination.is_some() {
            self.apply_multi_bind_raw_receivers(&producers, type_id)?;
        }
        Ok(type_id)
    }

    fn apply_multi_bind_raw_receivers(
        &mut self,
        producers: &[PendingMultiBindProducer],
        type_id: TypeId,
    ) -> Result<(), ExpressionParseError> {
        for producer in producers {
            if matches!(producer.kind, PendingMultiBindProducerKind::Raw(_)) {
                self.state.borrow_mut().set_then_value_expected_type(
                    producer.continuation,
                    producer.value_index,
                    type_id,
                )?;
            }
        }
        Ok(())
    }

    /// Complete one forward producer without turning its expression result into a raw origin.
    ///
    /// The destination is an immediate receiver's numeric literal, or `Infer` for an inferred
    /// slot. The ordinary expression owner still computes and validates the result before it
    /// returns to the authored `then` slot.
    fn complete_multi_bind_forward_producer(
        &mut self,
        producer: PendingMultiBindProducer,
        expected: ExpectedType,
    ) -> Result<TypeId, ExpressionParseError> {
        let expression = {
            let mut state = self.state.borrow_mut();
            state.take_then_value(producer.continuation, producer.value_index)?
        };
        let expression = match expression {
            StatementExpression::Built(expression) => StatementExpression::Built(expression),
            StatementExpression::Retained(mut deferred) => {
                deferred.expected = expected;
                StatementExpression::Retained(deferred)
            }
        };
        let expression = self.resolve_protected_statement_expression(expression)?;
        let type_id = self
            .expression_builder
            .view_owned(expression)?
            .require_single(ValueReceiverContext::Then)?;
        self.state.borrow_mut().complete_then_value(
            producer.continuation,
            producer.value_index,
            expression.id(),
        )?;
        Ok(type_id)
    }

    /// Upgrade the pending frame slot in place and record the resolved binding.
    fn install_binding(
        &mut self,
        binding: NumericBindingId,
        resolved: ResolvedBinding,
    ) -> Result<(), ExpressionParseError> {
        // A cyclic use already read this binding's seeded storage type, and the forward producers
        // that consumed it were checked against that type. Checking before the frame upgrade keeps
        // a violated invariant from leaving a half-installed slot.
        let seeded = match &self.bindings[binding.as_usize()] {
            BindingSlot::Resolving(resolving) => resolving.seed,
            BindingSlot::Pending | BindingSlot::Resolved(_) => None,
        };
        if seeded.is_some_and(|seeded| seeded != resolved.type_id) {
            return Err(CompilerError::compiler_error(
                "a cyclic numeric binding completed with a storage type other than its seeded type",
            )
            .into());
        }
        // Name-addressed upgrade: the frame slot is found by the authored binding name plus the
        // owning domain and the pending ID, so a same-named slot from another domain cannot be
        // replaced and the resolved declaration lands in the slot the parse created.
        let (frame, name) = {
            let state = self.state.borrow();
            let row = state.binding(binding);
            (row.frame, row.name)
        };
        let domain = self.state.borrow().domain_id();
        let declaration = resolved
            .declaration
            .as_ref()
            .ok_or_else(|| {
                CompilerError::compiler_error(
                    "a resolved binding reached its frame slot after its declaration payload moved",
                )
            })?
            .to_owned();
        let upgraded = self
            .context
            .arena
            .borrow_mut()
            .frame_mut(frame)
            .upgrade_pending_numeric_var(name, domain, binding, declaration);
        if !upgraded {
            return Err(CompilerError::compiler_error(
                "numeric construction completion found no pending frame slot for a resolved binding",
            )
            .into());
        }
        self.bindings[binding.as_usize()] = BindingSlot::Resolved(resolved);
        Ok(())
    }

    fn resolved_type(&self, binding: NumericBindingId) -> Result<TypeId, ExpressionParseError> {
        Ok(self.resolved_entry(binding)?.type_id)
    }

    /// Take one resolved binding's completed declaration for its single owning AST position.
    ///
    /// WHAT: moves the payload out; a second take is a broken invariant.
    /// WHY: each completed declaration belongs to exactly one authored position (its variable
    ///      declaration node, or a range loop's iterator bindings), and every other consumer
    ///      reads the retained metadata instead of a second declaration tree.
    fn take_declaration(
        &mut self,
        binding: NumericBindingId,
    ) -> Result<Declaration, ExpressionParseError> {
        let declaration = match self.bindings.get_mut(binding.as_usize()) {
            Some(BindingSlot::Resolved(resolved)) => resolved.declaration.take(),
            _ => None,
        };
        declaration.ok_or_else(|| {
            CompilerError::compiler_error(
                "a numeric binding's completed declaration moved to a second AST position",
            )
            .into()
        })
    }

    fn resolved_entry(
        &self,
        binding: NumericBindingId,
    ) -> Result<&ResolvedBinding, ExpressionParseError> {
        match self.bindings.get(binding.as_usize()) {
            Some(BindingSlot::Resolved(resolved)) => Ok(resolved),
            _ => Err(CompilerError::compiler_error(
                "numeric construction completion used a resolved binding before it resolved",
            )
            .into()),
        }
    }

    /// Resume one retained continuation, storing its nodes for its authored position.
    ///
    /// WHAT: takes the construction record exactly once and records one explicit
    ///       unresolved/resolving/completed/consumed state for it.
    /// WHY: authored body drafts pull continuations at their positions, while retained recipes
    ///      may pull dependencies first; the state prevents replay and double-splicing.
    fn resolve_continuation(
        &mut self,
        id: NumericContinuationId,
    ) -> Result<(), ExpressionParseError> {
        let index = id.as_usize();
        match self.resumed.get(index) {
            Some(ContinuationResolution::Completed(_) | ContinuationResolution::Consumed) => {
                return Ok(());
            }
            Some(ContinuationResolution::Resolving) => {
                return Err(CompilerError::compiler_error(
                    "numeric construction completion resumed a continuation that was still being \
                     resumed; the retained statement graph is not acyclic",
                )
                .into());
            }
            Some(ContinuationResolution::Unresolved) => {}
            None => {
                return Err(CompilerError::compiler_error(
                    "numeric construction completion addressed a continuation outside its own \
                     dense table",
                )
                .into());
            }
        }
        self.resumed[index] = ContinuationResolution::Resolving;
        let Some(continuation) = self.state.borrow_mut().take_continuation(id) else {
            return Err(CompilerError::compiler_error(
                "numeric construction completion requested a continuation that was already taken",
            )
            .into());
        };
        let nodes = match continuation {
            NumericContinuation::Binding(continuation) => self.resume_binding(continuation)?,
            NumericContinuation::Assignment(continuation) => {
                self.resume_assignment(continuation)?
            }
            NumericContinuation::CompoundAssignment(continuation) => {
                self.resume_compound_assignment(continuation)?
            }
            NumericContinuation::ExpressionStatement(continuation) => {
                self.resume_expression_statement(continuation)?
            }
            NumericContinuation::Return(continuation) => self.resume_return(continuation)?,
            NumericContinuation::ThenValue(continuation) => {
                // The enclosing value-producing site's resolved slot contract decides each
                // fallback's type: `None` means no override was pushed for this body, while
                // `Some(&[])` is a real zero-success contract. The finisher owns the node shape and
                // the ordinary receiver-aware validation.
                let resolved_slot_type_ids = self.resolved_value_slots.last().cloned().flatten();
                vec![
                    crate::compiler_frontend::ast::statements::value_production::complete_retained_then_value(
                        continuation,
                        resolved_slot_type_ids.as_deref(),
                        self,
                    )?,
                ]
            }
            NumericContinuation::MultiBind(continuation) => self.resume_multi_bind(continuation)?,
            NumericContinuation::Assertion(continuation) => self.resume_assertion(*continuation)?,
            NumericContinuation::Construct(construct) => self.resume_construct(*construct)?,
        };
        self.resumed[index] = ContinuationResolution::Completed(nodes);
        Ok(())
    }

    fn resume_binding(
        &mut self,
        continuation: BindingContinuation,
    ) -> Result<Vec<AstNode>, ExpressionParseError> {
        // The completed declaration moves into its single authored position; later uses read the
        // retained metadata and the ready frame slot.
        let declaration = self.take_declaration(continuation.binding)?;
        Ok(vec![AstNode {
            kind: NodeKind::VariableDeclaration(declaration),
            span: continuation.span,
            scope: continuation.scope,
        }])
    }

    /// Resolve one retained assignment target into the facts both mutation lanes need.
    ///
    /// WHAT: the target place, the authored declaration value mode and the declaration span.
    /// WHY: a ready target already carries its facts and a pending target's facts come from its
    ///      resolved binding row, so both lanes resume through one path with one mutability rule.
    fn assignment_target_facts(
        &mut self,
        target: DeferredAssignmentTarget,
    ) -> Result<(PlaceId, ValueMode, Option<SourceSpan>), ExpressionParseError> {
        match target {
            DeferredAssignmentTarget::Ready {
                target,
                declaration_value_mode,
                declaration_span,
            } => Ok((target, declaration_value_mode, declaration_span)),
            DeferredAssignmentTarget::Retained {
                target,
                declaration_value_mode,
                declaration_span,
            } => {
                let expression =
                    self.resolve_statement_expression(StatementExpression::Retained(target))?;
                let Some(place) =
                    place_expression_from_expression(self.expression_builder, expression)?
                else {
                    return Err(CompilerDiagnostic::invalid_assignment_target(
                        InvalidAssignmentTargetReason::TemporaryNotAssignable,
                        None,
                        Some(
                            self.expression_builder
                                .view_owned(expression)?
                                .require_single(ValueReceiverContext::ReceiverBase)?,
                        ),
                        None,
                        None,
                        None,
                        self.expression_builder.view_owned(expression)?.span(),
                    )
                    .into());
                };
                Ok((place, declaration_value_mode, declaration_span))
            }
            DeferredAssignmentTarget::Pending(pending) => {
                let place = self.pending_place(&pending)?;
                let (value_mode, declaration_span) = {
                    let resolved = self.resolved_entry(pending.binding)?;
                    (resolved.value_mode.to_owned(), resolved.binding_span)
                };
                Ok((place, value_mode, declaration_span))
            }
        }
    }

    fn resume_assignment(
        &mut self,
        continuation: AssignmentContinuation,
    ) -> Result<Vec<AstNode>, ExpressionParseError> {
        let (target, _declaration_value_mode, declaration_span) =
            self.assignment_target_facts(continuation.target)?;
        let target_type_id = self.expression_builder.place(target)?.metadata().type_id;
        if !place_expression_is_mutable(self.expression_builder.place(target)?) {
            return Err(immutable_binding_diagnostic(
                target_type_id,
                declaration_span,
                continuation.span,
            )
            .into());
        }
        let value = self.complete_mutation_value(continuation.value, target_type_id)?;
        let value = coerce_expression_to_explicit_type_boundary(
            value,
            target_type_id,
            self.type_interner.environment(),
            TypeMismatchContext::Assignment,
            self.expression_builder,
        )?;
        Ok(vec![AstNode {
            kind: NodeKind::Assignment {
                target,
                value: value.id(),
            },
            span: continuation.span,
            scope: continuation.scope,
        }])
    }

    fn resume_compound_assignment(
        &mut self,
        continuation: CompoundAssignmentContinuation,
    ) -> Result<Vec<AstNode>, ExpressionParseError> {
        let (target, declaration_value_mode, declaration_span) =
            self.assignment_target_facts(continuation.target)?;
        let target_type_id = self.expression_builder.place(target)?.metadata().type_id;
        if !place_expression_is_mutable(self.expression_builder.place(target)?) {
            return Err(immutable_binding_diagnostic(
                target_type_id,
                declaration_span,
                continuation.span,
            )
            .into());
        }
        let rhs = self.complete_mutation_value(continuation.value, target_type_id)?;
        let value = evaluate_compound_assignment_with_ready_rhs(CompoundAssignmentReadyInput {
            declaration_value_mode: &declaration_value_mode,
            target,
            target_type_id,
            operator: continuation.operator,
            rhs,
            context: self.context,
            type_interner: self.type_interner,
            expression_builder: self.expression_builder,
            const_values: self.const_values,
            string_table: self.string_table,
            path_fork: self.path_fork,
        })?;
        Ok(vec![AstNode {
            kind: NodeKind::Assignment {
                target,
                value: value.id(),
            },
            span: continuation.span,
            scope: continuation.scope,
        }])
    }

    fn resume_expression_statement(
        &mut self,
        continuation: ExpressionStatementContinuation,
    ) -> Result<Vec<AstNode>, ExpressionParseError> {
        let expression = self.complete_statement_expression(continuation.expression)?;
        validate_resumed_statement_expression(self.expression_builder.view_owned(expression)?)?;
        Ok(vec![AstNode {
            kind: NodeKind::ExpressionStatement(expression.id()),
            span: continuation.span,
            scope: continuation.scope,
        }])
    }

    /// Resume one retained multi-bind statement after its actual producer slots resolve.
    ///
    /// WHAT: rebuilds the authored targets, supplies numeric receivers to eligible raw and
    ///       forward `then` producers, and completes the retained block under its original contract.
    /// WHY: raw literal leaves need their selected type before fallback, while forward expression
    ///      results and the value block's inference still complete through their ordinary owners.
    fn resume_multi_bind(
        &mut self,
        continuation: MultiBindContinuation,
    ) -> Result<Vec<AstNode>, ExpressionParseError> {
        let MultiBindContinuation {
            targets: deferred_targets,
            producer_routes,
            value: deferred_value,
            span,
            scope,
        } = continuation;
        let mut targets = Vec::with_capacity(deferred_targets.len());
        let mut apply_receiver_routes = Vec::with_capacity(deferred_targets.len());
        for deferred_target in deferred_targets {
            match deferred_target {
                DeferredMultiBindTarget::Ready(target) => {
                    apply_receiver_routes.push(true);
                    targets.push(target);
                }
                DeferredMultiBindTarget::Pending {
                    binding,
                    kind,
                    span,
                    producer_resolves_type,
                } => {
                    apply_receiver_routes.push(!producer_resolves_type);
                    let (id, value_mode) = {
                        let state = self.state.borrow();
                        let row = state.binding(binding);
                        (row.path, row.value_mode.to_owned())
                    };
                    let type_id = self.resolved_type(binding)?;
                    targets.push(MultiBindTarget {
                        id,
                        type_id,
                        value_mode,
                        kind,
                        span,
                    });
                }
            }
        }

        for (slot_index, producer) in producer_routes {
            let Some(apply_receiver) = apply_receiver_routes.get(slot_index).copied() else {
                return Err(CompilerError::compiler_error(
                    "a retained multi-bind producer addressed a slot outside its targets",
                )
                .into());
            };
            if !apply_receiver {
                continue;
            }
            let type_id = targets[slot_index].type_id;
            let Some(receiver_type_id) =
                numeric_witness_type(type_id, self.type_interner.environment())
            else {
                continue;
            };
            match producer.kind {
                PendingMultiBindProducerKind::Raw(_) => {
                    self.state.borrow_mut().set_then_value_expected_type(
                        producer.continuation,
                        producer.value_index,
                        receiver_type_id,
                    )?;
                }
                PendingMultiBindProducerKind::Forward => {
                    self.complete_multi_bind_forward_producer(
                        producer,
                        ExpectedType::NumericLiteral(receiver_type_id),
                    )?;
                }
                PendingMultiBindProducerKind::Binding(_) => {
                    return Err(CompilerError::compiler_error(
                        "a retained multi-bind receiver route contained a direct binding producer",
                    )
                    .into());
                }
            }
        }
        let value = match deferred_value {
            DeferredMultiBindValue::Call(call) => self.finish_deferred_call(call)?,
            DeferredMultiBindValue::Catch {
                handler,
                success_slot_count,
            } => {
                if targets.len() != success_slot_count {
                    return Err(CompilerError::compiler_error(
                        "a retained catch-protected multi-bind call produced a different number of \
                         result slots than its parsed signature declared",
                    )
                    .into());
                }
                crate::compiler_frontend::ast::statements::fallible_handling::complete_deferred_catch_handler(
                    *handler,
                    self,
                )?
            }
            DeferredMultiBindValue::RetainedExpression { deferred, .. } => {
                let DeferredNumericExpression {
                    mut items,
                    expected,
                    value_mode,
                    span: _,
                    suffix: _,
                } = *deferred;
                let Some(deferred_item) = single_operand_mut(&mut items) else {
                    return Err(CompilerError::compiler_error(
                        "a retained multi-bind value block lost its single deferred value item",
                    )
                    .into());
                };
                let ExpressionRpnItem::PendingNumericExpression { expression, .. } = deferred_item
                else {
                    return Err(CompilerError::compiler_error(
                        "a retained multi-bind value block lost its deferred expression",
                    )
                    .into());
                };
                let DeferredExpression::ValueBlock(block) = expression.as_mut() else {
                    return Err(CompilerError::compiler_error(
                        "a retained multi-bind expression was not owned by the value-block constructor",
                    )
                    .into());
                };
                // A dead `then` group never reaches inference, yet it must still materialise at the
                // slot its target resolved to. Branch bodies resolve under these completed slots.
                block.resolved_slot_types =
                    Some(targets.iter().map(|target| target.type_id).collect());
                self.resolve_fragment(items, expected, &value_mode)?
            }
        };
        self.validate_multi_bind_slot_types(&targets, &value)?;
        validate_completed_typed_errors(self.expression_builder.view_owned(value)?)?;
        Ok(vec![AstNode {
            kind: NodeKind::MultiBind {
                targets,
                value: value.id(),
            },
            span,
            scope,
        }])
    }

    /// Check that each completed multi-bind slot still matches its target's type.
    ///
    /// WHAT: a target bound to a generic call's declared result took that slot's shape before the
    ///       call completed. The completed success slots must agree with the target types.
    /// WHY: the shape links carry numeric classes only, so a completed slot with a different type
    ///      would otherwise reach codegen silently.
    fn validate_multi_bind_slot_types(
        &self,
        targets: &[MultiBindTarget],
        value: &OwnedExpr,
    ) -> Result<(), ExpressionParseError> {
        let slot_types = self.expression_builder.view_owned(*value)?.success_types();
        if slot_types.len() != targets.len() {
            return Err(CompilerError::compiler_error(
                "a completed multi-bind value produced a different number of slots than its targets",
            )
            .into());
        }
        for (target, slot_type) in targets.iter().zip(slot_types) {
            if target.type_id != *slot_type {
                return Err(CompilerDiagnostic::type_mismatch(
                    target.type_id,
                    *slot_type,
                    TypeMismatchContext::General,
                    target.span,
                )
                .into());
            }
        }
        Ok(())
    }

    /// Resume one retained `assert` statement whose arguments resolved.
    ///
    /// WHAT: completes each argument and uses the shared finisher to classify the new request
    ///       suffix, then retains the original parse range if the condition completed as true.
    /// WHY: pending tickets publish into their original slots, outside this resume window. The
    ///      saved exact range catches those requests without reclassifying later siblings.
    fn resume_assertion(
        &mut self,
        continuation: crate::compiler_frontend::ast::statements::asserts::AssertionContinuation,
    ) -> Result<Vec<AstNode>, ExpressionParseError> {
        let request_provenance = self.context.generic_request_checkpoint();
        let mut arguments = Vec::with_capacity(continuation.arguments.len());
        for argument in continuation.arguments {
            arguments.push(self.complete_call_argument(argument)?);
        }
        let node = crate::compiler_frontend::ast::statements::asserts::finish_assert_statement(
            crate::compiler_frontend::ast::statements::asserts::AssertionFinishInput {
                arguments,
                expectations: &continuation.expectations,
                span: continuation.span,
                scope: continuation.scope,
                request_provenance,
            },
            self.context,
            self.type_interner,
            self.expression_builder,
            self.const_values,
            self.string_table,
            self.path_fork,
        )?;
        if let NodeKind::Assert { condition, .. } = &node.kind
            && assertion_condition_is_statically_true(
                self.expression_builder.view(*condition)?,
                self.const_values,
            )
        {
            self.context
                .retain_assertion_validation_requests_in_range(continuation.generic_request_range);
        }
        Ok(vec![node])
    }

    fn resume_return(
        &mut self,
        continuation: ReturnContinuation,
    ) -> Result<Vec<AstNode>, ExpressionParseError> {
        let mut values: Vec<ExprId> = Vec::with_capacity(continuation.slots.len());
        for slot in continuation.slots {
            values.push(self.complete_statement_expression(slot)?.id());
        }
        if continuation.is_error_return {
            let mut values = values.into_iter();
            let error_value = values.next().ok_or_else(|| {
                CompilerError::compiler_error(
                    "numeric construction completion resumed an error return without a value",
                )
            })?;
            // The resumed error channel re-applies the same return-boundary coercion the
            // parse-time path applied before retention.
            let Some(expected_error_type_id) = self
                .context
                .enclosing_callable
                .as_deref()
                .and_then(|contract| contract.error_type)
            else {
                return Err(CompilerError::compiler_error(
                    "numeric construction completion resumed an error return outside an error function",
                )
                .into());
            };
            let error_value = coerce_expression_to_explicit_type_boundary(
                self.expression_builder.own(error_value),
                expected_error_type_id,
                self.type_interner.environment(),
                TypeMismatchContext::ReturnValue,
                self.expression_builder,
            )?;
            return Ok(vec![AstNode {
                kind: NodeKind::ReturnError(error_value.id()),
                span: continuation.span,
                scope: continuation.scope,
            }]);
        }
        let expected_types = &self
            .context
            .enclosing_callable
            .as_deref()
            .ok_or_else(|| {
                CompilerError::compiler_error(
                    "numeric construction completion resumed a return without a callable contract",
                )
            })?
            .success_types;
        // One producer may forward all success slots, whether it is a call or a value block.
        let forwarded = values.len() == 1 && expected_types.len() > 1;
        if !forwarded && values.len() != expected_types.len() {
            return Err(CompilerError::compiler_error(
                "numeric construction completion resumed a return with the wrong number of slots",
            )
            .into());
        }
        if forwarded {
            crate::compiler_frontend::ast::statements::body_return::validate_forwarded_return(
                self.expression_builder.view(values[0])?,
                expected_types,
                self.type_interner.environment(),
            )?;
            return Ok(vec![AstNode {
                kind: NodeKind::Return(values),
                span: continuation.span,
                scope: continuation.scope,
            }]);
        }
        for (value, expected_type_id) in values.iter_mut().zip(expected_types.iter()) {
            *value = coerce_expression_to_explicit_type_boundary(
                self.expression_builder.own(*value),
                *expected_type_id,
                self.type_interner.environment(),
                TypeMismatchContext::ReturnValue,
                self.expression_builder,
            )?
            .id();
        }
        Ok(vec![AstNode {
            kind: NodeKind::Return(values),
            span: continuation.span,
            scope: continuation.scope,
        }])
    }

    fn resume_construct(
        &mut self,
        construct: DeferredConstruct,
    ) -> Result<Vec<AstNode>, ExpressionParseError> {
        match construct {
            DeferredConstruct::If(deferred) => self.finish_if(*deferred),
            DeferredConstruct::Match(deferred) => self.finish_match(*deferred),
            DeferredConstruct::Loop(deferred) => self.finish_loop(*deferred),
        }
    }

    fn finish_if(&mut self, deferred: DeferredIf) -> Result<Vec<AstNode>, ExpressionParseError> {
        let condition = self.complete_statement_expression(deferred.condition)?;
        ensure_boolean_condition(
            self.expression_builder.view_owned(condition)?,
            deferred.span,
            self.type_interner.environment(),
        )?;
        let then_block = self.resolve_deferred_body(deferred.then_body)?;
        let else_block = match deferred.else_body {
            Some(body) => Some(self.resolve_deferred_body(body)?),
            None => None,
        };
        let request_ranges = IfGenericRequestRanges {
            then_branch: GenericRequestRange::new(
                deferred.generic_request_start,
                deferred.then_request_end,
            ),
            else_branch: GenericRequestRange::new(
                deferred.then_request_end,
                deferred.else_request_end,
            ),
        };
        Ok(vec![AstNode {
            kind: NodeKind::If(
                condition.id(),
                then_block,
                else_block,
                IfBranchMetadata::new(request_ranges, deferred.then_scope, deferred.else_scope),
            ),
            span: deferred.span,
            scope: deferred.scope,
        }])
    }

    fn finish_match(
        &mut self,
        deferred: DeferredMatch,
    ) -> Result<Vec<AstNode>, ExpressionParseError> {
        // The retained-match owner rebuilds scrutinee, arms (ready-pattern compatibility and
        // retained-pattern materialisation at the selected subject type), guards and bodies in
        // authored order; this driver only places the finished node.
        let scope = deferred.scope;
        let resumed = crate::compiler_frontend::ast::statements::branching::resume_deferred_match(
            deferred,
            crate::compiler_frontend::ast::statements::branching::DeferredMatchBodyReceiver::Inherit,
            self,
        )?;
        Ok(vec![AstNode {
            kind: NodeKind::Match {
                scrutinee: resumed.scrutinee,
                arms: resumed.arms,
                default: resumed.default,
                exhaustiveness: resumed.exhaustiveness,
            },
            span: resumed.span,
            scope,
        }])
    }

    fn finish_loop(
        &mut self,
        deferred: DeferredLoop,
    ) -> Result<Vec<AstNode>, ExpressionParseError> {
        let kind = match deferred.header {
            DeferredLoopHeader::Conditional { condition } => {
                let condition = self.complete_statement_expression(condition)?;
                ensure_loop_condition(
                    self.expression_builder.view_owned(condition)?,
                    self.type_interner.environment(),
                )?;
                let body = self.resolve_deferred_body(deferred.body)?;
                NodeKind::WhileLoop(condition.id(), body)
            }
            DeferredLoopHeader::Range(header) => {
                // The header completes before the body: a bound header's spec was already produced
                // by its counter row in the binding pass, and an unbound header's operands are
                // authored positions that precede the body's statements.
                let (range, bindings) = match *header {
                    DeferredRangeHeader::Ready { bindings, range } => (range, bindings),
                    DeferredRangeHeader::Bound(retained) => {
                        let range = self.take_range_spec(retained.counter)?;
                        // The counter's completed declaration moves into the loop's iterator
                        // binding; the ready frame slot keeps the binding visible in the body and
                        // later uses read the retained metadata.
                        let item = self.take_declaration(retained.counter)?;
                        (
                            Box::new(range),
                            Box::new(LoopBindings {
                                item: Some(item),
                                index: retained.index,
                            }),
                        )
                    }
                    DeferredRangeHeader::Unbound(spec) => {
                        let range = self.complete_range_spec(*spec)?;
                        (
                            Box::new(range),
                            Box::new(LoopBindings {
                                item: None,
                                index: None,
                            }),
                        )
                    }
                };
                let body = self.resolve_deferred_body(deferred.body)?;
                NodeKind::RangeLoop {
                    bindings,
                    range,
                    body,
                }
            }
            DeferredLoopHeader::Collection {
                mut bindings,
                iterable,
                pending_item,
            } => {
                let iterable = self.complete_statement_expression(iterable)?;
                self.expression_builder
                    .view_owned(iterable)?
                    .require_single(ValueReceiverContext::General)?;
                if let Some(item) = pending_item {
                    // The item's row resolved its element descriptor in the binding pass. Its
                    // completed declaration moves into the loop's item binding, as a range counter's
                    // does, so the header carries the same declaration the body reads.
                    if bindings.item.is_some() {
                        return Err(ExpressionParseError::from(CompilerError::compiler_error(
                            "collection loop header carried both a resolved and a pending item binding",
                        )));
                    }
                    bindings.item = Some(self.take_declaration(item)?);
                }
                let body = self.resolve_deferred_body(deferred.body)?;
                NodeKind::CollectionLoop {
                    bindings,
                    iterable: iterable.id(),
                    body,
                }
            }
        };
        Ok(vec![AstNode {
            kind,
            span: deferred.span,
            scope: deferred.scope,
        }])
    }

    /// Complete one retained range spec and select/check its domain through the shared policy.
    ///
    /// WHAT: completes each authored operand slot at its natural type, then hands the completed
    ///       operands to the loop-header owner's shared range constructor.
    /// WHY: the operands' numeric origins are selected here (root decision), so the shared policy
    ///      — not this driver — derives the range domain, synthesises the omitted-start zero at
    ///      that domain and validates start/end/step compatibility. No operand is defaulted early
    ///      and no witness is registered on the header's behalf.
    fn complete_range_spec(
        &mut self,
        deferred: DeferredRangeSpec,
    ) -> Result<RangeLoopSpec, ExpressionParseError> {
        let start = match deferred.start {
            Some(slot) => Some(self.complete_statement_expression(slot)?),
            None => None,
        };
        let end = self.complete_statement_expression(deferred.end)?;
        let step = match deferred.step {
            Some(slot) => Some(self.complete_statement_expression(slot)?),
            None => None,
        };
        crate::compiler_frontend::ast::statements::loop_headers::range_loop_spec_from_completed_operands(
            start,
            end,
            deferred.end_kind,
            step,
            deferred.span,
            self.context,
            self.type_interner,
            self.expression_builder,
            self.const_values,
            self.string_table,
            self.path_fork,
        )
    }

    /// Take the range spec one retained loop counter produced during the binding pass.
    fn take_range_spec(
        &mut self,
        counter: NumericBindingId,
    ) -> Result<RangeLoopSpec, ExpressionParseError> {
        self.range_specs
            .get_mut(counter.as_usize())
            .and_then(Option::take)
            .ok_or_else(|| {
                CompilerError::compiler_error(
                    "a retained range loop resumed before its counter produced the range spec",
                )
                .into()
            })
    }

    /// Lend the completed range spec of one retained counter.
    ///
    /// WHAT: a borrowed view of the take-once memo, for a copied template header that shares the
    ///       counter and therefore cannot take the spec away from its owning loop.
    /// WHY: `take_range_spec` moves the spec into the one authored loop position; a shared
    ///      counter's additional positions clone from here instead.
    fn resolved_range_spec(
        &self,
        counter: NumericBindingId,
    ) -> Result<&RangeLoopSpec, ExpressionParseError> {
        self.range_specs
            .get(counter.as_usize())
            .and_then(Option::as_ref)
            .ok_or_else(|| {
                CompilerError::compiler_error(
                    "a retained range loop asked for its counter's spec before the counter resolved",
                )
                .into()
            })
    }

    /// Install every retained template range header waiting on one resolved counter.
    ///
    /// WHAT: drains the store's waiting positions for the counter, handing each the finished spec
    ///       and a copy of the counter's completed declaration.
    /// WHY: a bound template header owns no operand records — its operands belong to the
    ///      counter's recipe, which already ran the shared range policy exactly once — so the
    ///      position installs that result instead of re-reading operands. A runtime slot-plan copy
    ///      shares the same counter, so every waiting node completes; each loop binds its own
    ///      declaration copy while the take-once memo stays for the single owning loop.
    fn install_bound_template_range_headers(
        &mut self,
        counter: NumericBindingId,
        resolved: &ResolvedBinding,
    ) -> Result<(), ExpressionParseError> {
        let domain = self.context.numeric_domain_id();
        loop {
            let node = self
                .context
                .template_ir_store
                .borrow()
                .pending_range_header_node_for_counter(domain, counter);
            let Some(node) = node else {
                return Ok(());
            };
            let spec = self.resolved_range_spec(counter)?.clone();
            let declaration = resolved
                .declaration
                .as_ref()
                .ok_or_else(|| {
                    CompilerError::compiler_error(
                        "a retained template range header resolved after its counter's declaration \
                         payload moved",
                    )
                })?
                .to_owned();
            self.context
                .template_ir_store
                .borrow_mut()
                .install_completed_numeric_range_header_with_counter(node, spec, declaration)
                .map_err(ExpressionParseError::from)?;
        }
    }

    /// Install one retained template collection header's completed item declaration.
    ///
    /// WHAT: when the node is a collection loop header with a pending item, clones that item's
    ///       completed declaration into the header's bindings, once for this node.
    /// WHY: the item resolves in the binding pass, before template intakes complete. A copied
    ///      header shares the same pending item, so each waiting node clones the metadata; only the
    ///      statement-level owner takes the payload.
    fn install_pending_collection_item(
        &mut self,
        node: TemplateIrNodeId,
    ) -> Result<(), ExpressionParseError> {
        let pending = self
            .context
            .template_ir_store
            .borrow()
            .pending_collection_item(node);
        let Some(item) = pending else {
            return Ok(());
        };
        let declaration = self
            .resolved_entry(item)?
            .declaration
            .as_ref()
            .ok_or_else(|| {
                CompilerError::compiler_error(
                    "a retained template collection header's pending item resolved after its declaration payload moved",
                )
            })?
            .to_owned();
        self.context
            .template_ir_store
            .borrow_mut()
            .install_collection_item(node, declaration)
            .map_err(ExpressionParseError::from)
    }

    fn complete_statement_expression(
        &mut self,
        expression: StatementExpression,
    ) -> Result<OwnedExpr, ExpressionParseError> {
        match expression {
            StatementExpression::Built(built) => {
                validate_completed_typed_errors(self.expression_builder.view(built)?)?;
                Ok(self.expression_builder.own(built))
            }
            StatementExpression::Retained(deferred) => self.complete_deferred_expression(deferred),
        }
    }

    /// Complete one retained statement expression whose enclosing catch owns its recovery.
    ///
    /// WHAT: the same construction as `complete_statement_expression`, minus the closed-boundary
    ///       typed-error validation, applied to the whole protected subtree.
    /// WHY: the catch the caller attaches next consumes exactly the pending producers inside the
    ///      protected fragment — including nested call arguments — so validating any of them here
    ///      would report the authored catch as unhandled before it exists. The disposition lives
    ///      on the session, so it reaches every producer call path (source, receiver and external
    ///      finishers alike) rather than only one lane; it is restored when the subtree completes,
    ///      and the recovered expression is validated at its own enclosing boundary.
    fn resolve_protected_statement_expression(
        &mut self,
        expression: StatementExpression,
    ) -> Result<OwnedExpr, ExpressionParseError> {
        let previous = self.protected_typed_errors;
        self.protected_typed_errors = true;
        let result = self.complete_protected_statement_expression(expression);
        self.protected_typed_errors = previous;
        result
    }

    fn complete_protected_statement_expression(
        &mut self,
        expression: StatementExpression,
    ) -> Result<OwnedExpr, ExpressionParseError> {
        match expression {
            StatementExpression::Built(built) => {
                self.expression_builder.view(built)?;
                Ok(self.expression_builder.own(built))
            }
            StatementExpression::Retained(deferred) => self.complete_items_with_suffix(
                deferred.items,
                deferred.expected,
                &deferred.value_mode,
                deferred.suffix,
            ),
        }
    }

    /// Complete one mutation right-hand side against the now-selected target type.
    ///
    /// WHAT: a value that already built is reused as authored; a retained fragment completes with
    ///       the target's exact type as its literal destination.
    /// WHY: the mutation target's receiving context is only known after the body root selected the
    ///      binding's single static type, and a raw literal right-hand side must materialise in
    ///      that selected domain rather than at the language default.
    fn complete_mutation_value(
        &mut self,
        value: StatementExpression,
        destination: TypeId,
    ) -> Result<OwnedExpr, ExpressionParseError> {
        match value {
            StatementExpression::Built(built) => {
                validate_completed_typed_errors(self.expression_builder.view(built)?)?;
                Ok(self.expression_builder.own(built))
            }
            StatementExpression::Retained(deferred) => {
                let value = self.complete_items_with_suffix(
                    deferred.items,
                    ExpectedType::NumericLiteral(destination),
                    &deferred.value_mode,
                    deferred.suffix,
                )?;
                validate_completed_typed_errors(self.expression_builder.view_owned(value)?)?;
                Ok(value)
            }
        }
    }

    fn complete_deferred_expression(
        &mut self,
        deferred: DeferredNumericExpression,
    ) -> Result<OwnedExpr, ExpressionParseError> {
        let expression = self.complete_items_with_suffix(
            deferred.items,
            deferred.expected,
            &deferred.value_mode,
            deferred.suffix,
        )?;
        // The deferral site skipped the parse-time receiving-boundary validation, so the completed
        // value is validated here, in its authored position: an unhandled typed producer reports
        // its diagnostic, while a value rebuilt by a deferred catch is already recovered. Inside a
        // catch-protected subtree the producers stay open for the catch the enclosing handler
        // attaches, and the recovered expression is validated at its own boundary instead.
        if !self.protected_typed_errors {
            validate_completed_typed_errors(self.expression_builder.view_owned(expression)?)?;
        }
        Ok(expression)
    }

    /// Complete one pending forward producer into its connected class, exactly once.
    ///
    /// WHAT: claims the producer's retained fragment, completes it against the destination the
    /// caller supplies or the fragment's own expectation, then records the natural result beside
    /// the class.
    /// WHY: a forward result must fix its class's witness before a default or literal
    /// materialises, and its fragment must complete once, in authored order. Only an authored
    /// expectation or an immediate result supplies a literal destination; a later receiver
    /// validates the natural result instead.
    fn pull_forward_producer(
        &mut self,
        producer: ForwardProducerId,
        expected: Option<ExpectedType>,
    ) -> Result<(), ExpressionParseError> {
        let claimed = self
            .state
            .borrow_mut()
            .claim_pending_forward_producer(producer)?;
        let Some((shape, fragment)) = claimed else {
            return Ok(());
        };
        // A literal destination comes only from an authored expectation or an immediate result,
        // never from a later receiver: an unexpected producer completes naturally and its consumer
        // validates it.
        let destination = expected.unwrap_or(fragment.expected);
        let expression = self.complete_items_with_suffix(
            fragment.items,
            destination,
            &fragment.value_mode,
            fragment.suffix,
        )?;
        // This witness describes a single argument/collection element. Non-single producers stay
        // intact until that receiving owner can diagnose arity with its authored context.
        if let [produced_type] = self
            .expression_builder
            .view_owned(expression)?
            .success_types()
        {
            let span = self.expression_builder.view_owned(expression)?.span();
            let mut state = self.state.borrow_mut();
            // The result labels the retained fragment, so it reuses that fragment's parse-time
            // provenance rather than acquiring the order of this completion demand.
            let authored_order = state.forward_producer_authored_order(producer);
            register_ordered_shape_witness(
                &mut state,
                self.type_interner.environment(),
                shape,
                *produced_type,
                span,
                authored_order,
            );
        }
        self.state
            .borrow_mut()
            .complete_forward_producer(producer, expression.id())?;
        Ok(())
    }

    /// Substitute origins, complete retained casts and calls, then evaluate the fragment once.
    fn complete_items(
        &mut self,
        items: Vec<ExpressionRpnItem>,
        expected: ExpectedType,
        value_mode: &ValueMode,
    ) -> Result<OwnedExpr, ExpressionParseError> {
        self.complete_items_with_suffix(items, expected, value_mode, DeferredExpressionSuffix::None)
    }

    /// Complete one retained fragment and re-apply its authored trailing handling.
    ///
    /// WHAT: evaluates the fragment once, then applies the exact suffix the deferral site consumed
    ///       (nothing, or an authored postfix `!`).
    /// WHY: the deferral site had to consume the suffix tokens before the value existed, so
    ///      completion replays the recorded decision through the fallible-handling owner instead
    ///      of re-reading tokens; a value without an error slot still reports the authored mistake.
    fn complete_items_with_suffix(
        &mut self,
        items: Vec<ExpressionRpnItem>,
        expected: ExpectedType,
        value_mode: &ValueMode,
        suffix: DeferredExpressionSuffix,
    ) -> Result<OwnedExpr, ExpressionParseError> {
        let value = self.complete_items_inner(items, expected, value_mode)?;
        crate::compiler_frontend::ast::statements::fallible_handling::apply_completed_handling_suffix(
            value,
            suffix,
            self.context,
            self.type_interner,
            self.expression_builder,
        )
    }

    fn complete_items_inner(
        &mut self,
        items: Vec<ExpressionRpnItem>,
        expected: ExpectedType,
        value_mode: &ValueMode,
    ) -> Result<OwnedExpr, ExpressionParseError> {
        let items = self.substitute_bindings(items)?;
        let literal_destination = expected.known_type_id();
        let items = self.complete_fragment_items(items, value_mode, literal_destination)?;
        let mut expected = expected;
        let expression = evaluate_expression(
            self.context,
            items,
            self.type_interner,
            self.expression_builder,
            self.const_values,
            &mut expected,
            value_mode,
            self.string_table,
            self.path_fork,
        )?;
        Ok(expression)
    }

    /// Replace pending binding references with their resolved canonical operands.
    ///
    /// WHAT: rebuilds the ordered fragment by value in one flat pass over its items.
    /// WHY: the retained items belong to completion exactly once; substituting in place would
    ///      require a placeholder item, and cloning the fragment wholesale would copy RPN per probe.
    fn substitute_bindings(
        &mut self,
        items: Vec<ExpressionRpnItem>,
    ) -> Result<Vec<ExpressionRpnItem>, ExpressionParseError> {
        let mut substituted = Vec::with_capacity(items.len());
        for item in items {
            match item {
                ExpressionRpnItem::PendingNumericBinding {
                    binding,
                    span,
                    value_mode,
                    is_copy,
                } => {
                    let operand = self.binding_operand(binding, span, &value_mode, is_copy)?;
                    substituted.push(ExpressionRpnItem::Operand(operand.id()));
                }
                other => substituted.push(other),
            }
        }
        Ok(substituted)
    }

    /// Build the canonical operand for one resolved binding use.
    ///
    /// WHY: completion already owns the canonical type. Diagnostics and HIR consume its `TypeId`,
    ///      so an operand need not expand an owning display tree for every composite binding use.
    fn binding_operand(
        &mut self,
        binding: NumericBindingId,
        span: Option<SourceSpan>,
        authored_mode: &ValueMode,
        is_copy: bool,
    ) -> Result<OwnedExpr, ExpressionParseError> {
        let BindingUseFacts {
            type_id,
            declaration_id,
            value_mode: source_mode,
            const_record_state,
        } = self.binding_use_facts(binding)?;
        if is_copy {
            let projections = self
                .expression_builder
                .append_place_projections(Vec::new(), span)?;
            let place = self.expression_builder.append_place(
                crate::compiler_frontend::ast::expressions::expression_store::PlaceRecord {
                    root: declaration_id,
                    root_type_id: type_id,
                    projections,
                    type_id,
                    value_mode: source_mode,
                    span,
                },
                span,
            )?;
            return Ok(self.expression_builder.copy_with_type_id(
                place,
                type_id,
                span,
                *authored_mode,
            )?);
        }
        Ok(self.expression_builder.reference_with_type_id(
            declaration_id,
            type_id,
            span,
            source_mode,
            const_record_state,
        )?)
    }

    /// The facts one use of a numeric binding reads from its owner.
    ///
    /// WHAT: an installed binding answers from its resolved entry; a binding whose recipe is still
    ///       completing answers from its seeded cyclic facts.
    /// WHY: a forward producer can read the binding it updates while that binding resolves. Only
    ///      value references take the in-flight path; every other reader still requires the
    ///      installed entry.
    fn binding_use_facts(
        &mut self,
        binding: NumericBindingId,
    ) -> Result<BindingUseFacts, ExpressionParseError> {
        self.ensure_binding_resolved(binding)?;
        if matches!(
            self.bindings.get(binding.as_usize()),
            Some(BindingSlot::Resolving(_))
        ) {
            return self.resolving_binding_facts(binding);
        }
        let resolved = self.resolved_entry(binding)?;
        Ok(BindingUseFacts {
            type_id: resolved.type_id,
            declaration_id: resolved.declaration_id,
            value_mode: resolved.value_mode.to_owned(),
            const_record_state: resolved.const_record_state,
        })
    }

    /// Read a cyclic use of a binding whose recipe is still completing.
    ///
    /// WHAT: seeds the binding's storage type once, then reads its row identity and authored mode.
    /// WHY: the seed is the type the binding installs with, so the forward producer that depends
    ///      on it is checked against final storage. A numeric or collection value is never a const
    ///      record, so a cyclic reference records a runtime value.
    fn resolving_binding_facts(
        &mut self,
        binding: NumericBindingId,
    ) -> Result<BindingUseFacts, ExpressionParseError> {
        let type_id = self.resolving_binding_type(binding)?;
        let state = self.state.borrow();
        let row = state.binding(binding);
        Ok(BindingUseFacts {
            type_id,
            declaration_id: row.path,
            value_mode: row.value_mode.to_owned(),
            const_record_state: ConstRecordState::RuntimeValue,
        })
    }

    /// The storage type of a cyclic use of a binding whose recipe is still completing.
    ///
    /// WHAT: returns the seeded storage type, seeding it once when no nested demand has already
    ///       done so. Forward producers that feed the binding's class settle before the seed pins
    ///       its leaves.
    /// WHY: the seed is the initializer a cycle reads, so a forward producer must complete its
    ///      natural result first, or the leaves would pin to the default. A nested demand during
    ///      that settle reuses the seed it stores, so the leaves pin exactly once.
    fn resolving_binding_type(
        &mut self,
        binding: NumericBindingId,
    ) -> Result<TypeId, ExpressionParseError> {
        let index = binding.as_usize();
        let (shape, cached) = match self.bindings.get(index) {
            Some(BindingSlot::Resolving(resolving)) => (resolving.shape, resolving.seed),
            _ => {
                return Err(CompilerError::compiler_error(
                    "a cyclic numeric use found no resolving binding",
                )
                .into());
            }
        };
        if let Some(type_id) = cached {
            return Ok(type_id);
        }
        let settle_shape = match shape {
            Some(shape) => Some(shape),
            None => {
                let class = self.state.borrow().binding(binding).class;
                let forward = self.state.borrow().class_is_forward(class);
                if forward {
                    let authored_order = self.state.borrow().binding_authored_order(binding);
                    Some(
                        self.state
                            .borrow_mut()
                            .intern_ordered_shape(KnownShape::Numeric(class), authored_order)?,
                    )
                } else {
                    None
                }
            }
        };
        if let Some(settle_shape) = settle_shape {
            self.settle_forward_producers(settle_shape)?;
        }
        // A nested demand that ran during the settle may already have seeded this binding.
        let seeded = match self.bindings.get(index) {
            Some(BindingSlot::Resolving(resolving)) => resolving.seed,
            _ => None,
        };
        if let Some(type_id) = seeded {
            return Ok(type_id);
        }
        let type_id = self.seed_resolving_binding_type(binding, shape)?;
        if let Some(BindingSlot::Resolving(resolving)) = self.bindings.get_mut(index) {
            resolving.seed = Some(type_id);
        }
        Ok(type_id)
    }

    /// The storage type one direct source binding contributes to a multi-bind slot.
    ///
    /// WHAT: an installed source answers from its resolved type; a source whose recipe is still
    ///       completing answers from its seeded storage type.
    /// WHY: a multi-bind producer may read a binding that is still resolving. That read must not
    ///      demand an installed entry the cycle has not produced yet.
    fn binding_use_type(
        &mut self,
        binding: NumericBindingId,
    ) -> Result<TypeId, ExpressionParseError> {
        self.ensure_binding_resolved(binding)?;
        if matches!(
            self.bindings.get(binding.as_usize()),
            Some(BindingSlot::Resolving(_))
        ) {
            return self.resolving_binding_type(binding);
        }
        self.resolved_type(binding)
    }

    /// Seed the storage type of a binding whose recipe is still completing.
    ///
    /// WHAT: pins each numeric leaf of the binding's shape to its current canonical type, then
    ///       materialises the shape; a binding without a shape pins its own numeric class and
    ///       seeds its recorded annotation, the same storage type completion installs.
    /// WHY: the seed is the initializer and default fact a cycle can read before the recipe
    ///      finishes. Exact receivers make a later conflicting witness fail instead of retyping
    ///      storage silently.
    fn seed_resolving_binding_type(
        &mut self,
        binding: NumericBindingId,
        shape: Option<KnownShapeId>,
    ) -> Result<TypeId, ExpressionParseError> {
        let (class, span, authored_order) = {
            let state = self.state.borrow();
            let row = state.binding(binding);
            (
                row.class,
                row.binding_span,
                state.binding_authored_order(binding),
            )
        };
        let mut state = self.state.borrow_mut();
        // WHAT: storage seeds reuse the declaration's parse-time key.
        // WHY: completion happens after later receivers, but this witness labels the earlier
        // declaration; allocating a new key here would reverse their authored order.
        match shape {
            Some(shape) => {
                pin_numeric_leaves(
                    &mut state,
                    self.type_interner.environment(),
                    shape,
                    span,
                    authored_order,
                )?;
                // An annotation is the storage type completion installs, so a cyclic read seeds
                // that same type instead of the recipe's materialised shape.
                match binding_annotation_type(&state, binding) {
                    Some(annotated) => Ok(annotated),
                    None => materialise_known_shape(
                        &mut state,
                        self.type_interner.environment_mut_for_derived_types(),
                        shape,
                    ),
                }
            }
            None => {
                let type_id = numeric_class_type(&mut state, class)?;
                state.add_ordered_receiver_witness(
                    class,
                    type_id,
                    span,
                    authored_order,
                    self.type_interner.environment(),
                );
                // An annotation is the storage type completion installs, so a cyclic read seeds
                // that same type, exactly as the shaped arm does.
                Ok(binding_annotation_type(&state, binding).unwrap_or(type_id))
            }
        }
    }

    /// Complete retained casts and calls inside one ordered fragment.
    ///
    /// WHAT: `literal_destination` is the collection or map type the fragment's expected type names, when
    ///       it names one. It reaches only this fragment's literal operands, and only through their
    ///       optional entry check, which admits optional wraps at each raw nesting level; calls and casts complete
    ///       from their own expectations.
    fn complete_fragment_items(
        &mut self,
        items: Vec<ExpressionRpnItem>,
        value_mode: &ValueMode,
        literal_destination: Option<TypeId>,
    ) -> Result<Vec<ExpressionRpnItem>, ExpressionParseError> {
        let mut completed = Vec::with_capacity(items.len());
        for item in items {
            self.complete_fragment_item(item, value_mode, literal_destination, &mut completed)?;
        }
        Ok(completed)
    }

    // Keep cast, call and value-block scratch out of the completion loop frame.
    #[inline(never)]
    fn complete_fragment_item(
        &mut self,
        item: ExpressionRpnItem,
        value_mode: &ValueMode,
        literal_destination: Option<TypeId>,
        completed: &mut Vec<ExpressionRpnItem>,
    ) -> Result<(), ExpressionParseError> {
        match item {
            ExpressionRpnItem::PendingNumericCast {
                target_type_id,
                target,
                requires_optional_wrap_after_cast,
                handling,
                source,
                span,
            } => {
                let source_value = self.complete_items(source, ExpectedType::Infer, value_mode)?;
                let cast = resolve_cast_expression(CastResolutionInput {
                    source: source_value,
                    target_type_id,
                    target,
                    requires_optional_wrap_after_cast,
                    handling,
                    scope_context: self.context,
                    trait_environment: self.context.trait_environment(),
                    trait_evidence_environment: self.context.trait_evidence_environment(),
                    type_interner: &mut *self.type_interner,
                    expression_builder: &mut *self.expression_builder,
                    string_table: &mut *self.string_table,
                    path_fork: &mut *self.path_fork,
                    active_generic_type_context: self.context.active_generic_type_context(),
                    span,
                })?;
                completed.push(ExpressionRpnItem::Operand(cast.id()));
            }
            ExpressionRpnItem::PendingNumericCall { call, .. } => {
                let expression = self.finish_deferred_call(*call)?;
                completed.push(ExpressionRpnItem::Operand(expression.id()));
            }
            ExpressionRpnItem::PendingNumericExpression { expression, .. } => {
                // Rebuild the exact authored value-block/catch constructor through its owning
                // module; the retained expression owns its bodies and is consumed once.
                let value = match *expression {
                    DeferredExpression::ValueBlock(block) => {
                        crate::compiler_frontend::ast::statements::value_production::complete_deferred_value_block(block, self)?
                    }
                    DeferredExpression::CatchHandler(handler) => {
                        crate::compiler_frontend::ast::statements::fallible_handling::complete_deferred_catch_handler(*handler, self)?
                    }
                    DeferredExpression::OperandContinuation(payload) => {
                        crate::compiler_frontend::ast::field_access::complete_retained_operand_continuation(*payload, self)?
                    }
                    DeferredExpression::Collection(literal) => {
                        crate::compiler_frontend::ast::statements::collections::complete_deferred_collection(*literal, literal_destination, self)?
                    }
                    DeferredExpression::Map(literal) => {
                        crate::compiler_frontend::ast::statements::collections::complete_deferred_map(*literal, literal_destination, self)?
                    }
                };
                completed.push(ExpressionRpnItem::Operand(value.id()));
            }
            other => completed.push(other),
        }
        Ok(())
    }

    fn finish_deferred_call(
        &mut self,
        call: DeferredCall,
    ) -> Result<OwnedExpr, ExpressionParseError> {
        match call {
            DeferredCall::Source(deferred) => self.finish_source_call(*deferred),
            // The concrete-receiver producer owns the method constructor and its slot routing;
            // it resumes through this session's shared call-argument completion service.
            DeferredCall::Receiver(deferred) => {
                crate::compiler_frontend::ast::field_access::complete_deferred_receiver_call(
                    *deferred, self,
                )
            }
            // The external-call producer owns the host constructor and its ABI slot routing; it
            // resumes through this session's shared call-argument completion service.
            DeferredCall::External(deferred) => {
                crate::compiler_frontend::ast::expressions::function_calls::complete_deferred_external_call(
                    *deferred, self,
                )
            }
            // The generic-call producer owns the instantiation lane and its reserved request
            // ticket; it resumes through this session's shared call-argument completion service.
            DeferredCall::Generic(deferred) => {
                crate::compiler_frontend::ast::generic_functions::complete_deferred_generic_call(
                    *deferred, self,
                )
            }
            // The constructor producers own the struct/choice constructors and their slot
            // routing; they resume through this session's shared argument completion service.
            DeferredCall::Struct(deferred) => {
                crate::compiler_frontend::ast::expressions::struct_instance::complete_deferred_struct_call(
                    *deferred, self,
                )
            }
            DeferredCall::Choice(deferred) => {
                crate::compiler_frontend::ast::expressions::choice_constructor::complete_deferred_choice_call(
                    *deferred, self,
                )
            }
            // The generic nominal producer owns the generic struct/choice instance and its seeded
            // bindings; it resumes through this session's shared call-argument completion service.
            DeferredCall::GenericNominal(deferred) => {
                crate::compiler_frontend::ast::expressions::generic_nominal_retention::complete_deferred_generic_nominal_call(
                    *deferred, self,
                )
            }
        }
    }

    /// Resume one retained non-generic source call with its now-complete arguments.
    fn finish_source_call(
        &mut self,
        deferred: DeferredSourceCall,
    ) -> Result<OwnedExpr, ExpressionParseError> {
        let DeferredSourceCall {
            function_path,
            success_result_type_ids,
            error_return_type_id,
            expectations,
            arguments,
            call_span,
            authored_handling,
        } = deferred;
        let mut completed_arguments = Vec::with_capacity(arguments.len());
        for argument in arguments {
            completed_arguments.push(self.complete_call_argument(argument)?);
        }
        let args = crate::compiler_frontend::ast::expressions::function_calls::resolve_user_function_call_arguments(
            &function_path,
            completed_arguments,
            &expectations,
            call_span,
            self.string_table,
            self.type_interner,
            self.expression_builder,
            self.const_values,
            self.path_fork,
        )?;
        let call = HandledFallibleCall {
            name: function_path,
            result_type_ids: success_result_type_ids,
            args,
            call_span,
        };
        match authored_handling {
            DeferredCallHandling::Unhandled => self.finish_handled_call(call, error_return_type_id),
            DeferredCallHandling::Propagate { span } => {
                // Replay the authored `!` through the same constructor the completed call path
                // uses; the retention site already validated the callee's error slot and the
                // authored value requirement, so no second decision exists here.
                let propagation_span = span.or(call_span);
                let error_type = error_return_type_id.ok_or_else(|| {
                    CompilerError::compiler_error(
                        "retained propagated call lost its declared error slot",
                    )
                })?;
                call.into_expression(
                    FallibleHandling::Propagate,
                    propagation_span,
                    error_type,
                    self.expression_builder,
                )
            }
        }
    }

    fn complete_call_argument(
        &mut self,
        argument: DeferredCallArgument,
    ) -> Result<CallArgument<OwnedExpr>, ExpressionParseError> {
        let (argument_shape, value) = match argument {
            DeferredCallArgument::Ready(ready) => {
                return Ok(ready.map_value(|id| self.expression_builder.own(id)));
            }
            DeferredCallArgument::Retained { shape, expression } => {
                (shape, self.complete_deferred_expression(expression)?)
            }
            DeferredCallArgument::Forward {
                shape, producer, ..
            } => {
                let value = self.take_forward_producer(producer, None)?;
                // The forward fragment deferred its receiving-boundary validation to here,
                // exactly as the retained path does through `complete_deferred_expression`.
                if !self.protected_typed_errors {
                    validate_completed_typed_errors(self.expression_builder.view_owned(value)?)?;
                }
                (shape, value)
            }
        };
        let value_span = argument_shape
            .value_span
            .or(self.expression_builder.view_owned(value)?.span());
        let argument = match argument_shape.target_param {
            Some(name) => CallArgument::named(
                value,
                name,
                argument_shape.access_mode,
                value_span,
                argument_shape.target_span,
            ),
            None => CallArgument::positional(value, argument_shape.access_mode, value_span),
        };
        let argument = argument.with_marker_span(argument_shape.marker_span);
        let argument = match argument_shape.parameter_slot {
            Some(slot) => argument.with_parameter_slot(slot),
            None => argument,
        };
        Ok(argument.with_following_separator_span(argument_shape.following_separator_span))
    }

    fn finish_handled_call(
        &mut self,
        call: HandledFallibleCall,
        error_return_type_id: Option<TypeId>,
    ) -> Result<OwnedExpr, ExpressionParseError> {
        let Some(error_return_type_id) = error_return_type_id else {
            let call_path = call.name;
            let expression = call.into_plain_expression(self.expression_builder)?;
            return if self.context.source_call_has_private_failure_lane(call_path) {
                Ok(self
                    .expression_builder
                    .with_private_call_failure_candidate(expression, call_path)?)
            } else {
                Ok(expression)
            };
        };
        // A retained call resumes in the authored position without a suffix: `!` propagation is
        // decided from the token stream at parse time and a resumed fragment cannot apply it, so
        // the call keeps the handled Recover shape that whole-expression catch completion owns.
        let expression = self
            .expression_builder
            .handled_fallible_function_call_with_typed_arguments(
                call.name,
                call.args,
                call.result_type_ids,
                error_return_type_id,
                FallibleExpressionHandling::Recover,
                call.call_span,
            )?;
        Ok(self.expression_builder.with_typed_error_producer(
            expression,
            error_return_type_id,
            None,
        )?)
    }

    fn pending_place(
        &mut self,
        target: &crate::compiler_frontend::ast::numeric_construction::PendingPlaceTarget,
    ) -> Result<PlaceId, ExpressionParseError> {
        let type_id = self.resolved_type(target.binding)?;
        let (declaration_id, value_mode) = {
            let resolved = self.resolved_entry(target.binding)?;
            (resolved.declaration_id, resolved.value_mode.to_owned())
        };
        let projections = self
            .expression_builder
            .append_place_projections(Vec::new(), target.span)?;
        Ok(self.expression_builder.append_place(
            crate::compiler_frontend::ast::expressions::expression_store::PlaceRecord {
                root: declaration_id,
                root_type_id: type_id,
                projections,
                type_id,
                value_mode,
                span: target.span,
            },
            target.span,
        )?)
    }
}

impl<'a, 'env> DeferredExpressionCompletion<'env> for Completion<'a, 'env> {
    fn shape_receiver(
        &mut self,
        shape: KnownShapeId,
    ) -> Result<Option<TypeId>, ExpressionParseError> {
        let mut state = self.state.borrow_mut();
        let KnownShape::Numeric(class) = state.shape(shape) else {
            return Ok(None);
        };
        merged_receiver_type(&mut state, class)
    }

    fn session_services(&mut self) -> DeferredExpressionServices<'_, 'env> {
        DeferredExpressionServices {
            context: self.context,
            type_interner: &mut *self.type_interner,
            expression_builder: &mut *self.expression_builder,
            const_values: &mut *self.const_values,
            string_table: &mut *self.string_table,
            path_fork: &mut *self.path_fork,
        }
    }

    fn resolve_deferred_body(
        &mut self,
        body: DeferredBody,
    ) -> Result<Vec<AstNode>, ExpressionParseError> {
        // A nested body shares this root's construction state, so its retained positions are the
        // root's own continuations in authored order; the retained frame/scope are not re-entered.
        self.resolve_draft(body.draft)
    }

    fn resolve_deferred_body_with_expectations(
        &mut self,
        body: DeferredBody,
        expectations: Option<&[TypeId]>,
    ) -> Result<Vec<AstNode>, ExpressionParseError> {
        // `Some` installs the nested body's own receiving contract and `None` installs "no
        // override", so an inferred block masks an enclosing catch's slots rather than inheriting
        // them. The previous state is restored on both Ok and Err.
        self.resolved_value_slots
            .push(expectations.map(<[TypeId]>::to_vec));
        let result = self.resolve_deferred_body(body);
        self.resolved_value_slots.pop();
        result
    }

    fn resolve_continuation_nodes(
        &mut self,
        id: NumericContinuationId,
        expectations: &[TypeId],
    ) -> Result<Vec<AstNode>, ExpressionParseError> {
        // The enclosing site's slots stay visible to value-production completion for exactly this
        // continuation: a fallback that parsed against a provisional contract materialises at the
        // receiving slot, never at a parse-time default.
        self.resolved_value_slots.push(Some(expectations.to_vec()));
        let result = self.resolve_continuation(id).and_then(|()| {
            match std::mem::replace(
                &mut self.resumed[id.as_usize()],
                ContinuationResolution::Consumed,
            ) {
                ContinuationResolution::Completed(nodes) => Ok(nodes),
                _ => Err(CompilerError::compiler_error(
                    "a retained handler continuation did not publish its statement nodes",
                )
                .into()),
            }
        });
        self.resolved_value_slots.pop();
        result
    }

    fn resolve_statement_expression(
        &mut self,
        expression: StatementExpression,
    ) -> Result<OwnedExpr, ExpressionParseError> {
        self.complete_statement_expression(expression)
    }

    fn resolve_protected_statement_expression(
        &mut self,
        expression: StatementExpression,
    ) -> Result<OwnedExpr, ExpressionParseError> {
        // Fully qualified: the inherent method owns the construction and the protected-fragment
        // rule, and this impl must not resolve back to the ordinary entry point.
        Completion::resolve_protected_statement_expression(self, expression)
    }

    fn resolve_fragment(
        &mut self,
        items: Vec<ExpressionRpnItem>,
        expected: ExpectedType,
        value_mode: &ValueMode,
    ) -> Result<OwnedExpr, ExpressionParseError> {
        self.complete_items(items, expected, value_mode)
    }

    fn complete_call_argument(
        &mut self,
        argument: DeferredCallArgument,
    ) -> Result<CallArgument<OwnedExpr>, ExpressionParseError> {
        // Fully qualified: the inherent method owns the placement, and this impl must not recurse
        // back into the trait method.
        Completion::complete_call_argument(self, argument)
    }

    fn typed_errors_are_protected(&self) -> bool {
        self.protected_typed_errors
    }

    fn settle_forward_producers(
        &mut self,
        shape: KnownShapeId,
    ) -> Result<(), ExpressionParseError> {
        loop {
            let next = self
                .state
                .borrow_mut()
                .next_pending_forward_producer(shape)?;
            let Some(producer) = next else {
                return Ok(());
            };
            self.pull_forward_producer(producer, None)?;
        }
    }

    fn take_forward_producer(
        &mut self,
        producer: ForwardProducerId,
        expected: Option<ExpectedType>,
    ) -> Result<OwnedExpr, ExpressionParseError> {
        let completed = self
            .state
            .borrow_mut()
            .take_completed_forward_producer(producer)?;
        if let Some(expression) = completed {
            return Ok(self.expression_builder.own(expression));
        }
        self.pull_forward_producer(producer, expected)?;
        let taken = self
            .state
            .borrow_mut()
            .take_completed_forward_producer(producer)?;
        let expression = taken.ok_or_else(|| {
            CompilerError::compiler_error(
                "a forward producer was read while still resolving or after it was taken",
            )
        })?;
        Ok(self.expression_builder.own(expression))
    }
}

/// A pending declaration whose value is owned by a different authored position.
///
/// WHAT: an absent initializer with the resolved binding type and authored binding metadata.
/// WHY: range counters and inferred multi-bind targets are visible before their owning
///      continuation is emitted, but their values are either implicit or installed at another
///      authored position; neither needs a fabricated initializer.
fn absent_declaration(
    row: &PendingBindingRow,
    type_id: TypeId,
    environment: &crate::compiler_frontend::datatypes::environment::TypeEnvironment,
) -> Declaration {
    Declaration {
        id: row.path,
        type_id,
        diagnostic_type: diagnostic_type_spelling(type_id, environment),
        value_mode: row.value_mode,
        value: DeclarationValue::Absent,
        binding_span: row.binding_span,
        config_qualifier: None,
    }
}

fn declaration_for(
    row: &PendingBindingRow,
    value: OwnedExpr,
    expressions: &mut AstExpressionBuilder,
    environment: &crate::compiler_frontend::datatypes::environment::TypeEnvironment,
) -> Result<Declaration, ExpressionParseError> {
    // Reuse unchanged occurrence metadata; changed access belongs to an isolated replacement.
    let expression = expressions.view_owned(value)?;
    let type_id = expression.value_type_id()?;
    let value = if expression.value_mode() == row.value_mode {
        value
    } else {
        let mut input = expression.to_completed_input(expressions)?;
        input.value_mode = row.value_mode;
        expressions.commit_replacement(input)?
    };
    Ok(Declaration {
        id: row.path,
        type_id,
        diagnostic_type: diagnostic_type_spelling(type_id, environment),
        value_mode: row.value_mode,
        value: DeclarationValue::Initializer(value.id()),
        binding_span: row.binding_span,
        config_qualifier: None,
    })
}

/// Build the completed initializer of a copy or plain reference to another binding.
///
/// WHAT: the reference/copy keeps its source's own mode and const-record facts, while the
///       declaring expression's value mode is the target's authored marker (applied by
///       `declaration_for`).
/// WHY: ref/copy semantics read the source's declaration facts, but the target binding's
///      mutability is authored on the target statement, never inherited from the source.
fn alias_value(
    row: &PendingBindingRow,
    source_path: PathId,
    source_mode: &ValueMode,
    source_const_record_state: ConstRecordState,
    type_id: TypeId,
    is_copy: bool,
    expressions: &mut AstExpressionBuilder,
) -> Result<OwnedExpr, ExpressionParseError> {
    if is_copy {
        let projections = expressions.append_place_projections(Vec::new(), row.binding_span)?;
        let place = expressions.append_place(
            crate::compiler_frontend::ast::expressions::expression_store::PlaceRecord {
                root: source_path,
                root_type_id: type_id,
                projections,
                type_id,
                value_mode: *source_mode,
                span: row.binding_span,
            },
            row.binding_span,
        )?;
        Ok(expressions.copy_with_type_id(place, type_id, row.span, row.value_mode)?)
    } else {
        Ok(expressions.reference_with_type_id(
            source_path,
            type_id,
            row.span,
            *source_mode,
            source_const_record_state,
        )?)
    }
}

fn immutable_binding_diagnostic(
    type_id: TypeId,
    declaration_span: Option<SourceSpan>,
    assignment_span: Option<SourceSpan>,
) -> CompilerDiagnostic {
    CompilerDiagnostic::invalid_assignment_target(
        InvalidAssignmentTargetReason::ImmutableBinding,
        None,
        Some(type_id),
        None,
        None,
        declaration_span,
        assignment_span,
    )
}

/// Finishes one retained option selector's capture pattern against its completed subject.
///
/// WHAT: a pending option capture resolves its payload from the completed subject type, and a
///       ready pattern checks its compatibility with that type.
/// WHY: a template selector's subject type is known only once its value completes at the body
///      root, so its capture pattern cannot finish while the template head parses.
fn finish_retained_selector_pattern<'env>(
    pattern: DeferredArmPattern,
    subject_type_id: TypeId,
    completion: &mut impl DeferredExpressionCompletion<'env>,
) -> Result<MatchPattern, ExpressionParseError> {
    match pattern {
        DeferredArmPattern::Ready(ready_pattern) => {
            let services = completion.session_services();
            ensure_pattern_compatible_with_subject(
                &ready_pattern,
                subject_type_id,
                services.type_interner.environment(),
            )?;
            Ok(ready_pattern)
        }
        DeferredArmPattern::PendingOption(pending) => {
            finish_pending_option_pattern(pending, subject_type_id, completion)
        }
        DeferredArmPattern::PendingChoice(_) | DeferredArmPattern::Retained(_) => {
            Err(CompilerError::compiler_error(
                "A retained template option selector carried a pattern only a choice or \
                 retained arm can hold.",
            )
            .into())
        }
    }
}
