//! Consumer-side materialisation of published const function bodies.
//!
//! WHAT: reparses a provider's retained const body syntax through the consuming module's own
//!       expression and const stores, then installs the resulting bounded executors under the
//!       consumer's local paths.
//! WHY: const calls fold while the consumer's environment is still being built, so a consumer
//!      cannot reuse the provider's `ExprId`s or HIR. Publication carries canonical source tokens
//!      and stable semantic facts; this owner turns them back into executable consumer state.
//! MUST NOT: retain or interpret provider-local expression, value or HIR identities.

use super::artefact_emit::{ModuleMaterialisationContext, StableFunctionMaterialisationContext};
use super::nominal_blueprints::{
    intern_generated_canonical_type, materialised_nominal_declaration, materialised_struct_fields,
};
use super::stable_types::{
    ConstFunctionArtefact, GeneratedFoldedValueMaterialiser, GeneratedValueMaterialisationServices,
    MaterialisationNominalSource, StableCallableBinding, StableFunctionSignature,
    StableFunctionTarget, import_canonical_folded_value,
};
use crate::compiler_frontend::FrontendBuildProfile;
use crate::compiler_frontend::ast::ast_nodes::{Declaration, DeclarationShell, DeclarationValue};
use crate::compiler_frontend::ast::const_values::const_execution::{
    ConstBodyPreparationError, ConstBodyReferenceScope, ConstFunctionExecutable,
    executable_for_const_declaration,
};
use crate::compiler_frontend::ast::const_values::store::ConstValueStore;
use crate::compiler_frontend::ast::expressions::expression_store::AstExpressionBuilder;
use crate::compiler_frontend::ast::function_body_to_ast;
use crate::compiler_frontend::ast::module_ast::emission::emitter::enclosing_callable_contract;
use crate::compiler_frontend::ast::module_ast::environment::{
    ResolvedConstantSet, TopLevelDeclarationTable,
};
use crate::compiler_frontend::ast::module_ast::scope_context::{
    ConstFunctionExecutables, ContextKind, EarlyConstFunctionSignatures, EnclosingCallableOwner,
    FileValueResolutionServices, ScopeContext,
};
use crate::compiler_frontend::ast::statements::functions::{
    FunctionParameter, FunctionSignature, MaterialisedFunctionSignature,
};
use crate::compiler_frontend::ast::templates::tir::TemplateIrStore;
use crate::compiler_frontend::ast::type_interner::AstTypeInterner;
use crate::compiler_frontend::ast::type_resolution::{
    ResolvedFunctionSignature, ResolvedTypeAlias,
};
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::{CompilerStage, DiagnosticBag};
use crate::compiler_frontend::datatypes::CallableEvaluation;
use crate::compiler_frontend::datatypes::TypeId;
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::headers::module_symbols::DeclarationId;
use crate::compiler_frontend::module_compilation::ProviderMaterialisationRegistry;
use crate::compiler_frontend::paths::module_resources::ModuleResourceTable;
use crate::compiler_frontend::semantic_identity::{OriginDeclarationId, OriginFunctionId};
use crate::compiler_frontend::style_directives::StyleDirectiveRegistry;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::type_coercion::compatibility::TypeCompatibilityCache;
use crate::compiler_frontend::value_mode::ValueMode;
use rustc_hash::{FxHashMap, FxHashSet};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

/// Live consumer-owned state one provider const closure materialises into.
pub(crate) struct ConstFunctionMaterialisationTarget<'a> {
    pub(crate) declaration_table: &'a mut Rc<TopLevelDeclarationTable>,
    pub(crate) resolved_constants: &'a mut Rc<ResolvedConstantSet>,
    pub(crate) early_const_function_signatures: &'a EarlyConstFunctionSignatures,
    pub(crate) const_function_executables: &'a ConstFunctionExecutables,
    pub(crate) type_environment: &'a mut TypeEnvironment,
    pub(crate) expression_builder: &'a mut AstExpressionBuilder,
    pub(crate) const_values: &'a mut ConstValueStore,
    pub(crate) warnings: &'a mut DiagnosticBag,
    pub(crate) string_table: &'a mut StringTable,
    pub(crate) path_fork: &'a mut PathInternerFork,
    pub(crate) external_package_registry: &'a Arc<ExternalPackageRegistry>,
    pub(crate) style_directives: &'a StyleDirectiveRegistry,
    pub(crate) file_value_resolution: Option<&'a Rc<FileValueResolutionServices>>,
    pub(crate) build_profile: FrontendBuildProfile,
    pub(crate) template_ir_store: &'a Rc<RefCell<TemplateIrStore>>,
    pub(crate) provider_materialisations: Option<&'a ProviderMaterialisationRegistry>,
    pub(crate) materialised_const_provider_origins: &'a mut FxHashMap<OriginFunctionId, PathId>,
    pub(crate) materialised_const_function_paths: &'a mut FxHashSet<PathId>,
    pub(crate) materialised_const_constant_paths: &'a mut FxHashSet<PathId>,
}

