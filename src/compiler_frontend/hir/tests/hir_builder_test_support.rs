//! Shared HIR builder test hooks and validation helpers.
//!
//! WHAT: exposes extra builder utilities needed only by HIR unit tests.
//! WHY: tests need direct access to internal builder state without widening the production API.

use crate::compiler_frontend::ast::ast_nodes::{Declaration, SourceLocation};
use crate::compiler_frontend::ast::const_values::store::{ConstStringPiece, ConstStringValue};
use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::ast::templates::{
    OwnedRuntimeTemplateBody, OwnedRuntimeTemplateHandoff, OwnedRuntimeTemplateNode,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::datatypes::definitions::{
    ChoiceTypeDefinition, ChoiceVariantDefinition, ChoiceVariantPayloadDefinition, FieldDefinition,
    StructTypeDefinition,
};
use crate::compiler_frontend::datatypes::ids::{NominalTypeId, TypeId, TypeId as FrontendTypeId};
use crate::compiler_frontend::declaration_syntax::choice::ChoiceVariantPayload;
use crate::compiler_frontend::folded_value::{
    OwnedFoldedString, OwnedFoldedStringPiece, owned_folded_string_from_const_string,
};
use crate::compiler_frontend::hir::blocks::{HirBlock, HirLocal};
use crate::compiler_frontend::hir::functions::HirFunction;
use crate::compiler_frontend::hir::hir_builder::HirBuilder;
use crate::compiler_frontend::hir::hir_side_table::HirLocalOriginKind;
use crate::compiler_frontend::hir::ids::{
    BlockId, FieldId, FunctionId, LocalId, RegionId, StructId,
};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::structs::{HirField, HirStruct};
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::paths::module_resources::{ModuleResourceTable, ResourceId};
use crate::compiler_frontend::paths::resource_identity::{
    PortableResourcePath, StableResourceOriginId,
};
use crate::compiler_frontend::semantic_identity::{
    ModuleRootRole, StableModuleOriginIdentity, StablePackageIdentity,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::ast_fixture_support::test_source_location;
use crate::compiler_frontend::value_mode::ValueMode;
use std::path::Path;

// Re-export TypeId-first AST construction helpers from the bridge module so existing
// HIR test imports continue to work without mentioning parse-era type syntax.
pub(crate) use crate::compiler_frontend::tests::type_id_fixture_support::{
    HirTestChoiceDefinition, assert_no_placeholder_terminators, build_ast_with_choices,
    build_ast_with_registered_types, lower_ast, lower_ast_with_metadata,
};

pub(crate) fn validate_module_for_tests(
    module: &HirModule,
    _string_table: &StringTable,
    type_environment: &crate::compiler_frontend::datatypes::environment::TypeEnvironment,
) -> Result<(), CompilerError> {
    super::validate_hir_module(module, type_environment)
}

fn advance_counter_past(next_counter: &mut u32, used_id: u32) {
    *next_counter = (*next_counter).max(used_id.saturating_add(1));
}

impl<'a> HirBuilder<'a> {
    fn reserve_block_id(&mut self, block_id: BlockId) {
        advance_counter_past(&mut self.next_block_id, block_id.0);
    }

    fn reserve_region_id(&mut self, region_id: RegionId) {
        advance_counter_past(&mut self.next_region_id, region_id.0);
    }

    fn reserve_local_id(&mut self, local_id: LocalId) {
        advance_counter_past(&mut self.next_local_id, local_id.0);
    }

    fn reserve_function_id(&mut self, function_id: FunctionId) {
        advance_counter_past(&mut self.next_function_id, function_id.0);
    }

    fn reserve_struct_id(&mut self, struct_id: StructId) {
        advance_counter_past(&mut self.next_struct_id, struct_id.0);
    }

    fn reserve_field_id(&mut self, field_id: FieldId) {
        advance_counter_past(&mut self.next_field_id, field_id.0);
    }

    fn current_block_id(&self) -> Option<BlockId> {
        self.current_block
    }

    fn set_current_function_for_tests(&mut self, function_id: FunctionId) {
        self.current_function = Some(function_id);
    }

    fn set_current_block_for_tests(&mut self, block_id: BlockId) {
        self.current_block = Some(block_id);
    }

    fn set_current_region_for_tests(&mut self, region: RegionId) {
        self.current_region = Some(region);
    }

    pub(crate) fn test_push_block(&mut self, block: HirBlock) {
        self.reserve_block_id(block.id);
        self.reserve_region_id(block.region);
        self.push_block(block);
    }

    pub(crate) fn test_set_current_region(&mut self, region: RegionId) {
        self.set_current_region_for_tests(region);
    }

    pub(crate) fn test_set_current_block(&mut self, block_id: BlockId) {
        self.set_current_block_for_tests(block_id);
    }

    pub(crate) fn test_current_block_statements(
        &self,
    ) -> &[crate::compiler_frontend::hir::statements::HirStatement] {
        let block_id = self.current_block_id().unwrap_or(BlockId(0));
        self.module
            .blocks
            .get(block_id.0 as usize)
            .map(|block| block.statements.as_slice())
            .unwrap_or(&[])
    }

    /// Resolves the builtin `Error` type id if it was registered in the test type environment.
    pub(crate) fn test_builtin_error_type_id(
        &mut self,
    ) -> Option<crate::compiler_frontend::datatypes::ids::TypeId> {
        let error_path = crate::compiler_frontend::builtins::error_type::builtin_error_type_path(
            self.string_table,
        );
        let nominal_id = self.type_environment.nominal_id_for_path(&error_path)?;
        self.type_environment.type_id_for_nominal_id(nominal_id)
    }

    /// Registers the builtin `Error` nominal struct in the test type environment.
    ///
    /// WHAT: adds the canonical `Error { message: String, code: Int }` struct so tests can
    ///       construct fallible return types whose error slot is builtin `Error`.
    pub(crate) fn test_register_builtin_error_type(
        &mut self,
    ) -> crate::compiler_frontend::datatypes::ids::TypeId {
        use crate::compiler_frontend::builtins::error_type::{
            ERROR_FIELD_CODE, ERROR_FIELD_MESSAGE,
        };
        use crate::compiler_frontend::datatypes::definitions::{
            FieldDefinition, StructTypeDefinition,
        };
        use crate::compiler_frontend::datatypes::ids::NominalTypeId;
        use crate::compiler_frontend::tokenizer::tokens::SourceLocation;

        if let Some(existing) = self.test_builtin_error_type_id() {
            return existing;
        }

        let error_path = crate::compiler_frontend::builtins::error_type::builtin_error_type_path(
            self.string_table,
        );
        let message_path = error_path.join_str(ERROR_FIELD_MESSAGE, self.string_table);
        let code_path = error_path.join_str(ERROR_FIELD_CODE, self.string_table);

        let definition = StructTypeDefinition {
            id: NominalTypeId(0),
            path: error_path,
            fields: vec![
                FieldDefinition {
                    name: message_path,
                    type_id: crate::compiler_frontend::datatypes::ids::builtin_type_ids::STRING,
                    location: SourceLocation::default(),
                },
                FieldDefinition {
                    name: code_path,
                    type_id: crate::compiler_frontend::datatypes::ids::builtin_type_ids::INT,
                    location: SourceLocation::default(),
                },
            ]
            .into_boxed_slice(),
            generic_parameters: None,
            const_record: false,
        };

        let (_, error_type_id) = self.type_environment.register_nominal_struct(definition);
        error_type_id
    }

    pub(crate) fn test_set_current_function(&mut self, function_id: FunctionId) {
        self.set_current_function_for_tests(function_id);
    }

    pub(crate) fn test_register_local_in_block(&mut self, local: HirLocal, name: InternedPath) {
        let current_block = self.current_block_id().unwrap_or(BlockId(0));
        let _ =
            self.register_local_in_block(current_block, local.clone(), &SourceLocation::default());

        self.locals_by_name.insert(name.clone(), local.id);
        self.side_table.bind_local_name(local.id, name);
        self.side_table
            .bind_local_origin(local.id, HirLocalOriginKind::User, None, None);
        self.side_table.map_local_source(&local);
        self.reserve_local_id(local.id);
    }

    pub(crate) fn test_register_function_name(&mut self, name: InternedPath, id: FunctionId) {
        self.functions_by_name.insert(name.clone(), id);
        self.side_table.bind_function_name(id, name);
        self.reserve_function_id(id);
    }

    pub(crate) fn test_register_function_with_return_type(
        &mut self,
        name: InternedPath,
        id: FunctionId,
        return_type: crate::compiler_frontend::datatypes::ids::TypeId,
    ) {
        self.test_register_function_name(name, id);

        let entry = self.current_block_id().unwrap_or(BlockId(0));
        self.push_function(HirFunction {
            id,
            entry,
            params: vec![],
            return_type,
        });
    }

    pub(crate) fn test_register_struct_with_fields(
        &mut self,
        struct_id: StructId,
        name: InternedPath,
        frontend_type_id: crate::compiler_frontend::datatypes::ids::TypeId,
        fields: Vec<(
            FieldId,
            InternedPath,
            crate::compiler_frontend::datatypes::ids::TypeId,
        )>,
    ) {
        self.structs_by_name.insert(name.clone(), struct_id);
        self.side_table.bind_struct_name(struct_id, name);

        let mut hir_fields = Vec::with_capacity(fields.len());
        for (field_id, field_name, ty) in fields {
            self.fields_by_struct_and_name
                .insert((struct_id, field_name.clone()), field_id);
            self.side_table.bind_field_name(field_id, field_name);
            hir_fields.push(HirField { id: field_id, ty });
            self.reserve_field_id(field_id);
        }

        self.push_struct(HirStruct {
            id: struct_id,
            frontend_type_id,
            fields: hir_fields,
        });
        self.reserve_struct_id(struct_id);
    }

    /// Register one already-folded module constant in the store the builder lowers from.
    ///
    /// WHY: production module constants reach HIR as folded store values, never as expression
    /// trees, so a test constant must be a value the store can hold.
    pub(crate) fn test_register_module_constant(&mut self, name: InternedPath, value: Expression) {
        let declaration = Declaration {
            id: name.clone(),
            value,
        };
        self.module_const_values
            .insert_test_declaration(declaration, &self.type_environment);
        let value_id = self
            .module_const_values
            .value_for_path(&name)
            .expect("test module constant should be indexed");
        self.module_constants_by_name.insert(name, value_id);
    }

    pub(crate) fn test_register_nominal_struct_type(
        &mut self,
        path: InternedPath,
        fields: Vec<(InternedPath, FrontendTypeId, SourceLocation)>,
        const_record: bool,
    ) -> FrontendTypeId {
        let field_definitions = fields
            .into_iter()
            .map(|(name, type_id, location)| FieldDefinition {
                name,
                type_id,
                location,
            })
            .collect::<Vec<_>>();

        let definition = StructTypeDefinition {
            id: NominalTypeId(0),
            path,
            fields: field_definitions.into_boxed_slice(),
            generic_parameters: None,
            const_record,
        };

        let (_, type_id) = self.type_environment.register_nominal_struct(definition);
        type_id
    }

    pub(crate) fn test_register_nominal_choice_type(
        &mut self,
        path: InternedPath,
        variants: &[crate::compiler_frontend::declaration_syntax::choice::ChoiceVariant],
    ) -> FrontendTypeId {
        let variant_definitions = variants
            .iter()
            .enumerate()
            .map(|(tag, variant)| ChoiceVariantDefinition {
                name: variant.id,
                tag,
                payload: match &variant.payload {
                    ChoiceVariantPayload::Unit => ChoiceVariantPayloadDefinition::Unit,
                    ChoiceVariantPayload::Record { fields } => {
                        let field_definitions = fields
                            .iter()
                            .map(|field| FieldDefinition {
                                name: field.id.clone(),
                                type_id: field.value.type_id,
                                location: field.value.location.clone(),
                            })
                            .collect::<Vec<_>>();
                        ChoiceVariantPayloadDefinition::Record {
                            fields: field_definitions.into_boxed_slice(),
                        }
                    }
                },
                location: variant.location.clone(),
            })
            .collect::<Vec<_>>();

        let definition = ChoiceTypeDefinition {
            id: NominalTypeId(0),
            path,
            variants: variant_definitions.into_boxed_slice(),
            generic_parameters: None,
        };

        let (_, type_id) = self.type_environment.register_nominal_choice(definition);
        type_id
    }
}

// ---------------------------------------------------------------------------
//  Shared HIR expression fixtures
//
//  These are consumed by several sibling HIR test modules, so they are owned here rather than
//  by whichever test module needed them first.
// ---------------------------------------------------------------------------

pub(crate) fn setup_builder(string_table: &'_ mut StringTable) -> HirBuilder<'_> {
    let test_function_name = InternedPath::from_single_str("__expr_test_fn", string_table);
    let mut builder = HirBuilder::new(
        string_table,
        crate::compiler_frontend::datatypes::environment::TypeEnvironment::new(),
        crate::compiler_frontend::hir::functions::HirFunctionOriginLookup::default(),
    );

    let region = RegionId(0);
    let function_id = FunctionId(0);
    let block = HirBlock {
        id: BlockId(0),
        region,
        locals: vec![],
        statements: vec![],
        terminator: HirTerminator::Uninitialized,
    };

    builder.test_push_block(block);
    builder.test_set_current_region(region);
    builder.test_set_current_block(BlockId(0));
    builder.test_register_function_name(test_function_name, function_id);
    builder.test_set_current_function(function_id);
    builder.module.start_function = Some(function_id);

    builder
}

pub(crate) fn register_local(
    builder: &mut HirBuilder<'_>,
    name: InternedPath,
    local_id: LocalId,
    type_id: TypeId,
    location: SourceLocation,
) {
    let ty = type_id;
    builder.test_register_local_in_block(
        HirLocal {
            id: local_id,
            ty,
            mutable: true,
            region: RegionId(0),
            source_info: Some(location),
        },
        name,
    );
}

/// Builds the neutral render node used by HIR expression fixtures.
///
/// WHAT: delegates to the resource-capable mapper with an empty resource table.
/// WHY: fixture content built through this lane carries no resource-bearing piece; a
///      `Resource` piece against the empty table fails the fixture loudly instead of
///      silently rerouting the structural string, mirroring the handoff's absent-table rule.
pub(crate) fn expressions_to_owned_render_node(
    expressions: &[Expression],
    string_table: &StringTable,
) -> OwnedRuntimeTemplateNode {
    expressions_to_owned_render_node_with_resources(
        expressions,
        string_table,
        &ModuleResourceTable::new(),
    )
}

/// Builds the neutral render node used by HIR expression fixtures against one resource table.
///
/// WHAT: maps literal strings to `Text`, maps runtime `StructuralString` expressions to
///       piece-bearing `Text` nodes through the single shared owned folded-string converter,
///       and maps every other expression to `DynamicExpression`, preserving source locations.
/// WHY: the runtime handoff materializes a structural string as
///      `Text { text: OwnedFoldedString::Pieces(..) }` (see `handoff_materialization`), so HIR
///      fixtures must construct exactly that shape instead of a rerouted dynamic expression.
pub(crate) fn expressions_to_owned_render_node_with_resources(
    expressions: &[Expression],
    string_table: &StringTable,
    resources: &ModuleResourceTable,
) -> OwnedRuntimeTemplateNode {
    let children: Vec<OwnedRuntimeTemplateNode> = expressions
        .iter()
        .map(|expression| expression_to_owned_node(expression, string_table, resources))
        .collect();

    OwnedRuntimeTemplateNode::Sequence { children }
}

fn expression_to_owned_node(
    expression: &Expression,
    string_table: &StringTable,
    resources: &ModuleResourceTable,
) -> OwnedRuntimeTemplateNode {
    match &expression.kind {
        ExpressionKind::StringSlice(text) => OwnedRuntimeTemplateNode::Text {
            text: OwnedFoldedString::Text(string_table.resolve(*text).to_owned()),
            reactive_subscription: None,
            location: expression.location.to_owned(),
        },

        // WHAT: mirrors the runtime handoff: a structural string converts to a piece-bearing
        //       text node through the one shared owned folded-string helper.
        // WHY: a fixture must not flatten resource or site-root pieces into rendered text, and
        //      must not reroute the node into `DynamicExpression`.
        ExpressionKind::StructuralString { pieces } => {
            let value = ConstStringValue::Pieces(pieces.clone());

            let text = owned_folded_string_from_const_string(&value, resources, string_table)
                .expect("fixture structural string must convert against the resource table that issued its resource handles");

            OwnedRuntimeTemplateNode::Text {
                text,
                reactive_subscription: None,
                location: expression.location.to_owned(),
            }
        }

        _ => OwnedRuntimeTemplateNode::DynamicExpression {
            expression: Box::new(expression.clone()),
            reactive_subscription: None,
        },
    }
}

pub(crate) fn runtime_template_expression(
    location: SourceLocation,
    content: Vec<Expression>,
    string_table: &StringTable,
) -> Expression {
    let body = expressions_to_owned_render_node(&content, string_table);
    let handoff = OwnedRuntimeTemplateHandoff {
        body: OwnedRuntimeTemplateBody::Render(body),
        location: location.clone(),
    };

    Expression::runtime_template_handoff(handoff, ValueMode::ImmutableOwned)
}

// ---------------------------------------------------------------------------
//  Fixture mapper regression tests
//
//  These pin the mapper itself so its output cannot silently drift away from the node
//  shapes the runtime handoff materializes.
// ---------------------------------------------------------------------------

pub(crate) fn fixture_resource(
    resources: &mut ModuleResourceTable,
    relative_path: &str,
) -> (ResourceId, StableResourceOriginId) {
    let origin = StableResourceOriginId::module_owned(
        StableModuleOriginIdentity::from_portable_path(
            StablePackageIdentity::project_local("hir-fixture-tests"),
            String::new(),
            ModuleRootRole::Normal,
        ),
        PortableResourcePath::from_relative_logical_path(Path::new(relative_path))
            .expect("fixture resource path should be portable"),
    );

    let resource_id = resources.intern_origin(
        origin.clone(),
        crate::compiler_frontend::tokenizer::tokens::SourceLocation::default(),
    );

    (resource_id, origin)
}

#[test]
fn structural_string_fixture_materializes_a_piece_bearing_text_node() {
    let mut string_table = StringTable::new();
    let mut resources = ModuleResourceTable::new();
    let (resource_id, resource_origin) = fixture_resource(&mut resources, "assets/logo.svg");

    let before = string_table.intern("before");
    let after = string_table.intern("after");
    let location = test_source_location(7);
    let structural = Expression::structural_string(
        vec![
            ConstStringPiece::Text(before),
            ConstStringPiece::Resource(resource_id),
            ConstStringPiece::SiteRoot,
            ConstStringPiece::Text(after),
        ],
        location.clone(),
    );

    let node =
        expressions_to_owned_render_node_with_resources(&[structural], &string_table, &resources);

    let OwnedRuntimeTemplateNode::Sequence { children } = node else {
        panic!("fixture content should map to a sequence node");
    };
    let [single] = children.as_slice() else {
        panic!("one fixture expression should map to one owned node");
    };
    let OwnedRuntimeTemplateNode::Text {
        text,
        reactive_subscription: None,
        location: node_location,
    } = single
    else {
        panic!("structural string fixture should map to a piece-bearing text node, got {single:?}");
    };

    assert_eq!(
        text,
        &OwnedFoldedString::Pieces(vec![
            OwnedFoldedStringPiece::Text("before".to_owned()),
            OwnedFoldedStringPiece::Resource(resource_origin),
            OwnedFoldedStringPiece::SiteRoot,
            OwnedFoldedStringPiece::Text("after".to_owned()),
        ])
    );
    assert_eq!(node_location, &location);
}

#[test]
fn all_text_structural_fixture_keeps_pieces_without_a_resource_table() {
    let mut string_table = StringTable::new();
    let head = string_table.intern("docs/");
    let structural = Expression::structural_string(
        vec![ConstStringPiece::Text(head), ConstStringPiece::SiteRoot],
        test_source_location(8),
    );

    let node = expressions_to_owned_render_node(&[structural], &string_table);

    let OwnedRuntimeTemplateNode::Sequence { children } = node else {
        panic!("fixture content should map to a sequence node");
    };
    let [single] = children.as_slice() else {
        panic!("one fixture expression should map to one owned node");
    };
    let OwnedRuntimeTemplateNode::Text {
        text,
        reactive_subscription: None,
        ..
    } = single
    else {
        panic!("site-root fixture should map to a piece-bearing text node, got {single:?}");
    };

    assert_eq!(
        text,
        &OwnedFoldedString::Pieces(vec![
            OwnedFoldedStringPiece::Text("docs/".to_owned()),
            OwnedFoldedStringPiece::SiteRoot,
        ])
    );
}

#[should_panic(expected = "resource handle 0 is outside a module resource table of 0 origins")]
#[test]
fn resource_piece_without_its_table_fails_instead_of_rerouting() {
    let mut string_table = StringTable::new();
    let mut issuing_resources = ModuleResourceTable::new();
    let (phantom_resource, _) = fixture_resource(&mut issuing_resources, "assets/logo.svg");

    let head = string_table.intern("docs/");
    let structural = Expression::structural_string(
        vec![
            ConstStringPiece::Text(head),
            ConstStringPiece::Resource(phantom_resource),
        ],
        test_source_location(9),
    );

    // The mapper receives a table that issued none of the expression's handles, so the exact
    // try-origin error proves the structural string reached the conversion boundary instead of
    // being silently rerouted to a dynamic expression.
    let _ = expressions_to_owned_render_node(&[structural], &string_table);
}
