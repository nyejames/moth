//! Unit tests for `ScopeContext` construction invariants.
//!
//! WHAT: validates builder-pattern behaviour, context-kind transitions, and
//! visibility-gate synchronisation for the most commonly misused paths.
//! WHY: dropping an inherited callable contract silently changes nested exit semantics.

use super::environment::TopLevelDeclarationTable;
use super::scope_context::{
    ContextKind, EnclosingCallableContract, EnclosingCallableOwner, ScopeContext,
};
use crate::compiler_frontend::ast::ast_nodes::DeclarationValue;
use crate::compiler_frontend::ast::statements::value_production::{
    ActiveValueProductionTarget, ValueReceiverKind,
};
use std::sync::Arc;

use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::{DataType, builtin_type_ids};
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};

use moth_lexical::numeric::fixed_scalar::FixedScalar;
use rustc_hash::FxHashSet;
use std::rc::Rc;

fn empty_scope(path_fork: &mut PathInternerFork, string_table: &mut StringTable) -> PathId {
    path_fork
        .try_intern_portable_path("test_scope", string_table)
        .expect("test scope path should intern")
}

// ---------------------------------------------------------------------------
// new() invariants
// ---------------------------------------------------------------------------

#[test]
fn scope_context_new_leaves_no_visibility_gate() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let scope = empty_scope(&mut path_fork, &mut string_table);
    let context = ScopeContext::new_for_tests(
        ContextKind::Function,
        scope,
        Rc::new(TopLevelDeclarationTable::new(
            vec![],
            &PathInternerFork::empty(),
        )),
        Arc::new(ExternalPackageRegistry::new()),
        0,
    );
    assert!(
        context.visible_declaration_ids.is_none(),
        "ScopeContext::new must not install a visibility gate by default"
    );
}

// ---------------------------------------------------------------------------
// add_var — gate synchronisation
// ---------------------------------------------------------------------------

#[test]
fn add_var_extends_visibility_gate_when_gate_is_set() {
    use crate::compiler_frontend::ast::ast_nodes::Declaration;

    use crate::compiler_frontend::value_mode::ValueMode;

    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let scope = empty_scope(&mut path_fork, &mut string_table);
    let mut context = ScopeContext::new_for_tests(
        ContextKind::Function,
        scope,
        Rc::new(TopLevelDeclarationTable::new(
            vec![],
            &PathInternerFork::empty(),
        )),
        Arc::new(ExternalPackageRegistry::new()),
        0,
    );

    // Install an empty visibility gate.
    context = context.with_visible_declarations(Arc::new(FxHashSet::default()));

    let variable_path = path_fork
        .try_intern_components(&[string_table.intern("my_var")])
        .expect("test path fits");
    let declaration = Declaration {
        id: variable_path.to_owned(),
        type_id: builtin_type_ids::fixed_scalar(FixedScalar::I32),
        diagnostic_type: DataType::FixedScalar(FixedScalar::I32),
        value_mode: ValueMode::ImmutableOwned,
        value: DeclarationValue::Absent,
        binding_span: None,
        config_qualifier: None,
    };
    context.add_var(declaration, None, &path_fork);

    assert!(
        context
            .visible_declaration_ids
            .as_ref()
            .unwrap()
            .contains(&variable_path),
        "add_var must insert the new variable into the visibility gate"
    );
    assert_eq!(
        context.local_declarations().len(),
        1,
        "add_var must append the declaration to the local declaration layer"
    );
}

