//! TIR-native formatter view.
//!
//! WHAT: feeds existing formatter algorithms from `TirView` and effective
//! `TemplateIrNodeKind` snapshots. Formatted output is mapped directly back to
//! append-only TIR nodes after view extraction finishes.
//!
//! WHY: removes the formatter-dependent representation ping-pong for compile-time
//! template folding while preserving formatter behavior. Existing formatter
//! algorithms (`$md`, `$raw`, etc.) stay unchanged; this module is only
//! the adapter that presents TIR data as `FormatterInput` and rebuilds TIR from
//! `FormatterOutput`.
//!
//! ## Production authority
//!
//! Linear templates and control-flow body roots format directly from TIR through
//! this adapter. Node-root formatting avoids scratch `TemplateIr` entries, and
//! no intermediate content-to-TIR conversion remains in the render-unit path.

use crate::compiler_frontend::ast::const_values::store::ConstStringPiece;
use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::templates::formatter_contract::{
    FormatterAnchorId, FormatterInput, FormatterInputPiece, FormatterOpaqueKind,
    FormatterOpaquePiece, FormatterOutputPiece, FormatterTextPiece, output_to_input,
};
use crate::compiler_frontend::ast::templates::styles::whitespace::{
    TemplateBodyRunPosition, TemplateWhitespacePassProfile, apply_whitespace_passes_to_input,
};
use crate::compiler_frontend::ast::templates::template::{
    BodyWhitespacePolicy, ReactiveSubscription, Style, TemplateSegmentOrigin,
};
use crate::compiler_frontend::ast::templates::tir::ids::{TemplateIrId, TemplateIrNodeId};
use crate::compiler_frontend::ast::templates::tir::node::{TemplateIrNode, TemplateIrNodeKind};
use crate::compiler_frontend::ast::templates::tir::overlays::TemplateViewContext;
use crate::compiler_frontend::ast::templates::tir::refs::TemplateTirChildReference;
use crate::compiler_frontend::ast::templates::tir::store::TemplateIrStore;
use crate::compiler_frontend::ast::templates::tir::view::{
    TemplateTirPhase, TirView, TirViewIdentity, structural_transition_context,
};
use crate::compiler_frontend::compiler_errors::{CompilerError, CompilerMessages};
use crate::compiler_frontend::compiler_messages::CompilerDiagnostic;
use crate::compiler_frontend::datatypes::DataType;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
use crate::compiler_frontend::value_mode::ValueMode;
use std::collections::HashMap;

// -------------------------
//  Public result type
// -------------------------

/// Result of applying a style formatter to a TIR subtree.
///
/// WHAT: carries the new root node ID for the formatted tree and any formatter
/// warnings.
/// WHY: callers either publish a derived template version or install the root at
/// an explicit parser/control-flow mutation boundary before forwarding warnings.
pub(crate) struct TirFormatterResult {
    pub root: TemplateIrNodeId,
    pub warnings: Vec<CompilerDiagnostic>,
}

/// Mutable formatter state for one module-local TIR store.
///
/// WHAT: keeps append-only formatter writeback on the shared store while
///       constructing short-lived immutable `TirView`s for overlay-aware reads.
/// WHY: `TirView` deliberately exposes an immutable store reference. Formatter
///      output still needs to append nodes and update nested roots, so the
///      mutation owner must be explicit at this adapter boundary.
struct FormatterStore<'store> {
    store: &'store mut TemplateIrStore,
    /// Owning published template ID, or `None` for a node-root formatter that
    /// has no published template (control-flow body roots). When `None`,
    /// `effective_node` reads the store directly and `structural_child_view`
    /// creates child views without a parent view transition.
    root: Option<TemplateIrId>,
    root_node_id: TemplateIrNodeId,
    head_node_count: usize,
    phase: TemplateTirPhase,
    context: TemplateViewContext,
}

impl FormatterStore<'_> {
    fn view(&self) -> Result<TirView<'_>, CompilerError> {
        let root = self.root.ok_or_else(|| {
            CompilerError::compiler_error(
                "TIR formatter view: no owning template for a node-root formatter store.",
            )
        })?;
        TirView::new(&*self.store, root, self.phase, self.context)
    }

    fn effective_node(&self, node_id: TemplateIrNodeId) -> Result<&TemplateIrNode, CompilerError> {
        self.store.get_node(node_id).ok_or_else(|| {
            CompilerError::compiler_error(format!(
                "TIR formatter view: node {} does not exist in the store",
                node_id
            ))
        })
    }

    fn structural_child_view(
        &self,
        reference: TemplateTirChildReference,
    ) -> Result<TirView<'_>, CompilerError> {
        // For a published-template formatter, use the view's structural_child
        // transition to carry the current expression overlay. For a node-root
        // formatter (no owning template), create the child view directly with
        // the node-root's context, which is default for body roots.
        match self.root {
            Some(_) => self.view()?.structural_child(reference),
            None => {
                let context =
                    structural_transition_context(self.context, reference.phase, reference.context);
                TirView::new(&*self.store, reference.root, reference.phase, context)
            }
        }
    }

    fn node_reactive_subscription(
        &self,
        node_id: TemplateIrNodeId,
    ) -> Result<Option<ReactiveSubscription>, CompilerError> {
        Ok(self.store.node_reactive_subscription(node_id)?.cloned())
    }

    fn push_node(
        &mut self,
        node: TemplateIrNode,
        reactive_subscription: Option<ReactiveSubscription>,
    ) -> TemplateIrNodeId {
        let node_id = self.store.push_node(node);
        if let Some(subscription) = reactive_subscription {
            self.store
                .set_node_reactive_subscription(node_id, subscription)
                .expect("a just-pushed node must accept a reactive subscription");
        }
        node_id
    }
}

