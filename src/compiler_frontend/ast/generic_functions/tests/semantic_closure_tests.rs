//! Focused tests for the detached semantic closure of a generic module.
//!
//! WHAT: proves malformed retained aliases fail at their declaration, while selected aliases,
//! folded defaults and nested sibling constants survive release in a rebased requester domain.
//! WHY: generated sidecars must consume completed stable facts without retaining declaring
//! expression owners or constants used only to fold a signature default.
//! Alias-target reach into nominal blueprints is owned by the cross-module integration cases,
//! because that is where a missing blueprint is observable.

use super::frozen_body_tests::{
    closure_request, materialise_closure_request, prepare_closure_fixture,
};
use super::preparation_freeze::ModuleMaterialisationPreparationBuilder;
use crate::compiler_frontend::ast::ast_nodes::NodeKind;
use crate::compiler_frontend::ast::const_values::store::{ConstValuePayload, ConstValueStore};
use crate::compiler_frontend::compiler_errors::InfrastructureFailureKind;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::folded_value::PublicFoldedValue;
use crate::compiler_frontend::hir::expressions::HirExpressionKind;
use crate::compiler_frontend::hir::functions::HirFunctionOriginLookup;
use crate::compiler_frontend::hir::hir_builder::lower_module;
use crate::compiler_frontend::hir::terminators::HirTerminator;
use crate::compiler_frontend::paths::module_resources::ModuleResourceTable;
use crate::compiler_frontend::public_interface::PublicSemanticInterface;
use crate::compiler_frontend::semantic_identity::{
    GeneratedDeclarationIdentity, ModulePrivateExecutableCategory, ModulePrivateExecutableIdentity,
    ModuleRootRole, StableModuleOriginIdentity, StablePackageIdentity,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tests::parse_support::parse_single_file_ast_build_result;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};
use std::rc::Rc;
use std::sync::Arc;

struct PreparedModule {
    preparation: ModuleMaterialisationPreparationBuilder,
    const_values: ConstValueStore,
    public_interface: PublicSemanticInterface,
    path_fork: crate::compiler_frontend::symbols::path_interner::PathInternerFork,
    string_table: StringTable,
}

fn prepared_module(source: &str, package: &str) -> PreparedModule {
    let (mut build_result, path_fork, string_table) =
        parse_single_file_ast_build_result(source).expect("generic source should build");
    let module_origin = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local(package),
        "main".to_owned(),
        ModuleRootRole::Normal,
    );

    // Declaration identities are installed by module compilation, so this fixture assigns the
    // one identity its single template needs to reach the detach boundary.
    let template = build_result
        .materialisation_context
        .as_mut()
        .expect("module builds retain their materialisation context")
        .generic_function_templates_mut()
        .values_mut()
        .next()
        .expect("the source should retain one generic template");
    template.declaration_identity = Some(GeneratedDeclarationIdentity::ModulePrivate(
        ModulePrivateExecutableIdentity::new(
            module_origin.clone(),
            "@page.moth".to_owned(),
            ModulePrivateExecutableCategory::GenericFunction,
            "probe".to_owned(),
            None,
        ),
    ));

    let preparation = build_result
        .materialisation_context
        .expect("module builds retain their materialisation capture");

    PreparedModule {
        preparation,
        const_values: build_result.ast.const_values,
        public_interface: PublicSemanticInterface {
            module_origin,
            export_bindings: Vec::new(),
            export_diagnostic_provenance: Vec::new(),
            binding_exports: Vec::new(),
            declarations: Vec::new(),
            reusable_evidence: Vec::new(),
            concrete_call_summaries: Vec::new(),
        },
        path_fork,
        string_table,
    }
}

const ALIAS_SOURCE: &str = concat!(
    "Count as I32\n",
    "\n",
    "probe type T |value T| -> T:\n",
    "    seen Count = 1\n",
    "    return value\n",
    ";\n",
    "\n",
    "probe(1)\n",
);

