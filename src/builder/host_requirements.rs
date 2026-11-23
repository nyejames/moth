//! Produces the compiler-owned host import and export requirements for a package assembly.
//!
//! WHAT: joins the reachable external calls of the builder's selected facade exports with the
//! external package registry that resolves them, and resolves the selection itself against the
//! facade's public semantic interface, into one `moth_host::HostRequirements` value.
//! WHY: a native runtime must implement exactly the host bindings the artefact reaches and call
//! exactly the facade functions the builder selected. The selected exports are the artefact's
//! executable roots, so a binding only an unselected function reaches is not required. This
//! module is the single owner of that derivation, so the future host manifest reuses it instead
//! of rebuilding the join. Physical ABI facts, such as Wasm signatures, import module names,
//! result frames and offsets, are still missing from this compiler and are never placeholders
//! here.

use super::Error;
use super::compiled_project::NO_PACKAGE_FACADE_MESSAGE;
use super::host_conversion;
use crate::build_system::build::{PackageAssembly, ProjectCompilation};
use crate::builder_surface::PackageBacking;
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalBuiltinType, CanonicalTypeIdentity,
};
use crate::compiler_frontend::compiler_errors::InfrastructureFailure;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::external_packages::{
    ExternalAccessKind, ExternalFunctionDef, ExternalFunctionId, ExternalPackageRegistry,
    ExternalSignatureType, ExternalSymbolId,
};
use crate::compiler_frontend::public_call_summary::PublicCallParameterAccess;
use crate::compiler_frontend::public_interface::{
    PublicCallableReturnContract, PublicDeclarationSemantics, PublicFunctionCategory,
    PublicFunctionSemantics, PublicParameterContract, PublicSemanticInterface,
};
use crate::compiler_frontend::semantic_identity::{OriginDeclarationId, OriginFunctionId};
use moth_host::{
    HostBindingIdentity, HostExport, HostExportRequirement, HostExportSelection,
    HostImportRequirement, HostRequirements, HostSignature, RequirementsError,
};
use std::collections::BTreeMap;
use std::collections::btree_map::Entry;

/// Derives the host requirements of `compilation`'s project package assembly.
///
/// Imports are the reachable bindings of SDK host packages under the selected facade functions as
/// roots, deduplicated and ordered by canonical identity. Exports are the selected facade
/// functions in Wasm export-name order. Every unsupported export selection is a builder-facing
/// `Build` error; a registered host binding whose signature the host vocabulary cannot express is
/// a compiler invariant failure, because SDK registration rejects that shape before a build
/// starts.
// The native host manifest will call this when `compile_wasm` can lower a package, and the
// manifest must reuse this derivation rather than rebuild it.
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "the host manifest will consume this when compile_wasm can lower a package"
    )
)]
pub(super) fn host_requirements(
    compilation: &ProjectCompilation,
    selection: &HostExportSelection,
) -> Result<HostRequirements, Error> {
    let Some(assembly) = compilation.package_assembly() else {
        return Err(Error::build(NO_PACKAGE_FACADE_MESSAGE));
    };

    let interface = assembly.facade_interface(compilation);
    let mut roots = Vec::with_capacity(selection.exports().len());
    let mut exports = Vec::with_capacity(selection.exports().len());
    for selected in selection.exports() {
        let (origin, requirement) = export_requirement(interface, selected)?;
        roots.push(origin);
        exports.push(requirement);
    }

    let imports = import_requirements(assembly, compilation, &roots)?;

    HostRequirements::new(imports, exports).map_err(non_canonical_requirements)
}

/// Collects the required host imports reached from the selected exports, in canonical order.
///
/// A binding reached from several selected exports or several modules names one import, and the
/// canonical identity decides the order whatever order the selection, calls or host-package
/// registrations used. Each module decodes its reachable external IDs in the registry its compiled
/// artefact retains: the ID space belongs to the module that recorded the facts. Ownership, not an
/// observed divergence: one builder definition's registry is cloned once per compilation today, so
/// every module's snapshot assigns the same IDs.
fn import_requirements(
    assembly: &PackageAssembly,
    compilation: &ProjectCompilation,
    roots: &[OriginFunctionId],
) -> Result<Vec<HostImportRequirement>, Error> {
    let mut imports = BTreeMap::<HostBindingIdentity, HostSignature>::new();

    for (registry, reachability) in assembly
        .selected_export_call_sources(compilation, roots)
        .map_err(|error| Error::from_infrastructure_errors(error.into()))?
    {
        for function_id in &reachability.reachable_external_functions {
            let Some((identity, signature)) = host_import(registry, *function_id)? else {
                continue;
            };
            match imports.entry(identity) {
                Entry::Vacant(entry) => {
                    entry.insert(signature);
                }
                Entry::Occupied(entry) => {
                    if entry.get() != &signature {
                        return Err(internal_failure(format!(
                            "The host binding '{}' is registered with two different signatures.",
                            entry.key()
                        )));
                    }
                }
            }
        }
    }

    Ok(imports
        .into_iter()
        .map(|(identity, signature)| HostImportRequirement::new(identity, signature))
        .collect())
}

