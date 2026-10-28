//! Retained numeric intakes owned by one template TIR store.
//!
//! WHAT: a template expression position whose value needs a body-root-selected numeric type
//!       registers one retained intake with the store and records a payload-free
//!       `RetainedNumericIntake` node at the authored position. Body-root completion moves every
//!       retained intake out, completes its fragment through the ordinary typed expression
//!       intake, and installs the completed value at that position.
//! WHY: a statement-position template head is a formatting consumer, so it must not default or
//!      fold a pending numeric binding (plan N23). Retaining the parsed fragment once and
//!      resuming the ordinary intake at the body root keeps pending origins and recipes out of
//!      TIR expressions, canonical values and published expressions.
//!
//! Invariants:
//! - every issued `NumericTemplateIntakeSiteId` indexes one record in the store's intake table
//! - a record keeps its authored position until `install_completed_numeric_intake` succeeds
//! - `take_pending_numeric_intakes` is idempotent and never removes a position
//! - a position node carries no expression until completion installs the completed value
//! - body-root completion drains by owning numeric domain, so no probe walks a template subtree

use crate::compiler_frontend::compiler_messages::CompilerStage;
use std::collections::HashMap;

use crate::compiler_frontend::ast::ScopeContext;
use crate::compiler_frontend::ast::ast_nodes::{
    Declaration, LoopBindings, RangeEndKind, RangeLoopSpec,
};
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::expressions::expression_rpn::ExpressionRpnItem;
use crate::compiler_frontend::ast::expressions::expression_store::{AstExpressionBuilder, ExprId};
use crate::compiler_frontend::ast::numeric_construction::{
    DeferredArmPattern, DeferredExpressionSuffix, NumericBindingId, NumericConstructionDomainId,
};
use crate::compiler_frontend::ast::statements::loop_headers::range_loop_spec_from_completed_operands;
use crate::compiler_frontend::ast::statements::match_patterns::MatchPattern;
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::template::TemplateSegmentOrigin;
use crate::compiler_frontend::ast::templates::template_control_flow::{
    TemplateBranchSelector, TemplateLoopHeader,
};
use crate::compiler_frontend::ast::templates::tir::ids::{
    NumericTemplateIntakeSiteId, TemplateIrNodeId,
};
use crate::compiler_frontend::ast::templates::tir::node::{
    RetainedControlFlowRenderUnit, RetainedNumericIntakeRole, TemplateBranchSelectorKind,
    TemplateIrNodeKind, TemplateLoopHeaderExpressionSites,
};
use crate::compiler_frontend::ast::templates::tir::store::TemplateIrStore;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::instrumentation::{AstCounter, increment_ast_counter};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::type_coercion::parse_context::ExpectedType;
use crate::compiler_frontend::value_mode::ValueMode;

/// One numeric-dependent template position retained until body-root completion.
///
/// WHAT: the intake site, the retained fragment, and the receiving metadata needed to resume the
///       ordinary typed expression intake.
/// WHY: completion drives this fragment through the same `substitute_bindings` and
///      `evaluate_expression` path an unretained expression would take, then installs the result
///      at the authored position.
#[derive(Debug, Clone)]
pub(crate) struct PendingNumericTemplateIntake {
    pub(crate) site: NumericTemplateIntakeSiteId,
    /// Store row handle and dense body-local group, assigned in first-operand intake order.
    pub(crate) range_header: Option<(RetainedRangeHeaderId, usize)>,
    /// Authored position this fragment completes into.
    ///
    /// WHY: completion groups range operands by their position node before installing the header
    ///      once, so the taken record must name its position instead of a second store lookup.
    pub(crate) position: NumericTemplateIntakePosition,
    pub(crate) items: Vec<ExpressionRpnItem>,
    pub(crate) expected: ExpectedType,
    pub(crate) value_mode: ValueMode,
    /// Authored trailing handling consumed at the deferral point.
    ///
    /// WHY: completion materialises the exact authored suffix instead of re-reading tokens, so a
    ///      retained header operand's `!` behaves exactly like a built operand's.
    pub(crate) suffix: DeferredExpressionSuffix,
}

/// Authored TIR position of one retained numeric intake.
///
/// WHAT: identifies the payload-free position node the completed value must be installed into.
/// WHY: completion runs after the statement's TIR payloads were recorded, so the store must
///      remember the position instead of searching the graph again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NumericTemplateIntakePosition {
    /// The head-origin expression splice of one recorded `RetainedNumericIntake` node.
    HeadExpression(TemplateIrNodeId),

    /// The retained boolean condition of one recorded `RetainedNumericIntake` node that completes
    /// into a conditional `loop` node.
    ConditionalLoopCondition(TemplateIrNodeId),

    /// The retained selector of one recorded conditional position that completes into a
    /// `Conditional` node.
    ConditionalSelector(TemplateIrNodeId),

    /// The retained start operand of one recorded range `loop` position.
    LoopRangeStart(TemplateIrNodeId),

    /// The retained end operand of one recorded range `loop` position.
    LoopRangeEnd(TemplateIrNodeId),

    /// The retained step operand of one recorded range `loop` position.
    LoopRangeStep(TemplateIrNodeId),

    /// The retained iterable of one recorded collection `loop` position.
    ///
    /// WHY: a known-concrete callee's returned collection can still be reached through an
    ///      expression whose numeric origin is selected at the body root, so the iterable keeps its
    ///      authored fragment until then and the loop node installs at this position afterwards.
    LoopCollectionIterable(TemplateIrNodeId),
}

impl NumericTemplateIntakePosition {
    /// Returns the recorded node this position belongs to.
    pub(crate) fn node_id(self) -> TemplateIrNodeId {
        match self {
            Self::HeadExpression(node_id)
            | Self::ConditionalLoopCondition(node_id)
            | Self::ConditionalSelector(node_id)
            | Self::LoopRangeStart(node_id)
            | Self::LoopRangeEnd(node_id)
            | Self::LoopRangeStep(node_id)
            | Self::LoopCollectionIterable(node_id) => node_id,
        }
    }

    /// Returns this position retargeted onto another recorded node.
    ///
    /// WHY: a copied node duplicates the payload of its source, so the matching destination
    ///      position is the same position on the copied node.
    pub(crate) fn with_node_id(self, node_id: TemplateIrNodeId) -> Self {
        match self {
            Self::HeadExpression(_) => Self::HeadExpression(node_id),
            Self::ConditionalLoopCondition(_) => Self::ConditionalLoopCondition(node_id),
            Self::ConditionalSelector(_) => Self::ConditionalSelector(node_id),
            Self::LoopRangeStart(_) => Self::LoopRangeStart(node_id),
            Self::LoopRangeEnd(_) => Self::LoopRangeEnd(node_id),
            Self::LoopRangeStep(_) => Self::LoopRangeStep(node_id),
            Self::LoopCollectionIterable(_) => Self::LoopCollectionIterable(node_id),
        }
    }

