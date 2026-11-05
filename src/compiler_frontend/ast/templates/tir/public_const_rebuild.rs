//! Reference-time rebuild of imported const-template values into the importing module's TIR.
//!
//! WHAT: a `ConstValuePayload::Template` constant is stored as its stable `PublicConstTemplate`
//!       projection. A reference to that constant in a constant context rebuilds the projection
//!       into the consumer's `TemplateIrStore` and yields a real `Template` expression.
//! WHY: head composition, `$children(..)` and slot routing attach their semantics only to
//!      `ExpressionOperationView::Template` values. A bare `Constant` reference would drop the
//!      wrapper and insert structure, so the reference has to be a template again.

use crate::compiler_frontend::compiler_messages::CompilerStage;
use std::cell::RefCell;

use super::ids::{TemplateIrId, TemplateIrNodeId};
use super::node::{TemplateIr, TemplateIrNode, TemplateIrNodeKind, TirSlotPlaceholder};
use super::overlays::TemplateViewContext;
use super::refs::{TemplateTirReference, TemplateWrapperReference};
use super::store::TemplateIrStore;
use super::summary::summarize_existing_root;
use super::view::TemplateTirPhase;
use crate::compiler_frontend::ast::const_values::store::{
    ConstStringPiece, ConstStringValue, ConstValueMetadata,
};
use crate::compiler_frontend::ast::const_values::{
    ConstValueId, ConstValuePayload, ConstValueStore,
};
use crate::compiler_frontend::ast::expressions::error::ExpressionParseError;
use crate::compiler_frontend::ast::expressions::expression_store::AstExpressionBuilder;
use crate::compiler_frontend::ast::file_value_resolution::structural_string;
use crate::compiler_frontend::ast::templates::template::{
    SlotKey, Style, Template, TemplateSegmentOrigin, TemplateType,
};
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::folded_value::{
    OwnedFoldedString, OwnedFoldedStringPiece, PublicConstTemplate, PublicConstTemplateKind,
    PublicConstTemplatePiece, PublicConstTemplateSlot, PublicTemplateSlotKey,
};
use crate::compiler_frontend::paths::module_resources::{ModuleResourceTable, ResourceId};
use crate::compiler_frontend::paths::resource_identity::StableResourceOriginId;
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::synthetic_interface_provenance::SyntheticInterfaceProvenance;
use crate::compiler_frontend::value_mode::ValueMode;