// -------------------------
//  Public entry point
// -------------------------

/// Formats a TIR template body tree using its style formatter.
///
/// WHAT: walks the root node of a template already stored in `store`,
///       identifies contiguous body-eligible runs, and runs the shared
///       whitespace/formatter pipeline on each run. Opaque anchors (child
///       templates, dynamic expressions) are preserved; head-origin content and
///       structural nodes break runs and pass through unchanged.
/// WHY: this keeps formatter behavior on the authoritative TIR representation.
pub(crate) fn format_tir_template(
    store: &mut TemplateIrStore,
    root: TemplateIrId,
    phase: TemplateTirPhase,
    context: TemplateViewContext,
    style: &Style,
    string_table: &mut StringTable,
) -> Result<TirFormatterResult, CompilerMessages> {
    let (root_node_id, head_node_count) = {
        let template = store.get_template(root).ok_or_else(|| {
            compiler_error_messages(
                CompilerError::compiler_error(format!(
                    "TIR formatter view lost referenced template {}.",
                    root
                )),
                string_table,
            )
        })?;
        (template.root, template.summary.head_node_count as usize)
    };

    let mut formatter_store = FormatterStore {
        store,
        root: Some(root),
        root_node_id,
        head_node_count,
        phase,
        context,
    };

    format_tir_formatter_store(&mut formatter_store, style, string_table)
}

/// Formats a TIR body node root without pushing a scratch `TemplateIr`.
///
/// WHAT: runs the same formatter pipeline as `format_tir_template` directly on
///       a node root, with zero head-prefix nodes. The body root is not a
///       published template, so no durable `TemplateIr` entry is created.
/// WHY: control-flow body roots are parser-emitted TIR nodes, not template
///      identities. Formatting them directly avoids scratch templates that
///      would remain in the durable store without a referencing identity.
pub(crate) fn format_tir_body_node_root(
    store: &mut TemplateIrStore,
    owning_template: Option<TemplateIrId>,
    body_root: TemplateIrNodeId,
    phase: TemplateTirPhase,
    context: TemplateViewContext,
    style: &Style,
    string_table: &mut StringTable,
) -> Result<TirFormatterResult, CompilerMessages> {
    let mut formatter_store = FormatterStore {
        store,
        root: owning_template,
        root_node_id: body_root,
        head_node_count: 0,
        phase,
        context,
    };

    format_tir_formatter_store(&mut formatter_store, style, string_table)
}

fn format_tir_formatter_store(
    formatter_store: &mut FormatterStore<'_>,
    style: &Style,
    string_table: &mut StringTable,
) -> Result<TirFormatterResult, CompilerMessages> {
    let root_node_id = formatter_store.root_node_id;

    let formatter = style.formatter.as_ref();

    let implicit_default_whitespace_pass = (style.body_whitespace_policy
        == BodyWhitespacePolicy::DefaultTemplateBehavior
        && formatter.is_none())
    .then_some(TemplateWhitespacePassProfile::default_template_body());

    if implicit_default_whitespace_pass.is_none() && formatter.is_none() {
        return Ok(TirFormatterResult {
            root: root_node_id,
            warnings: Vec::new(),
        });
    }

    let pre_format_passes = formatter
        .map(|f| f.pre_format_whitespace_passes.as_slice())
        .unwrap_or_else(|| {
            if let Some(pass) = &implicit_default_whitespace_pass {
                std::slice::from_ref(pass)
            } else {
                &[]
            }
        });

    let post_format_passes = formatter
        .map(|f| f.post_format_whitespace_passes.as_slice())
        .unwrap_or(&[]);

    let root_node_ref = root_node_id;
    let result = format_tir_node(
        formatter_store,
        root_node_ref,
        pre_format_passes,
        post_format_passes,
        formatter,
        string_table,
    )?;

    // Child templates are opaque to the parent formatter, but they still need
    // their own formatter applied before folding. Recursively format every
    // reachable child template so the fold path sees formatted bodies.
    // This root-keyed map is mutation deduplication only: format each shared
    // stored root once. It is not semantic cycle identity; named TirView
    // transitions still determine every recursive view below.
    let mut formatted_templates = HashMap::new();
    let formatted_root_ref = result.root;
    format_child_templates_in_subtree(
        &mut *formatter_store,
        formatted_root_ref,
        &mut formatted_templates,
        string_table,
    )?;

    Ok(result)
}

