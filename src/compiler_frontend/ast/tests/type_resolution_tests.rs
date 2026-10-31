//! Parsed type-resolution regression tests.
//!
//! WHAT: validates parsed type annotations, canonical semantic identities, and diagnostics.
//! WHY: these paths sit at the boundary between authored syntax and canonical type identity;
//!      mistakes here produce misleading errors or silent wrong-types.

use crate::compiler_frontend::ast::ast_nodes::{Declaration, NodeKind};
use crate::compiler_frontend::ast::ast_nodes::{DeclarationShell, DeclarationValue};
use crate::compiler_frontend::ast::const_eval::{
    ConstantFoldOutcome, FoldRequirement, FoldServices,
};
use crate::compiler_frontend::ast::const_values::store::{
    ConstValueMetadata, ConstValuePayload, ConstValueStore,
};
use crate::compiler_frontend::ast::expressions::expression_kind::Operator;
use crate::compiler_frontend::ast::expressions::expression_store::AstExpressionBuilder;
use crate::compiler_frontend::ast::expressions::expression_types::{
    ConstRecordState, ConstValueKind,
};
use crate::compiler_frontend::ast::module_ast::scope_context::{ContextKind, ScopeContext};
use crate::compiler_frontend::ast::type_resolution::{
    StructFieldResolutionError, TypeResolutionContext, resolve_parsed_type_annotation,
    resolve_struct_field_types, validate_map_key_type,
};
use crate::compiler_frontend::ast::{Ast, TopLevelDeclarationTable};
use crate::compiler_frontend::compiler_messages::diagnostics::draft::DiagnosticDraft;
use crate::compiler_frontend::compiler_messages::diagnostics::families::calls_data::{
    InvalidCollectionTypeCode, InvalidCollectionTypeView, InvalidMapTypeCode, InvalidMapTypeView,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::names_declarations::{
    UnknownNamespaceMemberView, UnknownTypeNameView,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::project_config::NamespaceTypeValueMisuseView;
use crate::compiler_frontend::compiler_messages::diagnostics::foreign::SiteRef;
use crate::compiler_frontend::compiler_messages::diagnostics::test_support::{
    CompactDiagnosticBagForTest, compact_bag_for_test,
};
use crate::compiler_frontend::compiler_messages::{
    DiagnosticBag, NamespaceMemberRole, NamespaceRecordKind, NamespaceTypeValueMisuseKind,
};
use crate::compiler_frontend::datatypes::builtin_type_ids;
use crate::compiler_frontend::datatypes::definitions::ChoiceVariantPayloadDefinition;
use crate::compiler_frontend::datatypes::display::{TypeDisplayShape, TypeDisplayView};
use crate::compiler_frontend::datatypes::environment::TypeEnvironment;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::datatypes::parsed::{ParsedCollectionCapacity, ParsedTypeRef};
use crate::compiler_frontend::external_packages::ExternalPackageRegistry;
use crate::compiler_frontend::folded_value::{
    PublicConstTemplate, PublicConstTemplateKind, PublicConstTemplatePiece,
    PublicConstTemplateSlot, PublicTemplateSlotKey,
};
use crate::compiler_frontend::headers::binding_environment::{
    NamespaceRecord, NamespaceRecordSource, NamespaceTypeMember, NamespaceValueMember,
    SourceDeclarationTarget,
};
use crate::compiler_frontend::source::{ExtendedSpanBuilder, LocalSpan, SourceId, SourceSpan};
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::compiler_frontend::synthetic_interface_provenance::{
    SyntheticInterfaceClass, SyntheticInterfaceMemberIdentity, SyntheticInterfaceProvenance,
};
use crate::compiler_frontend::tests::parse_support::{
    assert_ast_diagnostic, ast_error_label_spans, ast_error_primary_span,
    ast_first_error_rendered_message, parse_single_file_ast, parse_single_file_ast_diagnostic,
};
use crate::compiler_frontend::value_mode::ValueMode;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};
use rustc_hash::{FxHashMap, FxHashSet};
use std::rc::Rc;
use std::sync::Arc;

fn compact_diagnostic(
    diagnostic: impl Into<DiagnosticDraft>,
    environment: Option<&TypeEnvironment>,
) -> CompactDiagnosticBagForTest {
    compact_bag_for_test(DiagnosticBag::from(diagnostic.into()), environment)
}

fn site_span(site: SiteRef<'_>) -> Option<SourceSpan> {
    match site {
        SiteRef::Absent => None,
        SiteRef::Local(span) | SiteRef::Foreign { span, .. } => Some(span),
    }
}
fn folded_expression_root(
    outcome: ConstantFoldOutcome,
) -> crate::compiler_frontend::ast::expressions::expression_store::ExprId {
    match outcome {
        ConstantFoldOutcome::Folded(root) => root,
        ConstantFoldOutcome::NotConstant(_) => {
            panic!("expected a foldable constant expression")
        }
        ConstantFoldOutcome::Refused { reason, .. } => {
            panic!("constant expression unexpectedly refused folding: {reason:?}")
        }
    }
}

fn expected_source_span(source: &str, text: &str) -> SourceSpan {
    let start = source
        .find(text)
        .unwrap_or_else(|| panic!("test source should contain {text:?}"));
    let mut builder = ExtendedSpanBuilder::new();
    let local = LocalSpan::exact(start as u32, text.len() as u32, &mut builder)
        .expect("test anchor should fit the local span table");
    SourceSpan::new(SourceId::COMPILATION_ROOT, local)
}

#[test]
fn signature_member_ast_diagnostic_retains_exact_member_anchor() {
    let source = "bad |items {F64 = I32}|:\n;\n";
    let failure = parse_single_file_ast_diagnostic(source);
    let diagnostic = assert_ast_diagnostic(
        &failure,
        "MOTH-SYNTAX-0016-MAP",
        Some("invalid_map_type.unsupported_key_type"),
    );
    assert!(
        ast_error_label_spans(diagnostic).contains(&Some(expected_source_span(source, "items"))),
        "AST signature diagnostics should retain the authored member anchor"
    );
}

#[test]
fn template_nonconstant_reference_in_struct_default_keeps_invalid_default_classification() {
    let source = "runtime || -> String:\n\
    return \"dynamic\"\n\
;\n\
Widget = |\n\
    content String = [: [runtime()]],\n\
|\n";
    let failure = parse_single_file_ast_diagnostic(source);
    let diagnostic = assert_ast_diagnostic(&failure, "MOTH-RULE-0030", None);

    let reference_start = source
        .rfind("runtime")
        .expect("the template default should reference the runtime function");
    let mut span_builder = ExtendedSpanBuilder::new();
    let reference_span = SourceSpan::new(
        SourceId::COMPILATION_ROOT,
        LocalSpan::exact(
            reference_start as u32,
            "runtime".len() as u32,
            &mut span_builder,
        )
        .expect("the template function reference should fit an inline span"),
    );
    assert_eq!(ast_error_primary_span(diagnostic), Some(reference_span));
}

#[test]
fn choice_variant_ast_diagnostic_retains_exact_variant_anchor() {
    let source = "Status :: WithValue |value {F64 = I32}|;\n";
    let failure = parse_single_file_ast_diagnostic(source);
    let diagnostic = assert_ast_diagnostic(
        &failure,
        "MOTH-SYNTAX-0016-MAP",
        Some("invalid_map_type.unsupported_key_type"),
    );
    assert!(
        ast_error_label_spans(diagnostic)
            .contains(&Some(expected_source_span(source, "WithValue"))),
        "AST choice diagnostics should retain the authored variant anchor"
    );
}

#[test]
fn ast_generic_parameter_collision_retains_exact_multibyte_offset() {
    let source = "-- é🦋\n\
Existing = |\n\
    value I32,\n\
|\n\
\n\
Box type Existing = |\n\
    value Existing,\n\
|\n";
    let diagnostic = parse_single_file_ast_diagnostic(source);

    let diagnostic = assert_ast_diagnostic(
        &diagnostic,
        "MOTH-RULE-0043",
        Some("invalid_declaration.generic_parameter_name_collision"),
    );

    let parameter_start = source
        .find("Box type ")
        .expect("generic declaration should be present")
        + "Box type ".len();
    let mut span_builder = ExtendedSpanBuilder::new();
    let parameter_span = LocalSpan::exact(
        parameter_start as u32,
        "Existing".len() as u32,
        &mut span_builder,
    )
    .expect("generic parameter span should fit the test span table");
    assert_eq!(
        ast_error_primary_span(diagnostic),
        Some(SourceSpan::new(SourceId::COMPILATION_ROOT, parameter_span)),
        "AST scope diagnostics should retain the colliding parameter's exact byte span"
    );
}

#[test]
fn callable_shell_type_lookup_rejects_bare_and_qualified_annotations() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let callable_path = path_fork
        .try_intern_portable_path("marker", &mut string_table)
        .expect("test path fits");
    let const_values = ConstValueStore::default();
    let callable_name = string_table.intern("marker");
    let namespace_name = string_table.intern("source");
    let span = Some(expected_source_span("value marker = \"ok\"", "marker"));
    let declaration_table = Rc::new(TopLevelDeclarationTable::new(
        vec![Declaration {
            id: callable_path,
            type_id: builtin_type_ids::NONE,
            type_syntax: None,
            value_mode: ValueMode::ImmutableReference,
            value: DeclarationValue::Shell(DeclarationShell::Function { receiver: None }),
            binding_span: None,
            config_qualifier: None,
        }],
        &path_fork,
    ));
    let source_bindings =
        FxHashMap::from_iter([(callable_name, SourceDeclarationTarget::Local(callable_path))]);
    // Even a type-member projection must check the declaration's authoritative shell kind.
    let mut record = NamespaceRecord::empty(NamespaceRecordSource::SourceFile(callable_path));
    record.type_members.insert(
        callable_name,
        NamespaceTypeMember::SourceDeclaration(SourceDeclarationTarget::Local(callable_path)),
    );
    let namespace_records = FxHashMap::from_iter([(namespace_name, record)]);

    for lookup_path in 0..3 {
        let mut type_environment = TypeEnvironment::new();
        let mut context = TypeResolutionContext::from_declaration_table(
            &declaration_table,
            &mut type_environment,
            &const_values,
        );
        if lookup_path == 1 {
            context.visible_source_bindings = Some(&source_bindings);
        }
        context.visible_namespace_records = Some(&namespace_records);
        let annotation = if lookup_path == 2 {
            ParsedTypeRef::Qualified {
                path: vec![namespace_name, callable_name],
                span,
            }
        } else {
            ParsedTypeRef::Named {
                name: callable_name,
                span,
            }
        };
        let diagnostic = resolve_parsed_type_annotation(
            &annotation,
            span,
            &mut context,
            &mut string_table,
            None,
        )
        .expect_err("an explicit callable annotation must not become inferred");

        let compact = compact_diagnostic(diagnostic, None);
        let diagnostic = compact.first().expect("misuse emits one diagnostic");
        assert_eq!(diagnostic.descriptor().code, "MOTH-RULE-0067");
        assert_eq!(site_span(diagnostic.primary_site()), span);
        let facts = diagnostic
            .family_view::<NamespaceTypeValueMisuseView>()
            .expect("callable annotation misuse should retain typed facts");
        assert_eq!(facts.name, callable_name);
        assert_eq!(facts.expected, NamespaceTypeValueMisuseKind::Type);
        assert_eq!(facts.found, NamespaceTypeValueMisuseKind::Value);
    }
}