#[test]
fn add_compile_time_var_extends_visibility_gate_when_gate_is_set() {
    use crate::compiler_frontend::ast::ast_nodes::Declaration;

    use crate::compiler_frontend::value_mode::ValueMode;

    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let scope = empty_scope(&mut path_fork, &mut string_table);
    let mut context = ScopeContext::new_for_tests(
        ContextKind::Function,
        scope,
        Rc::new(TopLevelDeclarationTable::new(
            vec![],
            &PathInternerFork::empty(),
        )),
        Arc::new(ExternalPackageRegistry::new()),
        0,
    )
    .with_visible_declarations(Arc::new(FxHashSet::default()));

    let constant_name = string_table.intern("local_const");
    let constant_path = path_fork
        .try_intern_components(&[constant_name])
        .expect("test path fits");
    let declaration = Declaration {
        id: constant_path.to_owned(),
        type_id: builtin_type_ids::fixed_scalar(FixedScalar::I32),
        diagnostic_type: DataType::FixedScalar(FixedScalar::I32),
        value_mode: ValueMode::ImmutableOwned,
        value: DeclarationValue::Absent,
        binding_span: None,
        config_qualifier: None,
    };
    context.add_compile_time_var(declaration, None, &path_fork);

    assert!(
        context
            .visible_declaration_ids
            .as_ref()
            .unwrap()
            .contains(&constant_path),
        "add_compile_time_var must keep the visibility gate synchronized"
    );
    let reference = context
        .get_reference(&constant_name)
        .expect("compile-time local must resolve after insertion");
    assert!(
        context.is_explicit_compile_time_constant(
            reference
                .ready_declaration()
                .expect("compile-time local reference must be a ready declaration")
        ),
        "compile-time local must retain its explicit # declaration fact"
    );
}

// ---------------------------------------------------------------------------
// new_template_parsing_context
// ---------------------------------------------------------------------------

#[test]
fn new_template_parsing_context_preserves_constant_kind() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let scope = empty_scope(&mut path_fork, &mut string_table);
    let context = ScopeContext::new_for_tests(
        ContextKind::Constant,
        scope,
        Rc::new(TopLevelDeclarationTable::new(
            vec![],
            &PathInternerFork::empty(),
        )),
        Arc::new(ExternalPackageRegistry::new()),
        0,
    );
    let template_context = context.new_template_parsing_context();
    assert_eq!(
        template_context.kind,
        ContextKind::Constant,
        "constant contexts must stay constant when creating a template child"
    );
}

#[test]
fn new_template_parsing_context_converts_function_kind_to_template() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let scope = empty_scope(&mut path_fork, &mut string_table);
    let context = ScopeContext::new_for_tests(
        ContextKind::Function,
        scope,
        Rc::new(TopLevelDeclarationTable::new(
            vec![],
            &PathInternerFork::empty(),
        )),
        Arc::new(ExternalPackageRegistry::new()),
        0,
    );
    let template_context = context.new_template_parsing_context();
    assert_eq!(
        template_context.kind,
        ContextKind::Template,
        "non-constant contexts must become Template kind in a template child"
    );
}

#[test]
fn child_contexts_preserve_callable_identity_independently_of_scope_kind() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let scope = empty_scope(&mut path_fork, &mut string_table);
    let string_type_id = TypeEnvironment::new().builtins().string;
    for owner in [
        EnclosingCallableOwner::Function,
        EnclosingCallableOwner::EntryStart,
    ] {
        let mut context = ScopeContext::new_for_tests(
            ContextKind::Module,
            scope,
            Rc::new(TopLevelDeclarationTable::new(vec![], &path_fork)),
            Arc::new(ExternalPackageRegistry::new()),
            0,
        );
        let contract = Rc::new(EnclosingCallableContract {
            owner,
            success_types: vec![string_type_id],
            authored_success_slot_count: 1,
            error_type: Some(string_type_id),
        });
        context.enclosing_callable = Some(Rc::clone(&contract));
        context.expected_result_type_ids = vec![builtin_type_ids::BOOL];
        let branch =
            context.new_child_control_flow(ContextKind::Branch, &mut string_table, &mut path_fork);
        let loop_context =
            branch.new_child_control_flow(ContextKind::Loop, &mut string_table, &mut path_fork);
        let catch = loop_context.new_child_control_flow(
            ContextKind::CatchHandler,
            &mut string_table,
            &mut path_fork,
        );
        let expression = catch.new_child_expression(vec![builtin_type_ids::BOOL]);
        let mut value_block = expression.new_child_control_flow(
            ContextKind::Branch,
            &mut string_table,
            &mut path_fork,
        );
        value_block.active_value_target = Some(ActiveValueProductionTarget::known(
            vec![builtin_type_ids::BOOL],
            ValueReceiverKind::Declaration,
        ));
        let template = value_block.new_template_parsing_context();
        let constant = ScopeContext::new_constant(scope, &template);
        for child in [
            context.clone(),
            branch,
            loop_context,
            catch,
            expression,
            value_block,
            template,
            constant,
        ] {
            let inherited = child
                .enclosing_callable
                .as_ref()
                .expect("installed body contract");
            assert!(Rc::ptr_eq(inherited, &contract));
            assert_eq!(inherited.owner, owner);
            assert_eq!(inherited.success_types, vec![string_type_id]);
            assert_eq!(inherited.authored_success_slot_count, 1);
            assert_eq!(inherited.error_type, Some(string_type_id));
        }
    }
}

