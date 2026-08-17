//! Checked control-flow lookup and body mutation on `TemplateIrStore`.

use super::TemplateIrStore;
use crate::compiler_frontend::ast::templates::tir::ids::{TemplateIrId, TemplateIrNodeId};
use crate::compiler_frontend::ast::templates::tir::node::TemplateIrNodeKind;
use crate::compiler_frontend::compiler_errors::CompilerError;
use std::collections::HashSet;

/// Identifies which body inside a control-flow TIR node should receive a
/// prepared simple TIR root.
#[derive(Clone, Copy, Debug)]
pub(crate) enum ControlFlowBodyKind {
    Branch { index: usize },
    Fallback,
    LoopBody,
}

impl TemplateIrStore {
    /// Returns the first control-flow node under a finalized template root.
    ///
    /// `Ok(None)` means the reachable tree is valid and has no owner
    /// control-flow node. Missing nodes, missing forwarding templates and
    /// cycles are `CompilerError`.
    pub(crate) fn control_flow_node_id_for_template(
        &self,
        owning_template_id: TemplateIrId,
    ) -> Result<Option<TemplateIrNodeId>, CompilerError> {
        let Some(template) = self.get_template(owning_template_id) else {
            return Err(CompilerError::compiler_error(format!(
                "TIR store has no template {owning_template_id} while locating its control-flow node."
            )));
        };

        self.control_flow_node_id_in_subtree(template.root)
    }

    /// Recursively searches a TIR subtree for the template-owned control-flow node.
    pub(crate) fn control_flow_node_id_in_subtree(
        &self,
        root: TemplateIrNodeId,
    ) -> Result<Option<TemplateIrNodeId>, CompilerError> {
        let mut path = HashSet::new();
        self.find_control_flow_node_in_subtree(root, &mut path)
    }

    fn find_control_flow_node_in_subtree(
        &self,
        node_id: TemplateIrNodeId,
        path: &mut HashSet<TemplateIrNodeId>,
    ) -> Result<Option<TemplateIrNodeId>, CompilerError> {
        if !path.insert(node_id) {
            return Err(CompilerError::compiler_error(format!(
                "TIR store found a cycle while locating a control-flow node at {node_id}."
            )));
        }

        let result = self.control_flow_node_at(node_id, path);
        path.remove(&node_id);
        result
    }

    fn control_flow_node_at(
        &self,
        node_id: TemplateIrNodeId,
        path: &mut HashSet<TemplateIrNodeId>,
    ) -> Result<Option<TemplateIrNodeId>, CompilerError> {
        let Some(node) = self.get_node(node_id) else {
            return Err(CompilerError::compiler_error(format!(
                "TIR store has no node {node_id} while locating a control-flow node."
            )));
        };

        match &node.kind {
            TemplateIrNodeKind::BranchChain { .. } | TemplateIrNodeKind::Loop { .. } => {
                Ok(Some(node_id))
            }

            TemplateIrNodeKind::Sequence { children } => {
                for child in children {
                    if let Some(control_flow_node) =
                        self.find_control_flow_node_in_subtree(*child, path)?
                    {
                        return Ok(Some(control_flow_node));
                    }
                }

                // Runtime slot/head-chain composition can produce a forwarding
                // template whose root sequence contains only one child-template
                // reference. In that narrow shape, the referenced child is the
                // owner's control-flow tree, not arbitrary nested content.
                let [only_child] = children.as_slice() else {
                    return Ok(None);
                };

                let Some(child_node) = self.get_node(*only_child) else {
                    return Err(CompilerError::compiler_error(format!(
                        "TIR store has no node {only_child} while following a forwarding control-flow sequence."
                    )));
                };

                let TemplateIrNodeKind::ChildTemplate { reference, .. } = &child_node.kind else {
                    return Ok(None);
                };

                let Some(template_ir) = self.get_template(reference.root) else {
                    return Err(CompilerError::compiler_error(format!(
                        "TIR store has no forwarding template {} while locating a control-flow node.",
                        reference.root
                    )));
                };

                self.find_control_flow_node_in_subtree(template_ir.root, path)
            }

            _ => Ok(None),
        }
    }

    pub(crate) fn replace_control_flow_body(
        &mut self,
        control_flow_node_id: TemplateIrNodeId,
        body_kind: ControlFlowBodyKind,
        new_body_root: TemplateIrNodeId,
    ) -> Result<(), CompilerError> {
        if self.get_node(new_body_root).is_none() {
            return Err(CompilerError::compiler_error(format!(
                "Control-flow body replacement cannot install missing body root {new_body_root}."
            )));
        }

        let control_flow_node = self.node_mut(control_flow_node_id)?;

        match (&mut control_flow_node.kind, body_kind) {
            (
                TemplateIrNodeKind::BranchChain { branches, .. },
                ControlFlowBodyKind::Branch { index },
            ) => {
                let Some(branch) = branches.get_mut(index) else {
                    return Err(CompilerError::compiler_error(format!(
                        "Control-flow body replacement could not find branch {index} on node {control_flow_node_id}."
                    )));
                };
                branch.body = new_body_root;
                Ok(())
            }

            (TemplateIrNodeKind::BranchChain { fallback, .. }, ControlFlowBodyKind::Fallback) => {
                let Some(fallback_body) = fallback.as_mut() else {
                    return Err(CompilerError::compiler_error(format!(
                        "Control-flow body replacement could not find a fallback on node {control_flow_node_id}."
                    )));
                };
                *fallback_body = new_body_root;
                Ok(())
            }

            (TemplateIrNodeKind::Loop { body, .. }, ControlFlowBodyKind::LoopBody) => {
                *body = new_body_root;
                Ok(())
            }

            _ => Err(CompilerError::compiler_error(format!(
                "Control-flow body replacement targeted a non-matching node {control_flow_node_id}."
            ))),
        }
    }

    pub(crate) fn replace_loop_aggregate_wrapper(
        &mut self,
        control_flow_node_id: TemplateIrNodeId,
        new_aggregate_wrapper_root: TemplateIrNodeId,
    ) -> Result<(), CompilerError> {
        if self.get_node(new_aggregate_wrapper_root).is_none() {
            return Err(CompilerError::compiler_error(format!(
                "Loop aggregate-wrapper installation cannot use missing root {new_aggregate_wrapper_root}."
            )));
        }

        let control_flow_node = self.node_mut(control_flow_node_id)?;

        match &mut control_flow_node.kind {
            TemplateIrNodeKind::Loop {
                aggregate_wrapper, ..
            } => {
                *aggregate_wrapper = Some(new_aggregate_wrapper_root);
                Ok(())
            }
            _ => Err(CompilerError::compiler_error(format!(
                "Loop aggregate-wrapper installation targeted a non-loop node {control_flow_node_id}."
            ))),
        }
    }
}
