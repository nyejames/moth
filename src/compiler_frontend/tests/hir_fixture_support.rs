//! HIR fixture support for frontend and backend unit tests.
//!
//! WHAT: wraps synthetic AST-to-HIR lowering, and constructs HIR expression, statement and
//!       local nodes directly for tests that start below lowering.
//! WHY: these helpers sit at the HIR boundary and must not depend on borrow validation. HIR is
//!      the first backend-facing semantic IR, so node construction is language-owned rather than
//!      target-owned and both backends consume these constructors instead of copying them.

use crate::compiler_frontend::ast::expressions::expression_store::{AstExpressionBuilder, ExprId};
use crate::compiler_frontend::ast::generic_functions::{
    ModuleMaterialisationPreparationBuilder, registered_type_projection,
};
use crate::compiler_frontend::ast::templates::template::Template;
use crate::compiler_frontend::ast::templates::template::{Style, TemplateType};
use crate::compiler_frontend::ast::templates::tir::{
    TemplateIrBuilder, TemplateIrStore, TemplateIrSummary, TemplateTirPhase, TemplateTirReference,
    TemplateViewContext,
};
use crate::compiler_frontend::ast::{Ast, AstBuildResult};
use crate::compiler_frontend::canonical_type_identity::CanonicalTypeProjectionContext;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::hir::blocks::HirLocal;
use crate::compiler_frontend::hir::expression_store::HirExpressionStore;
use crate::compiler_frontend::hir::expressions::{HirExpression, HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::functions::HirFunctionOriginLookup;
use crate::compiler_frontend::hir::hir_builder::lower_module;
use crate::compiler_frontend::hir::ids::{HirNodeId, HirValueId, LocalId, RegionId};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::statements::{HirStatement, HirStatementKind};
use crate::compiler_frontend::paths::module_resources::ModuleResourceTable;
use crate::compiler_frontend::semantic_identity::{
    ModuleRootRole, StableModuleOriginIdentity, StablePackageIdentity,
};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::ast_fixture_support::test_source_location;
use crate::compiler_frontend::tests::external_package_support::default_external_package_registry;
use crate::compiler_frontend::tests::parse_support::parse_single_file_ast_build_result;
use crate::compiler_frontend::value_mode::ValueMode;
use moth_lexical::numeric::fixed_scalar::FixedScalarValue;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

/// One lowered fixture module paired with the environment that owns its semantic type ids.
///
/// WHAT: dereferences to the HIR module so ordinary fixture code keeps treating the result as a
///       module while borrow-validation tests can pass the defining environment.
/// WHY: hand-built and parsed fixtures discard their `TypeEnvironment` at the lowering seam, but
///      type-class queries are only correct against the defining environment.
#[derive(Clone)]
pub(crate) struct LoweredHirFixture {
    pub(crate) hir_module: HirModule,
    pub(crate) type_environment: TypeEnvironment,
}

impl std::ops::Deref for LoweredHirFixture {
    type Target = HirModule;

    fn deref(&self) -> &Self::Target {
        &self.hir_module
    }
}

impl std::ops::DerefMut for LoweredHirFixture {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.hir_module
    }
}

/// A parsed module with the production declaring-type registrations installed before lowering.
/// Tests that deliberately exercise unresolved identities should use `lower_hir` instead.
pub(crate) struct PreparedHirFixture {
    pub(crate) hir: LoweredHirFixture,
    pub(crate) packages: Arc<ExternalPackageRegistry>,
    pub(crate) paths: PathInternerFork,
    pub(crate) strings: StringTable,
    declaring: ModuleMaterialisationPreparationBuilder,
}

impl PreparedHirFixture {
    pub(crate) fn new(source: &str) -> Self {
        let (build, mut paths, mut strings) =
            parse_single_file_ast_build_result(source).expect("fixture should parse");
        let AstBuildResult {
            ast,
            materialisation_context,
            module_resources,
            ..
        } = build;
        let mut declaring =
            materialisation_context.expect("module fixture retains declaring types");
        let origin = StableModuleOriginIdentity::from_portable_path(
            StablePackageIdentity::project_local("prepared-hir-fixture"),
            "main".to_owned(),
            ModuleRootRole::Normal,
        );
        let empty_resources = ModuleResourceTable::new();
        let resources = module_resources
            .as_ref()
            .map(|resources| resources.borrow());
        declaring
            .install_concrete_executable_contracts(
                &ast.const_values,
                &origin,
                &Default::default(),
                &Default::default(),
                resources.as_deref().unwrap_or(&empty_resources),
                &paths,
            )
            .expect("fixture private nominal identities should install before lowering");
        let packages = default_external_package_registry(&mut strings);
        let hir = lower_hir(ast, &mut strings, &mut paths, &packages);
        Self {
            hir,
            packages,
            paths,
            strings,
            declaring,
        }
    }