#[test]
fn context_kind_does_not_install_a_callable_contract() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let scope = empty_scope(&mut path_fork, &mut string_table);
    for kind in [
        ContextKind::Function,
        ContextKind::Module,
        ContextKind::CatchHandler,
    ] {
        let context = ScopeContext::new_for_tests(
            kind,
            scope,
            Rc::new(TopLevelDeclarationTable::new(vec![], &path_fork)),
            Arc::new(ExternalPackageRegistry::new()),
            0,
        );
        assert!(context.enclosing_callable.is_none());
        assert!(context.clone().enclosing_callable.is_none());
        assert!(
            context
                .new_child_expression(vec![])
                .enclosing_callable
                .is_none()
        );
        assert!(
            context
                .new_template_parsing_context()
                .enclosing_callable
                .is_none()
        );
    }
}

// ---------------------------------------------------------------------------
// new_child_control_flow — loop depth
// ---------------------------------------------------------------------------

#[test]
fn new_child_control_flow_increments_loop_depth_for_loop_kind() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let scope = empty_scope(&mut path_fork, &mut string_table);
    let context = ScopeContext::new_for_tests(
        ContextKind::Function,
        scope,
        Rc::new(TopLevelDeclarationTable::new(
            vec![],
            &PathInternerFork::empty(),
        )),
        Arc::new(ExternalPackageRegistry::new()),
        0,
    );
    assert_eq!(context.loop_depth, 0);

    let loop_context =
        context.new_child_control_flow(ContextKind::Loop, &mut string_table, &mut path_fork);
    assert_eq!(
        loop_context.loop_depth, 1,
        "entering a Loop scope must increment loop_depth"
    );

    let branch_context =
        loop_context.new_child_control_flow(ContextKind::Branch, &mut string_table, &mut path_fork);
    assert_eq!(
        branch_context.loop_depth, 1,
        "entering a Branch scope must not change loop_depth"
    );
}

// ---------------------------------------------------------------------------
// new_constant — inherits parent visibility gate
// ---------------------------------------------------------------------------

#[test]
fn new_constant_inherits_parent_visibility_gate() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let scope = empty_scope(&mut path_fork, &mut string_table);
    let mut context = ScopeContext::new_for_tests(
        ContextKind::Function,
        scope.to_owned(),
        Rc::new(TopLevelDeclarationTable::new(
            vec![],
            &PathInternerFork::empty(),
        )),
        Arc::new(ExternalPackageRegistry::new()),
        0,
    );

    let mut visibility_gate = FxHashSet::default();
    let gated_path = path_fork
        .try_intern_components(&[string_table.intern("gated")])
        .expect("test path fits");
    visibility_gate.insert(gated_path.to_owned());
    context = context.with_visible_declarations(Arc::new(visibility_gate));

    let constant_scope = path_fork
        .try_intern_components(&[string_table.intern("const_scope")])
        .expect("test path fits");
    let constant_context = ScopeContext::new_constant(constant_scope, &context);

    assert!(
        constant_context
            .visible_declaration_ids
            .as_ref()
            .unwrap()
            .contains(&gated_path),
        "new_constant must inherit the parent's visibility gate"
    );
}

// ---------------------------------------------------------------------------
// Scope-frame parent-chain lookup
// ---------------------------------------------------------------------------

#[test]
fn parent_frame_lookup_finds_ancestor_declaration() {
    use crate::compiler_frontend::ast::ast_nodes::Declaration;

    use crate::compiler_frontend::value_mode::ValueMode;

    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let scope = empty_scope(&mut path_fork, &mut string_table);
    let mut context = ScopeContext::new_for_tests(
        ContextKind::Function,
        scope,
        Rc::new(TopLevelDeclarationTable::new(
            vec![],
            &PathInternerFork::empty(),
        )),
        Arc::new(ExternalPackageRegistry::new()),
        0,
    );

    let name = string_table.intern("ancestor_var");
    let variable_path = path_fork
        .try_intern_components(&[name])
        .expect("test path fits");
    context.add_var(
        Declaration {
            id: variable_path.to_owned(),
            type_id: builtin_type_ids::fixed_scalar(FixedScalar::I32),
            diagnostic_type: DataType::FixedScalar(FixedScalar::I32),
            value_mode: ValueMode::ImmutableOwned,
            value: DeclarationValue::Absent,
            binding_span: None,
            config_qualifier: None,
        },
        None,
        &path_fork,
    );

    let child =
        context.new_child_control_flow(ContextKind::Branch, &mut string_table, &mut path_fork);
    assert!(
        child.get_reference(&name).is_some(),
        "child frame must resolve names declared in the parent frame"
    );
    assert_eq!(
        child.total_declaration_count(),
        1,
        "child should see one visible declaration across the frame chain"
    );
}

