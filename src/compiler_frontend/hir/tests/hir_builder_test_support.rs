//! Shared HIR builder test hooks and validation helpers.
//!
//! WHAT: exposes extra builder utilities needed only by HIR unit tests.
//! WHY: tests need direct access to internal builder state without widening the production API.

use crate::compiler_frontend::ast::ast_nodes::{Declaration, SourceLocation};
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
use crate::compiler_frontend::paths::path_format::PathStringFormatConfig;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::value_mode::ValueMode;

// Re-export TypeId-first AST construction helpers from the bridge module so existing
// HIR test imports continue to work without mentioning parse-era type syntax.
pub(crate) use crate::compiler_frontend::tests::type_id_fixture_support::{
    HirTestChoiceDefinition, assert_no_placeholder_terminators, build_ast, build_ast_with_choices,
    lower_ast, lower_ast_with_metadata,
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

    pub(crate) fn test_register_module_constant(&mut self, name: InternedPath, value: Expression) {
        self.module_constants_by_name
            .insert(name.to_owned(), Declaration { id: name, value });
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
        PathStringFormatConfig::default(),
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
/// WHAT: maps literal strings to `Text`, maps other expressions to
///       `DynamicExpression`, and preserves their source locations.
/// WHY: HIR tests should construct the owned AST/HIR boundary they consume.
pub(crate) fn expressions_to_owned_render_node(
    expressions: &[Expression],
    string_table: &StringTable,
) -> OwnedRuntimeTemplateNode {
    let children: Vec<OwnedRuntimeTemplateNode> = expressions
        .iter()
        .map(|expression| expression_to_owned_node(expression, string_table))
        .collect();

    OwnedRuntimeTemplateNode::Sequence { children }
}

fn expression_to_owned_node(
    expression: &Expression,
    _string_table: &StringTable,
) -> OwnedRuntimeTemplateNode {
    match &expression.kind {
        ExpressionKind::StringSlice(text) => OwnedRuntimeTemplateNode::Text {
            text: *text,
            reactive_subscription: None,
            location: expression.location.to_owned(),
        },
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