/// Resolve and materialise one published const origin into an already-live consumer target.
pub(crate) fn materialise_imported_const_function(
    origin: &OriginFunctionId,
    imported_local_path: PathId,
    registry: &ProviderMaterialisationRegistry,
    target: &mut ConstFunctionMaterialisationTarget<'_>,
) -> Result<(), InfrastructureFailure> {
    // A repeated import aliases the consumer-domain entry path already prepared for this stable
    // origin. This must happen before registry lookup and context rebase: both are cold-path work.
    if let Some(entry_path) = target
        .materialised_const_provider_origins
        .get(origin)
        .copied()
    {
        return reuse_materialised_const_function(origin, entry_path, imported_local_path, target);
    }
    let published = registry.const_function(origin).ok_or_else(|| {
        InfrastructureFailure::internal(
            CompilerStage::AstConstruction,
            format!("Imported const function {origin:?} has no published provider body"),
        )
    })?;
    #[cfg(test)]
    reuse_probe::record_rebase(origin);
    // Select the entry's reachable closure in the provider's own path domain before any row is
    // copied. A provider can retain many unrelated const bodies that this consumer cannot call.
    let (reachable_indices, entry_position) = reachable_const_row_indices(
        published.context.published_const_functions(),
        published.row_index,
        origin,
    )?;
    let selected = select_reachable_const_rows(
        published.context.as_ref(),
        published.rebase_required,
        &reachable_indices,
        target,
    )?;
    let declaring = selected
        .rebased_context
        .as_ref()
        .unwrap_or_else(|| published.context.as_ref());
    let rows = &selected.rows;
    let entry_row = &rows[entry_position];
    install_materialised_function_signature(
        &entry_row.signature,
        imported_local_path,
        entry_row,
        target,
    )?;
    let entry_path = entry_row.function_path;
    if target
        .materialised_const_function_paths
        .contains(&entry_path)
    {
        target
            .materialised_const_provider_origins
            .insert(origin.clone(), entry_path);
        return reuse_materialised_const_function(origin, entry_path, imported_local_path, target);
    }
    materialise_provider_const_functions(
        declaring,
        rows,
        entry_position,
        origin,
        imported_local_path,
        target,
    )
}

/// Reachable provider rows one consumer import will materialise, in the consumer's path domain.
struct SelectedConstRows {
    /// Rebased declaring context when the provider needed spelling-based rebasing.
    rebased_context: Option<ModuleMaterialisationContext>,
    rows: Box<[ConstFunctionArtefact]>,
}

/// Copy the entry's reachable rows into the consumer's path and string domain.
///
/// WHAT: prefers the provider's own const issuing tables so only the reachable rows are cloned,
///       then falls back to the generic identity pair when the provider retains no const pair.
/// WHY:  a provider can retain thousands of unrelated const bodies, so rebasing the whole module
///       context for one import would make a cold consumer import proportional to the provider.
fn select_reachable_const_rows(
    published_context: &ModuleMaterialisationContext,
    rebase_required: bool,
    reachable_indices: &[usize],
    target: &mut ConstFunctionMaterialisationTarget<'_>,
) -> Result<SelectedConstRows, InfrastructureFailure> {
    if let Some(rows) = published_context.rebased_const_closure_for_requester(
        reachable_indices,
        target.path_fork,
        target.string_table,
    )? {
        return Ok(SelectedConstRows {
            rebased_context: None,
            rows,
        });
    }
    if !rebase_required {
        return Ok(SelectedConstRows {
            rebased_context: None,
            rows: reachable_const_rows_from(published_context, reachable_indices)?,
        });
    }
    // Same-boundary providers without an issuing const identity pair rebase through the generic
    // identity pair, which already remaps every retained row.
    let rebased_context =
        published_context.rebased_for_requester(target.path_fork, target.string_table)?;
    let rows = reachable_const_rows_from(&rebased_context, reachable_indices)?;
    Ok(SelectedConstRows {
        rebased_context: Some(rebased_context),
        rows,
    })
}

/// Clone the reachable rows out of a declaring context that is already in the consumer domain.
fn reachable_const_rows_from(
    declaring: &ModuleMaterialisationContext,
    reachable_indices: &[usize],
) -> Result<Box<[ConstFunctionArtefact]>, InfrastructureFailure> {
    reachable_indices
        .iter()
        .map(|index| {
            declaring
                .published_const_functions()
                .get(*index)
                .cloned()
                .ok_or_else(|| {
                    InfrastructureFailure::internal(
                        CompilerStage::AstConstruction,
                        format!(
                            "Published const closure selected row {index} outside its retained rows"
                        ),
                    )
                })
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Vec::into_boxed_slice)
}

/// Collect the published rows one entry's private helper closure reaches.
///
/// WHAT: walks the published closure edges from `entry_index` and returns the selected row
///       indices in published order plus the entry's position among them.
/// WHY:  a cold import must know reachability before it rebases rows, so it copies only the
///       provider bodies this consumer can execute.
fn reachable_const_row_indices(
    rows: &[ConstFunctionArtefact],
    entry_index: usize,
    origin: &OriginFunctionId,
) -> Result<(Vec<usize>, usize), InfrastructureFailure> {
    let entry_row = rows
        .get(entry_index)
        .filter(|row| row.origin.as_ref() == Some(origin))
        .ok_or_else(|| {
            InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                format!("Imported const function {origin:?} has no published provider row"),
            )
        })?;
    let mut reachable_paths = FxHashSet::default();
    reachable_paths.insert(entry_row.function_path);
    loop {
        let mut changed = false;
        for row in rows {
            if reachable_paths.contains(&row.function_path) {
                for path in &row.closure {
                    changed |= reachable_paths.insert(*path);
                }
            }
        }
        if !changed {
            break;
        }
    }
    let mut indices = Vec::new();
    let mut entry_position = None;
    for (index, row) in rows.iter().enumerate() {
        if reachable_paths.contains(&row.function_path) {
            if index == entry_index {
                entry_position = Some(indices.len());
            }
            indices.push(index);
        }
    }
    let entry_position = entry_position.ok_or_else(|| {
        InfrastructureFailure::internal(
            CompilerStage::AstConstruction,
            format!("Imported const function {origin:?} entry row was not reachable from itself"),
        )
    })?;
    Ok((indices, entry_position))
}