    pub(crate) fn type_projection(&self) -> CanonicalTypeProjectionContext<'_> {
        registered_type_projection(&self.declaring.context().type_environment, &self.packages)
    }
}

pub(crate) fn lower_hir(
    ast: Ast,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
    external_packages: &ExternalPackageRegistry,
) -> LoweredHirFixture {
    let lowered = lower_hir_result(ast, string_table, path_fork, external_packages);
    LoweredHirFixture {
        hir_module: lowered.hir_module,
        type_environment: lowered.type_environment,
    }
}

/// Lowers one parsed fixture and keeps the paired type environment.
///
/// WHY: backend tests that consume real source need the exact environment the producer lowered
///      against, rather than reconstructing one from HIR node types.
pub(crate) fn lower_hir_result(
    ast: Ast,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
    external_packages: &ExternalPackageRegistry,
) -> crate::compiler_frontend::module_metadata::HirLoweringResult {
    lower_module(
        ast,
        string_table,
        path_fork,
        HirFunctionOriginLookup::default(),
        None,
        Default::default(),
        external_packages,
    )
    .expect("HIR lowering should succeed")
}

/// Builds a malformed raw-template expression for HIR boundary invariant tests.
///
/// AST finalization should replace this shape with an owned runtime handoff. The
/// returned shared store keeps the deliberately unnormalized template's TIR identity valid.
pub(crate) fn raw_template_expression_for_hir_invariant(
    expression_builder: &mut AstExpressionBuilder,
    kind: TemplateType,
    span: Option<SourceSpan>,
    value_mode: ValueMode,
) -> (ExprId, Rc<RefCell<TemplateIrStore>>) {
    let store_handle = Rc::new(RefCell::new(TemplateIrStore::new()));
    let context = TemplateViewContext::default();
    let template_id = {
        let mut store = store_handle.borrow_mut();
        let mut builder = TemplateIrBuilder::new(&mut store);
        let root = builder.push_sequence_node(vec![], span);
        builder.finish_template(
            root,
            Style::default(),
            kind.clone(),
            TemplateIrSummary::empty(),
            span,
        )
    };
    let template = Template {
        tir_reference: TemplateTirReference {
            root: template_id,
            phase: TemplateTirPhase::Parsed,
            context,
        },
        span,
        body_only_directive_span: None,
    };

    let expression = expression_builder
        .template(template, value_mode)
        .expect("raw template invariant fixture must have a valid build-time row");
    (expression.id(), store_handle)
}

// ---------------------------------------------------------------------------
//  Direct HIR node construction
//
//  These append plain HIR rows to the fixture's module store and return the row-position IDs.
//  Both backends consume the same representation, while target-specific fixture setup stays local.
// ---------------------------------------------------------------------------

pub(crate) fn expression(
    kind: HirExpressionKind,
    ty: TypeId,
    region: RegionId,
    value_kind: ValueKind,
    expressions: &mut HirExpressionStore,
) -> HirValueId {
    expressions
        .append_expression(HirExpression {
            kind,
            ty,
            value_kind,
            region,
            span: None,
        })
        .expect("test expression should fit the HIR expression store")
}

pub(crate) fn fixed_scalar_expression(
    value: FixedScalarValue,
    ty: TypeId,
    region: RegionId,
    expressions: &mut HirExpressionStore,
) -> HirValueId {
    expression(
        HirExpressionKind::FixedScalar(value),
        ty,
        region,
        ValueKind::Const,
        expressions,
    )
}

pub(crate) fn bool_expression(
    value: bool,
    ty: TypeId,
    region: RegionId,
    expressions: &mut HirExpressionStore,
) -> HirValueId {
    expression(
        HirExpressionKind::Bool(value),
        ty,
        region,
        ValueKind::Const,
        expressions,
    )
}

pub(crate) fn string_expression(
    value: &str,
    ty: TypeId,
    region: RegionId,
    expressions: &mut HirExpressionStore,
) -> HirValueId {
    expression(
        HirExpressionKind::StringLiteral(value.to_owned()),
        ty,
        region,
        ValueKind::Const,
        expressions,
    )
}

pub(crate) fn statement(id: u32, kind: HirStatementKind, line: i32) -> HirStatement {
    HirStatement {
        id: HirNodeId(id),
        kind,
        span: test_source_location(line),
    }
}

pub(crate) fn local(local_id: u32, ty: TypeId, region: RegionId) -> HirLocal {
    HirLocal {
        id: LocalId(local_id),
        ty,
        mutable: true,
        region,
        span: test_source_location(1),
    }
}
