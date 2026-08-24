//! Unit tests for `ScopeContext` construction invariants.
//!
//! WHAT: validates builder-pattern behaviour, context-kind transitions, and
//! visibility-gate synchronisation for the most commonly misused paths.
//! WHY: these properties are refactor seams — subtle mismatches (e.g. dropping
//! `expected_error_type` on context clone) silently corrupt later passes.

use super::environment::TopLevelDeclarationTable;
use super::scope_context::{ContextKind, ScopeContext};
use std::sync::Arc;

use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::{DataType, builtin_type_ids};
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;

use rustc_hash::FxHashSet;
use std::rc::Rc;

fn empty_scope(string_table: &mut StringTable) -> InternedPath {
    let mut path = InternedPath::new();
    path.push_str("test_scope", string_table);
    path
}

// ---------------------------------------------------------------------------
// new() invariants
// ---------------------------------------------------------------------------

#[test]
fn scope_context_new_leaves_no_visibility_gate() {
    let mut string_table = StringTable::new();
    let scope = empty_scope(&mut string_table);
    let context = ScopeContext::new_for_tests(
        ContextKind::Function,
        scope,
        Rc::new(TopLevelDeclarationTable::new(vec![])),
        Arc::new(ExternalPackageRegistry::new()),
        vec![],
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
    use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
    use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
    use crate::compiler_frontend::value_mode::ValueMode;

    let mut string_table = StringTable::new();
    let scope = empty_scope(&mut string_table);
    let mut context = ScopeContext::new_for_tests(
        ContextKind::Function,
        scope,
        Rc::new(TopLevelDeclarationTable::new(vec![])),
        Arc::new(ExternalPackageRegistry::new()),
        vec![],
        0,
    );

    // Install an empty visibility gate.
    context = context.with_visible_declarations(Arc::new(FxHashSet::default()));

    let variable_path = InternedPath::from_components(vec![string_table.intern("my_var")]);
    let declaration = Declaration {
        id: variable_path.to_owned(),
        value: Expression::new(
            ExpressionKind::NoValue,
            SourceLocation::default(),
            builtin_type_ids::INT,
            DataType::Int,
            ValueMode::ImmutableOwned,
        ),
    };
    context.add_var(declaration, SourceLocation::default());

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
    use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
    use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
    use crate::compiler_frontend::value_mode::ValueMode;

    let mut string_table = StringTable::new();
    let scope = empty_scope(&mut string_table);
    let mut context = ScopeContext::new_for_tests(
        ContextKind::Function,
        scope,
        Rc::new(TopLevelDeclarationTable::new(vec![])),
        Arc::new(ExternalPackageRegistry::new()),
        vec![],
        0,
    )
    .with_visible_declarations(Arc::new(FxHashSet::default()));

    let constant_name = string_table.intern("local_const");
    let constant_path = InternedPath::from_components(vec![constant_name]);
    let declaration = Declaration {
        id: constant_path.to_owned(),
        value: Expression::new(
            ExpressionKind::NoValue,
            SourceLocation::default(),
            builtin_type_ids::INT,
            DataType::Int,
            ValueMode::ImmutableOwned,
        ),
    };
    context.add_compile_time_var(declaration, SourceLocation::default());

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
        context.is_explicit_compile_time_constant(reference.as_declaration()),
        "compile-time local must retain its explicit # declaration fact"
    );
}

// ---------------------------------------------------------------------------
// new_template_parsing_context
// ---------------------------------------------------------------------------