// ---------------------------------------------------------------
//  Parsed annotation diagnostics
// ---------------------------------------------------------------

#[test]
fn parsed_annotation_resolution_preserves_unresolved_authored_name() {
    let mut string_table = StringTable::new();
    let path_fork = PathInternerFork::empty();
    let const_values = ConstValueStore::default();
    let declaration_table = Rc::new(TopLevelDeclarationTable::new(Vec::new(), &path_fork));
    let mut type_environment = TypeEnvironment::new();
    let mut context = TypeResolutionContext::from_declaration_table(
        &declaration_table,
        &mut type_environment,
        &const_values,
    );
    let missing = string_table.intern("Missing");
    let annotation = ParsedTypeRef::Named {
        name: missing,
        span: None,
    };

    let diagnostic =
        resolve_parsed_type_annotation(&annotation, None, &mut context, &mut string_table, None)
            .expect_err("an unresolved authored name must remain a user-facing type error");

    let compact = compact_diagnostic(diagnostic, None);
    let diagnostic = compact.first().expect("unknown type emits one diagnostic");
    assert_eq!(diagnostic.descriptor().code, "MOTH-RULE-0035");
    let facts = diagnostic
        .family_view::<UnknownTypeNameView>()
        .expect("unknown type should retain typed name facts");
    assert_eq!(facts.name, missing);
}

#[test]
fn parsed_annotation_resolution_prefers_inner_spans_for_nested_unknown_types() {
    let source =
        "outer_collection {MissingCollection} = {1}\nouter_map {String = MissingMap} = {}\n";
    let mut string_table = StringTable::new();
    let path_fork = PathInternerFork::empty();
    let const_values = ConstValueStore::default();
    let declaration_table = Rc::new(TopLevelDeclarationTable::new(Vec::new(), &path_fork));
    let mut type_environment = TypeEnvironment::new();
    let mut context = TypeResolutionContext::from_declaration_table(
        &declaration_table,
        &mut type_environment,
        &const_values,
    );

    let collection_name = string_table.intern("MissingCollection");
    let collection = ParsedTypeRef::Collection {
        element: Box::new(ParsedTypeRef::Named {
            name: collection_name,
            span: Some(expected_source_span(source, "MissingCollection")),
        }),
        span: Some(expected_source_span(source, "{MissingCollection}")),
        fixed_capacity: None,
    };
    let collection_diagnostic = resolve_parsed_type_annotation(
        &collection,
        Some(expected_source_span(source, "outer_collection")),
        &mut context,
        &mut string_table,
        None,
    )
    .expect_err("unknown collection elements should report their authored name");

    let compact = compact_diagnostic(collection_diagnostic, None);
    let diagnostic = compact
        .first()
        .expect("unknown collection item emits a diagnostic");
    assert_eq!(diagnostic.descriptor().code, "MOTH-RULE-0035");
    assert_eq!(
        site_span(diagnostic.primary_site()),
        Some(expected_source_span(source, "MissingCollection"))
    );
    let facts = diagnostic
        .family_view::<UnknownTypeNameView>()
        .expect("unknown collection element should retain typed name facts");
    assert_eq!(facts.name, collection_name);

    let map_value_name = string_table.intern("MissingMap");
    let map = ParsedTypeRef::Map {
        key: Box::new(ParsedTypeRef::BuiltinString { span: None }),
        value: Box::new(ParsedTypeRef::Named {
            name: map_value_name,
            span: Some(expected_source_span(source, "MissingMap")),
        }),
        span: Some(expected_source_span(source, "{String = MissingMap}")),
    };
    let map_diagnostic = resolve_parsed_type_annotation(
        &map,
        Some(expected_source_span(source, "outer_map")),
        &mut context,
        &mut string_table,
        None,
    )
    .expect_err("unknown map values should report their authored name");

    let compact = compact_diagnostic(map_diagnostic, None);
    let diagnostic = compact
        .first()
        .expect("unknown map value emits a diagnostic");
    assert_eq!(diagnostic.descriptor().code, "MOTH-RULE-0035");
    assert_eq!(
        site_span(diagnostic.primary_site()),
        Some(expected_source_span(source, "MissingMap"))
    );
    let facts = diagnostic
        .family_view::<UnknownTypeNameView>()
        .expect("unknown map value should retain typed name facts");
    assert_eq!(facts.name, map_value_name);
}

#[test]
fn parsed_annotation_resolution_rejects_unknown_qualified_type_name() {
    let source = "source.QualifiedMissing";
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let namespace_name = string_table.intern("source");
    let namespace_path = path_fork
        .try_intern_portable_path("source", &mut string_table)
        .expect("test path fits");
    let missing_name = string_table.intern("QualifiedMissing");
    let const_values = ConstValueStore::default();
    let declaration_table = Rc::new(TopLevelDeclarationTable::new(Vec::new(), &path_fork));
    let mut type_environment = TypeEnvironment::new();
    let mut context = TypeResolutionContext::from_declaration_table(
        &declaration_table,
        &mut type_environment,
        &const_values,
    );
    let namespace_records = FxHashMap::from_iter([(
        namespace_name,
        NamespaceRecord::empty(NamespaceRecordSource::SourceFile(namespace_path)),
    )]);
    context.visible_namespace_records = Some(&namespace_records);
    let annotation = ParsedTypeRef::Qualified {
        path: vec![namespace_name, missing_name],
        span: Some(expected_source_span(source, "source.QualifiedMissing")),
    };

    let diagnostic =
        resolve_parsed_type_annotation(&annotation, None, &mut context, &mut string_table, None)
            .expect_err("an unknown qualified type name must be rejected");

    let compact = compact_diagnostic(diagnostic, None);
    let diagnostic = compact
        .first()
        .expect("qualified unknown type emits a diagnostic");
    assert_eq!(diagnostic.descriptor().code, "MOTH-RULE-0089");
    assert_eq!(
        site_span(diagnostic.primary_site()),
        Some(expected_source_span(source, "source.QualifiedMissing"))
    );
    let facts = diagnostic
        .family_view::<UnknownNamespaceMemberView>()
        .expect("qualified unknown type should retain typed namespace-member facts");
    assert_eq!(facts.member, missing_name);
    assert_eq!(facts.namespace, namespace_name);
    assert_eq!(facts.member_role, NamespaceMemberRole::Type);
    assert_eq!(facts.namespace_kind, NamespaceRecordKind::Source);
}