/// Resolve the registry that publishes `callee_origin` as a const function.
///
/// WHY: a referenced namespace selects every member name, and ordinary callables are not const
///      dependencies. Returning `None` for them keeps runtime-only callees out of the compiler
///      execution closure. A missing registry is an internal invariant break because every
///      published const edge is created by a build that owns one.
fn published_const_callee_registry<'a>(
    callee_origin: &OriginFunctionId,
    provider_materialisations: Option<&'a ProviderMaterialisationRegistry>,
) -> Result<Option<&'a ProviderMaterialisationRegistry>, InfrastructureFailure> {
    let registry = provider_materialisations.ok_or_else(|| {
        InfrastructureFailure::internal(
            CompilerStage::AstConstruction,
            format!(
                "Published const callee {callee_origin:?} requires provider materialisations, but no registry is available"
            ),
        )
    })?;
    Ok(registry.const_function(callee_origin).map(|_| registry))
}

fn install_imported_const_callable(
    binding: &StableCallableBinding,
    nominal_source: &ConstFunctionArtefact,
    target: &mut ConstFunctionMaterialisationTarget<'_>,
) -> Result<(), InfrastructureFailure> {
    let StableFunctionTarget::Imported(callee_origin) = &binding.target else {
        return Ok(());
    };
    // Check before installing the signature: an ordinary imported callable must not gain a
    // const declaration shell merely because it appears in a referenced namespace.
    let Some(registry) =
        published_const_callee_registry(callee_origin, target.provider_materialisations)?
    else {
        return Ok(());
    };
    install_materialised_function_signature(
        &binding.signature,
        binding.local_path,
        nominal_source,
        target,
    )?;
    materialise_imported_const_function(callee_origin, binding.local_path, registry, target)
}

/// Install one const-callable declaration shell unless the consumer already owns that path.
///
/// WHY: an imported const entry already owns a declaration whose `binding_span` is the
///      consumer-authored import site. Provider-local helper paths have no consumer-visible name,
///      so they register without a span and never supply a diagnostic label.
fn register_materialised_const_declaration(
    function_path: PathId,
    function_type_id: TypeId,
    target: &mut ConstFunctionMaterialisationTarget<'_>,
) -> Result<(), InfrastructureFailure> {
    if target
        .declaration_table
        .get_by_path(&function_path)
        .is_some()
    {
        return Ok(());
    }
    let declaration = Declaration {
        id: function_path,
        type_id: function_type_id,
        type_syntax: None,
        value_mode: ValueMode::ImmutableReference,
        value: DeclarationValue::Shell(DeclarationShell::Function { receiver: None }),
        binding_span: None,
        config_contract: None,
    };
    append_materialised_declaration(target, declaration)?;
    Ok(())
}

/// Append one declaration to the consumer table, which materialisation must still own uniquely.
///
/// WHY: a provider row can add declarations after import projection ran, so the consumer table is
///      rebuilt through `Rc::get_mut`. A shared table or an already registered path means another
///      owner mutated consumer declarations during materialisation, which is an invariant break.
fn append_materialised_declaration(
    target: &mut ConstFunctionMaterialisationTarget<'_>,
    declaration: Declaration,
) -> Result<DeclarationId, InfrastructureFailure> {
    Rc::get_mut(target.declaration_table)
        .ok_or_else(|| {
            InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                "AST declaration table was shared during published const materialisation",
            )
        })?
        .append_for_construction(declaration, target.path_fork)
        .ok_or_else(|| {
            InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                "Published const declaration path was registered more than once",
            )
        })
}