    /// Whether this position is one operand of a retained range loop header.
    ///
    /// WHY: range operands do not install one by one: the owning body root assembles the range
    ///      spec from every operand and installs it through the range-header entry point.
    pub(crate) fn range_operand_slot(self) -> Option<RangeOperandSlot> {
        match self {
            Self::LoopRangeStart(_) => Some(RangeOperandSlot::Start),
            Self::LoopRangeEnd(_) => Some(RangeOperandSlot::End),
            Self::LoopRangeStep(_) => Some(RangeOperandSlot::Step),
            Self::HeadExpression(_)
            | Self::ConditionalLoopCondition(_)
            | Self::ConditionalSelector(_)
            | Self::LoopCollectionIterable(_) => None,
        }
    }
}

/// One operand slot of a retained range loop header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RangeOperandSlot {
    Start,
    End,
    Step,
}

/// Dense store row handle; never confused with the authored TIR node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RetainedRangeHeaderId(usize);

#[derive(Debug, Default, Clone, Copy)]
struct NumericNodeIndex {
    header: Option<RetainedRangeHeaderId>,
    sites: [Option<NumericTemplateIntakeSiteId>; 3],
    completion_group: Option<usize>,
    collection: Option<usize>,
    selector: Option<usize>,
}

/// Ordered handles for one counter. Installation advances the head without revisiting rows.
#[derive(Debug, Default)]
struct PendingCounterHeaders {
    headers: Vec<RetainedRangeHeaderId>,
    next: usize,
}

/// Registration-time indexes only; the site and header tables remain lifecycle owners.
///
/// WHY: bodies share a module store, so completion and copying must visit only their own rows.
/// Node IDs and body-local counter IDs address dense indexes; only domain identity needs hashing.
#[derive(Debug, Default)]
pub(crate) struct NumericTemplateIntakeIndex {
    domains: HashMap<NumericConstructionDomainId, Vec<NumericTemplateIntakeSiteId>>,
    nodes: Vec<NumericNodeIndex>,
    counters: HashMap<NumericConstructionDomainId, Vec<PendingCounterHeaders>>,
}

impl NumericTemplateIntakeIndex {
    fn node_mut(&mut self, node: TemplateIrNodeId) -> &mut NumericNodeIndex {
        if self.nodes.len() <= node.index() {
            self.nodes
                .resize(node.index() + 1, NumericNodeIndex::default());
        }
        &mut self.nodes[node.index()]
    }

    fn register_counter(
        &mut self,
        domain: NumericConstructionDomainId,
        counter: NumericBindingId,
        header: RetainedRangeHeaderId,
    ) {
        let counters = self.counters.entry(domain).or_default();
        if counters.len() <= counter.as_usize() {
            counters.resize_with(counter.as_usize() + 1, PendingCounterHeaders::default);
        }
        counters[counter.as_usize()].headers.push(header);
    }
}

impl RangeOperandSlot {
    fn index(self) -> usize {
        match self {
            Self::Start => 0,
            Self::End => 1,
            Self::Step => 2,
        }
    }
}

/// Prepared body and wrapper roots for one retained range loop.
#[derive(Clone, Copy)]
struct RetainedRangeRenderData {
    header_sites: TemplateLoopHeaderExpressionSites,
    body: TemplateIrNodeId,
    aggregate_wrapper: TemplateIrNodeId,
}

/// Lifecycle of one intake record.
///
/// WHAT: retained until `take_pending_numeric_intakes` moves the items out, then installed.
/// WHY: keeps `take` idempotent while the authored position survives for installation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NumericTemplateIntakeState {
    Retained,
    Taken,
    Installed,
}

/// One retained range loop header whose operands complete at the owning body root.
///
/// WHAT: the parse-time loop bindings (whose counter domain was already selected from the built
///       operands and the authored family of the retained ones), the authored end kind, and the
///       operands that were already built at parse time.
/// WHY: the position node must stay payload-free, but the finished `RangeLoopSpec` cannot be
///      assembled without the parse-time bindings, the end kind and the built operands. Keeping
///      them in the store's scratch table preserves the known iterator/positional types without
///      fabricating a default witness or a placeholder type.
#[derive(Debug)]
pub(crate) struct RetainedRangeHeaderRecord {
    node: TemplateIrNodeId,
    domain: NumericConstructionDomainId,
    operand_sites: [Option<NumericTemplateIntakeSiteId>; 3],
    /// How this header completes; taken once, when the root installs the concrete header.
    completion: Option<RetainedRangeCompletion>,
    state: NumericTemplateIntakeState,
}

/// How one retained range loop header completes.
///
/// WHAT: either the header's own operand slots or a retained counter row that owns them.
/// WHY: an unbound header completes each authored operand through template intake records and the
///      shared range policy; a bound header's operands belong to its counter's recipe, which the
///      driver's binding pass resolves before the loop construct, so the finished spec and the
///      counter declaration arrive together instead of re-running the policy.
#[derive(Debug, Clone)]
enum RetainedRangeCompletion {
    /// The header owns its operand slots; each retained operand has its own intake record.
    ///
    /// WHY: header rows are store-resident in one dense `Vec`, so the wide built operands are boxed
    /// to keep every row small; the box costs one parse-time allocation per retained range header.
    OperandSlots {
        /// Parse-time loop bindings for the header (empty when the author declared none).
        bindings: Box<LoopBindings>,
        end_kind: RangeEndKind,
        /// Authored site of an omitted start operand, used by the shared range policy for the
        /// implicit start.
        omitted_start_span: Option<SourceSpan>,
        /// Operands already built at parse time, indexed by `RangeOperandSlot` (Start, End, Step);
        /// retained operands arrive through their records.
        built_operands: Box<[Option<ExprId>; 3]>,
    },

    /// A retained `|counter|` binding owns the operands and the forward domain.
    RetainedCounter {
        counter: NumericBindingId,
        index: Option<Box<Declaration>>,
    },
}

/// One store-owned option-present capture pattern of a retained conditional selector.
///
/// WHAT: the parsed pattern of a branch whose option subject still awaits its selected type.
/// WHY: the conditional node only exists once the subject completes, so the pattern waits here
///      instead of inside TIR; installing it keeps the authored capture shape exactly.
#[derive(Debug)]
pub(crate) struct RetainedSelectorRecord {
    pattern: Option<Box<DeferredArmPattern>>,
    state: NumericTemplateIntakeState,
}

/// One store-owned concrete binding row for a retained collection loop header.
///
/// WHAT: the loop binding declarations, resolved at parse time from the callee's known return
///       metadata, kept beside the position until its iterable completes.
/// WHY: the header's bindings are final the moment the iterable is retained, so the store keeps
///      them instead of fabricating an AST payload or asking completion to re-derive the element
///      metadata from a fragment.
#[derive(Debug)]
pub(crate) struct RetainedCollectionHeaderRecord {
    bindings: Option<Box<LoopBindings>>,
    pending_item: Option<NumericBindingId>,
    state: NumericTemplateIntakeState,
}