#[test]
fn unprojectable_retained_alias_target_fails_at_the_alias_declaration() {
    let control = prepared_module(ALIAS_SOURCE, "closure-alias-control");
    let detached = control
        .preparation
        .detach(
            &control.const_values,
            &control.public_interface.declarations,
            &control.public_interface.reusable_evidence,
            &ModuleResourceTable::new(),
            &control.path_fork,
        )
        .expect("a completed alias target must detach");
    detached
        .freeze(&control.public_interface)
        .expect("the detached alias target must freeze")
        .expect("the retained generic should produce a materialisation context");

    let mut prepared = prepared_module(ALIAS_SOURCE, "closure-alias-tests");
    // The alias row is complete by construction, so the only way to reach the generated-side
    // defence is to replace its handle with one the type environment never interned.
    let alias_path = prepared
        .preparation
        .context
        .resolved_type_aliases_by_path
        .keys()
        .find(|path| {
            prepared
                .path_fork
                .component(**path)
                .map(|id| prepared.string_table.resolve(id))
                == Some("Count")
        })
        .cloned()
        .expect("the module should retain its Count alias");
    let alias = Rc::make_mut(&mut prepared.preparation.context.resolved_type_aliases_by_path)
        .get_mut(&alias_path)
        .expect("the alias row should be present");
    alias.target_type_id = TypeId(u32::MAX);

    let detach_result = prepared.preparation.detach(
        &prepared.const_values,
        &prepared.public_interface.declarations,
        &prepared.public_interface.reusable_evidence,
        &ModuleResourceTable::new(),
        &prepared.path_fork,
    );
    let Err(error) = detach_result else {
        panic!("an unprojectable alias target must not detach");
    };

    assert_eq!(error.kind, InfrastructureFailureKind::Internal);
    // The message renders the alias path through PathId's debug form (the fixture has no
    // production path-name spelling at the detach boundary), so assert the message identifies
    // the exact retained alias row this test corrupted.
    assert!(
        error.message.contains("completed-target invariant")
            && error.message.contains(&format!("{alias_path:?}")),
        "unexpected alias detachment error: {error:?}"
    );
}

