//! Semantic classification for resolved source declarations.
//!
//! WHAT: records the executable role of each top-level declaration after AST environment
//! construction has resolved signatures and nominal type identity.
//! WHY: body parsing needs to decide whether a visible source declaration is callable,
//! constructible, or value-like without inspecting diagnostic-only `DataType` spelling.

use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::const_values::resolver::classify_template_from_effective_tir;
use crate::compiler_frontend::ast::module_ast::environment::TopLevelDeclarationTable;
use crate::compiler_frontend::ast::templates::error::TemplateError;
use crate::compiler_frontend::ast::templates::tir::TemplateIrStore;
use crate::compiler_frontend::ast::type_resolution::ResolvedFunctionSignature;
use crate::compiler_frontend::datatypes::definitions::TypeDefinition;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
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

/// Immutable path-indexed classification table shared by body emission contexts.
#[derive(Clone, Debug)]
pub(crate) struct DeclarationSemanticTable {
    by_path: FxHashMap<InternedPath, DeclarationSemanticKind>,
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
        resolved_function_signatures_by_path: &FxHashMap<InternedPath, ResolvedFunctionSignature>,
        nominal_type_ids_by_path: &FxHashMap<InternedPath, TypeId>,
        type_environment: &TypeEnvironment,
        template_ir_store: &Rc<RefCell<TemplateIrStore>>,
    ) -> Result<Self, TemplateError> {
        let mut by_path = FxHashMap::default();

        for declaration in declaration_table.iter() {
            let kind = classify_declaration(
                declaration,
                resolved_function_signatures_by_path,
                nominal_type_ids_by_path,
                type_environment,
                template_ir_store,
            )?;
            by_path.insert(declaration.id.clone(), kind);
        }

        Ok(Self { by_path })
    }

    pub(crate) fn kind_for_path(&self, path: &InternedPath) -> Option<DeclarationSemanticKind> {
        self.by_path.get(path).copied()
    }

    /// Register a function reconstructed from a closed generated-materialisation artefact.
    ///
    /// Generated environments do not rerun declaration-shell preparation, so their stable
    /// callable blueprints join the ordinary semantic table directly after inverse projection.
    pub(crate) fn register_materialised_function(&mut self, path: InternedPath) {
        self.by_path.insert(path, DeclarationSemanticKind::Function);
    }

    /// Register a constant reconstructed from the stable generated-materialisation closure.
    pub(crate) fn register_materialised_constant(&mut self, path: InternedPath) {
        self.by_path.insert(path, DeclarationSemanticKind::Constant);
    }

    /// Register a transparent alias declaration reconstructed from the stable closure.
    pub(crate) fn register_materialised_value(&mut self, path: InternedPath) {
        self.by_path.insert(path, DeclarationSemanticKind::Value);
    }
}

fn classify_declaration(
    declaration: &Declaration,
    resolved_function_signatures_by_path: &FxHashMap<InternedPath, ResolvedFunctionSignature>,
    nominal_type_ids_by_path: &FxHashMap<InternedPath, TypeId>,
    type_environment: &TypeEnvironment,
    template_ir_store: &Rc<RefCell<TemplateIrStore>>,
) -> Result<DeclarationSemanticKind, TemplateError> {
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

    let value_is_compile_time_constant = declaration
        .value
        .const_value_kind_with_template_classifier(&mut |template| {
            classify_template_from_effective_tir(template, template_ir_store)
        })?
        .is_compile_time_value();

    if value_is_compile_time_constant {
        Ok(DeclarationSemanticKind::Constant)
    } else {
        Ok(DeclarationSemanticKind::Value)
    }
}