fn namespaced_record_with_mixed_members(
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> (StringId, StringId, StringId, StringId, NamespaceRecord) {
    let math_name = string_table.intern("math");
    let package = string_table.intern("@core/math");
    let point_name = string_table.intern("Point");
    let pi_name = string_table.intern("PI");
    let tools_name = string_table.intern("tools");
    let point_path = path_fork
        .try_intern_portable_path("Point", string_table)
        .expect("test path fits");
    let pi_path = path_fork
        .try_intern_portable_path("PI", string_table)
        .expect("test path fits");
    let mut record = NamespaceRecord::empty(NamespaceRecordSource::ExternalPackage(package));
    record.type_members.insert(
        point_name,
        NamespaceTypeMember::SourceDeclaration(SourceDeclarationTarget::Local(point_path)),
    );
    record.value_members.insert(
        pi_name,
        NamespaceValueMember::SourceDeclaration(SourceDeclarationTarget::Local(pi_path)),
    );
    record.child_namespaces.insert(
        tools_name,
        NamespaceRecord::empty(NamespaceRecordSource::ExternalPackage(package)),
    );
    (math_name, point_name, pi_name, tools_name, record)
}

#[test]
fn namespaced_type_member_candidates_exclude_values_and_child_namespaces() {
    let source = "math.Poin";
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (math_name, point_name, pi_name, tools_name, record) =
        namespaced_record_with_mixed_members(&mut string_table, &mut path_fork);
    let poin_name = string_table.intern("Poin");
    let const_values = ConstValueStore::default();
    let declaration_table = Rc::new(TopLevelDeclarationTable::new(Vec::new(), &path_fork));
    let mut type_environment = TypeEnvironment::new();
    let mut context = TypeResolutionContext::from_declaration_table(
        &declaration_table,
        &mut type_environment,
        &const_values,
    );
    let namespace_records = FxHashMap::from_iter([(math_name, record)]);
    context.visible_namespace_records = Some(&namespace_records);
    let annotation = ParsedTypeRef::Qualified {
        path: vec![math_name, poin_name],
        span: Some(expected_source_span(source, "math.Poin")),
    };

    let diagnostic =
        resolve_parsed_type_annotation(&annotation, None, &mut context, &mut string_table, None)
            .expect_err("unknown type member must be rejected");
    let compact = compact_diagnostic(diagnostic, None);
    let diagnostic = compact
        .first()
        .expect("unknown type member emits a diagnostic");
    assert_eq!(diagnostic.descriptor().code, "MOTH-RULE-0089");
    let facts = diagnostic
        .family_view::<UnknownNamespaceMemberView>()
        .expect("unknown type member should retain typed facts");
    assert_eq!(facts.member_role, NamespaceMemberRole::Type);
    assert!(
        diagnostic.string_ids().contains(&point_name),
        "type members must be candidates"
    );
    assert!(
        !diagnostic.string_ids().contains(&pi_name),
        "value members must not be type-position candidates"
    );
    assert!(
        !diagnostic.string_ids().contains(&tools_name),
        "child namespaces must not be type-position candidates"
    );
}

#[test]
fn namespaced_type_member_skips_candidates_when_requested() {
    let source = "math.Poin";
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (math_name, point_name, _, _, record) =
        namespaced_record_with_mixed_members(&mut string_table, &mut path_fork);
    let poin_name = string_table.intern("Poin");
    let const_values = ConstValueStore::default();
    let declaration_table = Rc::new(TopLevelDeclarationTable::new(Vec::new(), &path_fork));
    let mut type_environment = TypeEnvironment::new();
    let mut context = TypeResolutionContext::from_declaration_table(
        &declaration_table,
        &mut type_environment,
        &const_values,
    );
    let namespace_records = FxHashMap::from_iter([(math_name, record)]);
    context.visible_namespace_records = Some(&namespace_records);
    context.skip_unknown_type_name_candidates = true;
    let annotation = ParsedTypeRef::Qualified {
        path: vec![math_name, poin_name],
        span: Some(expected_source_span(source, "math.Poin")),
    };

    let diagnostic =
        resolve_parsed_type_annotation(&annotation, None, &mut context, &mut string_table, None)
            .expect_err("unknown type member must be rejected");
    let compact = compact_diagnostic(diagnostic, None);
    let diagnostic = compact
        .first()
        .expect("unknown type member emits a diagnostic");
    assert_eq!(diagnostic.descriptor().code, "MOTH-RULE-0089");
    assert!(
        diagnostic.string_ids().is_empty(),
        "skip_unknown_type_name_candidates must suppress member suggestions"
    );
    assert!(
        !diagnostic.string_ids().contains(&point_name),
        "skipped candidate collection must not retain type members"
    );
}

#[test]
fn namespaced_nested_type_member_names_the_walked_path() {
    let source = "math.tools.Missing";
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (math_name, _, _, tools_name, record) =
        namespaced_record_with_mixed_members(&mut string_table, &mut path_fork);
    let missing_name = string_table.intern("Missing");
    let const_values = ConstValueStore::default();
    let declaration_table = Rc::new(TopLevelDeclarationTable::new(Vec::new(), &path_fork));
    let mut type_environment = TypeEnvironment::new();
    let mut context = TypeResolutionContext::from_declaration_table(
        &declaration_table,
        &mut type_environment,
        &const_values,
    );
    let namespace_records = FxHashMap::from_iter([(math_name, record)]);
    context.visible_namespace_records = Some(&namespace_records);
    let annotation = ParsedTypeRef::Qualified {
        path: vec![math_name, tools_name, missing_name],
        span: Some(expected_source_span(source, "math.tools.Missing")),
    };

    let diagnostic =
        resolve_parsed_type_annotation(&annotation, None, &mut context, &mut string_table, None)
            .expect_err("unknown nested type member must be rejected");
    let compact = compact_diagnostic(diagnostic, None);
    let diagnostic = compact
        .first()
        .expect("unknown nested type member emits a diagnostic");
    let facts = diagnostic
        .family_view::<UnknownNamespaceMemberView>()
        .expect("nested unknown type should retain typed facts");
    let expected_namespace = string_table.intern("math.tools");
    assert_eq!(facts.namespace, expected_namespace);
    assert_eq!(facts.member, missing_name);
}

#[test]
fn namespaced_intermediate_member_candidates_are_child_namespaces_only() {
    let source = "math.toolz.Point";
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let (math_name, point_name, pi_name, tools_name, record) =
        namespaced_record_with_mixed_members(&mut string_table, &mut path_fork);
    let toolz_name = string_table.intern("toolz");
    let const_values = ConstValueStore::default();
    let declaration_table = Rc::new(TopLevelDeclarationTable::new(Vec::new(), &path_fork));
    let mut type_environment = TypeEnvironment::new();
    let mut context = TypeResolutionContext::from_declaration_table(
        &declaration_table,
        &mut type_environment,
        &const_values,
    );
    let namespace_records = FxHashMap::from_iter([(math_name, record)]);
    context.visible_namespace_records = Some(&namespace_records);
    let annotation = ParsedTypeRef::Qualified {
        path: vec![math_name, toolz_name, point_name],
        span: Some(expected_source_span(source, "math.toolz.Point")),
    };

    let diagnostic =
        resolve_parsed_type_annotation(&annotation, None, &mut context, &mut string_table, None)
            .expect_err("unknown child namespace must be rejected");
    let compact = compact_diagnostic(diagnostic, None);
    let diagnostic = compact
        .first()
        .expect("unknown child namespace emits a diagnostic");
    assert_eq!(diagnostic.descriptor().code, "MOTH-RULE-0089");
    let facts = diagnostic
        .family_view::<UnknownNamespaceMemberView>()
        .expect("unknown child namespace should retain typed facts");
    assert_eq!(facts.member_role, NamespaceMemberRole::ChildNamespace);
    assert!(
        diagnostic.string_ids().contains(&tools_name),
        "child namespaces must be candidates for an intermediate segment"
    );
    assert!(
        !diagnostic.string_ids().contains(&point_name),
        "types must not be intermediate-segment candidates"
    );
    assert!(
        !diagnostic.string_ids().contains(&pi_name),
        "values must not be intermediate-segment candidates"
    );
}

#[test]
fn unknown_type_name_suggests_close_visible_candidates() {
    let cases = [
        (
            "close local type",
            "Person = |\n    name String,\n|\nx Persn = Person(\"a\")\n",
            Some("Did you mean 'Person'"),
        ),
        (
            "close alias",
            "UserId as I32\nx UsrId = 1\n",
            Some("Did you mean 'UserId'"),
        ),
        (
            "close generic parameter",
            "identity type A |value A| -> Aa:\n    return value\n;\n",
            Some("Did you mean 'A'"),
        ),
        ("unrelated name", "x Xyzzy = 1\n", None),
        ("T outside generic scope", "value T = 1\n", None),
        (
            "declared alias is not suggested as its own target",
            "UserId as UsrId\n",
            None,
        ),
    ];

    for (label, source, expected_suggestion) in cases {
        let failure = parse_single_file_ast_diagnostic(source);
        assert_ast_diagnostic(&failure, "MOTH-RULE-0035", None);
        let message = ast_first_error_rendered_message(&failure);
        match expected_suggestion {
            Some(suggestion) => {
                assert!(
                    message.contains(suggestion),
                    "{label}: expected {suggestion:?} in {message}"
                );
            }
            None => {
                assert!(
                    !message.contains("Did you mean"),
                    "{label}: unexpected suggestion in {message}"
                );
            }
        }
    }
}

#[test]
fn unknown_type_name_does_not_suggest_invisible_local_types() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let gated_path = path_fork
        .try_intern_portable_path("GatedType", &mut string_table)
        .expect("test path fits");
    let visible_path = path_fork
        .try_intern_portable_path("VisibleType", &mut string_table)
        .expect("test path fits");
    let gated_name = string_table.intern("GatedType");
    let visible_name = string_table.intern("VisibleType");
    let unknown_name = string_table.intern("UnknownType");
    let const_values = ConstValueStore::default();
    let declaration_table = Rc::new(TopLevelDeclarationTable::new(
        vec![
            Declaration {
                id: gated_path,
                type_id: builtin_type_ids::NONE,
                type_syntax: None,
                value_mode: ValueMode::ImmutableReference,
                value: DeclarationValue::Shell(DeclarationShell::Struct),
                binding_span: None,
                config_qualifier: None,
            },
            Declaration {
                id: visible_path,
                type_id: builtin_type_ids::NONE,
                type_syntax: None,
                value_mode: ValueMode::ImmutableReference,
                value: DeclarationValue::Shell(DeclarationShell::Struct),
                binding_span: None,
                config_qualifier: None,
            },
        ],
        &path_fork,
    ));
    let source_bindings = FxHashMap::from_iter([
        (visible_name, SourceDeclarationTarget::Local(visible_path)),
        (gated_name, SourceDeclarationTarget::Local(gated_path)),
    ]);
    let visible_declaration_ids = Arc::new(FxHashSet::from_iter([visible_path]));
    let mut type_environment = TypeEnvironment::new();
    let mut context = TypeResolutionContext::from_declaration_table(
        &declaration_table,
        &mut type_environment,
        &const_values,
    );
    context.visible_source_bindings = Some(&source_bindings);
    context.visible_declaration_ids = Some(&visible_declaration_ids);
    let annotation = ParsedTypeRef::Named {
        name: unknown_name,
        span: None,
    };

    let diagnostic =
        resolve_parsed_type_annotation(&annotation, None, &mut context, &mut string_table, None)
            .expect_err("an unknown type must not resolve through a gated declaration");

    let compact = compact_diagnostic(diagnostic, None);
    let diagnostic = compact.first().expect("unknown type emits one diagnostic");
    assert_eq!(diagnostic.descriptor().code, "MOTH-RULE-0035");
    assert!(
        diagnostic.string_ids().contains(&visible_name),
        "visible types must enter unknown-type candidates"
    );
    assert!(
        !diagnostic.string_ids().contains(&gated_name),
        "types outside visible_declaration_ids must not enter unknown-type candidates"
    );
}

