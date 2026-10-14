//! Reference-time rebuild of imported const-template values into the importing module's TIR.
//!
//! WHAT: a `ConstValuePayload::Template` constant is stored as its stable `PublicConstTemplate`
//!       projection. A reference to that constant in a constant context rebuilds the projection
//!       into the consumer's `TemplateIrStore` and yields a real `Template` expression.
//! WHY: head composition, `$children(..)` and slot routing attach their semantics only to
//!      `ExpressionOperationView::Template` values. A bare `Constant` reference would drop the
//!      wrapper and insert structure, so the reference has to be a template again.

use std::cell::RefCell;

use super::ids::{TemplateIrId, TemplateIrNodeId};
use super::node::{TemplateIr, TemplateIrNode, TemplateIrNodeKind, TirSlotPlaceholder};
use super::overlays::TemplateViewContext;
use super::refs::{TemplateTirReference, TemplateWrapperReference};
use super::store::TemplateIrStore;
use super::summary::summarize_existing_root;
use super::view::TemplateTirPhase;
use crate::compiler_frontend::ast::const_values::store::ConstStringPiece;
use crate::compiler_frontend::ast::const_values::{
    ConstValueId, ConstValuePayload, ConstValueStore,
};
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression_store::AstExpressionBuilder;
use crate::compiler_frontend::ast::file_value_resolution::structural_string;
use crate::compiler_frontend::ast::templates::template::{
    SlotKey, Style, Template, TemplateSegmentOrigin, TemplateType,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::folded_value::{
    OwnedFoldedString, OwnedFoldedStringPiece, PublicConstTemplate, PublicConstTemplateKind,
    PublicConstTemplatePiece, PublicConstTemplateSlot, PublicTemplateSlotKey,
};
use crate::compiler_frontend::paths::module_resources::{ModuleResourceTable, ResourceId};
use crate::compiler_frontend::paths::resource_identity::StableResourceOriginId;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::value_mode::ValueMode;

/// Builds the `Template` expression payload for one imported const-template constant.
///
/// Returns `Ok(None)` when `value` is not a `Template` payload, so callers can keep the plain
/// constant path for every other payload.
///
/// WHAT: rebuilds the public projection once per `ConstValueId` per module, then reuses the
///       memoised TIR root for every later reference.
/// WHY: all references to one constant must share one TIR root, matching the single authored
///      initializer that imported references inlined before the typed store cutover.
pub(crate) fn rebuild_const_template_reference(
    value: ConstValueId,
    store: &mut TemplateIrStore,
    expression_builder: &mut AstExpressionBuilder,
    const_values: &mut ConstValueStore,
    module_resources: Option<&RefCell<ModuleResourceTable>>,
    string_table: &mut StringTable,
    span: Option<SourceSpan>,
) -> Result<Option<Template>, ExpressionParseError> {
    let root = match store.rebuilt_const_template_root(value) {
        Some(root) => root,
        None => {
            let Some(ConstValuePayload::Template { template, .. }) = const_values.payload(value)
            else {
                return Ok(None);
            };
            let template = template.clone();
            let root = Rebuild {
                store: &mut *store,
                expression_builder: &mut *expression_builder,
                const_values: &mut *const_values,
                module_resources,
                string_table: &mut *string_table,
                span,
            }
            .template(&template)?;
            store.remember_rebuilt_const_template_root(value, root);
            root
        }
    };

    Ok(Some(Template {
        tir_reference: TemplateTirReference {
            root,
            phase: TemplateTirPhase::Finalized,
            context: TemplateViewContext::default(),
        },
        span,
    }))
}

/// Borrowed services for one rebuild pass.
struct Rebuild<'a> {
    store: &'a mut TemplateIrStore,
    expression_builder: &'a mut AstExpressionBuilder,
    const_values: &'a mut ConstValueStore,
    module_resources: Option<&'a RefCell<ModuleResourceTable>>,
    string_table: &'a mut StringTable,
    span: Option<SourceSpan>,
}