/// Reuse one already-prepared provider entry under a newly imported local alias.
///
/// WHAT: installs the callable declaration and signature for `imported_local_path`, rebasing every
///       parameter path under that alias, then aliases the prepared executable row instead of
///       rebasing the provider context again.
/// WHY:  repeated imports of one stable origin carry the same implementation; the consumer-domain
///       entry path recorded on the origin map is the authority for that executable. The alias
///       still owns its own parameter paths, so it must match a freshly materialised signature.
fn reuse_materialised_const_function(
    origin: &OriginFunctionId,
    entry_path: PathId,
    imported_local_path: PathId,
    target: &mut ConstFunctionMaterialisationTarget<'_>,
) -> Result<(), InfrastructureFailure> {
    if entry_path == imported_local_path {
        return Ok(());
    }
    let mut resolved = target
        .early_const_function_signatures
        .borrow()
        .get(&entry_path)
        .cloned()
        .ok_or_else(|| {
            InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                format!(
                    "Materialised const origin {origin:?} is missing its prepared entry signature"
                ),
            )
        })?;
    let function_type_id = target
        .declaration_table
        .get_by_path(&entry_path)
        .map(|declaration| declaration.type_id)
        .ok_or_else(|| {
            InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                format!(
                    "Materialised const origin {origin:?} is missing its prepared entry declaration"
                ),
            )
        })?;
    for parameter in &mut resolved.signature.parameters {
        let parameter_name = target.path_fork.component(parameter.id).ok_or_else(|| {
            InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                format!(
                    "Materialised const origin {origin:?} has a parameter path with no component"
                ),
            )
        })?;
        parameter.id = target
            .path_fork
            .try_intern_child(imported_local_path, parameter_name)
            .ok_or_else(|| {
                InfrastructureFailure::internal(
                    CompilerStage::AstConstruction,
                    format!(
                        "Materialised const origin {origin:?} cannot intern a parameter path under its imported alias"
                    ),
                )
            })?;
    }
    register_materialised_const_declaration(imported_local_path, function_type_id, target)?;
    #[cfg(test)]
    reuse_probe::assert_signature_parameter_paths(
        imported_local_path,
        &resolved.signature,
        target.path_fork,
        target.string_table,
    );
    target
        .early_const_function_signatures
        .borrow_mut()
        .insert(imported_local_path, resolved);
    let executable = target
        .const_function_executables
        .borrow()
        .get(&entry_path)
        .cloned()
        .ok_or_else(|| {
            InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                format!(
                    "Materialised const origin {origin:?} is missing its prepared entry executable"
                ),
            )
        })?;
    target
        .const_function_executables
        .borrow_mut()
        .insert(imported_local_path, executable);
    Ok(())
}

fn install_materialised_function_signature<N: MaterialisationNominalSource>(
    signature: &StableFunctionSignature,
    function_path: PathId,
    nominal_source: &N,
    target: &mut ConstFunctionMaterialisationTarget<'_>,
) -> Result<FunctionSignature, InfrastructureFailure> {
    let MaterialisedFunctionSignature {
        signature,
        function_type_id,
    } = signature
        .materialise(
            &function_path,
            &mut StableFunctionMaterialisationContext {
                generic_parameter_type_ids: &[],
                nominal_source,
                type_environment: target.type_environment,
                const_values: target.const_values,
                external_package_registry: target.external_package_registry,
                module_resources: materialisation_module_resources(target.file_value_resolution),
                path_fork: target.path_fork,
                string_table: target.string_table,
            },
        )
        .map_err(|failure| {
            InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                format!(
                    "Published const function signature could not be materialised: {failure:?}"
                ),
            )
        })?;
    register_materialised_const_declaration(function_path, function_type_id, target)?;
    #[cfg(test)]
    reuse_probe::assert_signature_parameter_paths(
        function_path,
        &signature,
        target.path_fork,
        target.string_table,
    );
    target.early_const_function_signatures.borrow_mut().insert(
        function_path,
        ResolvedFunctionSignature {
            receiver: None,
            signature: signature.clone(),
            evaluation: CallableEvaluation::Const,
        },
    );
    Ok(signature)
}

/// Module resource table for materialised provider facts.
///
/// The requester's live table is authoritative when this build owns one. Generated and
/// config-only AST builds have none, so they get a private empty table instead.
fn materialisation_module_resources(
    file_value_resolution: Option<&Rc<FileValueResolutionServices>>,
) -> Rc<RefCell<ModuleResourceTable>> {
    file_value_resolution
        .map(|services| Rc::clone(&services.module_resources))
        .unwrap_or_else(|| Rc::new(RefCell::new(ModuleResourceTable::new())))
}

/// Materialise the selected reachable const closure of one provider origin into the consumer.
///
/// WHAT: registers each row's declaration, signature and member types, imports the closure's
///       dependent constants, materialises callees published by other providers, then reparses
///       each retained body and installs its bounded executable. The imported local path aliases
///       the entry executable so the consumer's own expression metadata finds it.
/// WHY: required calls resolve executables by the call's resolved declaration path, which is the
///      provider path for nested helper calls and the imported local path for the facade entry.
///      `rows` is the entry's complete reachable closure in published order, already rebased into
///      the consumer's path domain.
fn materialise_provider_const_functions(
    context: &ModuleMaterialisationContext,
    rows: &[ConstFunctionArtefact],
    entry_index: usize,
    origin: &OriginFunctionId,
    imported_local_path: PathId,
    target: &mut ConstFunctionMaterialisationTarget<'_>,
) -> Result<(), InfrastructureFailure> {
    let closure = register_materialised_const_rows(rows, target)?;
    #[cfg(test)]
    reuse_probe::record_preparations(&closure.rows, target.path_fork, target.string_table);
    materialise_dependent_constants(&closure.rows, target)?;
    materialise_foreign_const_callees(rows, &closure.rows, target)?;
    install_materialised_const_executables(
        context,
        closure,
        entry_index,
        imported_local_path,
        target,
    )?;
    target
        .materialised_const_provider_origins
        .insert(origin.clone(), rows[entry_index].function_path);
    Ok(())
}