// ---------------------------------------------------------------
//  Fixed-collection capacity folding tests
// ---------------------------------------------------------------

#[test]
fn literal_capacity_resolves_to_fixed_collection() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let mut type_environment = TypeEnvironment::new();
    let const_values = ConstValueStore::default();
    let declaration_table = Rc::new(TopLevelDeclarationTable::new(
        Vec::new(),
        &PathInternerFork::empty(),
    ));
    let mut resolution_context = TypeResolutionContext::from_declaration_table(
        &declaration_table,
        &mut type_environment,
        &const_values,
    );

    let parsed = ParsedTypeRef::Collection {
        element: Box::new(ParsedTypeRef::BuiltinFixedScalar {
            scalar: FixedScalar::I32,
            span: None,
        }),
        span: None,
        fixed_capacity: Some(ParsedCollectionCapacity::Literal {
            value: 64,
            span: None,
        }),
    };

    let resolved = resolve_parsed_type_annotation(
        &parsed,
        None,
        &mut resolution_context,
        &mut string_table,
        None,
    )
    .expect("literal capacity should resolve");

    let type_id = resolved.type_id.expect("should have a type id");
    assert_eq!(
        resolution_context
            .type_environment
            .collection_fixed_capacity(type_id),
        Some(64),
        "capacity should be folded to 64"
    );
}

#[test]
fn constant_capacity_resolves_to_fixed_collection() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut owned_constants = ConstValueStore::default();
    let mut type_environment = TypeEnvironment::new();
    let declaration_table = Rc::new(TopLevelDeclarationTable::new(
        Vec::new(),
        &PathInternerFork::empty(),
    ));

    let capacity_name = string_table.intern("capacity");

    let type_id = builtin_type_ids::fixed_scalar(FixedScalar::I32);
    let value_id = owned_constants
        .insert_resolved(
            type_id,
            ConstValuePayload::FixedScalar(
                FixedScalarValue::signed(FixedScalar::I32, 42).expect("42 fits I32"),
            ),
            None,
        )
        .expect("fixture constant fits store");
    let capacity_path = path_fork
        .try_intern_components(&[capacity_name])
        .expect("test path fits");
    owned_constants
        .bind_body_local(capacity_path, value_id)
        .expect("fixture constant binds");
    let constant_declaration = Declaration {
        id: capacity_path,
        type_id,
        type_syntax: None,
        value_mode: ValueMode::ImmutableOwned,
        value: DeclarationValue::Default(value_id),
        binding_span: None,
        config_qualifier: None,
    };
    let mut scope_context = ScopeContext::new_for_tests(
        ContextKind::Function,
        PathId::ROOT,
        declaration_table.clone(),
        Arc::new(ExternalPackageRegistry::new()),
        0,
    );
    scope_context.add_compile_time_var(constant_declaration, None, &path_fork);
    let mut resolution_context = TypeResolutionContext::from_declaration_table(
        &declaration_table,
        &mut type_environment,
        &owned_constants,
    );

    let parsed = ParsedTypeRef::Collection {
        element: Box::new(ParsedTypeRef::BuiltinFixedScalar {
            scalar: FixedScalar::I32,
            span: None,
        }),
        span: None,
        fixed_capacity: Some(ParsedCollectionCapacity::BareConstant {
            name: capacity_name,
            span: None,
        }),
    };

    let resolved = resolve_parsed_type_annotation(
        &parsed,
        None,
        &mut resolution_context,
        &mut string_table,
        Some(&scope_context),
    )
    .expect("constant capacity should resolve");

    let type_id = resolved.type_id.expect("should have a type id");
    assert_eq!(
        resolution_context
            .type_environment
            .collection_fixed_capacity(type_id),
        Some(42),
        "capacity should be folded from constant to 42"
    );
}

#[test]
fn nested_fixed_collections_fold_both_capacities() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let mut type_environment = TypeEnvironment::new();
    let const_values = ConstValueStore::default();
    let declaration_table = Rc::new(TopLevelDeclarationTable::new(
        Vec::new(),
        &PathInternerFork::empty(),
    ));
    let mut resolution_context = TypeResolutionContext::from_declaration_table(
        &declaration_table,
        &mut type_environment,
        &const_values,
    );

    let inner = ParsedTypeRef::Collection {
        element: Box::new(ParsedTypeRef::BuiltinFixedScalar {
            scalar: FixedScalar::I32,
            span: None,
        }),
        span: None,
        fixed_capacity: Some(ParsedCollectionCapacity::Literal {
            value: 4,
            span: None,
        }),
    };
    let outer = ParsedTypeRef::Collection {
        element: Box::new(inner),
        span: None,
        fixed_capacity: Some(ParsedCollectionCapacity::Literal {
            value: 8,
            span: None,
        }),
    };

    let resolved = resolve_parsed_type_annotation(
        &outer,
        None,
        &mut resolution_context,
        &mut string_table,
        None,
    )
    .expect("nested fixed collections should resolve");

    let outer_type_id = resolved.type_id.expect("should have a type id");
    let outer_shape = resolution_context
        .type_environment
        .collection_shape(outer_type_id)
        .expect("outer should be a collection");
    assert_eq!(
        outer_shape.fixed_capacity,
        Some(8),
        "outer capacity should be 8"
    );

    let inner_type_id = outer_shape.element_type;
    let inner_shape = resolution_context
        .type_environment
        .collection_shape(inner_type_id)
        .expect("inner should be a collection");
    assert_eq!(
        inner_shape.fixed_capacity,
        Some(4),
        "inner capacity should be 4"
    );
}