#[test]
fn new_template_parsing_context_preserves_constant_kind() {
    let mut string_table = StringTable::new();
    let scope = empty_scope(&mut string_table);
    let context = ScopeContext::new_for_tests(
        ContextKind::Constant,
        scope,
        Rc::new(TopLevelDeclarationTable::new(vec![])),
        Arc::new(ExternalPackageRegistry::new()),
        vec![],
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
    let scope = empty_scope(&mut string_table);
    let context = ScopeContext::new_for_tests(
        ContextKind::Function,
        scope,
        Rc::new(TopLevelDeclarationTable::new(vec![])),
        Arc::new(ExternalPackageRegistry::new()),
        vec![],
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
fn new_template_parsing_context_propagates_expected_error_type() {
    let mut string_table = StringTable::new();
    let scope = empty_scope(&mut string_table);
    let mut context = ScopeContext::new_for_tests(
        ContextKind::Function,
        scope,
        Rc::new(TopLevelDeclarationTable::new(vec![])),
        Arc::new(ExternalPackageRegistry::new()),
        vec![],
        0,
    );
    let string_type_id = TypeEnvironment::new().builtins().string;
    context.expected_error_type = Some(string_type_id);

    let template = context.new_template_parsing_context();
    assert_eq!(
        template.expected_error_type,
        Some(string_type_id),
        "new_template_parsing_context must propagate expected_error_type"
    );
}

// ---------------------------------------------------------------------------
// new_child_control_flow — loop depth
// ---------------------------------------------------------------------------

#[test]
fn new_child_control_flow_increments_loop_depth_for_loop_kind() {
    let mut string_table = StringTable::new();
    let scope = empty_scope(&mut string_table);
    let context = ScopeContext::new_for_tests(
        ContextKind::Function,
        scope,
        Rc::new(TopLevelDeclarationTable::new(vec![])),
        Arc::new(ExternalPackageRegistry::new()),
        vec![],
        0,
    );
    assert_eq!(context.loop_depth, 0);

    let loop_context = context.new_child_control_flow(ContextKind::Loop, &mut string_table);
    assert_eq!(
        loop_context.loop_depth, 1,
        "entering a Loop scope must increment loop_depth"
    );

    let branch_context =
        loop_context.new_child_control_flow(ContextKind::Branch, &mut string_table);
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
    let scope = empty_scope(&mut string_table);
    let mut context = ScopeContext::new_for_tests(
        ContextKind::Function,
        scope.to_owned(),
        Rc::new(TopLevelDeclarationTable::new(vec![])),
        Arc::new(ExternalPackageRegistry::new()),
        vec![],
        0,
    );

    let mut visibility_gate = FxHashSet::default();
    let gated_path = InternedPath::from_components(vec![string_table.intern("gated")]);
    visibility_gate.insert(gated_path.to_owned());
    context = context.with_visible_declarations(Arc::new(visibility_gate));

    let constant_scope = InternedPath::from_components(vec![string_table.intern("const_scope")]);
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
    use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
    use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
    use crate::compiler_frontend::value_mode::ValueMode;

    let mut string_table = StringTable::new();
    let scope = empty_scope(&mut string_table);
    let mut context = ScopeContext::new_for_tests(
        ContextKind::Function,
        scope,
        Rc::new(TopLevelDeclarationTable::new(vec![])),
        Arc::new(ExternalPackageRegistry::new()),
        vec![],
        0,
    );

    let name = string_table.intern("ancestor_var");
    let variable_path = InternedPath::from_components(vec![name]);
    context.add_var(
        Declaration {
            id: variable_path.to_owned(),
            value: Expression::new(
                ExpressionKind::NoValue,
                SourceLocation::default(),
                builtin_type_ids::INT,
                DataType::Int,
                ValueMode::ImmutableOwned,
            ),
        },
        SourceLocation::default(),
    );

    let child = context.new_child_control_flow(ContextKind::Branch, &mut string_table);
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
    use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
    use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
    use crate::compiler_frontend::value_mode::ValueMode;

    let mut string_table = StringTable::new();
    let scope = empty_scope(&mut string_table);
    let context = ScopeContext::new_for_tests(
        ContextKind::Function,
        scope,
        Rc::new(TopLevelDeclarationTable::new(vec![])),
        Arc::new(ExternalPackageRegistry::new()),
        vec![],
        0,
    );

    let mut child = context.new_child_control_flow(ContextKind::Branch, &mut string_table);
    let name = string_table.intern("child_var");
    let variable_path = InternedPath::from_components(vec![name]);
    child.add_var(
        Declaration {
            id: variable_path.to_owned(),
            value: Expression::new(
                ExpressionKind::NoValue,
                SourceLocation::default(),
                builtin_type_ids::INT,
                DataType::Int,
                ValueMode::ImmutableOwned,
            ),
        },
        SourceLocation::default(),
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
fn child_function_frame_does_not_capture_parent_locals() {
    use crate::compiler_frontend::ast::ast_nodes::Declaration;
    use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
    use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
    use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
    use crate::compiler_frontend::value_mode::ValueMode;

    let mut string_table = StringTable::new();
    let scope = empty_scope(&mut string_table);
    let mut context = ScopeContext::new_for_tests(
        ContextKind::Function,
        scope,
        Rc::new(TopLevelDeclarationTable::new(vec![])),
        Arc::new(ExternalPackageRegistry::new()),
        vec![],
        0,
    );

    let parent_name = string_table.intern("outer_local");
    let parent_path = InternedPath::from_components(vec![parent_name]);
    context.add_var(
        Declaration {
            id: parent_path,
            value: Expression::new(
                ExpressionKind::NoValue,
                SourceLocation::default(),
                builtin_type_ids::INT,
                DataType::Int,
                ValueMode::ImmutableOwned,
            ),
        },
        SourceLocation::default(),
    );

    let function_name = string_table.intern("inner_function");
    let child = context.new_child_function(
        function_name,
        FunctionSignature::default(),
        &mut string_table,
    );

    assert!(
        child.get_reference(&parent_name).is_none(),
        "body-local functions must not capture outer local declarations"
    );
    assert_eq!(
        child.total_declaration_count(),
        0,
        "function child frame should start with only its parameters"
    );
}

#[test]
fn same_frame_duplicate_lookup_returns_latest_declaration() {
    use crate::compiler_frontend::ast::ast_nodes::Declaration;
    use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
    use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
    use crate::compiler_frontend::value_mode::ValueMode;

    let mut string_table = StringTable::new();
    let scope = empty_scope(&mut string_table);
    let mut context = ScopeContext::new_for_tests(
        ContextKind::Function,
        scope,
        Rc::new(TopLevelDeclarationTable::new(vec![])),
        Arc::new(ExternalPackageRegistry::new()),
        vec![],
        0,
    );

    let name = string_table.intern("duplicated");
    let first_path = InternedPath::from_components(vec![string_table.intern("first"), name]);
    let second_path = InternedPath::from_components(vec![string_table.intern("second"), name]);

    context.add_var(
        Declaration {
            id: first_path.to_owned(),
            value: Expression::new(
                ExpressionKind::NoValue,
                SourceLocation::default(),
                builtin_type_ids::INT,
                DataType::Int,
                ValueMode::ImmutableOwned,
            ),
        },
        SourceLocation::default(),
    );
    context.add_var(
        Declaration {
            id: second_path.to_owned(),
            value: Expression::new(
                ExpressionKind::NoValue,
                SourceLocation::default(),
                builtin_type_ids::INT,
                DataType::Int,
                ValueMode::ImmutableOwned,
            ),
        },
        SourceLocation::default(),
    );

    let resolved = context
        .get_reference(&name)
        .expect("same-frame name must resolve");
    assert_eq!(
        resolved.id, second_path,
        "lookup must return the latest declaration in the same frame"
    );
}

#[test]
fn no_shadowing_across_ancestor_frames() {
    use crate::compiler_frontend::ast::ast_nodes::Declaration;
    use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
    use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
    use crate::compiler_frontend::value_mode::ValueMode;

    let mut string_table = StringTable::new();
    let scope = empty_scope(&mut string_table);
    let mut context = ScopeContext::new_for_tests(
        ContextKind::Function,
        scope,
        Rc::new(TopLevelDeclarationTable::new(vec![])),
        Arc::new(ExternalPackageRegistry::new()),
        vec![],
        0,
    );

    let name = string_table.intern("shadowed");
    let parent_path = InternedPath::from_components(vec![string_table.intern("shadowed")]);
    context.add_var(
        Declaration {
            id: parent_path.to_owned(),
            value: Expression::new(
                ExpressionKind::NoValue,
                SourceLocation::default(),
                builtin_type_ids::INT,
                DataType::Int,
                ValueMode::ImmutableOwned,
            ),
        },
        SourceLocation::default(),
    );

    let child = context.new_child_control_flow(ContextKind::Branch, &mut string_table);
    assert!(
        child.has_visible_local_declaration(&name),
        "ancestor declaration must be visible to child so redeclaration can be rejected"
    );
    assert!(
        context.has_visible_local_declaration(&name),
        "declaration must be visible in the frame that owns it"
    );
}

#[test]
fn new_child_control_flow_inherits_visibility_gate() {
    let mut string_table = StringTable::new();
    let scope = empty_scope(&mut string_table);
    let mut context = ScopeContext::new_for_tests(
        ContextKind::Function,
        scope,
        Rc::new(TopLevelDeclarationTable::new(vec![])),
        Arc::new(ExternalPackageRegistry::new()),
        vec![],
        0,
    );

    let mut gate = FxHashSet::default();
    let gated_path = InternedPath::from_components(vec![string_table.intern("gated")]);
    gate.insert(gated_path.to_owned());
    context = context.with_visible_declarations(Arc::new(gate));

    let child = context.new_child_control_flow(ContextKind::Branch, &mut string_table);
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
    use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
    use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
    use crate::compiler_frontend::value_mode::ValueMode;

    let mut string_table = StringTable::new();
    let scope = empty_scope(&mut string_table);
    let mut context = ScopeContext::new_for_tests(
        ContextKind::Function,
        scope,
        Rc::new(TopLevelDeclarationTable::new(vec![])),
        Arc::new(ExternalPackageRegistry::new()),
        vec![],
        0,
    );

    // The gate every scope starts from is the header-built set, shared by handle. A child that
    // declares a local must copy it before writing, or the local would become visible to every
    // other scope in the module.
    let shared_gate = Arc::new(FxHashSet::default());
    context = context.with_visible_declarations(Arc::clone(&shared_gate));

    let local_path = InternedPath::from_components(vec![string_table.intern("branch_local")]);
    let declaration = Declaration {
        id: local_path.to_owned(),
        value: Expression::new(
            ExpressionKind::NoValue,
            SourceLocation::default(),
            builtin_type_ids::INT,
            DataType::Int,
            ValueMode::ImmutableOwned,
        ),
    };

    let mut child = context.new_child_control_flow(ContextKind::Branch, &mut string_table);
    child.add_var(declaration, SourceLocation::default());

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
    let scope = empty_scope(&mut string_table);
    let mut context = ScopeContext::new_for_tests(
        ContextKind::Function,
        scope,
        Rc::new(TopLevelDeclarationTable::new(vec![])),
        Arc::new(ExternalPackageRegistry::new()),
        vec![],
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
    use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
    use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
    use crate::compiler_frontend::value_mode::ValueMode;

    let mut string_table = StringTable::new();
    let scope = empty_scope(&mut string_table);
    let context = ScopeContext::new_for_tests(
        ContextKind::Function,
        scope,
        Rc::new(TopLevelDeclarationTable::new(vec![])),
        Arc::new(ExternalPackageRegistry::new()),
        vec![],
        0,
    );

    let name = string_table.intern("capture");
    let path = InternedPath::from_components(vec![name]);

    let mut clone = context.clone();
    clone.add_var(
        Declaration {
            id: path.to_owned(),
            value: Expression::new(
                ExpressionKind::NoValue,
                SourceLocation::default(),
                builtin_type_ids::INT,
                DataType::Int,
                ValueMode::ImmutableOwned,
            ),
        },
        SourceLocation::default(),
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
    use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
    use crate::compiler_frontend::tokenizer::tokens::SourceLocation;
    use crate::compiler_frontend::value_mode::ValueMode;

    let mut string_table = StringTable::new();
    let scope = empty_scope(&mut string_table);
    let mut context = ScopeContext::new_for_tests(
        ContextKind::Function,
        scope,
        Rc::new(TopLevelDeclarationTable::new(vec![])),
        Arc::new(ExternalPackageRegistry::new()),
        vec![],
        0,
    );

    let parent_name = string_table.intern("parent_var");
    let parent_path = InternedPath::from_components(vec![parent_name]);
    context.add_var(
        Declaration {
            id: parent_path,
            value: Expression::new(
                ExpressionKind::NoValue,
                SourceLocation::default(),
                builtin_type_ids::INT,
                DataType::Int,
                ValueMode::ImmutableOwned,
            ),
        },
        SourceLocation::default(),
    );

    let mut child = context.new_child_control_flow(ContextKind::Branch, &mut string_table);
    let child_name = string_table.intern("child_var");
    let child_path = InternedPath::from_components(vec![child_name]);
    child.add_var(
        Declaration {
            id: child_path,
            value: Expression::new(
                ExpressionKind::NoValue,
                SourceLocation::default(),
                builtin_type_ids::INT,
                DataType::Int,
                ValueMode::ImmutableOwned,
            ),
        },
        SourceLocation::default(),
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