/// Cheap structural facts extracted from a TIR node for child-template
/// formatting traversal.
///
/// WHAT: carries only the IDs and references needed to continue recursion or
///       format a referenced child template, without cloning the entire
///       `TemplateIrNode`.
/// WHY: recursive formatting may append to the store. Extracting the IDs and
///      scalar facts first keeps the read borrow short.
enum FormatterChildFact {
    ChildTemplate {
        reference: TemplateTirChildReference,
    },
    Sequence(Vec<TemplateIrNodeId>),
    BranchChain {
        branch_bodies: Vec<TemplateIrNodeId>,
        fallback: Option<TemplateIrNodeId>,
    },
    Loop {
        body: TemplateIrNodeId,
        aggregate_wrapper: Option<TemplateIrNodeId>,
    },
    InsertContribution,
    Other,
}

/// Extracts the cheap structural facts needed for child-template formatting
/// traversal from a node kind, without cloning the entire node.
fn extract_formatter_child_fact(kind: &TemplateIrNodeKind) -> FormatterChildFact {
    match kind {
        TemplateIrNodeKind::ChildTemplate { reference, .. } => FormatterChildFact::ChildTemplate {
            reference: *reference,
        },
        TemplateIrNodeKind::Sequence { children } => FormatterChildFact::Sequence(children.clone()),
        TemplateIrNodeKind::BranchChain { branches, fallback } => FormatterChildFact::BranchChain {
            branch_bodies: branches.iter().map(|branch| branch.body).collect(),
            fallback: *fallback,
        },
        TemplateIrNodeKind::Loop {
            body,
            aggregate_wrapper,
            ..
        } => FormatterChildFact::Loop {
            body: *body,
            aggregate_wrapper: *aggregate_wrapper,
        },
        TemplateIrNodeKind::InsertContribution { .. } => FormatterChildFact::InsertContribution,
        _ => FormatterChildFact::Other,
    }
}

/// Recursively formats child templates reachable from a TIR subtree.
///
/// WHAT: walks the formatted tree under `node_id` and calls `format_tir_template`
///       on every `ChildTemplate` reference that has not already been formatted.
/// WHY: parent formatters treat children as opaque anchors, so the parent's own
///      formatting pass does not format nested children. This pass ensures each
///      child template is formatted independently before folding. The root-only
///      version map deduplicates shared mutations, not semantic view traversal.
fn format_child_templates_in_subtree(
    formatter_store: &mut FormatterStore<'_>,
    node_ref: TemplateIrNodeId,
    formatted_templates: &mut HashMap<TemplateIrId, TemplateIrId>,
    string_table: &mut StringTable,
) -> Result<(), CompilerMessages> {
    let fact = {
        let node = formatter_store
            .effective_node(node_ref)
            .map_err(|error| compiler_error_messages(error, string_table))?;
        extract_formatter_child_fact(&node.kind)
    };

    match fact {
        FormatterChildFact::ChildTemplate { reference } => {
            let child_identity = formatter_store
                .structural_child_view(reference)
                .map(|view| view.identity())
                .map_err(|error| compiler_error_messages(error, string_table))?;
            let formatted_id = if let Some(formatted_id) =
                formatted_templates.get(&child_identity.root).copied()
            {
                formatted_id
            } else {
                // Reserve the identity before descending so shared references
                // within this subtree are not formatted more than once.
                formatted_templates.insert(child_identity.root, child_identity.root);
                let formatted_id = format_referenced_child_template(
                    formatter_store,
                    child_identity,
                    string_table,
                )?
                .unwrap_or(child_identity.root);
                formatted_templates.insert(child_identity.root, formatted_id);
                formatted_id
            };
            if formatted_id != child_identity.root {
                formatter_store
                    .store
                    .replace_child_template_reference(node_ref, formatted_id)
                    .map_err(|error| compiler_error_messages(error, string_table))?;
            }
        }

        FormatterChildFact::Sequence(children) => {
            for child_id in children {
                let child_ref = child_id;
                format_child_templates_in_subtree(
                    formatter_store,
                    child_ref,
                    formatted_templates,
                    string_table,
                )?;
            }
        }

        FormatterChildFact::BranchChain {
            branch_bodies,
            fallback,
        } => {
            for body_id in branch_bodies {
                let branch_ref = body_id;
                format_child_templates_in_subtree(
                    formatter_store,
                    branch_ref,
                    formatted_templates,
                    string_table,
                )?;
            }

            if let Some(fallback_id) = fallback {
                let fallback_ref = fallback_id;
                format_child_templates_in_subtree(
                    formatter_store,
                    fallback_ref,
                    formatted_templates,
                    string_table,
                )?;
            }
        }

        FormatterChildFact::Loop {
            body,
            aggregate_wrapper,
        } => {
            let body_ref = body;
            format_child_templates_in_subtree(
                formatter_store,
                body_ref,
                formatted_templates,
                string_table,
            )?;

            if let Some(aggregate_id) = aggregate_wrapper {
                let aggregate_ref = aggregate_id;
                format_child_templates_in_subtree(
                    formatter_store,
                    aggregate_ref,
                    formatted_templates,
                    string_table,
                )?;
            }
        }

        FormatterChildFact::InsertContribution => {
            // Parser composition formats insert helpers before installing their
            // contribution nodes, so this traversal must not format them again.
        }

        _ => {}
    }

    Ok(())
}