#[test]
fn struct_field_alias_bare_capacity_constant_folds_after_constants() {
    let source = r#"
capacity #I32 = 4
Names as {capacity String}

Buffer = |
    items Names,
|
"#;
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
    let buffer_name = string_table.intern("Buffer");
    let buffer_path = path_fork
        .try_intern_portable_path("@page.moth", &mut string_table)
        .expect("test path fits");
    let buffer_path = path_fork
        .try_intern_child(buffer_path, buffer_name)
        .expect("test path fits");
    let buffer_type_id = ast
        .type_environment
        .nominal_id_for_path(&buffer_path)
        .and_then(|nominal_id| ast.type_environment.type_id_for_nominal_id(nominal_id))
        .expect("Buffer should have a nominal TypeId");
    let items_name = string_table.intern("items");
    let items_field = ast
        .type_environment
        .field_for(buffer_type_id, items_name, &path_fork)
        .expect("Buffer.items should be registered");

    assert_eq!(
        ast.type_environment
            .collection_fixed_capacity(items_field.type_id),
        Some(4),
        "type alias capacity should fold before final struct field TypeId registration"
    );
}

/// Builds a slot-bearing const template as owned content for field-default validation.
///
/// WHAT: the unresolved public template carries one default slot.
/// WHY: the fixture needs a template-shaped folded default without retaining TIR authority.
fn slot_field_default_template() -> PublicConstTemplate {
    PublicConstTemplate {
        kind: PublicConstTemplateKind::Wrapper,
        pieces: vec![PublicConstTemplatePiece::Slot(PublicConstTemplateSlot {
            key: PublicTemplateSlotKey::Default,
            applied_child_wrappers: Vec::new(),
            child_wrappers: Vec::new(),
            skip_parent_child_wrappers: false,
        })],
        conditional_child_wrappers: Vec::new(),
    }
}

#[test]
fn struct_field_default_inlines_slot_template_through_module_store() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut type_environment = TypeEnvironment::new();
    let wrapper_path = path_fork
        .try_intern_portable_path("wrapper", &mut string_table)
        .expect("test path fits");
    let mut owned_constants = ConstValueStore::default();
    let wrapper_template = slot_field_default_template();
    let wrapper_id = owned_constants
        .insert_value(
            ConstValueMetadata {
                type_id: builtin_type_ids::STRING,
                value_mode: ValueMode::ImmutableOwned,
                span: None,
                const_record_state: ConstRecordState::ConstRecord,
                contains_regular_division: false,
                synthetic_interface_provenance: SyntheticInterfaceProvenance::empty(),
                value_kind: ConstValueKind::TemplateWrapper,
                hir_visible: true,
            },
            ConstValuePayload::Template {
                template: wrapper_template,
                folded: None,
            },
        )
        .expect("fixture template constant fits store");
    owned_constants
        .bind_module_constant(wrapper_path, wrapper_id)
        .expect("fixture template binds");
    let wrapper_declaration = Declaration {
        id: wrapper_path,
        type_id: builtin_type_ids::STRING,
        type_syntax: None,
        value_mode: ValueMode::ImmutableOwned,
        value: DeclarationValue::Default(wrapper_id),
        binding_span: None,
        config_qualifier: None,
    };
    let declaration_table = Rc::new(TopLevelDeclarationTable::new(
        vec![wrapper_declaration],
        &path_fork,
    ));
    let struct_path = path_fork
        .try_intern_portable_path("Card", &mut string_table)
        .expect("test path fits");
    let field_path = path_fork
        .try_intern_child(struct_path, string_table.intern("content"))
        .expect("test path fits");
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let reference = expression_builder
        .reference_with_type_id(
            wrapper_path,
            builtin_type_ids::STRING,
            None,
            ValueMode::ImmutableReference,
            ConstRecordState::RuntimeValue,
        )
        .expect("fixture field reference fits store");
    let folded = crate::compiler_frontend::ast::const_eval::fold_expression_with_constants(
        reference.id(),
        &mut expression_builder,
        &mut owned_constants,
        FoldServices::new(&mut string_table, None, None),
        &type_environment,
        FoldRequirement::Opportunistic,
        |path| (path == wrapper_path).then_some(wrapper_id),
    )
    .expect("slot wrapper folds as constant content");
    let folded = folded_expression_root(folded);
    let crate::compiler_frontend::ast::expressions::expression_store::ExpressionOperationView::Constant(field_id) = expression_builder.view(folded).expect("folded wrapper belongs to builder").operation() else {
        panic!("slot wrapper fold must produce a bound constant occurrence");
    };
    let field = Declaration {
        id: field_path,
        type_id: builtin_type_ids::STRING,
        type_syntax: None,
        value_mode: ValueMode::ImmutableReference,
        value: DeclarationValue::Default(field_id),
        binding_span: None,
        config_qualifier: None,
    };

    let mut resolution_context = TypeResolutionContext::from_declaration_table(
        &declaration_table,
        &mut type_environment,
        &owned_constants,
    );
    let resolved_fields = resolve_struct_field_types(
        &struct_path,
        &[field],
        &mut resolution_context,
        &mut string_table,
        &path_fork,
        None,
    )
    .unwrap_or_else(|_| {
        panic!("slot-bearing const template should inline as a valid field default")
    });

    assert_eq!(
        resolved_fields[0].value.default(),
        Some(field_id),
        "template-shaped field defaults retain their folded constant identity"
    );
}
#[test]
fn struct_field_constant_inlining_preserves_surrounding_provenance() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut owned_constants = ConstValueStore::default();
    let mut type_environment = TypeEnvironment::new();
    let constant_path = path_fork
        .try_intern_portable_path("value", &mut string_table)
        .expect("test path fits");
    let project_member = SyntheticInterfaceMemberIdentity::new(
        SyntheticInterfaceClass::ProjectContext,
        "render",
        "html",
    );
    let builder_member =
        SyntheticInterfaceMemberIdentity::new(SyntheticInterfaceClass::Builder, "assets", "bundle");
    let scalar_id = expression_builder
        .fixed_scalar(
            &mut owned_constants,
            FixedScalarValue::signed(FixedScalar::I32, 7).expect("7 fits I32"),
            None,
            ValueMode::ImmutableOwned,
        )
        .expect("fixture scalar fits builder");
    let scalar_id = expression_builder
        .with_synthetic_interface_provenance(
            scalar_id,
            SyntheticInterfaceProvenance::single(project_member.clone()),
        )
        .expect("fixture provenance fits builder");
    let folded_scalar = crate::compiler_frontend::ast::const_eval::fold_expression_with_constants(
        scalar_id.id(),
        &mut expression_builder,
        &mut owned_constants,
        FoldServices::new(&mut string_table, None, None),
        &type_environment,
        FoldRequirement::Opportunistic,
        |_| None,
    )
    .expect("scalar constant folds as content");
    let folded_scalar = folded_expression_root(folded_scalar);
    let crate::compiler_frontend::ast::expressions::expression_store::ExpressionOperationView::Constant(constant_id) = expression_builder.view(folded_scalar).expect("folded scalar belongs to builder").operation() else {
        panic!("scalar fold must produce a bound constant occurrence");
    };
    owned_constants
        .bind_module_constant(constant_path, constant_id)
        .expect("fixture constant binds");
    let constant = Declaration {
        id: constant_path,
        type_id: builtin_type_ids::fixed_scalar(FixedScalar::I32),
        type_syntax: None,
        value_mode: ValueMode::ImmutableOwned,
        value: DeclarationValue::Default(constant_id),
        binding_span: None,
        config_qualifier: None,
    };
    let declaration_table = Rc::new(TopLevelDeclarationTable::new(vec![constant], &path_fork));
    let _collection_type_id =
        type_environment.intern_collection(builtin_type_ids::fixed_scalar(FixedScalar::I32), None);
    let element = expression_builder
        .reference_with_type_id(
            constant_path,
            builtin_type_ids::fixed_scalar(FixedScalar::I32),
            None,
            ValueMode::ImmutableReference,
            ConstRecordState::RuntimeValue,
        )
        .expect("fixture element fits builder");
    let element = expression_builder
        .with_synthetic_interface_provenance(
            element,
            SyntheticInterfaceProvenance::single(project_member.clone()),
        )
        .expect("fixture element provenance fits builder");
    let collection_id = crate::compiler_frontend::tests::type_id_fixture_support::collection_expr(
        &mut expression_builder,
        vec![element.id()],
        None,
        ValueMode::ImmutableOwned,
        &mut type_environment,
    );
    let collection_id = expression_builder
        .with_synthetic_interface_provenance(
            expression_builder.own(collection_id),
            SyntheticInterfaceProvenance::single(builder_member.clone()),
        )
        .expect("fixture collection provenance fits builder");
    let folded_collection =
        crate::compiler_frontend::ast::const_eval::fold_expression_with_constants(
            collection_id.id(),
            &mut expression_builder,
            &mut owned_constants,
            FoldServices::new(&mut string_table, None, None),
            &type_environment,
            FoldRequirement::Opportunistic,
            |path| (path == constant_path).then_some(constant_id),
        )
        .expect("collection constant folds as content");
    let folded_collection = folded_expression_root(folded_collection);
    let crate::compiler_frontend::ast::expressions::expression_store::ExpressionOperationView::Constant(collection_default) = expression_builder.view(folded_collection).expect("folded collection belongs to builder").operation() else {
        panic!("collection fold must produce a bound constant occurrence");
    };
    let collection_metadata = owned_constants
        .metadata(collection_default)
        .expect("folded collection has content metadata");
    let collection_type_id = collection_metadata.type_id;
    let mut resolution_context = TypeResolutionContext::from_declaration_table(
        &declaration_table,
        &mut type_environment,
        &owned_constants,
    );
    let struct_path = path_fork
        .try_intern_portable_path("Card", &mut string_table)
        .expect("test path fits");
    let field = Declaration {
        id: path_fork
            .try_intern_child(struct_path, string_table.intern("values"))
            .expect("test path fits"),
        type_id: collection_type_id,
        type_syntax: None,
        value_mode: ValueMode::ImmutableOwned,
        value: DeclarationValue::Default(collection_default),
        binding_span: None,
        config_qualifier: None,
    };

    let resolved_fields = resolve_struct_field_types(
        &struct_path,
        &[field],
        &mut resolution_context,
        &mut string_table,
        &path_fork,
        None,
    )
    .unwrap_or_else(|_| panic!("collection field default should resolve"));

    let resolved_default = resolved_fields[0]
        .value
        .default()
        .expect("resolved field keeps folded content");
    assert_eq!(
        owned_constants
            .metadata(resolved_default)
            .expect("resolved default belongs to fixture constants")
            .synthetic_interface_provenance
            .members(),
        &[builder_member]
    );
    let Some(crate::compiler_frontend::ast::const_values::store::ConstValuePayload::Collection(
        elements,
    )) = owned_constants.payload(resolved_default)
    else {
        panic!("expected collection field default");
    };
    assert_eq!(
        owned_constants
            .metadata(elements[0])
            .expect("collection element belongs to fixture constants")
            .synthetic_interface_provenance
            .members(),
        &[project_member]
    );
}