#[test]
fn child_frame_declaration_is_not_visible_to_parent() {
    use crate::compiler_frontend::ast::ast_nodes::Declaration;

    use crate::compiler_frontend::value_mode::ValueMode;

    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let scope = empty_scope(&mut path_fork, &mut string_table);
    let context = ScopeContext::new_for_tests(
        ContextKind::Function,
        scope,
        Rc::new(TopLevelDeclarationTable::new(
            vec![],
            &PathInternerFork::empty(),
        )),
        Arc::new(ExternalPackageRegistry::new()),
        0,
    );

    let mut child =
        context.new_child_control_flow(ContextKind::Branch, &mut string_table, &mut path_fork);
    let name = string_table.intern("child_var");
    let variable_path = path_fork
        .try_intern_components(&[name])
        .expect("test path fits");
    child.add_var(
        Declaration {
            id: variable_path.to_owned(),
            type_id: builtin_type_ids::fixed_scalar(FixedScalar::I32),
            diagnostic_type: DataType::FixedScalar(FixedScalar::I32),
            value_mode: ValueMode::ImmutableOwned,
            value: DeclarationValue::Absent,
            binding_span: None,
            config_qualifier: None,
        },
        None,
        &path_fork,
    );

    assert!(
        context.get_reference(&name).is_none(),
        "parent frame must not see declarations added to a child frame"
    );
    assert_eq!(
        context.total_declaration_count(),
        0,
        "parent should report zero visible declarations"
    );
    assert_eq!(
        child.local_declarations().len(),
        1,
        "child's own frame should contain exactly its declaration"
    );
}

#[test]
fn same_frame_duplicate_lookup_returns_latest_declaration() {
    use crate::compiler_frontend::ast::ast_nodes::Declaration;

    use crate::compiler_frontend::value_mode::ValueMode;

    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let scope = empty_scope(&mut path_fork, &mut string_table);
    let mut context = ScopeContext::new_for_tests(
        ContextKind::Function,
        scope,
        Rc::new(TopLevelDeclarationTable::new(
            vec![],
            &PathInternerFork::empty(),
        )),
        Arc::new(ExternalPackageRegistry::new()),
        0,
    );

    let name = string_table.intern("duplicated");
    let first_path = path_fork
        .try_intern_components(&[string_table.intern("first"), name])
        .expect("test path fits");
    let second_path = path_fork
        .try_intern_components(&[string_table.intern("second"), name])
        .expect("test path fits");

    context.add_var(
        Declaration {
            id: first_path.to_owned(),
            type_id: builtin_type_ids::fixed_scalar(FixedScalar::I32),
            diagnostic_type: DataType::FixedScalar(FixedScalar::I32),
            value_mode: ValueMode::ImmutableOwned,
            value: DeclarationValue::Absent,
            binding_span: None,
            config_qualifier: None,
        },
        None,
        &path_fork,
    );
    context.add_var(
        Declaration {
            id: second_path.to_owned(),
            type_id: builtin_type_ids::fixed_scalar(FixedScalar::I32),
            diagnostic_type: DataType::FixedScalar(FixedScalar::I32),
            value_mode: ValueMode::ImmutableOwned,
            value: DeclarationValue::Absent,
            binding_span: None,
            config_qualifier: None,
        },
        None,
        &path_fork,
    );

    let resolved = context
        .get_reference(&name)
        .expect("same-frame name must resolve");
    assert_eq!(
        resolved
            .ready_declaration()
            .expect("same-frame name must resolve to a ready declaration")
            .id,
        second_path,
        "lookup must return the latest declaration in the same frame"
    );
}

