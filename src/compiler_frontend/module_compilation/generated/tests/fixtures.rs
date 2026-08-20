//! Shared fixtures for generated-function tests.
//!
//! WHY: the compiler's module transaction and the build system's boundary store both need real
//!      generated identities, summaries and sidecars. One fixture owner keeps the two focused test
//!      suites building the same shapes.

use crate::compiler_frontend::analysis::borrow_checker::BorrowCheckReport;
use crate::compiler_frontend::canonical_type_identity::{
    CanonicalBuiltinType, CanonicalTypeIdentity,
};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::hir::functions::HirFunction;
use crate::compiler_frontend::hir::ids::{BlockId, FunctionId};
use crate::compiler_frontend::hir::module::HirModule;
use crate::compiler_frontend::hir::reachability::HirModuleLinkFacts;
use crate::compiler_frontend::module_compilation::artefact::{
    Module, ModuleCompilerMetadata, ModuleExecutable, ModuleLinkFacts, ModuleRootActivity,
};
use crate::compiler_frontend::module_compilation::generated::artefacts::GeneratedFunctionSidecar;
use crate::compiler_frontend::module_compilation::generated::transaction::GeneratedRequestFacts;
use crate::compiler_frontend::public_call_summary::{
    FunctionReturnAliasSummary, PublicCallSummary,
};
use crate::compiler_frontend::semantic_identity::{
    GeneratedDeclarationIdentity, GeneratedFunctionIdentity, ModulePrivateExecutableCategory,
    ModulePrivateExecutableIdentity, ModuleRootRole, StableModuleOriginIdentity,
    StablePackageIdentity,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{CharPosition, SourceLocation};

use std::path::PathBuf;
use std::sync::Arc;

pub(crate) fn module_origin() -> StableModuleOriginIdentity {
    StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("worklist-tests"),
        "main".to_owned(),
        ModuleRootRole::Normal,
    )
}

pub(crate) fn generated_identity(name: &str) -> GeneratedFunctionIdentity {
    GeneratedFunctionIdentity::new(
        GeneratedDeclarationIdentity::ModulePrivate(ModulePrivateExecutableIdentity::new(
            module_origin(),
            "@page.moth".to_owned(),
            ModulePrivateExecutableCategory::GenericFunction,
            name.to_owned(),
            None,
        )),
        Box::new([CanonicalTypeIdentity::Builtin(CanonicalBuiltinType::Int)]),
        Box::new([]),
    )
}

pub(crate) fn summary() -> PublicCallSummary {
    PublicCallSummary {
        parameters: Vec::new(),
        return_alias: FunctionReturnAliasSummary::Fresh,
    }
}

pub(crate) fn test_module() -> Module {
    Module {
        executable: ModuleExecutable {
            hir: HirModule::new(),
            type_environment: TypeEnvironment::new(),
            borrow_analysis: BorrowCheckReport::default(),
        },
        link_facts: ModuleLinkFacts {
            external_package_registry: Arc::new(ExternalPackageRegistry::new()),
            external_import_candidates: Vec::new(),
            functions: HirModuleLinkFacts::default(),
        },
        metadata: ModuleCompilerMetadata {
            entry_point: PathBuf::new(),
            warnings: Vec::new(),
            const_top_level_fragments: Vec::new(),
            root_activity: ModuleRootActivity::default(),
            doc_fragments: Vec::new(),
            rendered_path_usages: Vec::new(),
            materialisation_context: None,
        },
    }
}

pub(crate) fn test_sidecar(
    identity: GeneratedFunctionIdentity,
    summary: PublicCallSummary,
) -> GeneratedFunctionSidecar {
    let mut module = test_module();
    module.executable.hir.functions.push(HirFunction {
        id: FunctionId(0),
        entry: BlockId(0),
        params: Vec::new(),
        return_type: TypeId(0),
    });
    module
        .executable
        .hir
        .function_ids_by_generated
        .insert(identity.clone(), FunctionId(0));
    module
        .executable
        .borrow_analysis
        .analysis
        .public_call_summaries
        .insert(FunctionId(0), summary);
    GeneratedFunctionSidecar::new(identity, module)
}

pub(crate) fn facts(name: &str) -> GeneratedRequestFacts {
    GeneratedRequestFacts {
        identity: generated_identity(name),
        display_name: name.to_owned(),
        diagnostic_location: SourceLocation::new(
            InternedPath::from_single_str("src/@page.moth", &mut StringTable::new()),
            CharPosition::default(),
            CharPosition::default(),
        ),
    }
}