#[test]
fn selected_alias_and_folded_default_survive_nested_sibling_release_in_rebased_domains() {
    let (build_result, donor_paths, donor_strings) = parse_single_file_ast_build_result(concat!(
        "Count as I32\n",
        "fallback #I32 = 41\n",
        "hidden #I32 = 7\n",
        "inner type T |value T, suffix Count = fallback| -> I32:\n",
        "    seen Count = suffix\n",
        "    return hidden\n",
        ";\n",
        "outer type T |value T| -> I32:\n",
        "    return inner(value)\n",
        ";\n",
    ))
    .expect("the alias and folded-default sibling source should build");
    // The helper consumes the entire declaring build, including its constant and expression
    // stores. Only the detached preparation remains before either sibling materialises.
    let mut declaring = prepare_closure_fixture(build_result, &donor_paths);
    let outer_identity = closure_request(&declaring, "outer");
    let inner_identity = closure_request(&declaring, "inner");
    let donor_alias = declaring
        .stable_context
        .semantic_closure
        .aliases
        .iter()
        .find(|alias| {
            donor_paths
                .component(alias.local_path)
                .is_some_and(|id| donor_strings.resolve(id) == "Count")
        })
        .expect("the body-selected Count alias must survive detach")
        .local_path;
    let donor_alias_name = donor_paths
        .component(donor_alias)
        .expect("Count has a name");
    assert!(
        declaring
            .stable_context
            .semantic_closure
            .constants
            .iter()
            .all(|row| {
                donor_paths
                    .component(row.local_path)
                    .is_none_or(|id| donor_strings.resolve(id) != "fallback")
            }),
        "a constant used only to fold a signature default must not enter the semantic closure",
    );
    let expected_default = FixedScalarValue::signed(FixedScalar::I32, 41).expect("41 fits I32");
    let (_, inner_index) = declaring
        .declaring_context_for_identity(inner_identity.declaration())
        .expect("inner is retained");
    assert_eq!(
        declaring.stable_context.artefacts[inner_index]
            .signature
            .parameters[1]
            .folded_default,
        Some(PublicFoldedValue::FixedScalar(expected_default)),
    );
    let (_, outer_index) = declaring
        .declaring_context_for_identity(outer_identity.declaration())
        .expect("outer is retained");
    assert!(
        declaring.stable_context.artefacts[outer_index]
            .local_declarations
            .is_empty(),
        "outer must not select the alias and constant used only by its nested sibling",
    );
    assert!(
        declaring.stable_context.artefacts[inner_index]
            .local_declarations
            .contains(&donor_alias),
        "inner must own the body-selected alias independently of outer",
    );

    let (requester_build, mut paths, mut strings) =
        parse_single_file_ast_build_result("requester_padding #I32 = 99\n")
            .expect("the independent requester should build");
    let requester = prepare_closure_fixture(requester_build, &paths);
    // Occupy unrelated string and path handles before rebasing, so donor numeric IDs cannot
    // accidentally resolve to the right requester names.
    for index in 0..donor_paths.len() {
        paths
            .try_intern_portable_path(&format!("requester/padding-{index}"), &mut strings)
            .expect("the requester padding paths fit");
    }
    assert_ne!(strings.try_resolve(donor_alias_name), Some("Count"));
    declaring.stable_context = Arc::new(
        declaring
            .stable_context
            .rebased_for_requester_with(
                &donor_paths.snapshot_table(),
                &donor_strings,
                &mut paths,
                &mut strings,
            )
            .expect("the detached closure should rebase by spelling"),
    );
    let rebased_alias = declaring
        .stable_context
        .semantic_closure
        .aliases
        .iter()
        .find(|alias| {
            paths
                .component(alias.local_path)
                .is_some_and(|id| strings.resolve(id) == "Count")
        })
        .expect("the selected alias must retain its spelling after rebasing")
        .local_path;
    assert_ne!(rebased_alias, donor_alias);
    assert_ne!(paths.component(rebased_alias), Some(donor_alias_name));
    drop(donor_paths);
    drop(donor_strings);

    let mut outer = materialise_closure_request(
        &declaring,
        &requester,
        &outer_identity,
        &mut strings,
        &mut paths,
    );
    assert_eq!(outer.build_result.deferred_generic_requests.len(), 1);
    let resources = outer
        .build_result
        .module_resources
        .as_ref()
        .expect("the generated sidecar owns resources")
        .borrow();
    let nested = outer
        .build_result
        .materialisation_context
        .take()
        .expect("outer retains the sibling request capture")
        .finish_preparation(&outer.build_result.ast.const_values, &resources, &paths)
        .expect("the sibling request must detach with the original semantic closure");
    drop(resources);
    drop(outer);
    drop(declaring);
    drop(requester);

    let inner =
        materialise_closure_request(&nested, &nested, &inner_identity, &mut strings, &mut paths);
    assert!(inner.build_result.deferred_generic_requests.is_empty());
    let signature = inner
        .build_result
        .ast
        .nodes
        .iter()
        .find_map(|node| match &node.kind {
            NodeKind::Function(path, signature, _) if *path == inner.instance_path => {
                Some(signature)
            }
            _ => None,
        })
        .expect("the nested sibling must emit its generated function");
    assert!(
        matches!(
            inner.build_result.ast.const_values.payload(
                signature.parameters[1].default.expect("suffix retains its folded default"),
            ),
            Some(ConstValuePayload::FixedScalar(value)) if *value == expected_default
        ),
        "the generated default must retain 41 without looking up the released fallback constant",
    );
    let lowered = lower_module(
        inner.build_result.ast,
        &mut strings,
        &mut paths,
        HirFunctionOriginLookup::default(),
        inner.build_result.module_resources,
        Default::default(),
        &nested.external_package_registry,
    )
    .expect("the nested sibling's selected alias and constant must lower after release");
    let expected_return = FixedScalarValue::signed(FixedScalar::I32, 7).expect("7 fits I32");
    assert!(
        lowered.hir_module.blocks.iter().any(|block| {
            let HirTerminator::Return(values) = &block.terminator else {
                return false;
            };
            lowered
                .hir_module
                .expressions
                .values(*values)
                .iter()
                .any(|value| {
                    matches!(
                        &lowered.hir_module.expressions.expression(*value).kind,
                        HirExpressionKind::FixedScalar(scalar) if *scalar == expected_return
                    )
                })
        }),
        "inner must return its exact sibling-only constant after all declaring captures are released",
    );
}