/// One provider row whose declaration, signature and member types the consumer now owns.
struct MaterialisedConstRow<'rows> {
    /// Provider-local index of the row inside the selected closure.
    row_index: usize,
    row: &'rows ConstFunctionArtefact,
    signature: FunctionSignature,
}

/// One registered provider closure plus the member tables every body in it resolves against.
struct MaterialisedProviderClosure<'rows> {
    rows: Vec<MaterialisedConstRow<'rows>>,
    struct_fields: Rc<FxHashMap<PathId, Vec<Declaration>>>,
    type_aliases: Rc<FxHashMap<PathId, ResolvedTypeAlias>>,
}

/// Install each selected row's declaration, signature, type aliases and nominal types.
///
/// WHAT: skips rows this consumer already materialised, then registers the remaining rows in
///       published order.
/// WHY: every body in the closure resolves against the same immutable member tables, so they are
///      collected once here and shared rather than rebuilt per body.
fn register_materialised_const_rows<'rows>(
    rows: &'rows [ConstFunctionArtefact],
    target: &mut ConstFunctionMaterialisationTarget<'_>,
) -> Result<MaterialisedProviderClosure<'rows>, InfrastructureFailure> {
    let mut materialised_rows = Vec::with_capacity(rows.len());
    let mut struct_fields = FxHashMap::default();
    let mut type_aliases = FxHashMap::default();
    for (row_index, row) in rows.iter().enumerate() {
        if target
            .materialised_const_function_paths
            .contains(&row.function_path)
        {
            continue;
        }
        let signature = install_materialised_function_signature(
            &row.signature,
            row.function_path,
            row,
            target,
        )?;
        register_materialised_type_aliases(row, &mut type_aliases, target)?;
        register_materialised_nominals(row, &mut struct_fields, target)?;
        materialised_rows.push(MaterialisedConstRow {
            row_index,
            row,
            signature,
        });
    }
    Ok(MaterialisedProviderClosure {
        rows: materialised_rows,
        struct_fields: Rc::new(struct_fields),
        type_aliases: Rc::new(type_aliases),
    })
}

/// Intern one row's published type aliases into the consumer's environment.
fn register_materialised_type_aliases(
    row: &ConstFunctionArtefact,
    resolved_type_aliases: &mut FxHashMap<PathId, ResolvedTypeAlias>,
    target: &mut ConstFunctionMaterialisationTarget<'_>,
) -> Result<(), InfrastructureFailure> {
    for alias in &row.aliases {
        let target_type_id = intern_generated_canonical_type(
            &alias.target_type_identity,
            target.type_environment,
            target.external_package_registry,
            row,
            target.string_table,
            target.path_fork,
        )
        .map_err(|error| {
            InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                format!("Published const alias target could not be materialised: {error:?}"),
            )
        })?;
        resolved_type_aliases
            .entry(alias.local_path)
            .or_insert(ResolvedTypeAlias {
                target_type_id,
                declaration_span: alias.declaration_span,
            });
    }
    Ok(())
}

/// Intern one row's published nominal types and their struct field declarations.
fn register_materialised_nominals(
    row: &ConstFunctionArtefact,
    resolved_struct_fields: &mut FxHashMap<PathId, Vec<Declaration>>,
    target: &mut ConstFunctionMaterialisationTarget<'_>,
) -> Result<(), InfrastructureFailure> {
    for nominal in &row.nominals {
        let type_id = intern_generated_canonical_type(
            &nominal.identity,
            target.type_environment,
            target.external_package_registry,
            row,
            target.string_table,
            target.path_fork,
        )
        .map_err(|error| {
            InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                format!("Published const nominal type could not be materialised: {error:?}"),
            )
        })?;
        let declaration =
            materialised_nominal_declaration(nominal.local_path, type_id, target.type_environment)?;
        if let Some(blueprint) = row.nominal_blueprints.get(&nominal.identity) {
            let module_resources = materialisation_module_resources(target.file_value_resolution);
            let mut services = GeneratedValueMaterialisationServices {
                external_registry: target.external_package_registry,
                template_ir_store: target.template_ir_store,
                module_resources,
                expression_builder: target.expression_builder,
                const_values: target.const_values,
            };
            let fields = materialised_struct_fields(
                type_id,
                target.type_environment,
                blueprint,
                row,
                &mut services,
                target.string_table,
                target.path_fork,
            )
            .map_err(|error| {
                InfrastructureFailure::internal(
                    CompilerStage::AstConstruction,
                    format!("Published const nominal fields could not be materialised: {error:?}"),
                )
            })?;
            if let (Some(fields), Some(struct_path)) =
                (fields, target.type_environment.nominal_path(type_id))
            {
                resolved_struct_fields
                    .entry(struct_path.to_owned())
                    .or_insert(fields);
            }
        }
        if target
            .declaration_table
            .get_by_path(&nominal.local_path)
            .is_none()
        {
            append_materialised_declaration(target, declaration)?;
        }
    }
    Ok(())
}