#[test]
fn no_shadowing_across_ancestor_frames() {
    use crate::compiler_frontend::ast::ast_nodes::Declaration;

    use crate::compiler_frontend::value_mode::ValueMode;

    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let scope = empty_scope(&mut path_fork, &mut string_table);
    let mut context = ScopeContext::new_for_tests(
        ContextKind::Function,
        scope,
        Rc::new(TopLevelDeclarationTable::new(
            vec![],
            &PathInternerFork::empty(),
        )),
        Arc::new(ExternalPackageRegistry::new()),
        0,
    );

    let name = string_table.intern("shadowed");
    let parent_path = path_fork
        .try_intern_components(&[string_table.intern("shadowed")])
        .expect("test path fits");
    context.add_var(
        Declaration {
            id: parent_path.to_owned(),
            type_id: builtin_type_ids::fixed_scalar(FixedScalar::I32),
            diagnostic_type: DataType::FixedScalar(FixedScalar::I32),
            value_mode: ValueMode::ImmutableOwned,
            value: DeclarationValue::Absent,
            binding_span: None,
            config_qualifier: None,
        },
        None,
        &path_fork,
    );

    let child =
        context.new_child_control_flow(ContextKind::Branch, &mut string_table, &mut path_fork);
    assert!(
        child.has_visible_declaration(&name),
        "ancestor declaration must be visible to child so redeclaration can be rejected"
    );
    assert!(
        context.has_visible_declaration(&name),
        "declaration must be visible in the frame that owns it"
    );
}

#[test]
fn new_child_control_flow_inherits_visibility_gate() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let scope = empty_scope(&mut path_fork, &mut string_table);
    let mut context = ScopeContext::new_for_tests(
        ContextKind::Function,
        scope,
        Rc::new(TopLevelDeclarationTable::new(
            vec![],
            &PathInternerFork::empty(),
        )),
        Arc::new(ExternalPackageRegistry::new()),
        0,
    );

    let mut gate = FxHashSet::default();
    let gated_path = path_fork
        .try_intern_components(&[string_table.intern("gated")])
        .expect("test path fits");
    gate.insert(gated_path.to_owned());
    context = context.with_visible_declarations(Arc::new(gate));

    let child =
        context.new_child_control_flow(ContextKind::Branch, &mut string_table, &mut path_fork);
    assert!(
        child
            .visible_declaration_ids
            .as_ref()
            .unwrap()
            .contains(&gated_path),
        "child control-flow frame must inherit the parent's visibility gate"
    );
}

#[test]
fn child_scope_local_does_not_leak_into_the_shared_visibility_gate() {
    use crate::compiler_frontend::ast::ast_nodes::Declaration;

    use crate::compiler_frontend::value_mode::ValueMode;

    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let scope = empty_scope(&mut path_fork, &mut string_table);
    let mut context = ScopeContext::new_for_tests(
        ContextKind::Function,
        scope,
        Rc::new(TopLevelDeclarationTable::new(
            vec![],
            &PathInternerFork::empty(),
        )),
        Arc::new(ExternalPackageRegistry::new()),
        0,
    );

    // The gate every scope starts from is the header-built set, shared by handle. A child that
    // declares a local must copy it before writing, or the local would become visible to every
    // other scope in the module.
    let shared_gate = Arc::new(FxHashSet::default());
    context = context.with_visible_declarations(Arc::clone(&shared_gate));

    let local_path = path_fork
        .try_intern_components(&[string_table.intern("branch_local")])
        .expect("test path fits");
    let declaration = Declaration {
        id: local_path.to_owned(),
        type_id: builtin_type_ids::fixed_scalar(FixedScalar::I32),
        diagnostic_type: DataType::FixedScalar(FixedScalar::I32),
        value_mode: ValueMode::ImmutableOwned,
        value: DeclarationValue::Absent,
        binding_span: None,
        config_qualifier: None,
    };

    let mut child =
        context.new_child_control_flow(ContextKind::Branch, &mut string_table, &mut path_fork);
    child.add_var(declaration, None, &path_fork);

    assert!(
        child
            .visible_declaration_ids
            .as_ref()
            .unwrap()
            .contains(&local_path),
        "the declaring child must see its own local"
    );
    assert!(
        !shared_gate.contains(&local_path),
        "a child local must not be written back into the shared header-built gate"
    );
    assert!(
        !context
            .visible_declaration_ids
            .as_ref()
            .unwrap()
            .contains(&local_path),
        "a child local must not become visible to its parent scope"
    );
}