impl Rebuild<'_> {
    fn template(
        &mut self,
        template: &PublicConstTemplate,
    ) -> Result<TemplateIrId, ExpressionParseError> {
        let mut children = Vec::with_capacity(template.pieces.len());
        for piece in &template.pieces {
            let node = match piece {
                PublicConstTemplatePiece::Text(value) => self.text(value)?,
                PublicConstTemplatePiece::Slot(slot) => {
                    let placeholder = self.slot(slot)?;
                    self.store.push_node(TemplateIrNode::new(
                        TemplateIrNodeKind::Slot { placeholder },
                        self.span,
                    ))
                }
            };
            children.push(node);
        }

        let root = self.store.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::Sequence { children },
            self.span,
        ));
        let kind = match &template.kind {
            PublicConstTemplateKind::Wrapper => TemplateType::String,
            PublicConstTemplateKind::SlotInsert(key) => {
                TemplateType::SlotInsert(slot_key(key, self.string_table))
            }
        };
        let summary = summarize_existing_root(self.store, root)?;
        let mut template_ir = TemplateIr::new(root, Style::default(), kind, summary, self.span);

        let conditional_wrappers = self.wrappers(&template.conditional_child_wrappers)?;
        if !conditional_wrappers.is_empty() {
            template_ir.conditional_child_wrapper_set =
                Some(self.store.push_or_reuse_wrapper_set(conditional_wrappers));
        }

        Ok(self.store.push_template(template_ir))
    }

    fn text(
        &mut self,
        value: &OwnedFoldedString,
    ) -> Result<TemplateIrNodeId, ExpressionParseError> {
        let pieces = match value {
            OwnedFoldedString::Text(text) => return Ok(self.text_node(text)),
            OwnedFoldedString::Pieces(pieces) => pieces,
        };

        // WHY: runs without resource or site-root anchors fold back to plain text, so only
        //      structural runs pay for a dynamic constant splice.
        if pieces
            .iter()
            .all(|piece| matches!(piece, OwnedFoldedStringPiece::Text(_)))
        {
            let mut text = String::new();
            for piece in pieces {
                if let OwnedFoldedStringPiece::Text(part) = piece {
                    text.push_str(part);
                }
            }
            return Ok(self.text_node(&text));
        }

        let mut projected = Vec::with_capacity(pieces.len());
        for piece in pieces {
            projected.push(match piece {
                OwnedFoldedStringPiece::Text(text) => {
                    ConstStringPiece::Text(self.string_table.intern(text))
                }
                OwnedFoldedStringPiece::Resource(origin) => {
                    ConstStringPiece::Resource(self.resource(origin)?)
                }
                OwnedFoldedStringPiece::SiteRoot => ConstStringPiece::SiteRoot,
            });
        }

        let expression = structural_string(
            self.expression_builder,
            self.const_values,
            projected,
            self.span,
            &ValueMode::ImmutableReference,
        )?;
        let site_id = self.store.next_expression_site_id();
        Ok(self.store.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::DynamicExpression {
                expression: expression.id(),
                origin: TemplateSegmentOrigin::Head,
                site_id,
            },
            self.span,
        )))
    }

    fn text_node(&mut self, text: &str) -> TemplateIrNodeId {
        let text_id = self.string_table.intern(text);
        self.store.push_node(TemplateIrNode::new(
            TemplateIrNodeKind::Text {
                text: text_id,
                byte_len: text.len(),
                origin: TemplateSegmentOrigin::Head,
            },
            self.span,
        ))
    }

    fn slot(
        &mut self,
        slot: &PublicConstTemplateSlot,
    ) -> Result<TirSlotPlaceholder, ExpressionParseError> {
        let applied = self.wrappers(&slot.applied_child_wrappers)?;
        let child = self.wrappers(&slot.child_wrappers)?;
        let applied_set =
            (!applied.is_empty()).then(|| self.store.push_or_reuse_wrapper_set(applied));
        let child_set = (!child.is_empty()).then(|| self.store.push_or_reuse_wrapper_set(child));

        Ok(TirSlotPlaceholder::with_wrapper_sets(
            slot_key(&slot.key, self.string_table),
            self.store.next_slot_occurrence_id(),
            applied_set,
            child_set,
            slot.skip_parent_child_wrappers,
        ))
    }

    fn wrappers(
        &mut self,
        wrappers: &[PublicConstTemplate],
    ) -> Result<Vec<TemplateWrapperReference>, ExpressionParseError> {
        let mut references = Vec::with_capacity(wrappers.len());
        for wrapper in wrappers {
            let root = self.template(wrapper)?;
            references.push(TemplateWrapperReference::new(
                root,
                TemplateTirPhase::Finalized,
                TemplateViewContext::default(),
            ));
        }
        Ok(references)
    }

    fn resource(
        &mut self,
        origin: &StableResourceOriginId,
    ) -> Result<ResourceId, ExpressionParseError> {
        let resources = self.module_resources.ok_or_else(|| {
            CompilerError::compiler_error(
                "Imported structural string reference requires module file-value resolution services",
            )
        })?;
        Ok(resources
            .borrow_mut()
            .intern_origin(origin.clone(), self.span))
    }
}

fn slot_key(key: &PublicTemplateSlotKey, string_table: &mut StringTable) -> SlotKey {
    match key {
        PublicTemplateSlotKey::Default => SlotKey::Default,
        PublicTemplateSlotKey::Named(name) => SlotKey::Named(string_table.intern(name)),
        PublicTemplateSlotKey::Positional(position) => SlotKey::Positional(*position),
    }
}