#[test]
fn function_return_bare_capacity_constant_folds_to_type_id() {
    let source = r#"
capacity #I32 = 3

make || -> {capacity I32}:
    assert(false, "signature only")
;
"#;
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);

    let make_name = string_table.intern("make");
    let make_path = path_fork
        .try_intern_portable_path("@page.moth", &mut string_table)
        .expect("test path fits");
    let make_path = path_fork
        .try_intern_child(make_path, make_name)
        .expect("test path fits");
    let signature = ast
        .nodes
        .iter()
        .find_map(|node| match &node.kind {
            NodeKind::Function(path, signature, _) if *path == make_path => Some(signature),
            _ => None,
        })
        .expect("make signature should be registered");
    let return_type_id = signature.returns[0]
        .type_id
        .expect("return slot should have a TypeId");

    assert_eq!(
        ast.type_environment
            .collection_fixed_capacity(return_type_id),
        Some(3),
        "fixed capacity in return annotation should fold into the canonical return TypeId"
    );
}

fn page_path(
    name: &str,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) -> PathId {
    let scope = path_fork
        .try_intern_portable_path("@page.moth", string_table)
        .expect("test path fits");
    path_fork
        .try_intern_child(scope, string_table.intern(name))
        .expect("test path fits")
}
fn nominal_type_id(
    ast: &Ast,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
    name: &str,
) -> TypeId {
    let path = page_path(name, string_table, path_fork);
    ast.type_environment
        .nominal_id_for_path(&path)
        .and_then(|nominal_id| ast.type_environment.type_id_for_nominal_id(nominal_id))
        .unwrap_or_else(|| panic!("{name} should have a nominal TypeId"))
}

fn assert_map_shape(ast: &Ast, type_id: TypeId, expected_key: TypeId, expected_value: TypeId) {
    assert!(
        ast.type_environment.is_map_type(type_id),
        "expected {:?} to be a map TypeId",
        type_id
    );
    let shape = ast
        .type_environment
        .map_shape(type_id)
        .expect("expected map shape");
    assert_eq!(shape.key_type, expected_key, "map key TypeId mismatch");
    assert_eq!(
        shape.value_type, expected_value,
        "map value TypeId mismatch"
    );
}

fn map_value_type(ast: &Ast, type_id: TypeId) -> TypeId {
    ast.type_environment
        .map_shape(type_id)
        .map(|shape| shape.value_type)
        .expect("expected map value TypeId")
}

fn assert_source_invalid_map_type(
    source: &str,
    expected_reason_key: &str,
    expected_description: &str,
) {
    let failure = parse_single_file_ast_diagnostic(source);
    let diagnostic =
        assert_ast_diagnostic(&failure, "MOTH-SYNTAX-0016-MAP", Some(expected_reason_key));
    assert_eq!(
        diagnostic.reason_key(),
        Some(expected_reason_key),
        "{expected_description}",
    );
}

#[test]
fn map_type_syntax_resolves_across_source_type_surfaces() {
    let source = r#"
StringScores as {String = I32}
NestedScores as {String = StringScores}

User = |
    scores StringScores,
    grouped {Bool = StringScores},
|

Payload ::
    WithMap | values NestedScores |
;

use_maps |scores StringScores, direct {Bool = {String = I32}}| -> {Char = StringScores}:
    assert(false, "signature only")
;
"#;
    let (ast, mut path_fork, mut string_table) = parse_single_file_ast(source);
    let builtins = *ast.type_environment.builtins();
    let i32_type_id = builtin_type_ids::fixed_scalar(FixedScalar::I32);

    let user_type_id = nominal_type_id(&ast, &mut string_table, &mut path_fork, "User");
    let scores_name = string_table.intern("scores");
    let scores_type_id = ast
        .type_environment
        .field_for(user_type_id, scores_name, &path_fork)
        .expect("User.scores should be registered")
        .type_id;
    assert_map_shape(&ast, scores_type_id, builtins.string, i32_type_id);

    let grouped_name = string_table.intern("grouped");
    let grouped_type_id = ast
        .type_environment
        .field_for(user_type_id, grouped_name, &path_fork)
        .expect("User.grouped should be registered")
        .type_id;
    assert_map_shape(&ast, grouped_type_id, builtins.bool, scores_type_id);
    assert_map_shape(
        &ast,
        map_value_type(&ast, grouped_type_id),
        builtins.string,
        i32_type_id,
    );

    let function_path = page_path("use_maps", &mut string_table, &mut path_fork);
    let signature = ast
        .nodes
        .iter()
        .find_map(|node| match &node.kind {
            NodeKind::Function(path, signature, _) if *path == function_path => Some(signature),
            _ => None,
        })
        .expect("use_maps signature should be registered");
    assert_map_shape(
        &ast,
        signature.parameters[0].type_id,
        builtins.string,
        i32_type_id,
    );
    assert_map_shape(
        &ast,
        signature.parameters[1].type_id,
        builtins.bool,
        scores_type_id,
    );
    let return_type_id = signature.returns[0]
        .type_id
        .expect("map return should have a TypeId");
    assert_map_shape(&ast, return_type_id, builtins.char, scores_type_id);

    let payload_type_id = nominal_type_id(&ast, &mut string_table, &mut path_fork, "Payload");
    let payload_definition = ast
        .type_environment
        .choice_definition_for(payload_type_id)
        .expect("Payload should be a choice");
    let with_map_name = string_table.intern("WithMap");
    let with_map = payload_definition
        .variants
        .iter()
        .find(|variant| variant.name == with_map_name)
        .expect("WithMap variant should be registered");
    let ChoiceVariantPayloadDefinition::Record { fields } = &with_map.payload else {
        panic!("WithMap should have a record payload");
    };
    let values_type_id = fields
        .iter()
        .find(|field| {
            path_fork
                .component(field.name)
                .is_some_and(|name| string_table.resolve(name) == "values")
        })
        .expect("WithMap.values should be registered")
        .type_id;
    assert_map_shape(&ast, values_type_id, builtins.string, scores_type_id);
}