#[test]
fn new_child_expression_propagates_expected_result_type_ids() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let scope = empty_scope(&mut path_fork, &mut string_table);
    let mut context = ScopeContext::new_for_tests(
        ContextKind::Function,
        scope,
        Rc::new(TopLevelDeclarationTable::new(
            vec![],
            &PathInternerFork::empty(),
        )),
        Arc::new(ExternalPackageRegistry::new()),
        0,
    );

    let string_type_id = TypeEnvironment::new().builtins().string;
    context.expected_result_type_ids = vec![string_type_id];

    let child = context.new_child_expression(vec![string_type_id]);
    assert_eq!(
        child.expected_result_type_ids,
        vec![string_type_id],
        "new_child_expression must propagate expected result types"
    );
    assert_eq!(
        child.loop_depth, context.loop_depth,
        "new_child_expression must preserve loop depth"
    );
}

// ---------------------------------------------------------------------------
// typed Vec arena frame isolation
// ---------------------------------------------------------------------------

#[test]
fn cloned_context_does_not_share_current_frame() {
    use crate::compiler_frontend::ast::ast_nodes::Declaration;

    use crate::compiler_frontend::value_mode::ValueMode;

    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let scope = empty_scope(&mut path_fork, &mut string_table);
    let context = ScopeContext::new_for_tests(
        ContextKind::Function,
        scope,
        Rc::new(TopLevelDeclarationTable::new(
            vec![],
            &PathInternerFork::empty(),
        )),
        Arc::new(ExternalPackageRegistry::new()),
        0,
    );

    let name = string_table.intern("capture");
    let path = path_fork
        .try_intern_components(&[name])
        .expect("test path fits");

    let mut clone = context.clone();
    clone.add_var(
        Declaration {
            id: path.to_owned(),
            type_id: builtin_type_ids::fixed_scalar(FixedScalar::I32),
            diagnostic_type: DataType::FixedScalar(FixedScalar::I32),
            value_mode: ValueMode::ImmutableOwned,
            value: DeclarationValue::Absent,
            binding_span: None,
            config_qualifier: None,
        },
        None,
        &path_fork,
    );

    assert!(
        context.get_reference(&name).is_none(),
        "original context must not see declarations added to a clone"
    );
    assert!(
        clone.get_reference(&name).is_some(),
        "clone must see its own frame-local declaration"
    );
}

#[test]
fn child_frame_shares_ancestors_but_not_current_frame() {
    use crate::compiler_frontend::ast::ast_nodes::Declaration;

    use crate::compiler_frontend::value_mode::ValueMode;

    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let scope = empty_scope(&mut path_fork, &mut string_table);
    let mut context = ScopeContext::new_for_tests(
        ContextKind::Function,
        scope,
        Rc::new(TopLevelDeclarationTable::new(
            vec![],
            &PathInternerFork::empty(),
        )),
        Arc::new(ExternalPackageRegistry::new()),
        0,
    );

    let parent_name = string_table.intern("parent_var");
    let parent_path = path_fork
        .try_intern_components(&[parent_name])
        .expect("test path fits");
    context.add_var(
        Declaration {
            id: parent_path,
            type_id: builtin_type_ids::fixed_scalar(FixedScalar::I32),
            diagnostic_type: DataType::FixedScalar(FixedScalar::I32),
            value_mode: ValueMode::ImmutableOwned,
            value: DeclarationValue::Absent,
            binding_span: None,
            config_qualifier: None,
        },
        None,
        &path_fork,
    );

    let mut child =
        context.new_child_control_flow(ContextKind::Branch, &mut string_table, &mut path_fork);
    let child_name = string_table.intern("child_var");
    let child_path = path_fork
        .try_intern_components(&[child_name])
        .expect("test path fits");
    child.add_var(
        Declaration {
            id: child_path,
            type_id: builtin_type_ids::fixed_scalar(FixedScalar::I32),
            diagnostic_type: DataType::FixedScalar(FixedScalar::I32),
            value_mode: ValueMode::ImmutableOwned,
            value: DeclarationValue::Absent,
            binding_span: None,
            config_qualifier: None,
        },
        None,
        &path_fork,
    );

    assert!(
        context.get_reference(&parent_name).is_some(),
        "parent must still see its own declaration"
    );
    assert!(
        context.get_reference(&child_name).is_none(),
        "parent must not see child frame declarations"
    );
    assert!(
        child.get_reference(&parent_name).is_some(),
        "child must inherit ancestor declarations"
    );
    assert!(
        child.get_reference(&child_name).is_some(),
        "child must see its own frame declarations"
    );
}