/// Formats a single child/insert template and publishes a derived template version.
///
/// WHAT: looks up the referenced template, formats it with its own style, and
///       returns a new template ID for the formatted root.
/// WHY: both `ChildTemplate` and `InsertContribution` nodes reference nested
///      templates that need independent formatting before folding. A derived
///      template version keeps the published source record immutable.
fn format_referenced_child_template(
    formatter_store: &mut FormatterStore<'_>,
    identity: TirViewIdentity,
    string_table: &mut StringTable,
) -> Result<Option<TemplateIrId>, CompilerMessages> {
    let template_ref = identity.root;
    let phase = identity.phase;
    let context = identity.context;
    let style = {
        let template = formatter_store
            .store
            .get_template(template_ref)
            .ok_or_else(|| {
                compiler_error_messages(
                    CompilerError::compiler_error(format!(
                        "TIR formatter view lost referenced child template {}.",
                        template_ref
                    )),
                    string_table,
                )
            })?;
        template.style.clone()
    };

    // A child template whose reference phase has already reached Formatted
    // carries a formatted root and must not be re-formatted. Re-formatting
    // would double-escape output such as markdown paragraphs.
    let already_formatted =
        style.formatter.is_some() && phase.is_at_least(TemplateTirPhase::Formatted);

    if already_formatted {
        return Ok(None);
    }

    let result = format_tir_template(
        formatter_store.store,
        template_ref,
        phase,
        context,
        &style,
        string_table,
    )?;

    let formatted_template_id = formatter_store
        .store
        .push_structurally_derived_template(
            template_ref,
            result.root,
            crate::compiler_frontend::ast::templates::tir::DerivedTemplateMetadata::preserve_source(
            ),
        )
        .map_err(|error| compiler_error_messages(error, string_table))?;

    Ok(Some(formatted_template_id))
}

// -------------------------
//  Recursive node formatting
// -------------------------

/// Cheap structural facts extracted from a TIR node for formatter dispatch.
///
/// WHAT: carries only the children IDs and source location needed to format a
///       single node, without cloning the entire `TemplateIrNode`.
/// WHY: formatting may append to the store after the node facts are extracted,
///      so this boundary keeps reads and writes in separate steps.
enum FormatterNodeFact {
    Sequence {
        children: Vec<TemplateIrNodeId>,
        location: SourceLocation,
    },
    BodyEligible {
        location: SourceLocation,
    },
    Passthrough,
}

/// Extracts the cheap structural facts needed for formatter dispatch from a
/// node, without cloning the entire node.
fn extract_formatter_node_fact(node: &TemplateIrNode) -> FormatterNodeFact {
    match &node.kind {
        TemplateIrNodeKind::Sequence { children } => FormatterNodeFact::Sequence {
            children: children.clone(),
            location: node.location.clone(),
        },
        _ if is_body_eligible_kind(&node.kind) => FormatterNodeFact::BodyEligible {
            location: node.location.clone(),
        },
        _ => FormatterNodeFact::Passthrough,
    }
}