/// Import the module constants every materialised body in the closure can reference.
///
/// WHAT: interns each published constant's stable type and folded value into the consumer store,
///       binds the provider-local path and records the module constant declaration.
/// WHY: a provider body can read a provider constant without calling another provider function,
///      and the consumer reparses that body against its own const store.
fn materialise_dependent_constants(
    rows: &[MaterialisedConstRow<'_>],
    target: &mut ConstFunctionMaterialisationTarget<'_>,
) -> Result<(), InfrastructureFailure> {
    for materialised in rows {
        let row = materialised.row;
        for constant in &row.constants {
            let local_path = &constant.local_path;
            if !target.materialised_const_constant_paths.insert(*local_path) {
                continue;
            }
            let type_id = intern_generated_canonical_type(
                &constant.type_identity,
                target.type_environment,
                target.external_package_registry,
                row,
                target.string_table,
                target.path_fork,
            )
            .map_err(|error| {
                InfrastructureFailure::internal(
                    CompilerStage::AstConstruction,
                    format!("Published const closure type could not be materialised: {error:?}"),
                )
            })?;
            let module_resources = materialisation_module_resources(target.file_value_resolution);
            let mut materialiser = GeneratedFoldedValueMaterialiser {
                type_environment: target.type_environment,
                external_registry: target.external_package_registry,
                nominal_source: row,
                module_resources,
                path_fork: target.path_fork,
            };
            let value_id = import_canonical_folded_value(
                &mut materialiser,
                &constant.value,
                type_id,
                target.const_values,
                target.string_table,
                constant.span,
            )
            .map_err(|error| {
                InfrastructureFailure::internal(
                    CompilerStage::AstConstruction,
                    format!("Published const closure value could not be materialised: {error:?}"),
                )
            })?;
            if let Some(existing) = target.declaration_table.get_by_path(local_path)
                && !matches!(
                    &existing.value,
                    DeclarationValue::Default(existing_value) if *existing_value == value_id
                )
            {
                return Err(InfrastructureFailure::internal(
                    CompilerStage::AstConstruction,
                    "Published const constant path collides with an existing declaration",
                ));
            }
            target
                .const_values
                .bind_module_constant(*local_path, value_id)
                .map_err(|error| {
                    InfrastructureFailure::internal(
                        CompilerStage::AstConstruction,
                        format!(
                            "Published const closure binding could not be installed: {error:?}"
                        ),
                    )
                })?;
            let declaration_id = match target.declaration_table.declaration_id_by_path(local_path) {
                Some(declaration_id) => declaration_id,
                None => append_materialised_declaration(
                    target,
                    Declaration {
                        id: *local_path,
                        type_id,
                        type_syntax: None,
                        value_mode: ValueMode::ImmutableReference,
                        value: DeclarationValue::Default(value_id),
                        binding_span: constant.span,
                        config_contract: None,
                    },
                )?,
            };
            Rc::make_mut(target.resolved_constants).insert(declaration_id);
        }
    }
    Ok(())
}

/// Materialise the const callees that live outside this provider closure.
///
/// WHAT: walks each registered row's imported callable edges and resolved function declarations,
///       recursing into every foreign provider's own published closure.
/// WHY: a provider body can call a const function from another module. That callee owns a separate
///      published closure and must be installed before any body parses. Repeated imports alias the
///      already prepared executor instead of reparsing.
fn materialise_foreign_const_callees(
    published_rows: &[ConstFunctionArtefact],
    rows: &[MaterialisedConstRow<'_>],
    target: &mut ConstFunctionMaterialisationTarget<'_>,
) -> Result<(), InfrastructureFailure> {
    for materialised in rows {
        let row = materialised.row;
        for binding in &row.callables {
            install_imported_const_callable(binding, row, target)?;
        }
        // Imported declarations do not always own a resolved local signature in the provider
        // environment, so their stable function origins are the authoritative executable edge.
        for declaration in &row.declarations {
            let OriginDeclarationId::Function(callee_origin) = &declaration.origin else {
                continue;
            };
            // A reachable body can only name rows inside its own complete closure. Any provider
            // declaration not in `published_rows` belongs to another module's publication.
            if published_rows
                .iter()
                .any(|provider_row| provider_row.function_path == declaration.local_path)
                || target
                    .materialised_const_function_paths
                    .contains(&declaration.local_path)
            {
                continue;
            }
            let Some(registry) =
                published_const_callee_registry(callee_origin, target.provider_materialisations)?
            else {
                continue;
            };
            materialise_imported_const_function(
                callee_origin,
                declaration.local_path,
                registry,
                target,
            )?;
        }
    }
    Ok(())
}

/// Reparse every registered row's retained body and install its bounded executable.
///
/// All rows are registered before any body parses: a helper body may name another helper, and the
/// shared callable lookup must already classify that name as const. The maps are shared live with
/// every body scope, so no per-closure snapshot is needed.
fn install_materialised_const_executables(
    context: &ModuleMaterialisationContext,
    closure: MaterialisedProviderClosure<'_>,
    entry_index: usize,
    imported_local_path: PathId,
    target: &mut ConstFunctionMaterialisationTarget<'_>,
) -> Result<(), InfrastructureFailure> {
    for materialised in &closure.rows {
        let executable = materialise_const_executable(context, materialised, &closure, target)?;
        let function_path = materialised.row.function_path;
        target
            .const_function_executables
            .borrow_mut()
            .insert(function_path, executable.clone());
        target
            .materialised_const_function_paths
            .insert(function_path);
        // The facade entry aliases the provider executable: consumer expression metadata resolves
        // calls by the imported local path, not by the provider's own declaration path.
        if materialised.row_index == entry_index {
            target
                .const_function_executables
                .borrow_mut()
                .insert(imported_local_path, executable);
        }
    }
    Ok(())
}

/// Reparse one retained provider body against the consumer's stores and prepare its executor.
fn materialise_const_executable(
    context: &ModuleMaterialisationContext,
    materialised: &MaterialisedConstRow<'_>,
    closure: &MaterialisedProviderClosure<'_>,
    target: &mut ConstFunctionMaterialisationTarget<'_>,
) -> Result<ConstFunctionExecutable, InfrastructureFailure> {
    let row = materialised.row;
    // The retained source file came from the provider domain. The consumer's path fork is
    // authoritative for the rebased hierarchy, so derive the enclosing source directory here.
    let source_file = target
        .path_fork
        .parent(row.function_path)
        .unwrap_or(PathId::ROOT);
    let materialised_body = row.body.materialise(source_file, target.path_fork)?;
    let body_services = target.file_value_resolution.map(|services| {
        Rc::new(FileValueResolutionServices {
            stage0_resolution_facts: Some(Arc::clone(&materialised_body.resolution_facts)),
            module_resources: Rc::clone(&services.module_resources),
            qualified_reference_targets: Arc::clone(&materialised_body.qualified_reference_targets),
            // The retained body belongs to the provider. Resource identity must use the
            // declaring module's stable origin, not the consuming requester's origin.
            module_origin: context
                .module_origin
                .clone()
                .or_else(|| services.module_origin.clone()),
            public_source_trait_origins: Arc::clone(&services.public_source_trait_origins),
            frozen_identity_handle: materialised_body.frozen_identity_handle.clone(),
        })
    });
    let body = materialised_body.into_generic_body()?;
    let parse_owner = body.parse_owner();
    let (mut cursor, source_id) = parse_owner.cursor()?;
    let visibility = row.visibility.materialise(
        target.external_package_registry,
        target.path_fork,
        target.string_table,
    )?;
    let mut scope = ScopeContext::new(
        ContextKind::Function,
        row.function_path,
        Rc::clone(target.declaration_table),
        Arc::clone(target.external_package_registry),
        0,
        Rc::clone(target.template_ir_store),
    )
    .with_style_directives(target.style_directives)
    .with_build_profile(target.build_profile)
    .with_file_visibility(Arc::new(visibility))
    .with_early_const_function_signatures(Rc::clone(target.early_const_function_signatures))
    .with_const_function_executables(Rc::clone(target.const_function_executables))
    .with_resolved_module_constants(Rc::clone(target.resolved_constants))
    .with_resolved_struct_fields_by_path(Rc::clone(&closure.struct_fields))
    .with_resolved_type_aliases(Rc::clone(&closure.type_aliases))
    .with_source_file_scope(source_file)
    .with_declaring_file_id(source_id)
    // A provider body owns no consumer-domain declaration span. Diagnostics raised by this
    // reparse become infrastructure failures, so the const-body marker alone routes dispatch.
    .with_const_body(None);
    if let Some(services) = body_services {
        scope = scope.with_file_value_resolution(services);
    }
    scope.enclosing_callable = Some(enclosing_callable_contract(
        &materialised.signature,
        EnclosingCallableOwner::Function,
    ));
    scope
        .set_local_declarations(
            materialised
                .signature
                .parameters
                .iter()
                .map(FunctionParameter::to_local_declaration)
                .collect(),
            &*target.path_fork,
            target.string_table,
        )
        .map_err(|diagnostic| {
            InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                format!(
                    "Published const function body scope could not bind its parameters: {diagnostic:?}"
                ),
            )
        })?;
    let mut compatibility_cache = TypeCompatibilityCache::new();
    let mut type_interner = AstTypeInterner::new(target.type_environment, &mut compatibility_cache);
    // The consumer-domain scope must survive the body parse so the reference check sees the same
    // resolved module constants the parse used. The clone shares every module table.
    let reference_scope = scope.clone();
    let nodes = function_body_to_ast(
        &mut cursor,
        scope,
        None,
        &mut type_interner,
        target.expression_builder,
        target.const_values,
        target.warnings,
        target.string_table,
        target.path_fork,
    )
    .map_err(|error| {
        InfrastructureFailure::internal(
            CompilerStage::AstConstruction,
            format!("Published const function body could not be reparsed: {error:?}"),
        )
    })?
    .expect_complete_nodes("a published const function body")
    .map_err(|error| {
        InfrastructureFailure::internal(
            CompilerStage::AstConstruction,
            format!("Published const function body was incomplete: {error:?}"),
        )
    })?;
    match executable_for_const_declaration(
        &materialised.signature,
        &nodes,
        target.expression_builder,
        ConstBodyReferenceScope {
            scope: &reference_scope,
            path_fork: &*target.path_fork,
            constants: &*target.const_values,
        },
    ) {
        Ok(executable) => Ok(executable),
        Err(ConstBodyPreparationError::Unsupported(rejection)) => {
            Err(InfrastructureFailure::internal(
                CompilerStage::AstConstruction,
                format!(
                    "Published const function {:?} has an unsupported body operation {:?} at {:?}",
                    row.function_path, rejection.operation, rejection.span
                ),
            ))
        }
        Err(ConstBodyPreparationError::Infrastructure(error)) => Err(error.primary),
    }
}

/// Test-only observation of provider body preparation.
///
/// WHAT: records every provider row whose body is prepared into a consumer, keyed by the stable
///       origin debug identity or, for a private helper, by its rendered declaration path.
/// WHY:  compilation succeeding through two import aliases does not by itself prove that only one
///       provider body was prepared. Regression tests need the preparation site to expose the full
///       row set without adding runtime state to the production materialisation path.
#[cfg(test)]
pub(crate) mod reuse_probe {
    use super::OriginFunctionId;
    use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
    use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
    use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
    use std::sync::Mutex;

    static PREPARED: Mutex<Vec<String>> = Mutex::new(Vec::new());
    static REBASES: Mutex<Vec<String>> = Mutex::new(Vec::new());
    static SELECTED_ROWS: Mutex<Vec<String>> = Mutex::new(Vec::new());
    static SIGNATURE_CHECKS: Mutex<Vec<String>> = Mutex::new(Vec::new());

    /// Lock one probe log, treating a poisoned probe as a test bug.
    fn records<T>(log: &Mutex<Vec<T>>) -> std::sync::MutexGuard<'_, Vec<T>> {
        log.lock()
            .expect("test materialisation probe lock should not be poisoned")
    }

    /// Record the identity of every row one cold import prepares.
    ///
    /// WHAT: logs each registered row by its public origin, or by its rendered declaration path
    ///       when the row is a private helper without an origin.
    /// WHY:  an assertion that an unrelated row was not prepared can only observe rows outside the
    ///       requested origin if this log keeps their identities.
    pub(super) fn record_preparations(
        rows: &[super::MaterialisedConstRow<'_>],
        path_fork: &PathInternerFork,
        string_table: &mut StringTable,
    ) {
        let mut prepared = records(&PREPARED);
        let mut scratch = Vec::new();
        for materialised in rows {
            prepared.push(row_identity(
                materialised.row,
                path_fork,
                string_table,
                &mut scratch,
            ));
        }
    }

    /// Identify one prepared row by its public origin or its rendered declaration path.
    fn row_identity(
        row: &super::ConstFunctionArtefact,
        path_fork: &PathInternerFork,
        string_table: &mut StringTable,
        scratch: &mut Vec<StringId>,
    ) -> String {
        if let Some(origin) = row.origin.as_ref() {
            return format!("{origin:?}");
        }
        path_fork.render_portable(row.function_path, string_table, scratch)
    }

    pub(crate) fn record_rebase(origin: &OriginFunctionId) {
        records(&REBASES).push(format!("{origin:?}"));
    }

    /// Record the provider rows selected into one consumer closure.
    pub(crate) fn record_selected_rows(rows: &[super::ConstFunctionArtefact]) {
        let mut retained = records(&SELECTED_ROWS);
        for row in rows {
            if let Some(origin) = &row.origin {
                retained.push(format!("{origin:?}"));
            }
        }
    }

    /// Count body preparations whose origin debug identity names the function.
    pub(crate) fn preparations(defining_name: &str) -> usize {
        records(&PREPARED)
            .iter()
            .filter(|origin| origin.contains(defining_name))
            .count()
    }

    pub(crate) fn rebases(defining_name: &str) -> usize {
        records(&REBASES)
            .iter()
            .filter(|origin| origin.contains(defining_name))
            .count()
    }

    pub(crate) fn selected_rows(defining_name: &str) -> usize {
        records(&SELECTED_ROWS)
            .iter()
            .filter(|origin| origin.contains(defining_name))
            .count()
    }

    /// Assert every parameter path is a direct child of the signature's requested function path.
    ///
    /// WHAT: validates the identity contract shared by fresh materialisation and alias reuse.
    /// WHY:  a reused alias that keeps the original entry's parameter paths looks usable through
    ///       positional calls but diverges from a freshly materialised signature. Failing the
    ///       exercising fixture prevents that discrepancy from surviving as an unchecked invariant.
    pub(crate) fn assert_signature_parameter_paths(
        function_path: PathId,
        signature: &FunctionSignature,
        path_fork: &PathInternerFork,
        string_table: &StringTable,
    ) {
        let mut scratch = Vec::new();
        let rendered_path = path_fork.render_portable(function_path, string_table, &mut scratch);
        for parameter in &signature.parameters {
            assert_eq!(
                path_fork.parent(parameter.id),
                Some(function_path),
                "materialised signature for {rendered_path} must own parameter {:?} directly under its function path",
                parameter.id
            );
        }
        records(&SIGNATURE_CHECKS).push(rendered_path);
    }

    /// Count validated signatures whose rendered path contains `path_fragment`.
    pub(crate) fn signature_checks(path_fragment: &str) -> usize {
        records(&SIGNATURE_CHECKS)
            .iter()
            .filter(|checked| checked.contains(path_fragment))
            .count()
    }

    /// Names recorded by the signature identity invariant, in check order.
    pub(crate) fn checked_signatures() -> Vec<String> {
        records(&SIGNATURE_CHECKS).clone()
    }
}