// ---------------------------------------------------------------------------
// pending numeric bindings
// ---------------------------------------------------------------------------

/// Insert one pending numeric binding into the context's current frame.
///
/// WHAT: declares a real binding through the context's body-owned construction domain and
///       installs the pending slot in the frame name index, exactly as body parsing does before
///       the binding's type is selected.
/// WHY: `ScopeDeclarationRef::PendingNumeric` must be reachable from lookup without a fabricated
///      `Declaration`, and the slot's domain must be the live context domain so the dense row the
///      ID indexes actually exists.
fn insert_pending_binding(
    context: &ScopeContext,
    name: StringId,
    path_fork: &mut PathInternerFork,
) -> crate::compiler_frontend::ast::numeric_construction::NumericBindingId {
    use crate::compiler_frontend::ast::module_ast::scope_context::AuthoredBindingFacts;
    use crate::compiler_frontend::ast::numeric_construction::{NumericFamily, NumericRecipe};
    use crate::compiler_frontend::value_mode::ValueMode;

    let binding_path = path_fork
        .try_intern_components(&[name])
        .expect("test path fits")
        .to_owned();
    context
        .declare_pending_numeric_binding(
            AuthoredBindingFacts {
                name,
                path: binding_path,
                span: None,
                binding_span: None,
                value_mode: ValueMode::ImmutableOwned,
                mutable: false,
            },
            NumericRecipe::Raw {
                items: Vec::new(),
                family: NumericFamily::Whole,
            },
        )
        .expect("test pending binding allocation fits")
}

#[test]
fn numeric_expression_policy_scopes_do_not_copy_body_declarations() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let scope = empty_scope(&mut path_fork, &mut string_table);
    let context = ScopeContext::new_for_tests(
        ContextKind::Function,
        scope,
        Rc::new(TopLevelDeclarationTable::new(
            vec![],
            &PathInternerFork::empty(),
        )),
        Arc::new(ExternalPackageRegistry::new()),
        0,
    );
    let names: Vec<_> = (0..128)
        .map(|index| {
            let name = string_table.intern(&format!("pending_{index}"));
            insert_pending_binding(&context, name, &mut path_fork);
            name
        })
        .collect();

    let deferred = context.new_pending_typed_error_expression_context();
    let mutation = context.with_pending_catch_assignment_targets(&names[..1]);
    let handler = mutation.activate_pending_catch_assignment_targets();
    for policy in [&deferred, &mutation, &handler] {
        assert!(policy.local_declarations().is_empty());
        assert_eq!(policy.kind, context.kind);
        assert!(!policy.numeric_body_root);
        for name in &names {
            assert!(policy.get_reference(name).is_some());
        }
    }
    assert!(deferred.defer_typed_error_validation);
    assert!(!context.defer_typed_error_validation);
    assert!(handler.is_assignment_target_unavailable(names[0]));
    assert!(!context.is_assignment_target_unavailable(names[0]));
    assert!(!mutation.is_assignment_target_unavailable(names[0]));
}

#[test]
fn pending_numeric_binding_is_visible_without_a_ready_declaration() {
    use crate::compiler_frontend::ast::module_ast::scope_context::ScopeDeclarationRef;

    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let scope = empty_scope(&mut path_fork, &mut string_table);
    let context = ScopeContext::new_for_tests(
        ContextKind::Function,
        scope,
        Rc::new(TopLevelDeclarationTable::new(
            vec![],
            &PathInternerFork::empty(),
        )),
        Arc::new(ExternalPackageRegistry::new()),
        0,
    );

    let name = string_table.intern("pending_value");
    let binding = insert_pending_binding(&context, name, &mut path_fork);

    let reference = context
        .get_reference(&name)
        .expect("a pending numeric binding must stay visible to lookup, never read as unknown");
    assert!(
        matches!(
            &reference,
            ScopeDeclarationRef::PendingNumeric {
                binding: found,
                ..
            } if *found == binding
        ),
        "lookup must expose the pending binding arm, not a placeholder declaration"
    );
    assert!(
        reference.ready_declaration().is_none(),
        "a pending numeric binding has no ready declaration and must not fake one"
    );
    assert_eq!(reference.pending_binding(), Some(binding));
    assert!(reference.binding_span().is_none());
}

