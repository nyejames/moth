//! Published expression occurrence census, erased without `benchmark_counters`.
//!
//! WHAT: observes the emitted builtin+source AST at publication. Diagnostic-owned
//! signature defaults are a separate expression partition; neither partition is a
//! unique-allocation count. All kind/list/fact distributions include both partitions.
//! WHY: Phase 2 needs exact predecessor shapes and typed payload bounds, not an
//! observer's reads mistaken for semantic traffic. No source reconstruction or
//! expression cloning occurs here. AST/expression/datatype descent uses borrowed
//! worklists. Canonical TIR and neutral handoff visitors still recurse structurally.
//! Callback re-entry for nested template/handoff expressions remains recursive.
//! This adds no stack-budget exemption; the existing 1 MiB lanes remain required.

use crate::compiler_frontend::ast::ast_nodes::{AstNode, Declaration, LoopBindings, NodeKind};
use crate::compiler_frontend::ast::expressions::expression::{
    ConstRecordState, Expression, ExpressionKind, FallibleHandling,
};
use crate::compiler_frontend::ast::expressions::expression_rpn::{
    ExpressionRpnItem, PlaceExpression, PlaceExpressionKind,
};
use crate::compiler_frontend::ast::expressions::failure_facts::ExpressionFailureFacts;
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::ast::statements::match_patterns::{MatchArm, MatchPattern};
use crate::compiler_frontend::ast::statements::value_production::types::ValueBlock;
use crate::compiler_frontend::ast::templates::runtime_handoff::{
    self, OwnedRuntimeTemplateBody, OwnedRuntimeTemplateHandoff, OwnedRuntimeTemplateNode,
};
use crate::compiler_frontend::ast::templates::template_control_flow::{
    TemplateBranchSelector, TemplateLoopHeader,
};
use crate::compiler_frontend::ast::templates::tir::{
    TemplateIrNodeKind, TemplateIrStore, TirView, TirViewIdentity, finalized_tir_view_for_template,
    walk_tir_view_expression_payloads,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::folded_value::OwnedFoldedString;
use crate::compiler_frontend::instrumentation::{
    AstCounter, add_ast_counter, increment_ast_counter, record_ast_counter_max,
    with_expression_census_counters,
};
use std::mem::{align_of, size_of};

#[derive(Clone, Copy)]
enum Partition {
    Semantic,
    Diagnostic,
}

enum Item<'a> {
    Node(&'a AstNode),
    Expression(&'a Expression),
    Declaration(&'a Declaration),
    DataType(&'a DataType),
    Pattern(&'a MatchPattern),
    Signature(&'a FunctionSignature),
    Place(&'a PlaceExpression),
}

struct Census<'a> {
    store: &'a TemplateIrStore,
    active_views: Vec<TirViewIdentity>,
}
/// Native AST snapshots are additive per module/config, not build-global maxima.
/// Inline sizes are constant gauges, recorded once at this publication boundary.
/// Each finalizer call self-emits one census snapshot, including generated sidecars.
/// The helper isolates the census range and restores existing overlay traffic.
pub(super) fn record_published_expression_census(nodes: &Vec<AstNode>, store: &TemplateIrStore) {
    with_expression_census_counters(|| {
        increment_ast_counter(AstCounter::CensusModules);
        record_layouts();
        record_list(nodes, ListKind::BodyNodes);
        let mut census = Census {
            store,
            active_views: Vec::new(),
        };
        for node in nodes {
            census.drain(Item::Node(node), Partition::Semantic);
        }
    });
}

impl Census<'_> {
    fn drain(&mut self, initial: Item<'_>, partition: Partition) {
        let mut pending = vec![(initial, partition)];
        while let Some((item, partition)) = pending.pop() {
            match item {
                Item::Node(node) => self.node(node, partition, &mut pending),
                Item::Expression(expression) => {
                    self.expression(expression, partition, &mut pending)
                }
                Item::Declaration(declaration) => {
                    increment_ast_counter(AstCounter::CensusDeclarations);
                    pending.push((Item::Expression(&declaration.value), partition));
                }
                Item::DataType(data_type) => self.data_type(data_type, &mut pending),
                Item::Pattern(pattern) => enqueue_pattern(pattern, partition, &mut pending),
                Item::Signature(signature) => {
                    record_list(&signature.parameters, ListKind::SignatureParameters);
                    record_list(&signature.returns, ListKind::SignatureReturns);
                    enqueue_declarations(&signature.parameters, partition, &mut pending);
                    for slot in &signature.returns {
                        pending.push((Item::DataType(&slot.value), Partition::Diagnostic));
                    }
                }
                Item::Place(place) => {
                    let mut depth = 0;
                    let mut current = place;
                    increment_ast_counter(AstCounter::CensusPlaceRoots);
                    loop {
                        pending.push((
                            Item::DataType(&current.diagnostic_type),
                            Partition::Diagnostic,
                        ));
                        match &current.kind {
                            PlaceExpressionKind::Local(_) => {
                                increment_ast_counter(AstCounter::CensusPlaceLocals);
                                break;
                            }
                            PlaceExpressionKind::Field { base, .. } => {
                                increment_ast_counter(AstCounter::CensusPlaceFields);
                                depth += 1;
                                current = base;
                            }
                        }
                    }
                    add_ast_counter(AstCounter::CensusPlaceDepthSum, depth);
                    record_ast_counter_max(AstCounter::CensusPlaceDepthMax, depth);
                }
            }
        }
    }

    fn node<'a>(
        &mut self,
        node: &'a AstNode,
        partition: Partition,
        pending: &mut Vec<(Item<'a>, Partition)>,
    ) {
        increment_ast_counter(AstCounter::CensusStatements);
        match &node.kind {
            NodeKind::Return(values) => {
                record_list(values, ListKind::ProducedValues);
                for value in values {
                    pending.push((Item::Expression(value), partition));
                }
            }
            NodeKind::ReturnError(value)
            | NodeKind::PushStartRuntimeFragment(value)
            | NodeKind::ExpressionStatement(value) => {
                pending.push((Item::Expression(value), partition))
            }
            NodeKind::If(condition, then_body, else_body, _) => {
                pending.push((Item::Expression(condition), partition));
                enqueue_nodes(then_body, partition, pending);
                if let Some(body) = else_body {
                    enqueue_nodes(body, partition, pending);
                }
            }
            NodeKind::Match {
                scrutinee,
                arms,
                default,
                ..
            } => {
                pending.push((Item::Expression(scrutinee), partition));
                enqueue_arms(arms, partition, pending);
                if let Some(body) = default {
                    enqueue_nodes(body, partition, pending);
                }
            }
            NodeKind::LexicalScope { body } => enqueue_nodes(body, partition, pending),
            NodeKind::RangeLoop {
                bindings,
                range,
                body,
            } => {
                enqueue_bindings(bindings, partition, pending);
                pending.push((Item::Expression(&range.start), partition));
                pending.push((Item::Expression(&range.end), partition));
                if let Some(step) = &range.step {
                    pending.push((Item::Expression(step), partition));
                }
                enqueue_nodes(body, partition, pending);
            }
            NodeKind::CollectionLoop {
                bindings,
                iterable,
                body,
            } => {
                enqueue_bindings(bindings, partition, pending);
                pending.push((Item::Expression(iterable), partition));
                enqueue_nodes(body, partition, pending);
            }
            NodeKind::WhileLoop(condition, body) => {
                pending.push((Item::Expression(condition), partition));
                enqueue_nodes(body, partition, pending);
            }
            NodeKind::Assert { condition, message } => {
                pending.push((Item::Expression(condition), partition));
                pending.push((Item::Expression(message), partition));
            }
            NodeKind::ThenValue(values) => {
                record_list(&values.expressions, ListKind::ProducedValues);
                for value in &values.expressions {
                    pending.push((Item::Expression(value), partition));
                }
            }
            NodeKind::VariableDeclaration(declaration) => {
                pending.push((Item::Declaration(declaration), partition))
            }
            NodeKind::StructDefinition(_, fields) => {
                record_list(fields, ListKind::Fields);
                enqueue_declarations(fields, partition, pending);
            }
            NodeKind::Function(_, signature, body) => {
                pending.push((Item::Signature(signature), partition));
                enqueue_nodes(body, partition, pending);
            }
            NodeKind::Assignment { target, value } => {
                pending.push((Item::Place(target), partition));
                pending.push((Item::Expression(value), partition));
            }
            NodeKind::MultiBind { targets, value } => {
                record_list(targets, ListKind::MultiBindTargets);
                pending.push((Item::Expression(value), partition));
            }
            NodeKind::Break | NodeKind::Continue => {}
        }
    }

    fn expression<'a>(
        &mut self,
        expression: &'a Expression,
        partition: Partition,
        pending: &mut Vec<(Item<'a>, Partition)>,
    ) {
        increment_ast_counter(match partition {
            Partition::Semantic => AstCounter::CensusExpressions,
            Partition::Diagnostic => AstCounter::CensusDiagnosticExpressions,
        });
        record_kind(&expression.kind);
        record_facts(expression);
        pending.push((
            Item::DataType(&expression.diagnostic_type),
            Partition::Diagnostic,
        ));
        match &expression.kind {
            ExpressionKind::Runtime(rpn) => {
                record_list(&rpn.items, ListKind::RpnItems);
                for item in &rpn.items {
                    match item {
                        ExpressionRpnItem::Operand(value) => {
                            increment_ast_counter(AstCounter::CensusRpnOperands);
                            pending.push((Item::Expression(value), partition));
                        }
                        ExpressionRpnItem::Operator { .. } => {
                            increment_ast_counter(AstCounter::CensusRpnOperators)
                        }
                        ExpressionRpnItem::PendingNumericLiteral { .. } => {
                            increment_ast_counter(AstCounter::CensusPendingNumericLiterals)
                        }
                        ExpressionRpnItem::PendingGroup { .. } => {
                            increment_ast_counter(AstCounter::CensusPendingGroups)
                        }
                    }
                }
            }
            ExpressionKind::Copy(place) => pending.push((Item::Place(place), partition)),
            ExpressionKind::Function(signature) => {
                pending.push((Item::Signature(signature), partition))
            }
            ExpressionKind::FieldAccess { base, .. } => {
                pending.push((Item::Expression(base), partition))
            }
            ExpressionKind::FunctionCall {
                args,
                result_type_ids,
                ..
            }
            | ExpressionKind::HostFunctionCall {
                args,
                result_type_ids,
                ..
            }
            | ExpressionKind::HandledFallibleFunctionCall {
                args,
                result_type_ids,
                ..
            }
            | ExpressionKind::HandledFallibleHostFunctionCall {
                args,
                result_type_ids,
                ..
            } => {
                record_list(args, ListKind::CallArgs);
                record_list(result_type_ids, ListKind::CallResultIds);
                for argument in args {
                    pending.push((Item::Expression(&argument.value), partition));
                }
            }
            ExpressionKind::MethodCall {
                receiver,
                args,
                result_type_ids,
                ..
            }
            | ExpressionKind::CollectionBuiltinCall {
                receiver,
                args,
                result_type_ids,
                ..
            }
            | ExpressionKind::MapBuiltinCall {
                receiver,
                args,
                result_type_ids,
                ..
            } => {
                pending.push((Item::Expression(receiver), partition));
                record_list(args, ListKind::CallArgs);
                record_list(result_type_ids, ListKind::CallResultIds);
                for argument in args {
                    pending.push((Item::Expression(&argument.value), partition));
                }
            }
            ExpressionKind::Cast(cast) => pending.push((Item::Expression(&cast.source), partition)),
            ExpressionKind::HandledFallibleExpression { value, .. }
            | ExpressionKind::OptionPropagation { value }
            | ExpressionKind::Coerced { value, .. } => {
                pending.push((Item::Expression(value), partition))
            }
            #[cfg(test)]
            ExpressionKind::FallibleCarrierConstruct { value, .. } => {
                pending.push((Item::Expression(value), partition))
            }
            ExpressionKind::Template(template) => {
                let result =
                    finalized_tir_view_for_template(template, self.store).and_then(|view| {
                        let identity = view.identity();
                        if self.active_views.contains(&identity) {
                            increment_ast_counter(AstCounter::CensusIncompleteViews);
                            return Ok(());
                        }
                        self.active_views.push(identity);
                        let result = walk_tir_view_expression_payloads(&view, &mut |value| {
                            self.drain(Item::Expression(value), partition);
                            Ok(())
                        })
                        .and_then(|()| self.view_auxiliary_payloads(&view, partition));
                        self.active_views.pop();
                        result
                    });
                if result.is_err() {
                    increment_ast_counter(AstCounter::CensusIncompleteViews);
                }
            }
            ExpressionKind::RuntimeTemplateHandoff(handoff) => {
                self.handoff_lists(handoff);
                let result: Result<(), std::convert::Infallible> =
                    runtime_handoff::walk_owned_runtime_template_handoff(handoff, &mut |node| {
                        self.handoff_node(node, partition);
                        Ok(())
                    });
                match result {
                    Ok(()) => {}
                    Err(error) => match error {},
                }
            }
            ExpressionKind::RuntimeSlotApplicationHandoff(handoff) => {
                record_list(&handoff.contribution_sources, ListKind::HandoffSources);
                record_list(&handoff.slot_sites, ListKind::HandoffSites);
                let result: Result<(), std::convert::Infallible> =
                    runtime_handoff::walk_owned_runtime_slot_application_handoff(
                        handoff,
                        &mut |node| {
                            self.handoff_node(node, partition);
                            Ok(())
                        },
                    );
                match result {
                    Ok(()) => {}
                    Err(error) => match error {},
                }
            }
            ExpressionKind::Collection(values) => {
                record_list(values, ListKind::CollectionItems);
                for value in values {
                    pending.push((Item::Expression(value), partition));
                }
            }
            ExpressionKind::MapLiteral(entries) => {
                record_list(entries, ListKind::MapEntries);
                for entry in entries {
                    pending.push((Item::Expression(&entry.key), partition));
                    pending.push((Item::Expression(&entry.value), partition));
                }
            }
            ExpressionKind::StructDefinition(fields)
            | ExpressionKind::StructInstance(fields)
            | ExpressionKind::AnonymousConstRecord { fields }
            | ExpressionKind::ChoiceConstruct { fields, .. } => {
                record_list(fields, ListKind::Fields);
                enqueue_declarations(fields, partition, pending);
            }
            ExpressionKind::Range(start, end) => {
                pending.push((Item::Expression(start), partition));
                pending.push((Item::Expression(end), partition));
            }
            ExpressionKind::ValueBlock { block } => match block.as_ref() {
                ValueBlock::If(block) => {
                    record_list(&block.result_type_ids, ListKind::ValueResultIds);
                    pending.push((Item::Expression(&block.condition), partition));
                    enqueue_nodes(&block.then_body, partition, pending);
                    enqueue_nodes(&block.else_body, partition, pending);
                }
                ValueBlock::LexicalScope(block) => {
                    record_list(&block.result_type_ids, ListKind::ValueResultIds);
                    enqueue_nodes(&block.body, partition, pending);
                }
                ValueBlock::Match(block) => {
                    record_list(&block.result_type_ids, ListKind::ValueResultIds);
                    pending.push((Item::Expression(&block.scrutinee), partition));
                    enqueue_arms(&block.arms, partition, pending);
                    if let Some(body) = &block.default {
                        enqueue_nodes(body, partition, pending);
                    }
                }
                ValueBlock::Catch(block) => {
                    record_list(&block.result_type_ids, ListKind::ValueResultIds);
                    pending.push((Item::Expression(&block.handled_value), partition));
                    if let FallibleHandling::Handler { body, .. } = &block.handler {
                        enqueue_nodes(body, partition, pending);
                    }
                }
            },
            ExpressionKind::StructuralString { pieces } => {
                record_list(pieces, ListKind::StructuralPieces)
            }
            ExpressionKind::NoValue
            | ExpressionKind::OptionNone
            | ExpressionKind::Int(_)
            | ExpressionKind::Uint(_)
            | ExpressionKind::Float(_)
            | ExpressionKind::FixedScalar(_)
            | ExpressionKind::Number(_)
            | ExpressionKind::StringSlice(_)
            | ExpressionKind::Bool(_)
            | ExpressionKind::Char(_)
            | ExpressionKind::Reference(_) => {}
        }
    }

    fn data_type<'a>(&mut self, data_type: &'a DataType, pending: &mut Vec<(Item<'a>, Partition)>) {
        match data_type {
            DataType::Function(_, signature) => {
                pending.push((Item::Signature(signature), Partition::Diagnostic))
            }
            DataType::Parameters(declarations) => {
                record_list(declarations, ListKind::Fields);
                enqueue_declarations(declarations, Partition::Diagnostic, pending);
            }
            DataType::GenericInstance { arguments, .. } | DataType::Returns(arguments) => {
                for argument in arguments {
                    pending.push((Item::DataType(argument), Partition::Diagnostic));
                }
            }
            DataType::Option(inner) => pending.push((Item::DataType(inner), Partition::Diagnostic)),
            DataType::FallibleCarrier { success, error } => {
                pending.push((Item::DataType(success), Partition::Diagnostic));
                pending.push((Item::DataType(error), Partition::Diagnostic));
            }
            DataType::Inferred
            | DataType::NamedType(_)
            | DataType::NamespacedType { .. }
            | DataType::TypeParameter { .. }
            | DataType::Struct { .. }
            | DataType::Range
            | DataType::Template
            | DataType::Bool
            | DataType::Int
            | DataType::Uint
            | DataType::Float
            | DataType::Number(_)
            | DataType::StringSlice
            | DataType::Char
            | DataType::FixedScalar(_)
            | DataType::Choices { .. }
            | DataType::External { .. }
            | DataType::None
            | DataType::True
            | DataType::False => {}
        }
    }

    fn handoff_node(&mut self, node: &OwnedRuntimeTemplateNode, partition: Partition) {
        increment_ast_counter(AstCounter::CensusHandoffNodeOccurrences);
        match node {
            OwnedRuntimeTemplateNode::DynamicExpression { expression, .. } => {
                self.drain(Item::Expression(expression), partition)
            }
            OwnedRuntimeTemplateNode::Conditional { selector, .. } => match selector.as_ref() {
                TemplateBranchSelector::Bool(value) => {
                    self.drain(Item::Expression(value), partition)
                }
                TemplateBranchSelector::OptionPresentCapture { scrutinee, pattern } => {
                    self.drain(Item::Expression(scrutinee), partition);
                    self.drain(Item::Pattern(pattern), partition);
                }
            },
            OwnedRuntimeTemplateNode::Loop { header, .. } => match header {
                TemplateLoopHeader::Conditional { condition } => {
                    self.drain(Item::Expression(condition), partition)
                }
                TemplateLoopHeader::Range { bindings, range } => {
                    self.bindings(bindings, partition);
                    self.drain(Item::Expression(&range.start), partition);
                    self.drain(Item::Expression(&range.end), partition);
                    if let Some(step) = &range.step {
                        self.drain(Item::Expression(step), partition);
                    }
                }
                TemplateLoopHeader::Collection { bindings, iterable } => {
                    self.bindings(bindings, partition);
                    self.drain(Item::Expression(iterable), partition);
                }
            },
            OwnedRuntimeTemplateNode::Sequence { children, .. } => {
                record_list(children, ListKind::HandoffNodes)
            }
            OwnedRuntimeTemplateNode::Text { text, .. } => {
                if let OwnedFoldedString::Pieces(pieces) = text {
                    record_list(pieces, ListKind::OwnedStructuralPieces);
                }
            }
            OwnedRuntimeTemplateNode::ChildTemplate { template, .. } => {
                self.handoff_lists(template)
            }
            OwnedRuntimeTemplateNode::ConditionalWrapper { .. }
            | OwnedRuntimeTemplateNode::AggregateOutput
            | OwnedRuntimeTemplateNode::RuntimeSlotSite { .. }
            | OwnedRuntimeTemplateNode::RuntimeSlotContributionSource { .. }
            | OwnedRuntimeTemplateNode::Slot { .. } => {}
        }
    }

    // The canonical payload visitor owns overlay reads. This supplemental view
    // worklist visits only declaration/pattern positions that its expression-site
    // vocabulary does not expose, never stored expressions shadowed by overlays.
    fn view_auxiliary_payloads(
        &mut self,
        view: &TirView<'_>,
        partition: Partition,
    ) -> Result<(), CompilerError> {
        let mut visited = vec![view.identity()];
        let mut pending = vec![(view.clone(), view.root_template()?.root)];
        while let Some((view, node_id)) = pending.pop() {
            match &view.effective_node(node_id)?.kind {
                TemplateIrNodeKind::Sequence { children } => {
                    record_list(children, ListKind::TirChildIds);
                    for child in children {
                        pending.push((view.clone(), *child));
                    }
                }
                TemplateIrNodeKind::Conditional { selector, body, .. } => {
                    if let TemplateBranchSelector::OptionPresentCapture { pattern, .. } =
                        selector.as_ref()
                    {
                        self.drain(Item::Pattern(pattern), partition);
                    }
                    pending.push((view.clone(), *body));
                }
                TemplateIrNodeKind::Loop {
                    header,
                    body,
                    aggregate_wrapper,
                    ..
                } => {
                    match header {
                        TemplateLoopHeader::Range { bindings, .. }
                        | TemplateLoopHeader::Collection { bindings, .. } => {
                            self.bindings(bindings, partition)
                        }
                        TemplateLoopHeader::Conditional { .. } => {}
                    }
                    pending.push((view.clone(), *body));
                    if let Some(wrapper) = aggregate_wrapper {
                        pending.push((view.clone(), *wrapper));
                    }
                }
                TemplateIrNodeKind::ChildTemplate { reference, .. } => {
                    let child = view.structural_child(*reference)?;
                    if !visited.contains(&child.identity()) {
                        visited.push(child.identity());
                        let root = child.root_template()?.root;
                        pending.push((child, root));
                    }
                }
                TemplateIrNodeKind::InsertContribution { template } => {
                    let child = view.structural_helper(*template)?;
                    if !visited.contains(&child.identity()) {
                        visited.push(child.identity());
                        let root = child.root_template()?.root;
                        pending.push((child, root));
                    }
                }
                TemplateIrNodeKind::DynamicExpression { .. }
                | TemplateIrNodeKind::Text { .. }
                | TemplateIrNodeKind::Slot { .. }
                | TemplateIrNodeKind::AggregateOutput
                | TemplateIrNodeKind::RuntimeSlotSite { .. }
                | TemplateIrNodeKind::RuntimeSlotContributionSource { .. } => {}
            }
        }
        Ok(())
    }

    fn handoff_lists(&mut self, handoff: &OwnedRuntimeTemplateHandoff) {
        if let OwnedRuntimeTemplateBody::RuntimeSlotApplication(slots) = &handoff.body {
            record_list(&slots.contribution_sources, ListKind::HandoffSources);
            record_list(&slots.slot_sites, ListKind::HandoffSites);
        }
    }

    fn bindings(&mut self, bindings: &LoopBindings, partition: Partition) {
        if let Some(item) = &bindings.item {
            self.drain(Item::Declaration(item), partition);
        }
        if let Some(index) = &bindings.index {
            self.drain(Item::Declaration(index), partition);
        }
    }
}

fn enqueue_nodes<'a>(
    nodes: &'a Vec<AstNode>,
    partition: Partition,
    pending: &mut Vec<(Item<'a>, Partition)>,
) {
    record_list(nodes, ListKind::BodyNodes);
    for node in nodes {
        pending.push((Item::Node(node), partition));
    }
}

fn enqueue_declarations<'a>(
    declarations: &'a [Declaration],
    partition: Partition,
    pending: &mut Vec<(Item<'a>, Partition)>,
) {
    for declaration in declarations {
        pending.push((Item::Declaration(declaration), partition));
    }
}

fn enqueue_bindings<'a>(
    bindings: &'a LoopBindings,
    partition: Partition,
    pending: &mut Vec<(Item<'a>, Partition)>,
) {
    if let Some(item) = &bindings.item {
        pending.push((Item::Declaration(item), partition));
    }
    if let Some(index) = &bindings.index {
        pending.push((Item::Declaration(index), partition));
    }
}

fn enqueue_arms<'a>(
    arms: &'a Vec<MatchArm>,
    partition: Partition,
    pending: &mut Vec<(Item<'a>, Partition)>,
) {
    record_list(arms, ListKind::MatchArms);
    for arm in arms {
        pending.push((Item::Pattern(&arm.pattern), partition));
        if let Some(guard) = &arm.guard {
            pending.push((Item::Expression(guard), partition));
        }
        enqueue_nodes(&arm.body, partition, pending);
    }
}

fn enqueue_pattern<'a>(
    pattern: &'a MatchPattern,
    partition: Partition,
    pending: &mut Vec<(Item<'a>, Partition)>,
) {
    match pattern {
        MatchPattern::Literal(value)
        | MatchPattern::OptionValue { value, .. }
        | MatchPattern::Relational { value, .. } => {
            pending.push((Item::Expression(value), partition))
        }
        MatchPattern::ChoiceVariant { captures, .. } => {
            record_list(captures, ListKind::PatternCaptures)
        }
        MatchPattern::OptionNone { .. } | MatchPattern::OptionPresentCapture { .. } => {}
    }
}

fn record_facts(expression: &Expression) {
    // Comparing to empty facts only tests Vec length and scalar options. It does not
    // select witnesses or traverse contributors, and cannot mutate semantic state.
    let failure = expression.failure_facts != ExpressionFailureFacts::default();
    let receiver = expression.function_receiver.is_some();
    let absent_span = expression.span.is_none();
    let record = expression.const_record_state != ConstRecordState::RuntimeValue;
    let division = expression.contains_regular_division;
    let provenance = !expression.synthetic_interface_provenance.is_empty();
    for (present, counter) in [
        (failure, AstCounter::CensusFailurePresent),
        (receiver, AstCounter::CensusReceiverPresent),
        (absent_span, AstCounter::CensusSpanAbsent),
        (record, AstCounter::CensusConstRecordPresent),
        (division, AstCounter::CensusDivisionPresent),
        (provenance, AstCounter::CensusProvenancePresent),
        (failure && provenance, AstCounter::CensusFailureProvenance),
        (failure && division, AstCounter::CensusFailureDivision),
        (provenance && division, AstCounter::CensusProvenanceDivision),
        (
            absent_span && (failure || receiver || record || division || provenance),
            AstCounter::CensusSpanAbsentSparse,
        ),
    ] {
        if present {
            increment_ast_counter(counter);
        }
    }
    record_list(
        &expression.failure_facts.implicit,
        ListKind::ColdImplicitFacts,
    );
}

fn record_kind(kind: &ExpressionKind) {
    let counter = match kind {
        ExpressionKind::NoValue => AstCounter::CensusKindNoValue,
        ExpressionKind::OptionNone => AstCounter::CensusKindOptionNone,
        ExpressionKind::Runtime(..) => AstCounter::CensusKindRuntime,
        ExpressionKind::Int(..) => AstCounter::CensusKindInt,
        ExpressionKind::Uint(..) => AstCounter::CensusKindUint,
        ExpressionKind::Float(..) => AstCounter::CensusKindFloat,
        ExpressionKind::FixedScalar(..) => AstCounter::CensusKindFixedScalar,
        ExpressionKind::Number(..) => AstCounter::CensusKindNumber,
        ExpressionKind::StringSlice(..) => AstCounter::CensusKindStringSlice,
        ExpressionKind::Bool(..) => AstCounter::CensusKindBool,
        ExpressionKind::Char(..) => AstCounter::CensusKindChar,
        ExpressionKind::StructuralString { .. } => AstCounter::CensusKindStructuralString,
        ExpressionKind::Reference(..) => AstCounter::CensusKindReference,
        ExpressionKind::Copy(..) => AstCounter::CensusKindCopy,
        ExpressionKind::Function(..) => AstCounter::CensusKindFunction,
        ExpressionKind::FunctionCall { .. } => AstCounter::CensusKindFunctionCall,
        ExpressionKind::FieldAccess { .. } => AstCounter::CensusKindFieldAccess,
        ExpressionKind::MethodCall { .. } => AstCounter::CensusKindMethodCall,
        ExpressionKind::CollectionBuiltinCall { .. } => AstCounter::CensusKindCollectionBuiltinCall,
        ExpressionKind::MapBuiltinCall { .. } => AstCounter::CensusKindMapBuiltinCall,
        ExpressionKind::HandledFallibleFunctionCall { .. } => {
            AstCounter::CensusKindHandledFallibleFunctionCall
        }
        ExpressionKind::HandledFallibleHostFunctionCall { .. } => {
            AstCounter::CensusKindHandledFallibleHostFunctionCall
        }
        ExpressionKind::Cast(..) => AstCounter::CensusKindCast,
        ExpressionKind::HandledFallibleExpression { .. } => {
            AstCounter::CensusKindHandledFallibleExpression
        }
        ExpressionKind::OptionPropagation { .. } => AstCounter::CensusKindOptionPropagation,
        ExpressionKind::HostFunctionCall { .. } => AstCounter::CensusKindHostFunctionCall,
        ExpressionKind::Template(..) => AstCounter::CensusKindTemplate,
        ExpressionKind::RuntimeTemplateHandoff(..) => AstCounter::CensusKindRuntimeTemplateHandoff,
        ExpressionKind::RuntimeSlotApplicationHandoff(..) => {
            AstCounter::CensusKindRuntimeSlotApplicationHandoff
        }
        ExpressionKind::Collection(..) => AstCounter::CensusKindCollection,
        ExpressionKind::MapLiteral(..) => AstCounter::CensusKindMapLiteral,
        ExpressionKind::StructDefinition(..) => AstCounter::CensusKindStructDefinition,
        ExpressionKind::StructInstance(..) => AstCounter::CensusKindStructInstance,
        ExpressionKind::AnonymousConstRecord { .. } => AstCounter::CensusKindAnonymousConstRecord,
        ExpressionKind::Range(..) => AstCounter::CensusKindRange,
        ExpressionKind::Coerced { .. } => AstCounter::CensusKindCoerced,
        ExpressionKind::ChoiceConstruct { .. } => AstCounter::CensusKindChoiceConstruct,
        ExpressionKind::ValueBlock { .. } => AstCounter::CensusKindValueBlock,
        #[cfg(test)]
        ExpressionKind::FallibleCarrierConstruct { .. } => return,
    };
    increment_ast_counter(counter);
}

#[derive(Clone, Copy)]
enum ListKind {
    CallArgs,
    CallResultIds,
    CollectionItems,
    MapEntries,
    Fields,
    StructuralPieces,
    RpnItems,
    SignatureParameters,
    SignatureReturns,
    ProducedValues,
    BodyNodes,
    MatchArms,
    ColdImplicitFacts,
    ValueResultIds,
    MultiBindTargets,
    PatternCaptures,
    HandoffNodes,
    HandoffSources,
    HandoffSites,
    OwnedStructuralPieces,
    TirChildIds,
}

// Length/capacity units are elements of the named family, never mixed bytes.
fn record_list<T>(values: &Vec<T>, kind: ListKind) {
    let counters = match kind {
        ListKind::ValueResultIds => [
            AstCounter::CensusValueResultIdsLists,
            AstCounter::CensusValueResultIdsLengthSum,
            AstCounter::CensusValueResultIdsLengthMax,
            AstCounter::CensusValueResultIdsCapacitySum,
            AstCounter::CensusValueResultIdsCapacityMax,
            AstCounter::CensusValueResultIdsEmpty,
            AstCounter::CensusValueResultIdsSingle,
            AstCounter::CensusValueResultIdsMultiple,
        ],
        ListKind::MultiBindTargets => [
            AstCounter::CensusMultiBindTargetsLists,
            AstCounter::CensusMultiBindTargetsLengthSum,
            AstCounter::CensusMultiBindTargetsLengthMax,
            AstCounter::CensusMultiBindTargetsCapacitySum,
            AstCounter::CensusMultiBindTargetsCapacityMax,
            AstCounter::CensusMultiBindTargetsEmpty,
            AstCounter::CensusMultiBindTargetsSingle,
            AstCounter::CensusMultiBindTargetsMultiple,
        ],
        ListKind::PatternCaptures => [
            AstCounter::CensusPatternCapturesLists,
            AstCounter::CensusPatternCapturesLengthSum,
            AstCounter::CensusPatternCapturesLengthMax,
            AstCounter::CensusPatternCapturesCapacitySum,
            AstCounter::CensusPatternCapturesCapacityMax,
            AstCounter::CensusPatternCapturesEmpty,
            AstCounter::CensusPatternCapturesSingle,
            AstCounter::CensusPatternCapturesMultiple,
        ],
        ListKind::HandoffNodes => [
            AstCounter::CensusHandoffNodesLists,
            AstCounter::CensusHandoffNodesLengthSum,
            AstCounter::CensusHandoffNodesLengthMax,
            AstCounter::CensusHandoffNodesCapacitySum,
            AstCounter::CensusHandoffNodesCapacityMax,
            AstCounter::CensusHandoffNodesEmpty,
            AstCounter::CensusHandoffNodesSingle,
            AstCounter::CensusHandoffNodesMultiple,
        ],
        ListKind::HandoffSources => [
            AstCounter::CensusHandoffSourcesLists,
            AstCounter::CensusHandoffSourcesLengthSum,
            AstCounter::CensusHandoffSourcesLengthMax,
            AstCounter::CensusHandoffSourcesCapacitySum,
            AstCounter::CensusHandoffSourcesCapacityMax,
            AstCounter::CensusHandoffSourcesEmpty,
            AstCounter::CensusHandoffSourcesSingle,
            AstCounter::CensusHandoffSourcesMultiple,
        ],
        ListKind::HandoffSites => [
            AstCounter::CensusHandoffSitesLists,
            AstCounter::CensusHandoffSitesLengthSum,
            AstCounter::CensusHandoffSitesLengthMax,
            AstCounter::CensusHandoffSitesCapacitySum,
            AstCounter::CensusHandoffSitesCapacityMax,
            AstCounter::CensusHandoffSitesEmpty,
            AstCounter::CensusHandoffSitesSingle,
            AstCounter::CensusHandoffSitesMultiple,
        ],
        ListKind::OwnedStructuralPieces => [
            AstCounter::CensusOwnedStructuralPiecesLists,
            AstCounter::CensusOwnedStructuralPiecesLengthSum,
            AstCounter::CensusOwnedStructuralPiecesLengthMax,
            AstCounter::CensusOwnedStructuralPiecesCapacitySum,
            AstCounter::CensusOwnedStructuralPiecesCapacityMax,
            AstCounter::CensusOwnedStructuralPiecesEmpty,
            AstCounter::CensusOwnedStructuralPiecesSingle,
            AstCounter::CensusOwnedStructuralPiecesMultiple,
        ],
        ListKind::TirChildIds => [
            AstCounter::CensusTirChildIdsLists,
            AstCounter::CensusTirChildIdsLengthSum,
            AstCounter::CensusTirChildIdsLengthMax,
            AstCounter::CensusTirChildIdsCapacitySum,
            AstCounter::CensusTirChildIdsCapacityMax,
            AstCounter::CensusTirChildIdsEmpty,
            AstCounter::CensusTirChildIdsSingle,
            AstCounter::CensusTirChildIdsMultiple,
        ],
        ListKind::CallArgs => [
            AstCounter::CensusCallArgsLists,
            AstCounter::CensusCallArgsLengthSum,
            AstCounter::CensusCallArgsLengthMax,
            AstCounter::CensusCallArgsCapacitySum,
            AstCounter::CensusCallArgsCapacityMax,
            AstCounter::CensusCallArgsEmpty,
            AstCounter::CensusCallArgsSingle,
            AstCounter::CensusCallArgsMultiple,
        ],
        ListKind::CallResultIds => [
            AstCounter::CensusCallResultIdsLists,
            AstCounter::CensusCallResultIdsLengthSum,
            AstCounter::CensusCallResultIdsLengthMax,
            AstCounter::CensusCallResultIdsCapacitySum,
            AstCounter::CensusCallResultIdsCapacityMax,
            AstCounter::CensusCallResultIdsEmpty,
            AstCounter::CensusCallResultIdsSingle,
            AstCounter::CensusCallResultIdsMultiple,
        ],
        ListKind::CollectionItems => [
            AstCounter::CensusCollectionItemsLists,
            AstCounter::CensusCollectionItemsLengthSum,
            AstCounter::CensusCollectionItemsLengthMax,
            AstCounter::CensusCollectionItemsCapacitySum,
            AstCounter::CensusCollectionItemsCapacityMax,
            AstCounter::CensusCollectionItemsEmpty,
            AstCounter::CensusCollectionItemsSingle,
            AstCounter::CensusCollectionItemsMultiple,
        ],
        ListKind::MapEntries => [
            AstCounter::CensusMapEntriesLists,
            AstCounter::CensusMapEntriesLengthSum,
            AstCounter::CensusMapEntriesLengthMax,
            AstCounter::CensusMapEntriesCapacitySum,
            AstCounter::CensusMapEntriesCapacityMax,
            AstCounter::CensusMapEntriesEmpty,
            AstCounter::CensusMapEntriesSingle,
            AstCounter::CensusMapEntriesMultiple,
        ],
        ListKind::Fields => [
            AstCounter::CensusFieldsLists,
            AstCounter::CensusFieldsLengthSum,
            AstCounter::CensusFieldsLengthMax,
            AstCounter::CensusFieldsCapacitySum,
            AstCounter::CensusFieldsCapacityMax,
            AstCounter::CensusFieldsEmpty,
            AstCounter::CensusFieldsSingle,
            AstCounter::CensusFieldsMultiple,
        ],
        ListKind::StructuralPieces => [
            AstCounter::CensusStructuralPiecesLists,
            AstCounter::CensusStructuralPiecesLengthSum,
            AstCounter::CensusStructuralPiecesLengthMax,
            AstCounter::CensusStructuralPiecesCapacitySum,
            AstCounter::CensusStructuralPiecesCapacityMax,
            AstCounter::CensusStructuralPiecesEmpty,
            AstCounter::CensusStructuralPiecesSingle,
            AstCounter::CensusStructuralPiecesMultiple,
        ],
        ListKind::RpnItems => [
            AstCounter::CensusRpnItemsLists,
            AstCounter::CensusRpnItemsLengthSum,
            AstCounter::CensusRpnItemsLengthMax,
            AstCounter::CensusRpnItemsCapacitySum,
            AstCounter::CensusRpnItemsCapacityMax,
            AstCounter::CensusRpnItemsEmpty,
            AstCounter::CensusRpnItemsSingle,
            AstCounter::CensusRpnItemsMultiple,
        ],
        ListKind::SignatureParameters => [
            AstCounter::CensusSignatureParametersLists,
            AstCounter::CensusSignatureParametersLengthSum,
            AstCounter::CensusSignatureParametersLengthMax,
            AstCounter::CensusSignatureParametersCapacitySum,
            AstCounter::CensusSignatureParametersCapacityMax,
            AstCounter::CensusSignatureParametersEmpty,
            AstCounter::CensusSignatureParametersSingle,
            AstCounter::CensusSignatureParametersMultiple,
        ],
        ListKind::SignatureReturns => [
            AstCounter::CensusSignatureReturnsLists,
            AstCounter::CensusSignatureReturnsLengthSum,
            AstCounter::CensusSignatureReturnsLengthMax,
            AstCounter::CensusSignatureReturnsCapacitySum,
            AstCounter::CensusSignatureReturnsCapacityMax,
            AstCounter::CensusSignatureReturnsEmpty,
            AstCounter::CensusSignatureReturnsSingle,
            AstCounter::CensusSignatureReturnsMultiple,
        ],
        ListKind::ProducedValues => [
            AstCounter::CensusProducedValuesLists,
            AstCounter::CensusProducedValuesLengthSum,
            AstCounter::CensusProducedValuesLengthMax,
            AstCounter::CensusProducedValuesCapacitySum,
            AstCounter::CensusProducedValuesCapacityMax,
            AstCounter::CensusProducedValuesEmpty,
            AstCounter::CensusProducedValuesSingle,
            AstCounter::CensusProducedValuesMultiple,
        ],
        ListKind::BodyNodes => [
            AstCounter::CensusBodyNodesLists,
            AstCounter::CensusBodyNodesLengthSum,
            AstCounter::CensusBodyNodesLengthMax,
            AstCounter::CensusBodyNodesCapacitySum,
            AstCounter::CensusBodyNodesCapacityMax,
            AstCounter::CensusBodyNodesEmpty,
            AstCounter::CensusBodyNodesSingle,
            AstCounter::CensusBodyNodesMultiple,
        ],
        ListKind::MatchArms => [
            AstCounter::CensusMatchArmsLists,
            AstCounter::CensusMatchArmsLengthSum,
            AstCounter::CensusMatchArmsLengthMax,
            AstCounter::CensusMatchArmsCapacitySum,
            AstCounter::CensusMatchArmsCapacityMax,
            AstCounter::CensusMatchArmsEmpty,
            AstCounter::CensusMatchArmsSingle,
            AstCounter::CensusMatchArmsMultiple,
        ],
        ListKind::ColdImplicitFacts => [
            AstCounter::CensusColdImplicitFactsLists,
            AstCounter::CensusColdImplicitFactsLengthSum,
            AstCounter::CensusColdImplicitFactsLengthMax,
            AstCounter::CensusColdImplicitFactsCapacitySum,
            AstCounter::CensusColdImplicitFactsCapacityMax,
            AstCounter::CensusColdImplicitFactsEmpty,
            AstCounter::CensusColdImplicitFactsSingle,
            AstCounter::CensusColdImplicitFactsMultiple,
        ],
    };
    let [
        lists,
        length_sum,
        length_max,
        capacity_sum,
        capacity_max,
        empty,
        single,
        multiple,
    ] = counters;
    increment_ast_counter(lists);
    add_ast_counter(length_sum, values.len());
    record_ast_counter_max(length_max, values.len());
    add_ast_counter(capacity_sum, values.capacity());
    record_ast_counter_max(capacity_max, values.capacity());
    increment_ast_counter(match values.len() {
        0 => empty,
        1 => single,
        _ => multiple,
    });
}

fn record_layouts() {
    record_ast_counter_max(AstCounter::CensusExpressionSizeMax, size_of::<Expression>());
    record_ast_counter_max(
        AstCounter::CensusExpressionAlignMax,
        align_of::<Expression>(),
    );
    record_ast_counter_max(
        AstCounter::CensusExpressionKindSizeMax,
        size_of::<ExpressionKind>(),
    );
    record_ast_counter_max(
        AstCounter::CensusExpressionKindAlignMax,
        align_of::<ExpressionKind>(),
    );
    record_ast_counter_max(
        AstCounter::CensusRpnItemSizeMax,
        size_of::<ExpressionRpnItem>(),
    );
    record_ast_counter_max(
        AstCounter::CensusRpnItemAlignMax,
        align_of::<ExpressionRpnItem>(),
    );
    record_ast_counter_max(
        AstCounter::CensusDeclarationSizeMax,
        size_of::<Declaration>(),
    );
    record_ast_counter_max(
        AstCounter::CensusDeclarationAlignMax,
        align_of::<Declaration>(),
    );
    record_ast_counter_max(
        AstCounter::CensusPlaceExpressionSizeMax,
        size_of::<PlaceExpression>(),
    );
    record_ast_counter_max(
        AstCounter::CensusPlaceExpressionAlignMax,
        align_of::<PlaceExpression>(),
    );
}