/// One store-owned intake record, indexed by its dense site.
#[derive(Debug)]
pub(crate) struct NumericTemplateIntakeRecord {
    domain: NumericConstructionDomainId,
    position: NumericTemplateIntakePosition,
    items: Vec<ExpressionRpnItem>,
    expected: ExpectedType,
    value_mode: ValueMode,
    suffix: DeferredExpressionSuffix,
    state: NumericTemplateIntakeState,
}

impl NumericTemplateIntakeRecord {
    fn new(
        domain: NumericConstructionDomainId,
        position: NumericTemplateIntakePosition,
        items: Vec<ExpressionRpnItem>,
        expected: ExpectedType,
        value_mode: ValueMode,
        suffix: DeferredExpressionSuffix,
    ) -> Self {
        Self {
            domain,
            position,
            items,
            expected,
            value_mode,
            suffix,
            state: NumericTemplateIntakeState::Retained,
        }
    }
}

/// Whether one RPN item is a construction-only pending numeric item.
///
/// WHAT: the retained-intake marker; `PendingNumericLiteral` is a broken-invariant item at this
///       boundary and is deliberately not treated as retained construction. Group markers are
///       never pending, so a flat scan of a fragment reaches each pending item inside a group.
/// WHY: one predicate keeps the store and the template boundaries agreeing on what a retained
///      numeric recipe is.
/// NOTE: a bound range header keeps its operands inside its counter row and registers no operand
///       records, so it is pending through its retained position node and store row rather than
///       through an item in a TIR payload.
pub(crate) fn is_pending_numeric_intake_item(item: &ExpressionRpnItem) -> bool {
    matches!(
        item,
        ExpressionRpnItem::PendingNumericBinding { .. }
            | ExpressionRpnItem::PendingNumericCast { .. }
            | ExpressionRpnItem::PendingNumericCall { .. }
            | ExpressionRpnItem::PendingNumericExpression { .. }
    )
}

impl TemplateIrStore {
    /// Registers one retained numeric intake for a payload-free TIR position.
    ///
    /// WHAT: takes the owning numeric body domain, the authored position, the retained fragment
    ///       and the receiving metadata, then issues the next dense site ID.
    /// WHY: registration happens once, when the template boundary retains the fragment, so body
    ///      completion never has to reinterpret the expression tree. The domain key keeps
    ///      body-local numeric binding IDs from crossing into a sibling body's completion.
    pub(crate) fn register_pending_numeric_template_intake(
        &mut self,
        domain: NumericConstructionDomainId,
        position: NumericTemplateIntakePosition,
        items: Vec<ExpressionRpnItem>,
        expected: ExpectedType,
        value_mode: ValueMode,
        suffix: DeferredExpressionSuffix,
    ) -> Result<NumericTemplateIntakeSiteId, InfrastructureFailure> {
        debug_assert!(
            items.iter().any(is_pending_numeric_intake_item),
            "numeric template intake registration requires at least one pending numeric item"
        );

        let slot = position
            .range_operand_slot()
            .map_or(0, RangeOperandSlot::index);
        if self
            .numeric_intake_index
            .nodes
            .get(position.node_id().index())
            .is_some_and(|node| node.sites[slot].is_some())
        {
            return Err(InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                format!("numeric template intake position {position:?} was registered twice."),
            ));
        }