/// Resolves one reachable external call into a host import, or `None` when it is not a host
/// binding.
///
/// A reachable external function from a backend-provided binding package, such as a compiler Core
/// package or a first-party JavaScript package, is not a native host import: this compiler does
/// not ask a native runtime to implement it, so it stays outside the requirement set and its own
/// lowering owner decides whether a native build can carry it.
fn host_import(
    registry: &ExternalPackageRegistry,
    function_id: ExternalFunctionId,
) -> Result<Option<(HostBindingIdentity, HostSignature)>, Error> {
    let package_id = registry
        .resolve_function_package_id(function_id)
        .ok_or_else(|| {
            internal_failure(format!(
                "Reachable external function {function_id:?} has no owning package."
            ))
        })?;
    let package = registry.get_package_by_id(package_id).ok_or_else(|| {
        internal_failure(format!(
            "Reachable external function {function_id:?} names an unregistered package."
        ))
    })?;
    if package.metadata.backing != PackageBacking::HostBinding {
        return Ok(None);
    }

    let canonical = registry
        .canonical_symbol_identity(ExternalSymbolId::Function(function_id))
        .ok_or_else(|| {
            internal_failure(format!(
                "Reachable host function {function_id:?} has no canonical symbol identity."
            ))
        })?;
    let identity = HostBindingIdentity::new(
        canonical.package.name(),
        canonical.symbol_path.components().iter().cloned(),
    );
    let function = registry.get_function_by_id(function_id).ok_or_else(|| {
        internal_failure(format!(
            "Reachable host binding '{identity}' has no registered definition."
        ))
    })?;

    let signature = host_import_signature(function, &identity)?;
    Ok(Some((identity, signature)))
}

/// Converts one registered host binding's semantic signature into the shared vocabulary.
fn host_import_signature(
    function: &ExternalFunctionDef,
    identity: &HostBindingIdentity,
) -> Result<HostSignature, Error> {
    let mut signature = HostSignature::new();

    for (index, parameter) in function.parameters.iter().enumerate() {
        if parameter.access_kind != ExternalAccessKind::Shared {
            return Err(unsupported_host_binding(
                identity,
                format!("parameter {index} is not a shared read"),
            ));
        }
        let Some(host_type) =
            host_conversion::host_type_of_signature_type(&parameter.language_type)
        else {
            return Err(unsupported_host_binding(
                identity,
                format!("parameter {index} has no V1 host type"),
            ));
        };
        signature = signature.parameter(host_type);
    }

    let Some(returns) = function.return_contract.ordinary() else {
        return Err(unsupported_host_binding(
            identity,
            "its contract never returns".to_owned(),
        ));
    };
    for (index, slot) in returns.success.iter().enumerate() {
        let Some(host_type) = host_conversion::host_type_of_signature_type(&slot.value_type) else {
            return Err(unsupported_host_binding(
                identity,
                format!("success value {index} has no V1 host type"),
            ));
        };
        signature = signature.returns(host_type);
    }
    match returns.error {
        None => {}
        Some(ExternalSignatureType::BuiltinError) => signature = signature.fallible(),
        Some(_) => {
            return Err(unsupported_host_binding(
                identity,
                "it declares a custom error channel".to_owned(),
            ));
        }
    }

    Ok(signature)
}

/// Resolves one selected export against the facade interface into its runtime root and its
/// requirement.
///
/// The declaration origin is the native artefact's reachability root: the selection names a
/// facade function, and the requirement set is exactly what calling it reaches.
fn export_requirement(
    interface: &PublicSemanticInterface,
    selected: &HostExport,
) -> Result<(OriginFunctionId, HostExportRequirement), Error> {
    let Some(binding) = interface.export_bindings.iter().find(|binding| {
        binding.route().is_root() && binding.public_name() == selected.facade_name()
    }) else {
        return Err(Error::build(format!(
            "The export selection '{}' names '{}', but the package facade exports no declaration \
             with that name.",
            selected.export_name(),
            selected.facade_name()
        )));
    };
    let declaration = interface.declaration(binding.origin()).ok_or_else(|| {
        internal_failure(format!(
            "The facade export '{}' has no published declaration record.",
            selected.facade_name()
        ))
    })?;

    let PublicDeclarationSemantics::Function(function) = &declaration.semantics else {
        return Err(Error::build(format!(
            "The export selection '{}' names '{}', but the package facade exports a {} declaration \
             rather than a function.",
            selected.export_name(),
            selected.facade_name(),
            declaration_kind(&declaration.semantics)
        )));
    };

    let signature = export_signature(selected, function)?;
    let OriginDeclarationId::Function(origin) = &declaration.origin else {
        return Err(internal_failure(format!(
            "The facade export '{}' has function semantics but a non-function origin.",
            selected.facade_name()
        )));
    };
    Ok((
        origin.clone(),
        HostExportRequirement::new(selected.facade_name(), selected.export_name(), signature),
    ))
}

