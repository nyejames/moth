use crate::compiler_frontend::analysis::borrow_checker::{
    BorrowDropSite, BorrowDropSiteKind, BorrowFacts,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::hir::blocks::{HirBlock, HirLocal};
use crate::compiler_frontend::hir::expressions::{HirExpression, HirExpressionKind, ValueKind};
use crate::compiler_frontend::hir::functions::{HirFunction, HirFunctionOrigin};
use crate::compiler_frontend::hir::ids::{
    BlockId, FunctionId, HirNodeId, HirValueId, LocalId, RegionId,
};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::places::HirPlace;
use crate::compiler_frontend::hir::regions::HirRegion;
use crate::compiler_frontend::hir::statements::{HirStatement, HirStatementKind};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::ast_fixture_support::test_source_location;
use crate::compiler_frontend::tokenizer::tokens::SourceLocation;

#[derive(Clone, Copy)]
pub(crate) struct TypeIds {
    pub unit: TypeId,
    pub int: TypeId,
    pub boolean: TypeId,
    pub string: TypeId,
}

pub(crate) fn loc(line: i32) -> SourceLocation {
    test_source_location(line)
}

pub(crate) fn build_type_environment() -> (TypeEnvironment, TypeIds) {
    let env = TypeEnvironment::new();
    let builtins = env.builtins();

    let unit = builtins.none;
    let int = builtins.int;
    let boolean = builtins.bool;
    let string = builtins.string;

    (
        env,
        TypeIds {
            unit,
            int,
            boolean,
            string,
        },
    )
}

pub(crate) fn expression(
    id: u32,
    kind: HirExpressionKind,
    ty: TypeId,
    region: RegionId,
    value_kind: ValueKind,
) -> HirExpression {
    HirExpression {
        id: HirValueId(id),
        kind,
        ty,
        value_kind,
        region,
    }
}

pub(crate) fn unit_expression(id: u32, ty: TypeId, region: RegionId) -> HirExpression {
    expression(
        id,
        HirExpressionKind::TupleConstruct { elements: vec![] },
        ty,
        region,
        ValueKind::Const,
    )
}

pub(crate) fn int_expression(id: u32, value: i32, ty: TypeId, region: RegionId) -> HirExpression {
    expression(
        id,
        HirExpressionKind::Int(value),
        ty,
        region,
        ValueKind::Const,
    )
}

pub(crate) fn bool_expression(id: u32, value: bool, ty: TypeId, region: RegionId) -> HirExpression {
    expression(
        id,
        HirExpressionKind::Bool(value),
        ty,
        region,
        ValueKind::Const,
    )
}

pub(crate) fn string_expression(
    id: u32,
    value: &str,
    ty: TypeId,
    region: RegionId,
) -> HirExpression {
    expression(
        id,
        HirExpressionKind::StringLiteral(value.to_owned()),
        ty,
        region,
        ValueKind::Const,
    )
}

pub(crate) fn statement(id: u32, kind: HirStatementKind, line: i32) -> HirStatement {
    HirStatement {
        id: HirNodeId(id),
        kind,
        location: loc(line),
    }
}

pub(crate) fn local(local_id: u32, ty: TypeId, region: RegionId) -> HirLocal {
    HirLocal {
        id: LocalId(local_id),
        ty,
        mutable: true,
        region,
        source_info: Some(loc(1)),
    }
}

pub(crate) fn build_module(
    string_table: &mut StringTable,
    functions: Vec<(HirFunction, InternedPath, HirFunctionOrigin)>,
    blocks: Vec<HirBlock>,
    start_function: FunctionId,
) -> HirModule {
    let mut module = HirModule::new();
    module.functions = functions
        .iter()
        .map(|(function, _, _)| function.clone())
        .collect();
    module.blocks = blocks;
    module.start_function = Some(start_function);

    let mut max_region_id = 0u32;
    for block in &module.blocks {
        max_region_id = max_region_id.max(block.region.0);
    }

    module.regions = (0..=max_region_id)
        .map(|region_id| {
            let parent = (region_id != 0).then_some(RegionId(0));
            HirRegion::lexical(RegionId(region_id), parent)
        })
        .collect();

    for (function, path, origin) in functions {
        module.side_table.bind_function_name(function.id, path);
        module.function_origins.insert(function.id, origin);
    }

    for block in &module.blocks {
        for local in &block.locals {
            let local_path =
                InternedPath::from_single_str(&format!("local_{}", local.id.0), string_table);
            module.side_table.bind_local_name(local.id, local_path);
        }
    }

    module
}

pub(crate) fn default_borrow_facts() -> BorrowFacts {
    BorrowFacts::default()
}

pub(crate) fn borrow_facts_with_drop_site(
    block: BlockId,
    kind: BorrowDropSiteKind,
    locals: Vec<LocalId>,
) -> BorrowFacts {
    let mut facts = BorrowFacts::default();
    facts
        .advisory_drop_sites
        .insert(block, vec![BorrowDropSite { kind, locals }]);
    facts
}

pub(crate) fn load_local(
    id: u32,
    local_id: LocalId,
    ty: TypeId,
    region: RegionId,
) -> HirExpression {
    expression(
        id,
        HirExpressionKind::Load(HirPlace::Local(local_id)),
        ty,
        region,
        ValueKind::Place,
    )
}