        let site = self.next_numeric_template_intake_site_id();
        self.numeric_template_intakes
            .push(NumericTemplateIntakeRecord::new(
                domain, position, items, expected, value_mode, suffix,
            ));
        self.numeric_intake_index
            .domains
            .entry(domain)
            .or_default()
            .push(site);
        let node = self.numeric_intake_index.node_mut(position.node_id());
        node.sites[slot] = Some(site);
        Ok(site)
    }

    /// Registers one retained range loop header whose operands complete at the owning body root.
    ///
    /// WHAT: appends the store-owned header row holding the parse-time bindings, end kind and
    ///       already built operands; retained operands register separately through
    ///       `register_pending_numeric_template_intake` with their own position slots.
    /// WHY: the range spec can only be assembled once every operand's type is selected, and the
    ///      spec assembly uses the producer's shared range policy, so the parse-time data waits in
    ///      store scratch instead of inside a TIR node.
    pub(crate) fn register_retained_range_header_operand_slots(
        &mut self,
        domain: NumericConstructionDomainId,
        node: TemplateIrNodeId,
        bindings: Box<LoopBindings>,
        end_kind: RangeEndKind,
        omitted_start_span: Option<SourceSpan>,
        built_operands: [Option<ExprId>; 3],
    ) {
        let id = RetainedRangeHeaderId(self.retained_range_headers.len());
        let range = self.numeric_intake_index.node_mut(node);
        range.header = Some(id);
        let operand_sites = range.sites;
        self.retained_range_headers.push(RetainedRangeHeaderRecord {
            node,
            domain,
            operand_sites,
            completion: Some(RetainedRangeCompletion::OperandSlots {
                bindings,
                end_kind,
                omitted_start_span,
                built_operands: Box::new(built_operands),
            }),
            state: NumericTemplateIntakeState::Retained,
        });
    }

    /// Registers one retained range loop header whose operands belong to a retained counter.
    ///
    /// WHAT: appends the store-owned header row holding the counter row's binding id and the
    ///       concrete positional index binding, if the author declared one.
    /// WHY: the counter's recipe owns the operands and computes the header's forward domain during
    ///      the driver's binding pass, so this position needs no operand records of its own.
    pub(crate) fn register_retained_range_header_counter(
        &mut self,
        domain: NumericConstructionDomainId,
        node: TemplateIrNodeId,
        counter: NumericBindingId,
        index: Option<Declaration>,
    ) {
        let id = RetainedRangeHeaderId(self.retained_range_headers.len());
        self.numeric_intake_index.node_mut(node).header = Some(id);
        self.numeric_intake_index
            .register_counter(domain, counter, id);
        self.retained_range_headers.push(RetainedRangeHeaderRecord {
            node,
            operand_sites: [None, None, None],
            domain,
            completion: Some(RetainedRangeCompletion::RetainedCounter {
                counter,
                index: index.map(Box::new),
            }),
            state: NumericTemplateIntakeState::Retained,
        });
    }

    /// Registers the parsed option-present capture pattern of one retained selector.
    ///
    /// WHY: the pattern is authored AST data that the conditional node needs at completion; keeping
    ///      it in store scratch keeps the position itself payload-free.
    pub(crate) fn register_retained_selector_pattern(
        &mut self,
        node: TemplateIrNodeId,
        pattern: Box<DeferredArmPattern>,
    ) {
        self.numeric_intake_index.node_mut(node).selector =
            Some(self.retained_selector_patterns.len());
        self.retained_selector_patterns
            .push(RetainedSelectorRecord {
                pattern: Some(pattern),
                state: NumericTemplateIntakeState::Retained,
            });
    }

    /// Registers the concrete bindings of one retained collection loop header.
    ///
    /// WHY: the element metadata is already final when the iterable is retained, and the completed
    ///      `Loop` node needs those exact bindings, so they wait in store scratch. A pending numeric
    ///      collection item has no declaration until the body root completes its source, so the
    ///      header keeps its id and receives a copy through `install_collection_item`.
    pub(crate) fn register_retained_collection_header(
        &mut self,
        node: TemplateIrNodeId,
        bindings: Box<LoopBindings>,
        pending_item: Option<NumericBindingId>,
    ) {
        self.numeric_intake_index.node_mut(node).collection =
            Some(self.retained_collection_headers.len());
        self.retained_collection_headers
            .push(RetainedCollectionHeaderRecord {
                bindings: Some(bindings),
                pending_item,
                state: NumericTemplateIntakeState::Retained,
            });
    }

    /// Returns the pending numeric item of one retained collection header awaiting completion.
    pub(crate) fn pending_collection_item(
        &self,
        node: TemplateIrNodeId,
    ) -> Option<NumericBindingId> {
        let index = self
            .numeric_intake_index
            .nodes
            .get(node.index())?
            .collection?;
        let record = &self.retained_collection_headers[index];
        (record.state == NumericTemplateIntakeState::Retained)
            .then_some(record.pending_item)
            .flatten()
    }

    /// Installs the completed declaration of a pending collection item into its retained header.
    ///
    /// WHAT: fills the header's bindings with the item declared from the source's completed element
    ///       type, exactly once.
    /// WHY: a pending item cannot be declared at parse time, and the body root completes it before
    ///      the header's iterable installs.
    pub(crate) fn install_collection_item(
        &mut self,
        node: TemplateIrNodeId,
        item: Declaration,
    ) -> Result<(), InfrastructureFailure> {
        let record = self
            .numeric_intake_index
            .nodes
            .get(node.index())
            .and_then(|node| node.collection)
            .map(|index| &mut self.retained_collection_headers[index])
            .filter(|record| record.state == NumericTemplateIntakeState::Retained)
            .ok_or_else(|| {
                InfrastructureFailure::internal(
                    CompilerStage::AstConstruction,
                    format!("no retained collection header at {node:?} awaits a pending item."),
                )
            })?;
        let bindings = record.bindings.as_mut().ok_or_else(|| {
            InfrastructureFailure::internal(CompilerStage::AstConstruction, format!(
                "retained collection header at {node:?} lost its bindings before its pending item."
            ))
        })?;
        if bindings.item.is_some() {
            return Err(InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                format!("retained collection header at {node:?} received its pending item twice."),
            ));
        }
        bindings.item = Some(item);
        Ok(())
    }

    /// Takes the concrete bindings of one completing retained collection header, once.
    ///
    /// WHY: a pending item must already be installed; completing the header without one would
    ///      build a `Loop` with no item.
    fn pending_collection_bindings(
        &mut self,
        node: TemplateIrNodeId,
    ) -> Result<Option<Box<LoopBindings>>, InfrastructureFailure> {
        let Some(record) = self
            .numeric_intake_index
            .nodes
            .get(node.index())
            .and_then(|node| node.collection)
            .map(|index| &mut self.retained_collection_headers[index])
            .filter(|record| record.state == NumericTemplateIntakeState::Retained)
        else {
            return Ok(None);
        };
        let Some(bindings) = record.bindings.take() else {
            return Ok(None);
        };
        if record.pending_item.is_some() && bindings.item.is_none() {
            return Err(InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                format!(
                    "collection loop header at {node:?} completed before its pending item was installed."
                ),
            ));
        }
        record.state = NumericTemplateIntakeState::Installed;
        Ok(Some(bindings))
    }

    /// Stores a render-unit shape prepared for a retained control-flow position.
    ///
    /// WHAT: updates the position's existing body root and carries the shape-specific loop
    ///       wrapper, if any, until the retained header completes.
    /// WHY: style and body structure are already owned at parser time, so completion installs
    ///      those exact roots without replaying the owning template.
    pub(crate) fn record_retained_control_flow_render_unit(
        &mut self,
        node_id: TemplateIrNodeId,
        body_root: TemplateIrNodeId,
        render_unit: RetainedControlFlowRenderUnit,
    ) -> Result<(), InfrastructureFailure> {
        if self.get_node(body_root).is_none() {
            return Err(InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                format!(
                    "retained control-flow position {node_id} received missing render body {body_root}."
                ),
            ));
        }

        let aggregate_wrapper = match render_unit {
            RetainedControlFlowRenderUnit::Conditional => None,
            RetainedControlFlowRenderUnit::Loop { aggregate_wrapper } => Some(aggregate_wrapper),
        };
        if aggregate_wrapper.is_some_and(|wrapper| self.get_node(wrapper).is_none()) {
            return Err(InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                format!(
                    "retained control-flow position {node_id} received a missing aggregate wrapper."
                ),
            ));
        }

        let node = self.node_mut(node_id)?;
        let (body, stored_render_unit, needs_loop) = match &mut node.kind {
            TemplateIrNodeKind::RetainedNumericIntake(
                RetainedNumericIntakeRole::ConditionalLoopCondition {
                    body, render_unit, ..
                },
            )
            | TemplateIrNodeKind::RetainedNumericIntake(RetainedNumericIntakeRole::LoopRange {
                body,
                render_unit,
                ..
            })
            | TemplateIrNodeKind::RetainedNumericIntake(
                RetainedNumericIntakeRole::LoopCollection {
                    body, render_unit, ..
                },
            ) => (body, render_unit, true),
            TemplateIrNodeKind::RetainedNumericIntake(
                RetainedNumericIntakeRole::ConditionalSelector {
                    body, render_unit, ..
                },
            ) => (body, render_unit, false),
            _ => {
                return Err(InfrastructureFailure::internal(
                    CompilerStage::AstConstruction,
                    format!(
                        "render-unit preparation targeted non-control-flow retained position {node_id}."
                    ),
                ));
            }
        };

        if stored_render_unit.is_some() {
            return Err(InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                format!(
                    "retained control-flow position {node_id} received render-unit preparation twice."
                ),
            ));
        }

        if needs_loop != matches!(render_unit, RetainedControlFlowRenderUnit::Loop { .. }) {
            return Err(InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                format!(
                    "retained control-flow position {node_id} received a render unit with the wrong shape."
                ),
            ));
        }

        *body = body_root;
        *stored_render_unit = Some(render_unit);
        Ok(())
    }

    /// Takes the deferred capture pattern of one retained selector that is completing.
    ///
    /// WHAT: returns the pattern once and moves its record to `Taken`.
    /// WHY: the pattern finishes against the completed subject's type in the body root, outside
    ///      the store borrow, so the store does not hold it past its selector's completion.
    pub(crate) fn take_pending_selector_pattern(
        &mut self,
        node: TemplateIrNodeId,
    ) -> Option<Box<DeferredArmPattern>> {
        let index = self
            .numeric_intake_index
            .nodes
            .get(node.index())?
            .selector?;
        let record = &mut self.retained_selector_patterns[index];
        if record.state != NumericTemplateIntakeState::Retained {
            return None;
        }
        let pattern = record.pattern.take()?;
        record.state = NumericTemplateIntakeState::Taken;
        Some(pattern)
    }

    /// The retained template position waiting on one range counter in a body domain, when the
    /// counter's header came from a template suffix.
    ///
    /// WHY: binding IDs are dense and body-local while the TIR store is module-shared, so a
    ///      resolved counter may install only headers from its issuing body. A copied header
    ///      retains that same domain and still installs through this lookup.
    pub(crate) fn pending_range_header_node_for_counter(
        &self,
        domain: NumericConstructionDomainId,
        counter: NumericBindingId,
    ) -> Option<TemplateIrNodeId> {
        let queue = self
            .numeric_intake_index
            .counters
            .get(&domain)?
            .get(counter.as_usize())?;
        let header = *queue.headers.get(queue.next)?;
        increment_ast_counter(AstCounter::TirNumericCounterHeadersInspected);
        Some(self.retained_range_headers[header.0].node)
    }

    /// Moves one numeric body domain's retained intakes out for body-root completion.
    ///
    /// WHAT: drains the domain's ordered site queue and clears only those retained records' items;
    ///       records owned by another domain and already drained queues are never revisited.
    /// WHY: numeric binding IDs are body-local, so a body root must never complete a sibling
    ///      body's retained intake (an outer pending head can outlive a nested local function).
    pub(crate) fn take_pending_numeric_intakes(
        &mut self,
        domain: NumericConstructionDomainId,
    ) -> Vec<PendingNumericTemplateIntake> {
        let sites = self
            .numeric_intake_index
            .domains
            .remove(&domain)
            .unwrap_or_default();
        let mut taken = Vec::with_capacity(sites.len());
        let mut next_group = 0;

        for site in sites {
            let record = &mut self.numeric_template_intakes[site.index()];
            if record.state != NumericTemplateIntakeState::Retained {
                continue;
            }
            let range_header = record.position.range_operand_slot().map(|_| {
                let range = &mut self.numeric_intake_index.nodes[record.position.node_id().index()];
                let group = *range.completion_group.get_or_insert_with(|| {
                    let group = next_group;
                    next_group += 1;
                    group
                });
                (range.header.expect("registered range header"), group)
            });

            taken.push(PendingNumericTemplateIntake {
                site,
                range_header,
                position: record.position,
                items: std::mem::take(&mut record.items),
                expected: record.expected,
                value_mode: record.value_mode,
                suffix: record.suffix,
            });
            record.state = NumericTemplateIntakeState::Taken;
        }

        taken
    }

    /// Puts the completed value back in its authored intake position.
    ///
    /// WHAT: replaces the payload-free position with the completed value and closes the intake.
    ///       An option-present selector also takes its finished capture pattern.
    /// WHY: the TIR position must carry the ordinary intake result, never a pending recipe or a
    ///      typed placeholder.
    pub(crate) fn install_completed_numeric_intake(
        &mut self,
        site: NumericTemplateIntakeSiteId,
        value: ExprId,
        selector_pattern: Option<MatchPattern>,
    ) -> Result<(), InfrastructureFailure> {
        let Some(record) = self.numeric_template_intakes.get(site.index()) else {
            return Err(InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                format!("TIR store has no numeric template intake site {site}."),
            ));
        };

        match record.state {
            NumericTemplateIntakeState::Taken => {}
            NumericTemplateIntakeState::Retained => {
                return Err(InfrastructureFailure::internal(
                    CompilerStage::AstConstruction,
                    format!(
                        "numeric template intake site {site} was not taken before installation."
                    ),
                ));
            }
            NumericTemplateIntakeState::Installed => {
                return Err(InfrastructureFailure::internal(
                    CompilerStage::AstConstruction,
                    format!("numeric template intake site {site} was installed twice."),
                ));
            }
        }

        let position = record.position;
        self.install_at_numeric_intake_position(site, position, value, selector_pattern)?;

        self.numeric_template_intakes[site.index()].state = NumericTemplateIntakeState::Installed;
        Ok(())
    }

    /// Writes one completed value into the payload shape its position recorded.
    fn install_at_numeric_intake_position(
        &mut self,
        site: NumericTemplateIntakeSiteId,
        position: NumericTemplateIntakePosition,
        value: ExprId,
        selector_pattern: Option<MatchPattern>,
    ) -> Result<(), InfrastructureFailure> {
        let node_id = position.node_id();

        let role = match self.get_node(node_id) {
            Some(node) => match &node.kind {
                TemplateIrNodeKind::RetainedNumericIntake(role) => *role,
                _ => {
                    return Err(InfrastructureFailure::internal(
                        CompilerStage::AstConstruction,
                        format!(
                            "numeric template intake site {site} no longer matches its recorded \
                         position."
                        ),
                    ));
                }
            },
            None => {
                return Err(InfrastructureFailure::internal(
                    CompilerStage::AstConstruction,
                    format!(
                        "numeric template intake site {site} names a position node that is not in the \
                     store."
                    ),
                ));
            }
        };

        if let RetainedNumericIntakeRole::LoopRange { .. } = role {
            // Range operands do not install one by one: the owning body root assembles the range
            // spec from every operand through the shared range policy and installs it with
            // `install_completed_numeric_range_header`.
            return Err(InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                format!(
                    "numeric template intake site {site} is one operand of a retained range loop \
                 header, which installs through the range-header entry point."
                ),
            ));
        }
        let render_unit = match role {
            RetainedNumericIntakeRole::HeadExpression { .. }
            | RetainedNumericIntakeRole::LoopRange { .. } => None,
            role => Some(require_retained_control_flow_render_unit(role, node_id)?),
        };

        // An option-present selector installs with the capture pattern its body root finished
        // against the completed subject.
        let capture_pattern = match (role, selector_pattern) {
            (
                RetainedNumericIntakeRole::ConditionalSelector {
                    kind: TemplateBranchSelectorKind::OptionPresent,
                    ..
                },
                Some(pattern),
            ) => Some(Box::new(pattern)),
            (
                RetainedNumericIntakeRole::ConditionalSelector {
                    kind: TemplateBranchSelectorKind::OptionPresent,
                    ..
                },
                None,
            ) => {
                return Err(InfrastructureFailure::internal(
                    CompilerStage::AstConstruction,
                    format!(
                        "numeric template intake site {site} completed an option-present selector \
                     without its capture pattern."
                    ),
                ));
            }
            (_, Some(_)) => {
                return Err(InfrastructureFailure::internal(
                    CompilerStage::AstConstruction,
                    format!(
                        "numeric template intake site {site} received a capture pattern for a \
                     position that takes none."
                    ),
                ));
            }
            (_, None) => None,
        };

        // A retained collection header needs its concrete bindings, which the store owns.
        let collection_bindings = match role {
            RetainedNumericIntakeRole::LoopCollection { .. } => {
                let bindings = self.pending_collection_bindings(node_id)?.ok_or_else(|| {
                    InfrastructureFailure::internal(
                        CompilerStage::AstConstruction,
                        "collection loop iterable completion lost the header's concrete bindings.",
                    )
                })?;
                Some(bindings)
            }
            _ => None,
        };

        let completed_kind = match role {
            RetainedNumericIntakeRole::HeadExpression { expression_site } => {
                TemplateIrNodeKind::DynamicExpression {
                    expression: value,
                    origin: TemplateSegmentOrigin::Head,
                    site_id: expression_site,
                }
            }

            RetainedNumericIntakeRole::ConditionalLoopCondition {
                header_sites, body, ..
            } => {
                let aggregate_wrapper =
                    require_retained_loop_aggregate_wrapper(render_unit, node_id)?;
                TemplateIrNodeKind::Loop {
                    header: TemplateLoopHeader::Conditional { condition: value },
                    header_sites,
                    body,
                    aggregate_wrapper: Some(aggregate_wrapper),
                }
            }

            RetainedNumericIntakeRole::ConditionalSelector {
                selector_site_id,
                body,
                kind,
                ..
            } => {
                let selector = match (kind, capture_pattern) {
                    (TemplateBranchSelectorKind::Bool, _) => {
                        Box::new(TemplateBranchSelector::Bool(value))
                    }
                    (TemplateBranchSelectorKind::OptionPresent, Some(pattern)) => {
                        Box::new(TemplateBranchSelector::OptionPresentCapture {
                            scrutinee: value,
                            pattern,
                        })
                    }
                    (TemplateBranchSelectorKind::OptionPresent, None) => {
                        return Err(InfrastructureFailure::internal(
                            CompilerStage::AstConstruction,
                            format!(
                                "numeric template intake site {site} completed an option-present \
                             selector without its capture pattern."
                            ),
                        ));
                    }
                };

                TemplateIrNodeKind::Conditional {
                    selector,
                    selector_site_id,
                    body,
                }
            }

            RetainedNumericIntakeRole::LoopCollection {
                header_sites, body, ..
            } => {
                let aggregate_wrapper =
                    require_retained_loop_aggregate_wrapper(render_unit, node_id)?;
                let bindings = collection_bindings.ok_or_else(|| {
                    InfrastructureFailure::internal(CompilerStage::AstConstruction, format!(
                        "numeric template intake site {site} completed a collection loop iterable \
                         without its header bindings."
                    ))
                })?;

                TemplateIrNodeKind::Loop {
                    header: TemplateLoopHeader::Collection {
                        bindings,
                        iterable: value,
                    },
                    header_sites,
                    body,
                    aggregate_wrapper: Some(aggregate_wrapper),
                }
            }

            RetainedNumericIntakeRole::LoopRange { .. } => {
                return Err(InfrastructureFailure::internal(
                    CompilerStage::AstConstruction,
                    format!(
                        "numeric template intake site {site} is one operand of a retained range loop \
                     header, which installs through the range-header entry point."
                    ),
                ));
            }
        };

        self.node_mut(node_id)?.kind = completed_kind;
        Ok(())
    }

    /// Installs the finished range loop header at its retained position.
    ///
    /// WHAT: for an unbound header, merges the operands completed at the body root with the
    ///       operands that were already built at parse time, runs the producer's shared range
    ///       policy once, and writes the concrete `Loop` node into the payload-free position.
    /// WHY: a range header's operand types are selected at the body root, so the spec can only be
    ///      assembled there; the position stays payload-free until then, and no placeholder type,
    ///      fabricated default or skipped operand is involved.
    /// Errors: an authored range policy violation (missing float step, zero step) surfaces as the
    ///         producer's diagnostic; a position without its retained header row, a bound header
    ///         installed through this entry point, or a missing operand is a broken invariant.
    #[allow(
        clippy::too_many_arguments,
        reason = "the header id and completed operands are extra inputs beside the parser borrows"
    )]
    pub(crate) fn install_completed_numeric_range_header(
        &mut self,
        header_id: RetainedRangeHeaderId,
        completed_operands: [Option<ExprId>; 3],
        context: &ScopeContext,
        type_interner: &mut AstTypeInterner<'_>,
        expression_builder: &mut AstExpressionBuilder,
        const_values: &mut ConstValueStore,
        string_table: &mut StringTable,
        path_fork: &mut PathInternerFork,
    ) -> Result<(), TemplateError> {
        // Operands arrive in `RangeOperandSlot` order: Start, End, Step.
        let [completed_start, completed_end, completed_step] = completed_operands;
        let header_index = header_id.0;
        let node_id = self.retained_range_headers
            .get(header_index)
            .ok_or_else(|| {
                InfrastructureFailure::internal(CompilerStage::AstConstruction,
                    "TIR range-header completion targeted a position without a retained header row.",
                )
            })?
            .node;
        let operand_sites = self.retained_range_headers[header_index].operand_sites;
        let render_data = self.range_header_render_data(node_id)?;

        // Every retained operand completes exactly once; otherwise a missing operand could
        // silently turn into an omitted one (a miscompiled range) instead of failing loudly.
        for (slot, site) in operand_sites.iter().enumerate() {
            if site.is_none() {
                continue;
            }

            let completed = match slot {
                0 => completed_start.is_some(),
                1 => completed_end.is_some(),
                _ => completed_step.is_some(),
            };

            if !completed {
                return Err(InfrastructureFailure::internal(
                    CompilerStage::AstConstruction,
                    "TIR range-header completion installed a header whose retained operand was \
                     never completed.",
                )
                .into());
            }
        }

        // Take the parse-time header data. The borrow ends before the node write below.
        let (end_kind, omitted_start_span, start, end, step, bindings) = {
            let header = &mut self.retained_range_headers[header_index];

            if header.state != NumericTemplateIntakeState::Retained {
                return Err(InfrastructureFailure::internal(
                    CompilerStage::AstConstruction,
                    "TIR range-header completion installed a retained range loop header twice.",
                )
                .into());
            }

            let Some(RetainedRangeCompletion::OperandSlots {
                bindings,
                end_kind,
                omitted_start_span,
                built_operands,
            }) = header.completion.take()
            else {
                return Err(InfrastructureFailure::internal(
                    CompilerStage::AstConstruction,
                    "TIR range-header completion reached a bound range header through the operand \
                     entry point; a bound header installs with its finished spec and counter \
                     declaration.",
                )
                .into());
            };

            let [built_start, built_end, built_step] = *built_operands;
            let start = completed_start.or(built_start);
            let end = completed_end.or(built_end).ok_or_else(|| {
                InfrastructureFailure::internal(
                    CompilerStage::AstConstruction,
                    "TIR range-header completion reached a range loop header without an end \
                     operand.",
                )
            })?;
            let step = completed_step.or(built_step);

            (end_kind, omitted_start_span, start, end, step, bindings)
        };

        let range = range_loop_spec_from_completed_operands(
            start.map(|id| expression_builder.own(id)),
            expression_builder.own(end),
            end_kind,
            step.map(|id| expression_builder.own(id)),
            omitted_start_span,
            context,
            type_interner,
            expression_builder,
            const_values,
            string_table,
            path_fork,
        )
        .map_err(TemplateError::from)?;

        self.node_mut(node_id)?.kind = TemplateIrNodeKind::Loop {
            header: TemplateLoopHeader::Range {
                bindings,
                range: Box::new(range),
            },
            header_sites: render_data.header_sites,
            body: render_data.body,
            aggregate_wrapper: Some(render_data.aggregate_wrapper),
        };

        self.retained_range_headers[header_index].state = NumericTemplateIntakeState::Installed;

        // Every operand record of this header was taken before the header installed; close them so
        // a late operand completion cannot rewrite an already concrete header.
        for site in operand_sites.into_iter().flatten() {
            let record = &mut self.numeric_template_intakes[site.index()];

            match record.state {
                NumericTemplateIntakeState::Taken => {
                    record.state = NumericTemplateIntakeState::Installed;
                }
                NumericTemplateIntakeState::Retained => {
                    return Err(InfrastructureFailure::internal(
                        CompilerStage::AstConstruction,
                        "TIR range-header completion installed a header whose operand was never \
                         taken for completion.",
                    )
                    .into());
                }
                NumericTemplateIntakeState::Installed => {}
            }
        }

        Ok(())
    }

    /// Installs a bound range header with the counter's finished spec and declaration.
    ///
    /// WHAT: takes the retained positional index binding from the header row, pairs the finished
    ///       spec with the counter's completed declaration as the loop's iterator binding, and
    ///       writes the concrete `Loop` node into the payload-free position.
    /// WHY: a bound header's operands belong to its counter row, whose recipe already ran the
    ///      shared range policy exactly once during the driver's binding pass, so this entry point
    ///      installs the result instead of re-running the policy or re-reading operands.
    pub(crate) fn install_completed_numeric_range_header_with_counter(
        &mut self,
        node_id: TemplateIrNodeId,
        range: RangeLoopSpec,
        counter_declaration: Declaration,
    ) -> Result<(), TemplateError> {
        let header_index = self
            .numeric_intake_index
            .nodes
            .get(node_id.index())
            .and_then(|node| node.header)
            .map(|id| id.0)
            .ok_or_else(|| {
                InfrastructureFailure::internal(
                    CompilerStage::AstConstruction,
                    "TIR bound range-header completion targeted a position without a retained \
                     header row.",
                )
            })?;
        let render_data = self.range_header_render_data(node_id)?;

        let index_declaration = {
            let header = &mut self.retained_range_headers[header_index];

            if header.state != NumericTemplateIntakeState::Retained {
                return Err(InfrastructureFailure::internal(
                    CompilerStage::AstConstruction,
                    "TIR bound range-header completion installed a retained range loop header \
                     twice.",
                )
                .into());
            }

            let Some(RetainedRangeCompletion::RetainedCounter { counter, index }) =
                header.completion.take()
            else {
                return Err(InfrastructureFailure::internal(
                    CompilerStage::AstConstruction,
                    "TIR bound range-header completion reached a header that owns its own operand \
                     slots.",
                )
                .into());
            };

            let queue = &mut self
                .numeric_intake_index
                .counters
                .get_mut(&header.domain)
                .expect("registered counter domain")[counter.as_usize()];
            // The completion driver installs the ordered pending head, including copied headers.
            debug_assert_eq!(queue.headers[queue.next].0, header_index);
            queue.next += 1;
            index.map(|index| *index)
        };

        // A bound header's step operand only becomes known with the finished spec, so its site is
        // allocated here when the counter's recipe produced one.
        let header_sites = match (render_data.header_sites, range.step.is_some()) {
            (
                TemplateLoopHeaderExpressionSites::Range {
                    start,
                    end,
                    step: None,
                },
                true,
            ) => TemplateLoopHeaderExpressionSites::Range {
                start,
                end,
                step: Some(self.next_expression_site_id()),
            },
            (header_sites, _) => header_sites,
        };

        let bindings = LoopBindings {
            item: Some(counter_declaration),
            index: index_declaration,
        };
        self.node_mut(node_id)?.kind = TemplateIrNodeKind::Loop {
            header: TemplateLoopHeader::Range {
                bindings: Box::new(bindings),
                range: Box::new(range),
            },
            header_sites,
            body: render_data.body,
            aggregate_wrapper: Some(render_data.aggregate_wrapper),
        };

        self.retained_range_headers[header_index].state = NumericTemplateIntakeState::Installed;

        Ok(())
    }

    /// Reads the header sites, prepared body and aggregate wrapper of a retained range position.
    fn range_header_render_data(
        &self,
        node_id: TemplateIrNodeId,
    ) -> Result<RetainedRangeRenderData, InfrastructureFailure> {
        let role = self
            .retained_numeric_intake_role(node_id)
            .copied()
            .ok_or_else(|| {
                InfrastructureFailure::internal(
                    CompilerStage::AstConstruction,
                    "TIR range-header completion found a position that is no longer retained.",
                )
            })?;

        match role {
            RetainedNumericIntakeRole::LoopRange {
                header_sites, body, ..
            } => Ok(RetainedRangeRenderData {
                header_sites,
                body,
                aggregate_wrapper: require_retained_loop_aggregate_wrapper(
                    Some(require_retained_control_flow_render_unit(role, node_id)?),
                    node_id,
                )?,
            }),
            _ => Err(InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                "TIR range-header completion found a retained position that is not a range header.",
            )),
        }
    }

    /// Replicates every retained numeric intake inside one copied TIR node.
    ///
    /// WHAT: registers one fresh intake for the copy of every record whose position names the
    ///       source node.
    /// WHY: a runtime slot plan copy taken before body-root completion duplicates a retained
    ///      position, and both copies must complete through the ordinary intake. A copy is its
    ///      own authored position, so it must not reuse the source's intake; the copy machinery
    ///      re-mints the position's own expression sites when it duplicates the node.
    /// NOTE: a copied bound range header shares its source's counter (the same declaration), and
    ///       the counter's finished `RangeLoopSpec` is take-once, so the copy installs through the
    ///       driver's lending accessor for that counter instead of re-taking the spec.
    /// Errors: a copied position whose source node has no intake record must own a pending bound
    ///         range counter header; otherwise the source position was never registered.
    ///
    pub(crate) fn replicate_pending_numeric_intakes_for_copied_node(
        &mut self,
        source_node_id: TemplateIrNodeId,
        copied_node_id: TemplateIrNodeId,
    ) -> Result<(), InfrastructureFailure> {
        if self.retained_numeric_intake_role(copied_node_id).is_none() {
            return Ok(());
        }

        let source = self
            .numeric_intake_index
            .nodes
            .get(source_node_id.index())
            .copied()
            .unwrap_or_default();
        let has_pending_retained_counter_header = source.header.is_some_and(|id| {
            let header = &self.retained_range_headers[id.0];
            header.state == NumericTemplateIntakeState::Retained
                && matches!(
                    header.completion,
                    Some(RetainedRangeCompletion::RetainedCounter { .. })
                )
        });

        if source.sites.iter().all(Option::is_none) && !has_pending_retained_counter_header {
            return Err(InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                "TIR copy duplicated a retained position whose source owns neither a numeric \
                 template intake record nor a pending retained-counter range header.",
            ));
        }

        for site in source.sites.into_iter().flatten() {
            increment_ast_counter(AstCounter::TirNumericCopyRowsInspected);
            let record = &self.numeric_template_intakes[site.index()];
            let (domain, position, items, expected, value_mode, suffix) = (
                record.domain,
                record.position,
                record.items.clone(),
                record.expected,
                record.value_mode,
                record.suffix,
            );
            self.register_pending_numeric_template_intake(
                domain,
                position.with_node_id(copied_node_id),
                items,
                expected,
                value_mode,
                suffix,
            )?;
        }

        // A copied collection header keeps its own concrete bindings, so the copy installs into its
        // own `Loop` node instead of the source's.
        if let Some(index) = source.collection {
            increment_ast_counter(AstCounter::TirNumericCopyRowsInspected);
            let source_row = &self.retained_collection_headers[index];
            let bindings = source_row.bindings.clone().ok_or_else(|| {
                InfrastructureFailure::internal(
                    CompilerStage::AstConstruction,
                    "TIR copy duplicated a collection loop header that was already installed.",
                )
            })?;
            let pending_item = source_row.pending_item;
            self.register_retained_collection_header(copied_node_id, bindings, pending_item);
        }

        // A copied range header keeps its own parse-time data, so the copy completes into its own
        // concrete `Loop` node instead of the source's.
        if let Some(header) = source.header {
            increment_ast_counter(AstCounter::TirNumericCopyRowsInspected);
            let source_header = &self.retained_range_headers[header.0];
            let domain = source_header.domain;
            let Some(completion) = source_header.completion.clone() else {
                return Err(InfrastructureFailure::internal(
                    CompilerStage::AstConstruction,
                    "TIR copy duplicated a range loop header that was already installed.",
                ));
            };
            let id = RetainedRangeHeaderId(self.retained_range_headers.len());
            if let RetainedRangeCompletion::RetainedCounter { counter, .. } = &completion {
                self.numeric_intake_index
                    .register_counter(domain, *counter, id);
            }
            let range = self.numeric_intake_index.node_mut(copied_node_id);
            range.header = Some(id);
            let operand_sites = range.sites;
            self.retained_range_headers.push(RetainedRangeHeaderRecord {
                node: copied_node_id,
                domain,
                operand_sites,
                completion: Some(completion),
                state: NumericTemplateIntakeState::Retained,
            });
        }

        // A copied selector keeps its own capture pattern, if it has one.
        if let Some(index) = source.selector {
            increment_ast_counter(AstCounter::TirNumericCopyRowsInspected);
            let source_pattern = &self.retained_selector_patterns[index];
            if source_pattern.state == NumericTemplateIntakeState::Retained
                && let Some(pattern) = source_pattern.pattern.as_ref()
            {
                self.register_retained_selector_pattern(copied_node_id, pattern.clone());
            }
        }

        Ok(())
    }

    /// The payload-free intake position one recorded node carries, when it carries one.
    fn retained_numeric_intake_role(
        &self,
        node_id: TemplateIrNodeId,
    ) -> Option<&RetainedNumericIntakeRole> {
        let node = self.get_node(node_id)?;
        match &node.kind {
            TemplateIrNodeKind::RetainedNumericIntake(role) => Some(role),
            _ => None,
        }
    }
}
fn require_retained_control_flow_render_unit(
    role: RetainedNumericIntakeRole,
    node_id: TemplateIrNodeId,
) -> Result<RetainedControlFlowRenderUnit, InfrastructureFailure> {
    let render_unit = match role {
        RetainedNumericIntakeRole::ConditionalLoopCondition { render_unit, .. }
        | RetainedNumericIntakeRole::LoopRange { render_unit, .. }
        | RetainedNumericIntakeRole::LoopCollection { render_unit, .. }
        | RetainedNumericIntakeRole::ConditionalSelector { render_unit, .. } => render_unit,
        RetainedNumericIntakeRole::HeadExpression { .. } => {
            return Err(InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                format!("non-control-flow retained position {node_id} requested a render unit."),
            ));
        }
    };

    let Some(render_unit) = render_unit else {
        return Err(InfrastructureFailure::internal(
            CompilerStage::AstConstruction,
            format!("retained control-flow position {node_id} has no prepared render unit."),
        ));
    };

    let expects_loop = !matches!(role, RetainedNumericIntakeRole::ConditionalSelector { .. });
    if expects_loop != matches!(render_unit, RetainedControlFlowRenderUnit::Loop { .. }) {
        return Err(InfrastructureFailure::internal(
            CompilerStage::AstConstruction,
            format!(
                "retained control-flow position {node_id} has a render unit with the wrong shape."
            ),
        ));
    }

    Ok(render_unit)
}

fn require_retained_loop_aggregate_wrapper(
    render_unit: Option<RetainedControlFlowRenderUnit>,
    node_id: TemplateIrNodeId,
) -> Result<TemplateIrNodeId, InfrastructureFailure> {
    match render_unit {
        Some(RetainedControlFlowRenderUnit::Loop { aggregate_wrapper }) => Ok(aggregate_wrapper),
        Some(RetainedControlFlowRenderUnit::Conditional) => Err(InfrastructureFailure::internal(
            CompilerStage::AstConstruction,
            format!("retained loop position {node_id} received a conditional render unit."),
        )),
        None => Err(InfrastructureFailure::internal(
            CompilerStage::AstConstruction,
            format!("retained loop position {node_id} has no prepared render unit."),
        )),
    }
}