/// Formats a single TIR node and returns the formatted root for that subtree.
///
/// WHAT: sequences are scanned for body runs; single body-eligible nodes are
/// wrapped in a synthetic run; all other node kinds pass through unchanged.
/// WHY: formatter bodies are flat runs of text and opaque anchors. Recursing
/// into child templates would violate opacity, and control-flow nodes are not
/// expected in a simple formatter body.
fn format_tir_node(
    formatter_store: &mut FormatterStore<'_>,
    node_ref: TemplateIrNodeId,
    pre_format_passes: &[TemplateWhitespacePassProfile],
    post_format_passes: &[TemplateWhitespacePassProfile],
    formatter: Option<&crate::compiler_frontend::ast::templates::template::Formatter>,
    string_table: &mut StringTable,
) -> Result<TirFormatterResult, CompilerMessages> {
    let fact = {
        let node = formatter_store
            .effective_node(node_ref)
            .map_err(|error| compiler_error_messages(error, string_table))?;
        extract_formatter_node_fact(node)
    };

    match fact {
        FormatterNodeFact::Sequence { children, location } => {
            // The root template carries the authoritative head-prefix count
            // for its own root sequence. A sequence that is not the template
            // root (for example a nested body run) has no head prefix, so its
            // head count is zero. The root-template lookup is required internal
            // authority: a missing root is a compiler bug, not a silent skip.
            let head_node_count = if node_ref == formatter_store.root_node_id {
                formatter_store.head_node_count
            } else {
                0
            };

            format_tir_sequence(
                formatter_store,
                node_ref,
                &children,
                location,
                head_node_count,
                pre_format_passes,
                post_format_passes,
                formatter,
                string_table,
            )
        }

        FormatterNodeFact::BodyEligible { location } => {
            // A single body-eligible node is treated as a run of one. It is not
            // wrapped in a sequence unless the formatter expands it.
            let representative_location =
                representative_location_for_single_node(formatter_store, node_ref, string_table)?;
            let (replacement_nodes, warnings, content_changed) = process_formatter_run(
                formatter_store,
                std::slice::from_ref(&node_ref),
                TemplateBodyRunPosition::Only,
                &representative_location,
                pre_format_passes,
                post_format_passes,
                formatter,
                string_table,
            )?;

            let root = if replacement_nodes.len() == 1 && !content_changed {
                replacement_nodes[0]
            } else {
                push_formatter_node(
                    formatter_store,
                    TemplateIrNode::new(
                        TemplateIrNodeKind::Sequence {
                            children: replacement_nodes,
                        },
                        location,
                    ),
                    None,
                )?
            };

            Ok(TirFormatterResult { root, warnings })
        }

        // Structural nodes that are not body-eligible pass through unchanged.
        FormatterNodeFact::Passthrough => Ok(TirFormatterResult {
            root: node_ref,
            warnings: Vec::new(),
        }),
    }
}

/// Cheap eligibility facts for a child node during sequence formatting.
///
/// WHAT: carries only the two boolean facts needed for run-membership decisions.
struct ChildRunEligibility {
    is_child_template: bool,
    is_body_eligible: bool,
}

/// Formats a sequence node by scanning its children for contiguous body runs.
///
/// WHAT: children that are body-eligible form formatter runs; everything else
/// terminates the current run. Each run is processed independently, and the
/// resulting nodes are spliced back in order.
/// WHY: formatter runs operate on TIR node IDs while keeping structural nodes
/// outside the formatter-visible surface.
#[allow(clippy::too_many_arguments)]
fn format_tir_sequence(
    formatter_store: &mut FormatterStore<'_>,
    original_node_ref: TemplateIrNodeId,
    children: &[TemplateIrNodeId],
    location: SourceLocation,
    head_node_count: usize,
    pre_format_passes: &[TemplateWhitespacePassProfile],
    post_format_passes: &[TemplateWhitespacePassProfile],
    formatter: Option<&crate::compiler_frontend::ast::templates::template::Formatter>,
    string_table: &mut StringTable,
) -> Result<TirFormatterResult, CompilerMessages> {
    let mut new_children: Vec<TemplateIrNodeId> = Vec::with_capacity(children.len());
    let mut current_run: Vec<TemplateIrNodeId> = Vec::new();
    let mut all_warnings: Vec<CompilerDiagnostic> = Vec::new();
    let mut content_changed = false;
    let mut is_first_run = true;

    for (child_index, &child_id) in children.iter().enumerate() {
        let child_ref = child_id;
        let child_eligibility = {
            let child = formatter_store
                .effective_node(child_ref)
                .map_err(|error| compiler_error_messages(error, string_table))?;
            ChildRunEligibility {
                is_child_template: matches!(child.kind, TemplateIrNodeKind::ChildTemplate { .. }),
                is_body_eligible: is_body_eligible_kind(&child.kind),
            }
        };

        let is_head_child_template =
            child_index < head_node_count && child_eligibility.is_child_template;
        let is_eligible = child_eligibility.is_body_eligible && !is_head_child_template;

        if is_eligible {
            current_run.push(child_id);
            continue;
        }

        if !current_run.is_empty() {
            let run_position = run_position_for_run(is_first_run, false);
            let representative_location =
                representative_location_for_run(formatter_store, &current_run, string_table)?;

            let (replacement, warnings, run_changed) = process_formatter_run(
                formatter_store,
                &current_run,
                run_position,
                &representative_location,
                pre_format_passes,
                post_format_passes,
                formatter,
                string_table,
            )?;

            new_children.extend(replacement);
            all_warnings.extend(warnings);
            content_changed |= run_changed;
            current_run.clear();
            is_first_run = false;
        }

        new_children.push(child_id);
    }

    if !current_run.is_empty() {
        let run_position = run_position_for_run(is_first_run, true);
        let representative_location =
            representative_location_for_run(formatter_store, &current_run, string_table)?;

        let (replacement, warnings, run_changed) = process_formatter_run(
            formatter_store,
            &current_run,
            run_position,
            &representative_location,
            pre_format_passes,
            post_format_passes,
            formatter,
            string_table,
        )?;

        new_children.extend(replacement);
        all_warnings.extend(warnings);
        content_changed |= run_changed;
    }

    let root = if !content_changed && new_children.len() == children.len() {
        // Fast path: nothing changed, so the original node is still valid.
        original_node_ref
    } else {
        push_formatter_node(
            formatter_store,
            TemplateIrNode::new(
                TemplateIrNodeKind::Sequence {
                    children: new_children,
                },
                location,
            ),
            None,
        )?
    };

    Ok(TirFormatterResult {
        root,
        warnings: all_warnings,
    })
}