/// Builds the `Template` expression payload for one imported const-template constant.
///
/// Returns `Ok(None)` when `value` is not a `Template` payload, so callers can keep the plain
/// constant path for every other payload.
///
/// WHAT: rebuilds the public projection once per `ConstValueId` per module, then reuses the
///       memoised TIR root for every later reference.
/// WHY: all references to one constant must share one TIR root, matching the single authored
///      initializer that imported references inline.
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
                Some(self.store.push_or_reuse_wrapper_set(&conditional_wrappers));
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
            (!applied.is_empty()).then(|| self.store.push_or_reuse_wrapper_set(&applied));
        let child_set = (!child.is_empty()).then(|| self.store.push_or_reuse_wrapper_set(&child));
        let default = slot
            .default
            .as_ref()
            .map(|default| self.default(default, &slot.default_provenance))
            .transpose()?;

        Ok(TirSlotPlaceholder::with_wrapper_sets(
            slot_key(&slot.key, self.string_table),
            self.store.next_slot_occurrence_id(),
            default,
            applied_set,
            child_set,
            slot.skip_parent_child_wrappers,
        ))
    }

    /// Rebuilds one imported occurrence default as a local constant String.
    ///
    /// WHAT: interns owned text and resource origins into this module's tables, then inserts the
    /// resolved `String` value into the local constant store with the donor default's
    /// synthetic-interface provenance.
    /// WHY: `TIR` keeps module-local `ConstValueId`s for its fallback path, while the
    /// public projection crosses modules with owned text/pieces and stable provenance member
    /// identities. Rebuilding the value here restores the one TIR representation without
    /// exporting donor IDs, and retaining provenance keeps the fresh local constant from
    /// reporting fewer synthetic-interface dependencies than the donor constant it replaces.
    fn default(
        &mut self,
        default: &OwnedFoldedString,
        provenance: &SyntheticInterfaceProvenance,
    ) -> Result<ConstValueId, ExpressionParseError> {
        let value = match default {
            OwnedFoldedString::Text(text) => ConstStringValue::Text(self.string_table.intern(text)),
            OwnedFoldedString::Pieces(pieces) => {
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
                ConstStringValue::Pieces(projected)
            }
        };

        let mut metadata = ConstValueMetadata::resolved(
            builtin_type_ids::STRING,
            self.span,
            ValueMode::ImmutableOwned,
        );
        metadata.synthetic_interface_provenance = provenance.clone();
        self.const_values
            .insert_value(metadata, ConstValuePayload::String(value))
            .map_err(ExpressionParseError::from)
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
            InfrastructureFailure::internal(CompilerStage::AstConstruction,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compiler_frontend::folded_value::resolve_const_string_value;
    use crate::compiler_frontend::paths::resource_identity::PortableResourcePath;
    use crate::compiler_frontend::semantic_identity::{
        ModuleRootRole, StableModuleOriginIdentity, StablePackageIdentity,
    };
    use crate::compiler_frontend::synthetic_interface_provenance::{
        SyntheticInterfaceClass, SyntheticInterfaceMemberIdentity, SyntheticInterfaceProvenance,
    };
    use std::path::Path;

    fn public_slot_default(default: Option<OwnedFoldedString>) -> PublicConstTemplate {
        public_slot_default_with_provenance(default, SyntheticInterfaceProvenance::empty())
    }

    fn public_slot_default_with_provenance(
        default: Option<OwnedFoldedString>,
        default_provenance: SyntheticInterfaceProvenance,
    ) -> PublicConstTemplate {
        PublicConstTemplate {
            kind: PublicConstTemplateKind::Wrapper,
            pieces: vec![PublicConstTemplatePiece::Slot(PublicConstTemplateSlot {
                key: PublicTemplateSlotKey::Default,
                default,
                default_provenance,
                applied_child_wrappers: Vec::new(),
                child_wrappers: Vec::new(),
                skip_parent_child_wrappers: false,
            })],
            conditional_child_wrappers: Vec::new(),
        }
    }

    fn rebuilt_placeholder(template: &Template, store: &TemplateIrStore) -> TirSlotPlaceholder {
        let template_ir = store
            .get_template(template.tir_reference.root)
            .expect("rebuilt template exists");
        let root = store
            .get_node(template_ir.root)
            .expect("rebuilt root exists");
        let TemplateIrNodeKind::Sequence { children } = &root.kind else {
            panic!("rebuilt const template root must be a sequence");
        };
        let [slot_id] = children.as_slice() else {
            panic!("rebuilt const template must contain exactly the authored slot");
        };
        match &store.get_node(*slot_id).expect("rebuilt slot exists").kind {
            TemplateIrNodeKind::Slot { placeholder } => placeholder.clone(),
            _ => panic!("rebuilt const template child must be a slot"),
        }
    }

    #[test]
    fn rebuilt_owned_default_becomes_local_string_constant() {
        let mut store = TemplateIrStore::new();
        let mut expression_builder = AstExpressionBuilder::new().unwrap();
        let mut const_values = ConstValueStore::default();
        let mut string_table = StringTable::new();
        let value = const_values
            .insert_resolved(
                builtin_type_ids::STRING,
                ConstValuePayload::Template {
                    template: public_slot_default(Some(OwnedFoldedString::Text(
                        "fallback".to_owned(),
                    ))),
                    folded: None,
                },
                None,
            )
            .expect("public template payload inserts");

        let template = rebuild_const_template_reference(
            value,
            &mut store,
            &mut expression_builder,
            &mut const_values,
            None,
            &mut string_table,
            None,
        )
        .expect("template payload rebuilds")
        .expect("template payload produces a template expression");
        let placeholder = rebuilt_placeholder(&template, &store);
        let default = placeholder.default.expect("rebuilt slot keeps its default");
        let (value, _) = resolve_const_string_value(&const_values, default)
            .expect("rebuilt default resolves through the local constant store");
        assert_eq!(
            value,
            &ConstStringValue::Text(string_table.intern("fallback"))
        );
    }

    #[test]
    fn rebuilt_structural_default_preserves_piece_identity() {
        let mut store = TemplateIrStore::new();
        let mut expression_builder = AstExpressionBuilder::new().unwrap();
        let mut const_values = ConstValueStore::default();
        let mut string_table = StringTable::new();
        let resources = RefCell::new(ModuleResourceTable::new());
        let origin = StableResourceOriginId::module_owned(
            StableModuleOriginIdentity::from_portable_path(
                StablePackageIdentity::project_local("rebuild-test"),
                "pages".to_owned(),
                ModuleRootRole::Normal,
            ),
            PortableResourcePath::from_relative_logical_path(Path::new("assets/logo.svg"))
                .expect("test resource path is portable"),
        );
        let value = const_values
            .insert_resolved(
                builtin_type_ids::STRING,
                ConstValuePayload::Template {
                    template: public_slot_default(Some(OwnedFoldedString::Pieces(vec![
                        OwnedFoldedStringPiece::Text("open".to_owned()),
                        OwnedFoldedStringPiece::Resource(origin.clone()),
                        OwnedFoldedStringPiece::SiteRoot,
                    ]))),
                    folded: None,
                },
                None,
            )
            .expect("public template payload inserts");

        let template = rebuild_const_template_reference(
            value,
            &mut store,
            &mut expression_builder,
            &mut const_values,
            Some(&resources),
            &mut string_table,
            None,
        )
        .expect("structural template payload rebuilds")
        .expect("template payload produces a template expression");
        let placeholder = rebuilt_placeholder(&template, &store);
        let default = placeholder.default.expect("rebuilt slot keeps its default");
        let (value, _) = resolve_const_string_value(&const_values, default)
            .expect("rebuilt structural default resolves through the local store");
        let ConstStringValue::Pieces(pieces) = value else {
            panic!("structural default must stay piece-bearing");
        };
        assert!(matches!(
            pieces.as_slice(),
            [
                ConstStringPiece::Text(_),
                ConstStringPiece::Resource(_),
                ConstStringPiece::SiteRoot
            ]
        ));
    }

    #[test]
    fn rebuilt_default_retains_synthetic_interface_provenance() {
        let mut store = TemplateIrStore::new();
        let mut expression_builder = AstExpressionBuilder::new().unwrap();
        let mut const_values = ConstValueStore::default();
        let mut string_table = StringTable::new();
        let member = SyntheticInterfaceMemberIdentity::new(
            SyntheticInterfaceClass::ProjectContext,
            "source-config",
            "site_name",
        );
        let provenance = SyntheticInterfaceProvenance::single(member.clone());
        let value = const_values
            .insert_resolved(
                builtin_type_ids::STRING,
                ConstValuePayload::Template {
                    template: public_slot_default_with_provenance(
                        Some(OwnedFoldedString::Text("fallback".to_owned())),
                        provenance,
                    ),
                    folded: None,
                },
                None,
            )
            .expect("public template payload inserts");

        let template = rebuild_const_template_reference(
            value,
            &mut store,
            &mut expression_builder,
            &mut const_values,
            None,
            &mut string_table,
            None,
        )
        .expect("template payload rebuilds")
        .expect("template payload produces a template expression");
        let placeholder = rebuilt_placeholder(&template, &store);
        let default = placeholder.default.expect("rebuilt slot keeps its default");
        let (_, resolved) = resolve_const_string_value(&const_values, default)
            .expect("rebuilt default resolves through the local constant store");
        assert_eq!(
            resolved.members(),
            &[member],
            "the rebuilt local default must retain the donor constant's stable provenance"
        );
    }

    #[test]
    fn rebuilt_explicit_empty_default_stays_present() {
        let mut store = TemplateIrStore::new();
        let mut expression_builder = AstExpressionBuilder::new().unwrap();
        let mut const_values = ConstValueStore::default();
        let mut string_table = StringTable::new();
        let value = const_values
            .insert_resolved(
                builtin_type_ids::STRING,
                ConstValuePayload::Template {
                    template: public_slot_default(Some(OwnedFoldedString::Text(String::new()))),
                    folded: None,
                },
                None,
            )
            .expect("public template payload inserts");

        let template = rebuild_const_template_reference(
            value,
            &mut store,
            &mut expression_builder,
            &mut const_values,
            None,
            &mut string_table,
            None,
        )
        .expect("template payload rebuilds")
        .expect("template payload produces a template expression");
        let placeholder = rebuilt_placeholder(&template, &store);
        let default = placeholder
            .default
            .expect("an authored empty default stays present rather than becoming absent");
        let (value, provenance) = resolve_const_string_value(&const_values, default)
            .expect("rebuilt empty default resolves through the local constant store");
        assert_eq!(value, &ConstStringValue::Text(string_table.intern("")));
        assert!(provenance.is_empty());
    }
}
