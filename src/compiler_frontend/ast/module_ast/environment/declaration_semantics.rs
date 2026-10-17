//! Semantic classification for resolved source declarations.
//!
//! WHAT: records the executable role of each top-level declaration after AST environment
//! construction has resolved signatures and nominal type identity.
//! WHY: body parsing dispatches on resolved declaration roles for callables, nominal constructors,
//!      and values without rediscovering those roles from type spellings.

use crate::compiler_frontend::ast::ast_nodes::{Declaration, DeclarationShell, DeclarationValue};
use crate::compiler_frontend::ast::const_values::resolver::classify_template_from_effective_tir;
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::expressions::expression_store::AstExpressionBuilder;
use crate::compiler_frontend::ast::module_ast::environment::TopLevelDeclarationTable;
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::tir::TemplateIrStore;
use crate::compiler_frontend::ast::type_resolution::ResolvedFunctionSignature;
use crate::compiler_frontend::datatypes::definitions::TypeDefinition;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::symbols::path_interner::PathId;
// PathId is the build-lifetime key used by every resolved AST environment table.
use rustc_hash::FxHashMap;
use std::cell::RefCell;
use std::rc::Rc;

/// Source-level role assigned to a resolved top-level declaration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DeclarationSemanticKind {
    Function,
    Struct,
    Choice,
    Constant,
    Value,
}

/// Borrowed inputs one declaration classification pass reads.
///
/// WHAT: keeps the resolved signature/type maps beside the phase owners that resolve one
///       initializer's template and constant content.
/// WHY: classification reads the same seven inputs for every declaration, so one bundle keeps
///      the entry points small without inventing per-declaration state.
pub(crate) struct DeclarationSemanticInputs<'a> {
    pub(crate) resolved_function_signatures_by_path:
        &'a FxHashMap<PathId, ResolvedFunctionSignature>,
    pub(crate) nominal_type_ids_by_path: &'a FxHashMap<PathId, TypeId>,
    pub(crate) type_environment: &'a TypeEnvironment,
    pub(crate) template_ir_store: &'a Rc<RefCell<TemplateIrStore>>,
    pub(crate) expression_builder: &'a AstExpressionBuilder,
    pub(crate) const_values: &'a ConstValueStore,
    pub(crate) external_package_registry: Option<&'a ExternalPackageRegistry>,
}

/// Immutable path-indexed classification table shared by body emission contexts.
#[derive(Clone, Debug)]
pub(crate) struct DeclarationSemanticTable {
    by_path: FxHashMap<PathId, DeclarationSemanticKind>,
}

impl DeclarationSemanticTable {
    /// Build the semantic table from the resolved environment.
    ///
    /// WHAT: classifies every top-level declaration by inspecting resolved
    ///       function signatures, nominal type identity, and compile-time
    ///       constness of the initializer expression.
    /// WHY: the table is built after constant resolution and nominal type
    ///      registration, so only the remaining Constant-vs-Value distinction
    ///      needs expression constness classification. Template-valued
    ///      declarations use their exact effective TIR views so module-local
    ///      store, phase, and overlay identity remain authoritative.
    pub(crate) fn from_environment(
        declaration_table: &TopLevelDeclarationTable,
        inputs: DeclarationSemanticInputs<'_>,
    ) -> Result<Self, TemplateError> {
        let mut by_path = FxHashMap::default();

        for declaration in declaration_table.iter() {
            let kind = classify_declaration(declaration, &inputs)?;
            by_path.insert(declaration.id, kind);
        }

        Ok(Self { by_path })
    }

    pub(crate) fn kind_for_path(&self, path: &PathId) -> Option<DeclarationSemanticKind> {
        self.by_path.get(path).copied()
    }

    /// Register a function reconstructed from a closed generated-materialisation artefact.
    ///
    /// Generated environments do not rerun declaration-shell preparation, so their stable
    /// callable blueprints join the ordinary semantic table directly after inverse projection.
    pub(crate) fn register_materialised_function(&mut self, path: PathId) {
        self.by_path.insert(path, DeclarationSemanticKind::Function);
    }

    /// Register a constant reconstructed from the stable generated-materialisation closure.
    pub(crate) fn register_materialised_constant(&mut self, path: PathId) {
        self.by_path.insert(path, DeclarationSemanticKind::Constant);
    }

    /// Register a transparent alias declaration reconstructed from the stable closure.
    pub(crate) fn register_materialised_value(&mut self, path: PathId) {
        self.by_path.insert(path, DeclarationSemanticKind::Value);
    }
}

fn classify_declaration(
    declaration: &Declaration,
    inputs: &DeclarationSemanticInputs<'_>,
) -> Result<DeclarationSemanticKind, TemplateError> {
    let DeclarationSemanticInputs {
        resolved_function_signatures_by_path,
        nominal_type_ids_by_path,
        type_environment,
        template_ir_store,
        expression_builder,
        const_values,
        external_package_registry,
    } = *inputs;
    match &declaration.value {
        DeclarationValue::Shell(shell) => {
            return Ok(match shell {
                DeclarationShell::Function { .. } => DeclarationSemanticKind::Function,
                DeclarationShell::Struct => DeclarationSemanticKind::Struct,
                DeclarationShell::Choice => DeclarationSemanticKind::Choice,
                DeclarationShell::TypeAlias => DeclarationSemanticKind::Value,
            });
        }
        DeclarationValue::PendingConstant => return Ok(DeclarationSemanticKind::Constant),
        DeclarationValue::StructDefinition(_) => return Ok(DeclarationSemanticKind::Value),
        DeclarationValue::Default(_) => return Ok(DeclarationSemanticKind::Constant),
        DeclarationValue::Absent | DeclarationValue::Initializer(_) => {}
    }

    if resolved_function_signatures_by_path.contains_key(&declaration.id) {
        return Ok(DeclarationSemanticKind::Function);
    }

    if let Some(type_id) = nominal_type_ids_by_path.get(&declaration.id) {
        return Ok(match type_environment.get(*type_id) {
            Some(TypeDefinition::Struct(..)) => DeclarationSemanticKind::Struct,
            Some(TypeDefinition::Choice(..)) => DeclarationSemanticKind::Choice,
            _ => DeclarationSemanticKind::Value,
        });
    }

    let Some(initializer) = declaration.value.initializer() else {
        return Ok(DeclarationSemanticKind::Value);
    };
    let initializer = expression_builder
        .view(initializer)
        .map_err(TemplateError::from)?;
    let value_is_compile_time_constant = initializer
        .const_value_kind_with_template_classifier(const_values, &mut |template| {
            classify_template_from_effective_tir(
                template,
                template_ir_store,
                expression_builder,
                const_values,
                external_package_registry,
            )
        })?
        .is_compile_time_value();

    if value_is_compile_time_constant {
        Ok(DeclarationSemanticKind::Constant)
    } else {
        Ok(DeclarationSemanticKind::Value)
    }
}