// -------------------------
//  Run membership
// -------------------------

/// Returns true when a node kind can participate in a contiguous formatter run.
///
/// WHAT: body text, body dynamic expressions, and opaque child templates are
/// formatter-visible. Head-origin text/expressions and structural nodes break
/// runs.
/// WHY: head nodes and structural control flow aren't body-formatting input.
fn is_body_eligible_kind(kind: &TemplateIrNodeKind) -> bool {
    match kind {
        TemplateIrNodeKind::Text { origin, .. } => *origin == TemplateSegmentOrigin::Body,

        TemplateIrNodeKind::DynamicExpression { origin, .. } => {
            *origin == TemplateSegmentOrigin::Body
        }

        TemplateIrNodeKind::ChildTemplate { .. } => true,

        _ => false,
    }
}

/// Returns true when a `ChildTemplate` TIR node references a head-expression
/// insert child.
///
/// WHAT: a head-expression insert child has a TIR root that consists only of
/// head-origin `Text` nodes. Such children are opaque expression anchors to the
/// parent formatter, not sealed child-template boundaries, so they must be
/// classified as `FormatterOpaqueKind::DynamicExpression`.
/// WHY: markdown inline-code pairing must work across inserted scalar strings
/// without opening body-bearing child templates to the parent formatter.
fn child_template_is_head_expression_insert_in_tir(
    formatter_store: &FormatterStore<'_>,
    reference: &TemplateTirChildReference,
) -> Result<bool, CompilerError> {
    let child_view = formatter_store.structural_child_view(*reference)?;
    let child_template = child_view.root_template()?;
    let root_node_ref = child_template.root;
    let root_node = child_view.effective_node(root_node_ref)?;

    let candidate_ids = match &root_node.kind {
        TemplateIrNodeKind::Sequence { children } => children.as_slice(),
        _ => std::slice::from_ref(&root_node_ref),
    };

    if candidate_ids.is_empty() {
        return Ok(false);
    }

    for node_id in candidate_ids {
        let node = child_view.effective_node(*node_id)?;

        match &node.kind {
            TemplateIrNodeKind::Text { origin, .. } if *origin == TemplateSegmentOrigin::Head => {}
            _ => return Ok(false),
        }
    }

    Ok(true)
}

/// Classifies a body-eligible node kind into the opaque anchor kind used by the
/// formatter pipeline.
///
/// WHAT: child-template nodes become `ChildTemplate` anchors unless they are
/// head-expression inserts, which become `DynamicExpression` anchors; body
/// dynamic expressions become `DynamicExpression` anchors.
/// WHY: the `$md` inline-code pass distinguishes these two kinds without
/// inspecting its content, and head-expression inserts must pair like direct
/// dynamic-expression anchors. Accepting the kind directly avoids a repeated
/// `effective_node` read when the caller already holds the node borrow.
fn opaque_kind_for_kind(
    formatter_store: &FormatterStore<'_>,
    kind: &TemplateIrNodeKind,
) -> Result<FormatterOpaqueKind, CompilerError> {
    match kind {
        TemplateIrNodeKind::ChildTemplate { reference, .. } => {
            if child_template_is_head_expression_insert_in_tir(formatter_store, reference)? {
                Ok(FormatterOpaqueKind::DynamicExpression)
            } else {
                Ok(FormatterOpaqueKind::ChildTemplate)
            }
        }
        TemplateIrNodeKind::DynamicExpression { .. } => Ok(FormatterOpaqueKind::DynamicExpression),

        _ => Err(CompilerError::compiler_error(format!(
            "TIR formatter view attempted to anchor unsupported node kind: {:?}",
            kind
        ))),
    }
}

// -------------------------
//  Run processing
// -------------------------