#[test]
fn map_type_syntax_rejects_invalid_source_key_types() {
    assert_source_invalid_map_type(
        "bad |items {F64 = I32}|:\n;\n",
        "invalid_map_type.unsupported_key_type",
        "UnsupportedKeyType for F64 keys",
    );
    assert_source_invalid_map_type(
        "User = |\n    id I32,\n|\n\nbad |items {User = I32}|:\n;\n",
        "invalid_map_type.unsupported_key_type",
        "UnsupportedKeyType for struct keys",
    );
    assert_source_invalid_map_type(
        "bad |items {{String} = I32}|:\n;\n",
        "invalid_map_type.unsupported_key_type",
        "UnsupportedKeyType for collection keys",
    );
    assert_source_invalid_map_type(
        "bad |items {{String = I32} = I32}|:\n;\n",
        "invalid_map_type.unsupported_key_type",
        "UnsupportedKeyType for map keys",
    );
    assert_source_invalid_map_type(
        "bad |items {String = {String = {String = I32}}}|:\n;\n",
        "invalid_map_type.excessive_inline_nesting",
        "ExcessiveInlineNesting for depth-three inline maps",
    );
    assert_source_invalid_map_type(
        "Holder type Key = |\n    values {Key = I32},\n|\n",
        "invalid_map_type.unsupported_key_type",
        "UnsupportedKeyType for generic map keys",
    );
}

#[test]
fn map_type_resolves_for_supported_key() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let mut type_environment = TypeEnvironment::new();
    let const_values = ConstValueStore::default();
    let declaration_table = Rc::new(TopLevelDeclarationTable::new(
        Vec::new(),
        &PathInternerFork::empty(),
    ));
    let mut resolution_context = TypeResolutionContext::from_declaration_table(
        &declaration_table,
        &mut type_environment,
        &const_values,
    );

    let parsed = ParsedTypeRef::Map {
        key: Box::new(ParsedTypeRef::BuiltinString { span: None }),
        value: Box::new(ParsedTypeRef::BuiltinFixedScalar {
            scalar: FixedScalar::I32,
            span: None,
        }),
        span: None,
    };

    let resolved = resolve_parsed_type_annotation(
        &parsed,
        None,
        &mut resolution_context,
        &mut string_table,
        None,
    )
    .expect("String key map should resolve");

    let type_id = resolved.type_id.expect("should have a type id");
    assert!(resolution_context.type_environment.is_map_type(type_id));
    let shape = resolution_context
        .type_environment
        .map_shape(type_id)
        .expect("String key map should have a shape");
    assert_eq!(
        shape.key_type,
        resolution_context.type_environment.builtins().string
    );
    assert_eq!(
        shape.value_type,
        builtin_type_ids::fixed_scalar(FixedScalar::I32)
    );
}

#[test]
fn map_type_rejects_unsupported_key() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let mut type_environment = TypeEnvironment::new();
    let const_values = ConstValueStore::default();
    let declaration_table = Rc::new(TopLevelDeclarationTable::new(
        Vec::new(),
        &PathInternerFork::empty(),
    ));

    let parsed = ParsedTypeRef::Map {
        key: Box::new(ParsedTypeRef::BuiltinFixedScalar {
            scalar: FixedScalar::F64,
            span: None,
        }),
        value: Box::new(ParsedTypeRef::BuiltinFixedScalar {
            scalar: FixedScalar::I32,
            span: None,
        }),
        span: None,
    };

    let error = {
        let mut resolution_context = TypeResolutionContext::from_declaration_table(
            &declaration_table,
            &mut type_environment,
            &const_values,
        );
        resolve_parsed_type_annotation(
            &parsed,
            None,
            &mut resolution_context,
            &mut string_table,
            None,
        )
        .expect_err("F64 key should be rejected")
    };
    let compact = compact_diagnostic(error, Some(&type_environment));
    let diagnostic = compact
        .first()
        .expect("unsupported map key emits a diagnostic");
    let facts = diagnostic
        .family_view::<InvalidMapTypeView>()
        .expect("unsupported map key should retain typed map facts");
    assert_eq!(facts.reason, InvalidMapTypeCode::UnsupportedKeyType);
    let key_type = facts
        .key_type
        .expect("unsupported map key should retain its semantic type");
    let path_table = _path_fork.snapshot_table();
    assert_eq!(
        diagnostic.types().shape(key_type),
        TypeDisplayShape::FixedScalar(FixedScalar::F64)
    );
    assert_eq!(
        diagnostic
            .types()
            .display(key_type, &string_table, &path_table),
        "F64"
    );
}

#[test]
fn map_key_capability_rejects_generic_key_as_unsupported() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let mut type_environment = TypeEnvironment::new();
    let parameter_name = string_table.intern("Key");
    let key_type_id = type_environment.register_synthetic_generic_parameter(parameter_name);

    let error = validate_map_key_type(key_type_id, &type_environment, None)
        .expect_err("generic map keys should be rejected by the scalar-key policy");

    let compact = compact_diagnostic(error, Some(&type_environment));
    let diagnostic = compact
        .first()
        .expect("generic unsupported map key emits a diagnostic");
    let facts = diagnostic
        .family_view::<InvalidMapTypeView>()
        .expect("generic map key should retain typed map facts");
    assert_eq!(facts.reason, InvalidMapTypeCode::UnsupportedKeyType);
    let key_type = facts
        .key_type
        .expect("generic map key should retain its semantic type");
    let path_table = _path_fork.snapshot_table();
    assert_eq!(
        diagnostic.types().shape(key_type),
        TypeDisplayShape::GenericParameter {
            name: parameter_name,
        }
    );
    assert_eq!(
        diagnostic
            .types()
            .display(key_type, &string_table, &path_table),
        "Key"
    );
}

#[test]
fn map_type_rejects_excessive_inline_nesting() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let mut type_environment = TypeEnvironment::new();
    let const_values = ConstValueStore::default();
    let declaration_table = Rc::new(TopLevelDeclarationTable::new(
        Vec::new(),
        &PathInternerFork::empty(),
    ));

    // {String = {String = {String = I32}}} is depth 3 and should be rejected
    let parsed = ParsedTypeRef::Map {
        key: Box::new(ParsedTypeRef::BuiltinString { span: None }),
        value: Box::new(ParsedTypeRef::Map {
            key: Box::new(ParsedTypeRef::BuiltinString { span: None }),
            value: Box::new(ParsedTypeRef::Map {
                key: Box::new(ParsedTypeRef::BuiltinString { span: None }),
                value: Box::new(ParsedTypeRef::BuiltinFixedScalar {
                    scalar: FixedScalar::I32,
                    span: None,
                }),
                span: None,
            }),
            span: None,
        }),
        span: None,
    };

    let error = {
        let mut resolution_context = TypeResolutionContext::from_declaration_table(
            &declaration_table,
            &mut type_environment,
            &const_values,
        );
        resolve_parsed_type_annotation(
            &parsed,
            None,
            &mut resolution_context,
            &mut string_table,
            None,
        )
        .expect_err("depth-3 nesting should be rejected")
    };
    let compact = compact_diagnostic(error, Some(&type_environment));
    let diagnostic = compact
        .first()
        .expect("excessive map nesting emits a diagnostic");
    let facts = diagnostic
        .family_view::<InvalidMapTypeView>()
        .expect("excessive map nesting should retain typed map facts");
    assert_eq!(facts.reason, InvalidMapTypeCode::ExcessiveInlineNesting);
    assert_eq!(facts.depth, Some(3));
}