#[test]
fn pending_numeric_binding_upgrades_in_place_and_keeps_its_name() {
    use crate::compiler_frontend::ast::ast_nodes::Declaration;
    use crate::compiler_frontend::value_mode::ValueMode;

    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let scope = empty_scope(&mut path_fork, &mut string_table);
    let context = ScopeContext::new_for_tests(
        ContextKind::Function,
        scope,
        Rc::new(TopLevelDeclarationTable::new(
            vec![],
            &PathInternerFork::empty(),
        )),
        Arc::new(ExternalPackageRegistry::new()),
        0,
    );

    let name = string_table.intern("pending_value");
    let binding = insert_pending_binding(&context, name, &mut path_fork);
    let binding_path = path_fork
        .try_intern_components(&[name])
        .expect("test path fits")
        .to_owned();
    let declaration = Declaration {
        id: binding_path.to_owned(),
        type_id: builtin_type_ids::fixed_scalar(FixedScalar::I32),
        diagnostic_type: DataType::FixedScalar(FixedScalar::I32),
        value_mode: ValueMode::ImmutableOwned,
        value: DeclarationValue::Absent,
        binding_span: None,
        config_qualifier: None,
    };

    let upgraded = context
        .arena
        .borrow_mut()
        .frame_mut(context.current_frame_id)
        .upgrade_pending_numeric_var(name, context.numeric_domain_id(), binding, declaration);
    assert!(upgraded, "the pending slot must upgrade in place");

    let reference = context
        .get_reference(&name)
        .expect("the upgraded name must keep resolving");
    assert_eq!(
        reference
            .ready_declaration()
            .expect("upgraded slot must be ready")
            .id,
        binding_path,
        "in-place upgrade must preserve the authored binding identity"
    );
    assert_eq!(reference.pending_binding(), None);
    assert_eq!(
        context.local_declarations().len(),
        1,
        "upgrade must replace the slot without adding a second name entry"
    );
}

#[test]
fn explicit_constant_lookup_uses_content_owner_and_authored_binding_role() {
    use crate::compiler_frontend::ast::ast_nodes::Declaration;
    use crate::compiler_frontend::ast::const_values::store::{ConstValuePayload, ConstValueStore};
    use crate::compiler_frontend::value_mode::ValueMode;

    let mut strings = StringTable::new();
    let mut paths = PathInternerFork::empty();
    let scope = empty_scope(&mut paths, &mut strings);
    let mut context = ScopeContext::new_for_tests(
        ContextKind::Function,
        scope,
        Rc::new(TopLevelDeclarationTable::new(vec![], &paths)),
        Arc::new(ExternalPackageRegistry::new()),
        0,
    );
    let mut constants = ConstValueStore::default();
    let content = constants
        .insert_resolved(builtin_type_ids::BOOL, ConstValuePayload::Bool(true), None)
        .expect("fixture constant belongs to this content owner");
    let constant_path = paths
        .try_intern_child(scope, strings.intern("constant"))
        .expect("fixture path fits");
    let runtime_path = paths
        .try_intern_child(scope, strings.intern("runtime"))
        .expect("fixture path fits");

    for (path, is_constant) in [(constant_path, true), (runtime_path, false)] {
        constants
            .bind_body_local(path, content)
            .expect("fixture binding fits");
        let declaration = Declaration {
            id: path,
            type_id: builtin_type_ids::BOOL,
            diagnostic_type: DataType::Bool,
            value_mode: ValueMode::ImmutableOwned,
            value: DeclarationValue::Absent,
            binding_span: None,
            config_qualifier: None,
        };
        if is_constant {
            context.add_compile_time_var(declaration, None, &paths);
        } else {
            context.add_var(declaration, None, &paths);
        }
    }

    assert_eq!(
        context.explicit_constant_value(&constant_path, &paths, &constants),
        Some(content)
    );
    assert_eq!(
        context.explicit_constant_value(&runtime_path, &paths, &constants),
        None
    );
}