/// Processes one contiguous formatter run through whitespace passes and the
/// style formatter.
///
/// WHAT: builds `FormatterInput` from the run, runs pre-format whitespace
/// passes, the formatter, and post-format whitespace passes, then maps the
/// output back to TIR node IDs using a local anchor side-table.
/// WHY: this is the core adapter step that lets existing formatters consume TIR
/// data and produce TIR data.
#[allow(clippy::too_many_arguments)]
fn process_formatter_run(
    formatter_store: &mut FormatterStore<'_>,
    run: &[TemplateIrNodeId],
    run_position: TemplateBodyRunPosition,
    representative_location: &SourceLocation,
    pre_format_passes: &[TemplateWhitespacePassProfile],
    post_format_passes: &[TemplateWhitespacePassProfile],
    formatter: Option<&crate::compiler_frontend::ast::templates::template::Formatter>,
    string_table: &mut StringTable,
) -> Result<(Vec<TemplateIrNodeId>, Vec<CompilerDiagnostic>, bool), CompilerMessages> {
    if run.is_empty() {
        return Ok((Vec::new(), Vec::new(), false));
    }

    let mut input_pieces: Vec<FormatterInputPiece> = Vec::with_capacity(run.len());
    let mut anchor_side_table: Vec<TemplateIrNodeId> = Vec::with_capacity(run.len());
    let mut run_reactive_subscription: Option<ReactiveSubscription> = None;

    for &node_id in run {
        let node_ref = node_id;
        let node = formatter_store
            .effective_node(node_ref)
            .map_err(|error| compiler_error_messages(error, string_table))?;

        match &node.kind {
            TemplateIrNodeKind::Text { text, .. } => {
                if run_reactive_subscription.is_none() {
                    run_reactive_subscription = formatter_store
                        .node_reactive_subscription(node_id)
                        .map_err(|error| compiler_error_messages(error, string_table))?;
                }

                input_pieces.push(FormatterInputPiece::Text(FormatterTextPiece {
                    text: *text,
                    location: node.location.clone(),
                }));
            }

            _ => {
                let anchor_id = FormatterAnchorId(anchor_side_table.len());
                anchor_side_table.push(node_id);

                input_pieces.push(FormatterInputPiece::Opaque(FormatterOpaquePiece {
                    id: anchor_id,
                    kind: opaque_kind_for_kind(formatter_store, &node.kind)
                        .map_err(|error| CompilerMessages::from_error_ref(error, string_table))?,
                }));
            }
        }
    }

    let input = FormatterInput {
        pieces: input_pieces,
    };

    // 1. Pre-format whitespace passes.
    let mut output =
        apply_whitespace_passes_to_input(input, pre_format_passes, run_position, string_table);

    // 2. Style formatter.
    let mut formatter_warnings = Vec::new();

    if let Some(fmt) = formatter {
        let next_input = output_to_input(output, representative_location, string_table);
        let formatter_result = fmt.formatter.format(next_input, string_table)?;

        formatter_warnings.extend(formatter_result.warnings);
        output = formatter_result.output;
    }

    // 3. Post-format whitespace passes.
    if !post_format_passes.is_empty() {
        let post_input = output_to_input(output, representative_location, string_table);

        output = apply_whitespace_passes_to_input(
            post_input,
            post_format_passes,
            run_position,
            string_table,
        );
    }

    // 4. Map formatter output back to TIR nodes.
    let (replacement_nodes, content_changed) = output_to_tir_nodes(
        formatter_store,
        output,
        representative_location,
        &anchor_side_table,
        run_reactive_subscription,
        string_table,
    )?;

    // A run is considered changed if its output node IDs differ from the input.
    let run_changed = content_changed
        || replacement_nodes.len() != run.len()
        || !replacement_nodes
            .iter()
            .zip(run.iter())
            .all(|(a, b)| a == b);

    Ok((replacement_nodes, formatter_warnings, run_changed))
}

/// Maps formatter output pieces back to TIR node IDs.
///
/// WHAT: text output becomes a new body `Text` node; ordinary opaque anchors
///      look up the local side-table and reuse the original TIR node; a
///      formatter-generated site-root anchor becomes a structural expression node.
/// WHY: preserving original nodes for source anchors keeps child-template opacity
///      and dynamic-expression metadata intact, while `$md` site-root links use
///      the same structural string path as ordinary file values.
fn output_to_tir_nodes(
    formatter_store: &mut FormatterStore<'_>,
    output: crate::compiler_frontend::ast::templates::formatter_contract::FormatterOutput,
    representative_location: &SourceLocation,
    anchor_side_table: &[TemplateIrNodeId],
    run_reactive_subscription: Option<ReactiveSubscription>,
    string_table: &mut StringTable,
) -> Result<(Vec<TemplateIrNodeId>, bool), CompilerMessages> {
    let mut nodes = Vec::with_capacity(output.pieces.len());
    let mut content_changed = false;

    for piece in output.pieces {
        match piece {
            FormatterOutputPiece::Text(text) => {
                let text_id = string_table.intern(&text);
                let byte_len = text.len();

                nodes.push(push_formatter_node(
                    formatter_store,
                    TemplateIrNode::new(
                        TemplateIrNodeKind::Text {
                            text: text_id,
                            byte_len,
                            origin: TemplateSegmentOrigin::Body,
                        },
                        representative_location.clone(),
                    ),
                    run_reactive_subscription.clone(),
                )?);

                content_changed = true;
            }

            FormatterOutputPiece::Opaque(anchor) => {
                if anchor.kind == FormatterOpaqueKind::SiteRoot {
                    let site_id = formatter_store.store.next_expression_site_id();
                    let expression = Expression::new(
                        ExpressionKind::StructuralString {
                            pieces: vec![ConstStringPiece::SiteRoot],
                        },
                        representative_location.clone(),
                        builtin_type_ids::STRING,
                        DataType::StringSlice,
                        ValueMode::ImmutableOwned,
                    );
                    nodes.push(push_formatter_node(
                        formatter_store,
                        TemplateIrNode::new(
                            TemplateIrNodeKind::DynamicExpression {
                                expression: Box::new(expression),
                                origin: TemplateSegmentOrigin::Body,
                                reactive_subscription: None,
                                site_id,
                            },
                            representative_location.clone(),
                        ),
                        None,
                    )?);
                    content_changed = true;
                    continue;
                }

                let Some(node_id) = anchor_side_table.get(anchor.id.0).copied() else {
                    return Err(CompilerMessages::from_error_ref(
                        CompilerError::compiler_error(format!(
                            "TIR formatter view received invalid opaque anchor id {}; only {} anchors exist for this formatter run.",
                            anchor.id.0,
                            anchor_side_table.len()
                        )),
                        string_table,
                    ));
                };

                nodes.push(node_id);
            }
        }
    }

    Ok((nodes, content_changed))
}