#[test]
fn map_type_allows_two_level_nesting() {
    let mut string_table = StringTable::new();
    let _path_fork = PathInternerFork::empty();
    let mut type_environment = TypeEnvironment::new();
    let const_values = ConstValueStore::default();
    let declaration_table = Rc::new(TopLevelDeclarationTable::new(
        Vec::new(),
        &PathInternerFork::empty(),
    ));
    let mut resolution_context = TypeResolutionContext::from_declaration_table(
        &declaration_table,
        &mut type_environment,
        &const_values,
    );

    // {String = {String = I32}} is depth 2 and should be allowed
    let parsed = ParsedTypeRef::Map {
        key: Box::new(ParsedTypeRef::BuiltinString { span: None }),
        value: Box::new(ParsedTypeRef::Map {
            key: Box::new(ParsedTypeRef::BuiltinString { span: None }),
            value: Box::new(ParsedTypeRef::BuiltinFixedScalar {
                scalar: FixedScalar::I32,
                span: None,
            }),
            span: None,
        }),
        span: None,
    };

    let resolved = resolve_parsed_type_annotation(
        &parsed,
        None,
        &mut resolution_context,
        &mut string_table,
        None,
    )
    .expect("depth-2 nesting should be allowed");

    let type_id = resolved.type_id.expect("should have a type id");
    assert!(resolution_context.type_environment.is_map_type(type_id));
}

#[test]
fn struct_field_default_folds_above_i32_range_under_int64() {
    let mut string_table = StringTable::new();
    let mut path_fork = PathInternerFork::empty();
    let mut expression_builder = AstExpressionBuilder::new().unwrap();
    let mut owned_constants = ConstValueStore::default();
    let mut type_environment = TypeEnvironment::new();
    let limit_path = path_fork
        .try_intern_portable_path("LIMIT", &mut string_table)
        .expect("test path fits");
    let limit_id = expression_builder
        .fixed_scalar(
            &mut owned_constants,
            FixedScalarValue::signed(FixedScalar::I64, 2_000_000_000)
                .expect("2_000_000_000 fits I64"),
            None,
            ValueMode::ImmutableOwned,
        )
        .expect("fixture limit fits builder");
    let folded_limit = crate::compiler_frontend::ast::const_eval::fold_expression_with_constants(
        limit_id.id(),
        &mut expression_builder,
        &mut owned_constants,
        FoldServices::new(&mut string_table, None, None),
        &type_environment,
        FoldRequirement::Opportunistic,
        |_| None,
    )
    .expect("limit folds as constant content");
    let folded_limit = folded_expression_root(folded_limit);
    let crate::compiler_frontend::ast::expressions::expression_store::ExpressionOperationView::Constant(limit_default) = expression_builder.view(folded_limit).expect("folded limit belongs to builder").operation() else {
        panic!("limit fold must produce a bound constant occurrence");
    };
    owned_constants
        .bind_module_constant(limit_path, limit_default)
        .expect("fixture limit binds");
    let limit_declaration = Declaration {
        id: limit_path,
        type_id: builtin_type_ids::fixed_scalar(FixedScalar::I64),
        type_syntax: None,
        value_mode: ValueMode::ImmutableOwned,
        value: DeclarationValue::Default(limit_default),
        binding_span: None,
        config_qualifier: None,
    };
    let declaration_table = Rc::new(TopLevelDeclarationTable::new(
        vec![limit_declaration],
        &path_fork,
    ));
    let struct_path = path_fork
        .try_intern_portable_path("Holder", &mut string_table)
        .expect("test path fits");
    let limit_reference = |builder: &mut AstExpressionBuilder| {
        builder
            .reference_with_type_id(
                limit_path,
                builtin_type_ids::fixed_scalar(FixedScalar::I64),
                None,
                ValueMode::ImmutableReference,
                ConstRecordState::RuntimeValue,
            )
            .expect("fixture limit reference fits builder")
    };
    let left = limit_reference(&mut expression_builder);
    let right = limit_reference(&mut expression_builder);
    let sum_view = crate::compiler_frontend::tests::type_id_fixture_support::runtime_expr(
        &mut expression_builder,
        &mut owned_constants,
        vec![
            crate::compiler_frontend::tests::type_id_fixture_support::runtime_operand_item(
                left.id(),
            ),
            crate::compiler_frontend::tests::type_id_fixture_support::runtime_operand_item(
                right.id(),
            ),
            crate::compiler_frontend::tests::type_id_fixture_support::runtime_operator_item(
                Operator::Add,
                None,
            ),
        ],
        builtin_type_ids::fixed_scalar(FixedScalar::I64),
        None,
        ValueMode::ImmutableOwned,
        &type_environment,
    );
    let folded_sum = crate::compiler_frontend::ast::const_eval::fold_expression_with_constants(
        sum_view,
        &mut expression_builder,
        &mut owned_constants,
        FoldServices::new(&mut string_table, None, None),
        &type_environment,
        FoldRequirement::Opportunistic,
        |path| (path == limit_path).then_some(limit_default),
    )
    .expect("sum folds as constant content");
    let folded_sum = folded_expression_root(folded_sum);
    let crate::compiler_frontend::ast::expressions::expression_store::ExpressionOperationView::Constant(sum_default) = expression_builder.view(folded_sum).expect("folded sum belongs to builder").operation() else {
        panic!("sum fold must produce a bound constant occurrence");
    };
    let field = Declaration {
        id: path_fork
            .try_intern_child(struct_path, string_table.intern("total"))
            .expect("test path fits"),
        type_id: builtin_type_ids::fixed_scalar(FixedScalar::I64),
        type_syntax: None,
        value_mode: ValueMode::ImmutableOwned,
        value: DeclarationValue::Default(sum_default),
        binding_span: None,
        config_qualifier: None,
    };

    let mut resolution_context = TypeResolutionContext::from_declaration_table(
        &declaration_table,
        &mut type_environment,
        &owned_constants,
    );
    let resolved_fields = resolve_struct_field_types(
        &struct_path,
        &[field],
        &mut resolution_context,
        &mut string_table,
        &path_fork,
        None,
    )
    .unwrap_or_else(|error| match error {
        StructFieldResolutionError::Diagnostic(diagnostic) => {
            panic!("an I64 field default above the i32 range should fold: {diagnostic:?}")
        }
        StructFieldResolutionError::Infrastructure(error) => {
            panic!(
                "an I64 field default above the i32 range should fold: {}",
                error.message
            )
        }
    });

    let resolved_default = resolved_fields[0]
        .value
        .default()
        .expect("resolved field keeps folded content");
    let expected =
        FixedScalarValue::signed(FixedScalar::I64, 4_000_000_000).expect("4_000_000_000 fits I64");
    assert!(
        matches!(
            owned_constants.payload(resolved_default),
            Some(crate::compiler_frontend::ast::const_values::store::ConstValuePayload::FixedScalar(value))
                if *value == expected
        ),
        "LIMIT + LIMIT should fold to 4_000_000_000, got {:?}",
        owned_constants.payload(resolved_default)
    );
}

#[test]
fn capacity_i32_boundary_is_rejected_above_range() {
    let mut string_table = StringTable::new();
    let const_values = ConstValueStore::default();
    let path_fork = PathInternerFork::empty();
    let declaration_table = Rc::new(TopLevelDeclarationTable::new(Vec::new(), &path_fork));
    let parsed = |value: i64| ParsedTypeRef::Collection {
        element: Box::new(ParsedTypeRef::BuiltinFixedScalar {
            scalar: FixedScalar::I32,
            span: None,
        }),
        span: None,
        fixed_capacity: Some(ParsedCollectionCapacity::Literal { value, span: None }),
    };

    // Capacity is an exact `I32` value, so the boundary itself resolves and the first value
    // above it is rejected.
    let mut boundary_environment = TypeEnvironment::new();
    let mut boundary_context = TypeResolutionContext::from_declaration_table(
        &declaration_table,
        &mut boundary_environment,
        &const_values,
    );
    let resolved = resolve_parsed_type_annotation(
        &parsed(i64::from(i32::MAX)),
        None,
        &mut boundary_context,
        &mut string_table,
        None,
    )
    .expect("an I32 capacity at the exact boundary should resolve");
    let type_id = resolved.type_id.expect("should have a type id");
    assert_eq!(
        boundary_context
            .type_environment
            .collection_fixed_capacity(type_id),
        usize::try_from(i32::MAX).ok(),
        "the boundary capacity should fold to the exact I32 maximum"
    );

    let mut environment = TypeEnvironment::new();
    let mut context = TypeResolutionContext::from_declaration_table(
        &declaration_table,
        &mut environment,
        &const_values,
    );
    let error = resolve_parsed_type_annotation(
        &parsed(i64::from(i32::MAX) + 1),
        None,
        &mut context,
        &mut string_table,
        None,
    )
    .expect_err("capacity above i32::MAX should be rejected");
    let compact = compact_diagnostic(error, None);
    let diagnostic = compact
        .first()
        .expect("capacity overflow emits a diagnostic");
    let facts = diagnostic
        .family_view::<InvalidCollectionTypeView>()
        .expect("capacity overflow should retain typed collection facts");
    assert_eq!(facts.reason, InvalidCollectionTypeCode::CapacityOverflow);
}