/// Converts one selected facade function's contract into the shared vocabulary.
fn export_signature(
    selected: &HostExport,
    function: &PublicFunctionSemantics,
) -> Result<HostSignature, Error> {
    let reject = |reason: String| {
        Error::build(format!(
            "The export selection '{}' names '{}', but {reason}.",
            selected.export_name(),
            selected.facade_name()
        ))
    };

    if !matches!(function.category, PublicFunctionCategory::ConcreteLocal) {
        return Err(reject(
            "that facade function is generic, so it has no concrete host signature".to_owned(),
        ));
    }
    if !function.evaluation.has_runtime_body() {
        return Err(reject(
            "that facade function is a compile-time function, so it has no runtime body".to_owned(),
        ));
    }

    let mut signature = HostSignature::new();
    for (index, parameter) in function.parameters.iter().enumerate() {
        match &parameter.contract {
            PublicParameterContract::Value => {}
            PublicParameterContract::Wire => {
                return Err(reject(format!(
                    "parameter {index} is a wire, which the V1 host ABI cannot carry"
                )));
            }
            PublicParameterContract::Route { .. } => {
                return Err(reject(format!(
                    "parameter {index} is a routing constructor parameter, which the V1 host ABI \
                     cannot carry"
                )));
            }
        }
        if parameter.access != PublicCallParameterAccess::Shared {
            return Err(reject(format!(
                "parameter {index} takes a mutable value, but the V1 host ABI passes shared reads \
                 only"
            )));
        }
        let Some(host_type) =
            host_conversion::host_type_of_canonical_type(&parameter.type_identity)
        else {
            return Err(reject(format!(
                "parameter {index} has a type the V1 host ABI cannot carry"
            )));
        };
        signature = signature.parameter(host_type);
    }

    let PublicCallableReturnContract::Returns { success, error } = &function.return_contract else {
        return Err(reject(
            "that facade function never returns, so it has no host result".to_owned(),
        ));
    };
    if let Some(error) = error {
        if !matches!(
            error,
            CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Error)
        ) {
            return Err(reject(
                "its custom error channel cannot cross the V1 host ABI, which carries only the \
                 builtin `Error!` channel"
                    .to_owned(),
            ));
        }
        signature = signature.fallible();
    }
    for (index, slot) in success.iter().enumerate() {
        let Some(host_type) = host_conversion::host_type_of_canonical_type(&slot.type_identity)
        else {
            return Err(reject(format!(
                "success value {index} has a type the V1 host ABI cannot carry"
            )));
        };
        signature = signature.returns(host_type);
    }

    Ok(signature)
}

/// Names the declaration kind a selection resolved to, so the rejection reads as a sentence.
fn declaration_kind(semantics: &PublicDeclarationSemantics) -> &'static str {
    match semantics {
        PublicDeclarationSemantics::Function(_) => "function",
        PublicDeclarationSemantics::Struct(_) => "struct",
        PublicDeclarationSemantics::Choice(_) => "choice",
        PublicDeclarationSemantics::TransparentAlias(_) => "transparent alias",
        PublicDeclarationSemantics::Constant(_) => "constant",
        PublicDeclarationSemantics::Trait(_) => "trait",
    }
}

/// Reports a registered host binding the requirement vocabulary cannot express.
///
/// `BuilderDefinition::finish` applies the same restricted profile to every SDK descriptor, so
/// reaching this branch means the registry holds a definition the registration path refused.
fn unsupported_host_binding(identity: &HostBindingIdentity, reason: String) -> Error {
    internal_failure(format!(
        "The registered host binding '{identity}' is outside the V1 host profile: {reason}."
    ))
}

/// Reports a requirement list that is not canonical.
fn non_canonical_requirements(error: RequirementsError) -> Error {
    internal_failure(format!(
        "The compiler produced non-canonical host requirements: {error}"
    ))
}

fn internal_failure(message: String) -> Error {
    Error::from_infrastructure_errors(
        InfrastructureFailure::internal(CompilerStage::ProjectBuildAssembly, message).into(),
    )
}