/// Appends a formatter-produced node to the store that owns the current view.
///
/// WHAT: obtains the mutable store borrow only after formatter input has been
///       extracted into owned local data. WHY: `TirView` reads through the
///       module store `RefCell`, so writeback must be a separate short phase
///       rather than holding a mutable store borrow during view reads.
fn push_formatter_node(
    formatter_store: &mut FormatterStore<'_>,
    node: TemplateIrNode,
    reactive_subscription: Option<ReactiveSubscription>,
) -> Result<TemplateIrNodeId, CompilerMessages> {
    let node_id = formatter_store.push_node(node, reactive_subscription);

    Ok(node_id)
}

// -------------------------
//  Source locations
// -------------------------

/// Chooses a `TemplateBodyRunPosition` for a run based on whether it is the
/// first/last run in the parent sequence.
fn run_position_for_run(is_first_run: bool, is_last_run: bool) -> TemplateBodyRunPosition {
    match (is_first_run, is_last_run) {
        (true, true) => TemplateBodyRunPosition::Only,
        (true, false) => TemplateBodyRunPosition::First,
        (false, true) => TemplateBodyRunPosition::Last,
        (false, false) => TemplateBodyRunPosition::Middle,
    }
}

/// Derives a coarse representative source location for a run of TIR nodes.
///
/// WHAT: aggregates body-text node locations when possible; falls back to the
/// location of the first text/child/dynamic node in the run. Both phases share
/// a single pass to avoid reading each node twice.
/// WHY: formatter output can rewrite arbitrary text, so exact per-character
/// provenance is not feasible. A representative span preserves useful
/// diagnostics locations without pretending to be precise.
fn representative_location_for_run(
    formatter_store: &FormatterStore<'_>,
    run: &[TemplateIrNodeId],
    string_table: &StringTable,
) -> Result<SourceLocation, CompilerMessages> {
    let mut first_text_location: Option<SourceLocation> = None;
    let mut last_text_location: Option<SourceLocation> = None;
    let mut fallback_location: Option<SourceLocation> = None;

    for &node_id in run {
        let node_ref = node_id;
        let node = formatter_store
            .effective_node(node_ref)
            .map_err(|error| compiler_error_messages(error, string_table))?;

        match &node.kind {
            TemplateIrNodeKind::Text { origin, .. } => {
                if *origin == TemplateSegmentOrigin::Body {
                    if first_text_location.is_none() {
                        first_text_location = Some(node.location.clone());
                    }
                    last_text_location = Some(node.location.clone());
                }

                if fallback_location.is_none() {
                    fallback_location = Some(node.location.clone());
                }
            }

            TemplateIrNodeKind::ChildTemplate { .. }
            | TemplateIrNodeKind::DynamicExpression { .. }
                if fallback_location.is_none() =>
            {
                fallback_location = Some(node.location.clone());
            }

            _ => {}
        }
    }

    // Prefer the aggregated body-text span when body-text nodes exist.
    if let (Some(start), Some(end)) = (first_text_location, last_text_location) {
        if start.scope != end.scope {
            return Ok(start);
        }

        return Ok(SourceLocation {
            scope: start.scope,
            start_pos: start.start_pos,
            end_pos: end.end_pos,
        });
    }

    // Fall back to the first text/child/dynamic node location.
    Ok(fallback_location.unwrap_or_default())
}

/// Derives a representative location for a single body-eligible node.
fn representative_location_for_single_node(
    formatter_store: &FormatterStore<'_>,
    node_ref: TemplateIrNodeId,
    string_table: &StringTable,
) -> Result<SourceLocation, CompilerMessages> {
    representative_location_for_run(
        formatter_store,
        std::slice::from_ref(&node_ref),
        string_table,
    )
}

// -------------------------
//  Diagnostics
// -------------------------

fn compiler_error_messages(error: CompilerError, string_table: &StringTable) -> CompilerMessages {
    CompilerMessages::from_error_ref(error, string_table)
}
