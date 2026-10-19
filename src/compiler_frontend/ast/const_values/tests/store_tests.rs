//! Content graph invariants: exact scalars, structural strings, ordered fields and local imports.
use super::*;
use crate::compiler_frontend::ast::expressions::expression_store::{
    AstExpressionBuilder, ExpressionOp,
};
use crate::compiler_frontend::ast::templates::tir::{
    TemplateIrId, TemplateTirPhase, TemplateTirReference, TemplateViewContext,
};
use crate::compiler_frontend::compiler_messages::{
    AstCapacityResource, CapacityResource, DiagnosticPayload,
};
use crate::compiler_frontend::datatypes::ids::builtin_type_ids;
use crate::compiler_frontend::paths::module_resources::{ModuleResourceTable, ResourceId};
use crate::compiler_frontend::paths::resource_identity::{
    PortableResourcePath, StableResourceOriginId,
};
use crate::compiler_frontend::semantic_identity::{
    ModuleRootRole, StableModuleOriginIdentity, StablePackageIdentity,
};
use crate::compiler_frontend::source::{LocalSpan, SourceId};
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_tests::test_support::run_stack_budgeted_child;
use moth_lexical::numeric::fixed_scalar::{FixedScalar, FixedScalarValue};
use std::path::Path;

#[test]
fn compact_const_value_extent_checks_preserve_capacity_resource_and_span() {
    let span = Some(SourceSpan::new(
        SourceId::COMPILATION_ROOT,
        LocalSpan::source_start(),
    ));
    let max = u32::MAX as usize;
    assert_eq!(checked_const_value_index(max, 0, span).unwrap(), u32::MAX);
    assert_eq!(
        checked_const_value_index(max - 1, 1, span).unwrap(),
        u32::MAX
    );
    for (prefix, offset) in [(max, 1), (usize::MAX, 1)] {
        let diagnostic = checked_const_value_index(prefix, offset, span).unwrap_err();
        assert!(matches!(
            diagnostic.payload,
            DiagnosticPayload::CompilerCapacityExceeded {
                resource: CapacityResource::Ast(AstCapacityResource::ConstValues),
            }
        ));
        assert_eq!(diagnostic.primary_span, span);
    }
}

fn resource_id(table: &mut ModuleResourceTable, relative: &str) -> ResourceId {
    let module = StableModuleOriginIdentity::from_portable_path(
        StablePackageIdentity::project_local("site"),
        String::new(),
        ModuleRootRole::Normal,
    );
    let path = PortableResourcePath::from_relative_logical_path(Path::new(relative))
        .expect("portable resource");
    table.intern_origin(StableResourceOriginId::module_owned(module, path), None)
}

fn visit_string(store: &ConstValueStore, id: ConstValueId) -> ConstStringValue {
    store
        .fold_value(id, &mut |_, value| -> Result<_, CompilerError> {
            match value {
                ConstValueVisit::String(value) => Ok(value.clone()),
                _ => Err(CompilerError::compiler_error("Expected string payload.")),
            }
        })
        .expect("string visitor")
}

#[test]
fn plain_text_stays_on_the_fast_path() {
    let mut strings = StringTable::new();
    let text = strings.intern("plain text");
    let mut store = ConstValueStore::default();
    let id = store
        .insert_resolved(
            builtin_type_ids::STRING,
            ConstValuePayload::String(ConstStringValue::Text(text)),
            None,
        )
        .expect("resolved string");
    assert_eq!(visit_string(&store, id), ConstStringValue::Text(text));
    assert_eq!(store.string_value(id), Some(text));
    assert!(matches!(
        store.payload(id),
        Some(ConstValuePayload::String(ConstStringValue::Text(_)))
    ));
}

#[test]
fn pieces_round_trip_through_the_visitor_in_order() {
    let mut resources = ModuleResourceTable::new();
    let resource = resource_id(&mut resources, "assets/logo.svg");
    let pieces = vec![
        ConstStringPiece::Resource(resource),
        ConstStringPiece::SiteRoot,
    ];
    let mut store = ConstValueStore::default();
    let id = store
        .insert_resolved(
            builtin_type_ids::STRING,
            ConstValuePayload::String(ConstStringValue::Pieces(pieces.clone())),
            None,
        )
        .expect("pieces");
    assert_eq!(visit_string(&store, id), ConstStringValue::Pieces(pieces));
}

#[test]
fn mixed_pieces_keep_authored_order() {
    let mut strings = StringTable::new();
    let mut resources = ModuleResourceTable::new();
    let pieces = vec![
        ConstStringPiece::Text(strings.intern("prefix/")),
        ConstStringPiece::Resource(resource_id(&mut resources, "assets/logo.svg")),
        ConstStringPiece::SiteRoot,
        ConstStringPiece::Text(strings.intern("/suffix")),
    ];
    let mut store = ConstValueStore::default();
    let id = store
        .insert_resolved(
            builtin_type_ids::STRING,
            ConstValuePayload::String(ConstStringValue::Pieces(pieces.clone())),
            None,
        )
        .expect("pieces");
    assert_eq!(visit_string(&store, id), ConstStringValue::Pieces(pieces));
}

fn ingest_template(folded: ConstStringValue) -> (ConstValueStore, ConstValueId) {
    let mut builder = AstExpressionBuilder::new().unwrap();
    let template = Template {
        tir_reference: TemplateTirReference {
            root: TemplateIrId::new(0),
            phase: TemplateTirPhase::Finalized,
            context: TemplateViewContext::default(),
        },
        span: None,
    };
    let template = builder
        .append_template(template, None)
        .expect("template fact");
    let root = builder
        .append_expression(
            ExpressionOp::Template(template),
            None,
            builtin_type_ids::STRING,
            ValueMode::ImmutableOwned,
        )
        .expect("template occurrence")
        .id();
    let mut store = ConstValueStore::default();
    let id = store
        .ingest_view(
            builder.view(root).expect("bound root"),
            &TypeEnvironment::default(),
            &mut |_| {
                Ok(ConstTemplateValue::Folded {
                    value: folded.clone(),
                    provenance: SyntheticInterfaceProvenance::empty(),
                })
            },
        )
        .expect("finalizer supplies exact template fold");
    (store, id)
}

#[test]
fn a_plain_text_template_fold_stays_on_the_text_fast_path() {
    let mut strings = StringTable::new();
    let text = strings.intern("rendered body");
    let (store, id) = ingest_template(ConstStringValue::Text(text));
    assert_eq!(visit_string(&store, id), ConstStringValue::Text(text));
    assert_eq!(store.string_value(id), Some(text));
    assert_eq!(
        store.metadata(id).expect("metadata").value_kind,
        ConstValueKind::RenderableTemplate
    );
}

#[test]
fn a_piece_bearing_template_fold_round_trips_its_pieces_in_order() {
    let mut strings = StringTable::new();
    let mut resources = ModuleResourceTable::new();
    let pieces = vec![
        ConstStringPiece::Text(strings.intern("docs/")),
        ConstStringPiece::Resource(resource_id(&mut resources, "assets/logo.svg")),
        ConstStringPiece::SiteRoot,
    ];
    let (store, id) = ingest_template(ConstStringValue::Pieces(pieces.clone()));
    assert_eq!(visit_string(&store, id), ConstStringValue::Pieces(pieces));
    assert_eq!(store.string_value(id), None);
}

#[test]
fn the_text_only_accessor_refuses_pieces() {
    let mut store = ConstValueStore::default();
    let id = store
        .insert_resolved(
            builtin_type_ids::STRING,
            ConstValuePayload::String(ConstStringValue::Pieces(vec![ConstStringPiece::SiteRoot])),
            None,
        )
        .expect("pieces");
    assert_eq!(store.string_value(id), None);
}

#[test]
fn record_fields_keep_authored_order() {
    let mut strings = StringTable::new();
    let mut paths = PathInternerFork::empty();
    let alpha = paths
        .try_intern_portable_path("alpha", &mut strings)
        .expect("field path");
    let beta = paths
        .try_intern_portable_path("beta", &mut strings)
        .expect("field path");
    let mut store = ConstValueStore::default();
    let value = store
        .insert_resolved(builtin_type_ids::BOOL, ConstValuePayload::Bool(true), None)
        .expect("field");
    let root = store
        .insert_resolved(
            TypeEnvironment::default().anonymous_const_record_type(),
            ConstValuePayload::Record(vec![
                ConstValueField { name: alpha, value },
                ConstValueField { name: beta, value },
            ]),
            None,
        )
        .expect("record");
    let Some(ConstValuePayload::Record(fields)) = store.payload(root) else {
        panic!("record payload");
    };
    assert_eq!(
        fields.iter().map(|field| field.name).collect::<Vec<_>>(),
        vec![alpha, beta]
    );
    store
        .fold_value(root, &mut |_, visit| -> Result<(), CompilerError> {
            if let ConstValueVisit::Record(fields) = visit {
                assert_eq!(
                    fields.iter().map(|field| *field.name).collect::<Vec<_>>(),
                    vec![alpha, beta]
                );
                assert!(
                    fields
                        .iter()
                        .all(|field| field.type_id == builtin_type_ids::BOOL)
                );
            }
            Ok(())
        })
        .expect("record visitor");
}

#[test]
fn duplicate_record_field_name_is_a_construction_error() {
    let mut strings = StringTable::new();
    let mut paths = PathInternerFork::empty();
    let name = paths
        .try_intern_portable_path("name", &mut strings)
        .expect("field path");
    let mut store = ConstValueStore::default();
    let value = store
        .insert_resolved(builtin_type_ids::BOOL, ConstValuePayload::Bool(true), None)
        .expect("field");
    let error = store
        .insert_resolved(
            TypeEnvironment::default().anonymous_const_record_type(),
            ConstValuePayload::Record(vec![
                ConstValueField { name, value },
                ConstValueField { name, value },
            ]),
            None,
        )
        .expect_err("duplicate fields");
    assert!(matches!(error, ConstValueStoreError::Infrastructure(_)));
    assert_eq!(store.values.len(), 1, "failed parent is never committed");
}

#[test]
fn fixed_scalar_values_round_trip_with_exact_bits_and_type() {
    let values = [
        FixedScalarValue::unsigned(FixedScalar::U64, u64::MAX).expect("U64 maximum"),
        FixedScalarValue::signed(FixedScalar::I64, i64::MIN).expect("I64 minimum"),
        FixedScalarValue::binary_float(FixedScalar::F16, -0.0).expect("negative zero"),
        FixedScalarValue::binary_float(FixedScalar::F16, 2f64.powi(-24)).expect("F16 subnormal"),
    ];
    let mut store = ConstValueStore::default();
    for value in values {
        let type_id = builtin_type_ids::fixed_scalar(value.scalar());
        let id = store
            .insert_resolved(type_id, ConstValuePayload::FixedScalar(value), None)
            .expect("scalar");
        assert!(
            matches!(store.payload(id), Some(ConstValuePayload::FixedScalar(stored)) if *stored == value)
        );
        assert_eq!(store.metadata(id).expect("metadata").type_id, type_id);
        let visited = store
            .fold_value(id, &mut |_, visit| -> Result<_, CompilerError> {
                match visit {
                    ConstValueVisit::FixedScalar(value) => Ok(value),
                    _ => Err(CompilerError::compiler_error("Expected fixed scalar.")),
                }
            })
            .expect("scalar visitor");
        assert_eq!(visited, value);
    }
    assert_eq!(
        values[2].as_f64().expect("float").to_bits(),
        (-0.0f64).to_bits()
    );
    assert_ne!(
        values[2],
        FixedScalarValue::binary_float(FixedScalar::F16, 0.0).expect("positive zero")
    );
}

#[test]
fn const_record_aliases_share_the_target_root() {
    let mut strings = StringTable::new();
    let mut paths = PathInternerFork::empty();
    let meta = paths
        .try_intern_portable_path("meta", &mut strings)
        .expect("path");
    let alias = paths
        .try_intern_portable_path("alias", &mut strings)
        .expect("path");
    let alpha = paths
        .try_intern_portable_path("alpha", &mut strings)
        .expect("field path");
    let mut store = ConstValueStore::default();
    let child = store
        .insert_resolved(builtin_type_ids::BOOL, ConstValuePayload::Bool(true), None)
        .expect("field");
    let root = store
        .insert_resolved(
            TypeEnvironment::default().anonymous_const_record_type(),
            ConstValuePayload::Record(vec![ConstValueField {
                name: alpha,
                value: child,
            }]),
            None,
        )
        .expect("record");
    store.bind_module_constant(meta, root).expect("target");
    store.bind_module_constant(alias, root).expect("alias");
    assert_eq!(store.value_for_path(&meta), store.value_for_path(&alias));
    let alpha = strings.intern("alpha");
    assert_eq!(store.field_value(root, alpha, &paths), Some(child));
}

/// Shared graph depth for the one-mebibyte traversal tests.
const DEEP_CHAIN_DEPTH: usize = 2048;

/// Marks the child copy that runs the bounded constant-graph worker.
const STACK_BUDGET_CHILD_ENV: &str = "MOTH_CONST_VALUE_STACK_BUDGET_CHILD";

/// Keep depth-sensitive graph walks on a bounded native stack inside their own child test.
fn on_one_mib_stack(test_suffix: &str, body: impl FnOnce() + Send + 'static) {
    if std::env::var_os(STACK_BUDGET_CHILD_ENV).is_none() {
        run_stack_budgeted_child(
            module_path!(),
            test_suffix,
            STACK_BUDGET_CHILD_ENV,
            "deep constant chain must fit a 1 MiB stack",
        );
        return;
    }

    std::thread::Builder::new()
        .name("const-graph-deep-chain".into())
        .stack_size(1024 * 1024)
        .spawn(body)
        .expect("spawn one mebibyte test thread")
        .join()
        .expect("deep constant chain completes");
}

/// Walks one single-child collection chain with a loop and checks the boolean leaf it ends on.
fn assert_collection_chain_leaf(store: &ConstValueStore, root: ConstValueId, depth: usize) {
    let mut id = root;
    for _ in 0..depth {
        let Some(ConstValuePayload::Collection(children)) = store.payload(id) else {
            panic!("chain level is a collection");
        };
        assert_eq!(children.len(), 1);
        id = children[0];
    }
    assert!(matches!(
        store.payload(id),
        Some(ConstValuePayload::Bool(true))
    ));
}

#[test]
fn stage_values_marks_a_deep_staged_chain_without_recursion() {
    on_one_mib_stack(
        "stage_values_marks_a_deep_staged_chain_without_recursion",
        || {
            let mut store = ConstValueStore::default();
            let root = store
                .stage_values(|candidate| -> Result<_, ConstValueStoreError> {
                    let mut id = candidate.insert_value(
                        ConstValueMetadata::resolved(
                            builtin_type_ids::BOOL,
                            None,
                            ValueMode::ImmutableOwned,
                        ),
                        ConstValuePayload::Bool(true),
                    )?;
                    for _ in 0..DEEP_CHAIN_DEPTH {
                        id = candidate.insert_value(
                            ConstValueMetadata::resolved(
                                builtin_type_ids::BOOL,
                                None,
                                ValueMode::ImmutableOwned,
                            ),
                            ConstValuePayload::Collection(vec![id]),
                        )?;
                    }
                    Ok(Some(id))
                })
                .expect("staged chain")
                .expect("selected root");
            assert_collection_chain_leaf(&store, root, DEEP_CHAIN_DEPTH);
        },
    );
}

/// Resolved `Bool` metadata for the staging tests, which only need distinct leaf rows.
fn resolved_bool_metadata() -> ConstValueMetadata {
    ConstValueMetadata::resolved(builtin_type_ids::BOOL, None, ValueMode::ImmutableOwned)
}

#[test]
fn stage_values_commits_each_root_of_a_multi_root_candidate_and_leaves_unreachable_rows_out() {
    let mut store = ConstValueStore::default();
    let roots = store
        .stage_values(|candidate| -> Result<_, ConstValueStoreError> {
            // Staged but never selected as a root, so it is unreachable and must not be committed.
            candidate.insert_value(resolved_bool_metadata(), ConstValuePayload::Bool(false))?;
            let first =
                candidate.insert_value(resolved_bool_metadata(), ConstValuePayload::Bool(true))?;
            let second =
                candidate.insert_value(resolved_bool_metadata(), ConstValuePayload::Bool(false))?;
            Ok(Some(vec![first, second]))
        })
        .expect("multi-root candidate should stage")
        .expect("the candidate should select its roots");
    assert_eq!(roots.len(), 2);
    assert!(matches!(
        store.payload(roots[0]),
        Some(ConstValuePayload::Bool(true))
    ));
    assert!(matches!(
        store.payload(roots[1]),
        Some(ConstValuePayload::Bool(false))
    ));
    // Only the two selected roots were committed, so no row exists at the next index.
    assert!(store.value(ConstValueId(2)).is_none());
}

#[test]
fn stage_values_discards_every_staged_row_when_a_multi_root_producer_fails() {
    let mut store = ConstValueStore::default();
    let failed = store.stage_values(
        |candidate| -> Result<Option<Vec<ConstValueId>>, ConstValueStoreError> {
            let staged =
                candidate.insert_value(resolved_bool_metadata(), ConstValuePayload::Bool(true))?;
            // The child belongs to no row of this candidate, so this insert fails after `staged`
            // is already staged: the partial state that staging must throw away.
            candidate.insert_value(
                resolved_bool_metadata(),
                ConstValuePayload::Collection(vec![ConstValueId(u32::MAX)]),
            )?;
            Ok(Some(vec![staged]))
        },
    );
    assert!(failed.is_err());
    assert!(
        store.value(ConstValueId(0)).is_none(),
        "a failed multi-root candidate must leave no committed row"
    );
}

#[test]
fn stage_values_refuses_a_root_that_names_no_row_and_commits_nothing() {
    let mut store = ConstValueStore::default();
    let refused = store.stage_values(
        |candidate| -> Result<Option<Vec<ConstValueId>>, ConstValueStoreError> {
            let staged =
                candidate.insert_value(resolved_bool_metadata(), ConstValuePayload::Bool(false))?;
            // Committed roots are valid without staging, so the refused root must resolve to
            // neither a committed row nor a candidate row: index 99 does neither.
            Ok(Some(vec![staged, ConstValueId(99)]))
        },
    );
    assert!(refused.is_err());
    assert!(
        store.value(ConstValueId(0)).is_none(),
        "a refused candidate must leave no committed row"
    );
}

#[test]
fn ingest_view_reads_a_deep_expression_chain_without_recursion() {
    on_one_mib_stack(
        "ingest_view_reads_a_deep_expression_chain_without_recursion",
        || {
            let mut store = ConstValueStore::default();
            let leaf = store
                .insert_resolved(builtin_type_ids::BOOL, ConstValuePayload::Bool(true), None)
                .expect("leaf");
            let mut builder = AstExpressionBuilder::new().unwrap();
            let mut expression = builder
                .append_expression(
                    ExpressionOp::Constant(leaf),
                    None,
                    builtin_type_ids::BOOL,
                    ValueMode::ImmutableOwned,
                )
                .expect("constant leaf")
                .id();
            for _ in 0..DEEP_CHAIN_DEPTH {
                let items = builder
                    .append_children(vec![builder.own(expression)], None)
                    .expect("collection items");
                expression = builder
                    .append_expression(
                        ExpressionOp::Collection(items),
                        None,
                        builtin_type_ids::BOOL,
                        ValueMode::ImmutableOwned,
                    )
                    .expect("collection row")
                    .id();
            }
            let id = store
                .ingest_view(
                    builder.view(expression).expect("bound root"),
                    &TypeEnvironment::default(),
                    &mut |_| Err(CompilerError::compiler_error("templates are absent").into()),
                )
                .expect("deep collection ingests");
            assert_collection_chain_leaf(&store, id, DEEP_CHAIN_DEPTH);
        },
    );
}

#[test]
fn fold_value_traverses_a_deep_chain_without_recursion() {
    on_one_mib_stack(
        "fold_value_traverses_a_deep_chain_without_recursion",
        || {
            let mut store = ConstValueStore::default();
            let leaf = store
                .insert_resolved(builtin_type_ids::BOOL, ConstValuePayload::Bool(true), None)
                .expect("chain leaf");
            let mut chain = leaf;
            let mut chain_ids = vec![leaf];
            for _ in 0..DEEP_CHAIN_DEPTH {
                chain = store
                    .insert_resolved(
                        builtin_type_ids::BOOL,
                        ConstValuePayload::Collection(vec![chain]),
                        None,
                    )
                    .expect("one chain level");
                chain_ids.push(chain);
            }
            let mut visited_depths = Vec::new();
            let root_depth = store
                .fold_value(
                    chain,
                    &mut |metadata, visit| -> Result<usize, CompilerError> {
                        let depth = match visit {
                            ConstValueVisit::Bool(true) => 0,
                            ConstValueVisit::Collection(mut children) => {
                                assert_eq!(children.len(), 1);
                                children.pop().expect("single child") + 1
                            }
                            _ => panic!("only the collection chain and its true leaf are visited"),
                        };
                        assert!(std::ptr::eq(
                            metadata,
                            store.metadata(chain_ids[depth]).expect("chain metadata")
                        ));
                        visited_depths.push(depth);
                        Ok(depth)
                    },
                )
                .expect("deep visitor completes");
            assert_eq!(root_depth, DEEP_CHAIN_DEPTH);
            assert_eq!(visited_depths, (0..=DEEP_CHAIN_DEPTH).collect::<Vec<_>>());
        },
    );
}

#[test]
fn fold_value_revisits_each_repeated_child_edge_independently() {
    // Deliberately not Clone: each shared edge must produce an independently owned output.
    struct Mapped(String);

    let mut store = ConstValueStore::default();
    let leaf = store
        .insert_resolved(builtin_type_ids::BOOL, ConstValuePayload::Bool(true), None)
        .expect("shared leaf");
    let wrapper = store
        .insert_resolved(
            builtin_type_ids::BOOL,
            ConstValuePayload::Collection(vec![leaf]),
            None,
        )
        .expect("shared wrapper");
    let root = store
        .insert_resolved(
            builtin_type_ids::BOOL,
            ConstValuePayload::Collection(vec![wrapper, wrapper]),
            None,
        )
        .expect("repeated wrapper edges");
    let mut leaf_visits = 0;
    let mut callback_order = Vec::new();
    let mapped = store
        .fold_value(root, &mut |_, visit| -> Result<Mapped, CompilerError> {
            match visit {
                ConstValueVisit::Bool(true) => {
                    leaf_visits += 1;
                    callback_order.push("leaf");
                    Ok(Mapped(format!("leaf{leaf_visits}")))
                }
                ConstValueVisit::Collection(children) => {
                    callback_order.push("collection");
                    let children = children
                        .into_iter()
                        .map(|child| child.0)
                        .collect::<Vec<_>>();
                    Ok(Mapped(format!("[{}]", children.join(","))))
                }
                _ => panic!("only collections and their shared leaf are visited"),
            }
        })
        .expect("repeated edge visitor");
    assert_eq!(mapped.0, "[[leaf1],[leaf2]]");
    assert_eq!(
        callback_order,
        vec!["leaf", "collection", "leaf", "collection", "collection"]
    );
}

#[test]
fn fold_value_maps_mixed_composites_in_postorder() {
    let mut strings = StringTable::new();
    let mut paths = PathInternerFork::empty();
    let nominal_path = paths
        .try_intern_portable_path("Selection", &mut strings)
        .expect("choice path");
    let field_name = paths
        .try_intern_portable_path("selected", &mut strings)
        .expect("field path");
    let mut store = ConstValueStore::default();
    let mut insert = |payload| {
        store
            .insert_resolved(builtin_type_ids::CHAR, payload, None)
            .expect("mixed graph node")
    };
    let prefix = insert(ConstValuePayload::Char('p'));
    let start = insert(ConstValuePayload::Char('a'));
    let end = insert(ConstValuePayload::Char('z'));
    let range = insert(ConstValuePayload::Range { start, end });
    let coerced = insert(ConstValuePayload::Coerced(range));
    let optional = insert(ConstValuePayload::OptionSome(coerced));
    let choice = insert(ConstValuePayload::Choice {
        nominal_path,
        tag: 3,
        fields: vec![ConstValueField {
            name: field_name,
            value: optional,
        }],
    });
    let empty = insert(ConstValuePayload::Collection(Vec::new()));
    let root = insert(ConstValuePayload::Collection(vec![prefix, choice, empty]));
    let mut callback_order = Vec::new();
    let mapped = store
        .fold_value(root, &mut |_, visit| -> Result<String, CompilerError> {
            let (kind, mapped) = match visit {
                ConstValueVisit::Char(character) => ("char", character.to_string()),
                ConstValueVisit::Range { start, end } => ("range", format!("({start}..{end})")),
                ConstValueVisit::Coerced(child) => ("coerced", format!("cast{child}")),
                ConstValueVisit::OptionSome(child) => ("optional", format!("some({child})")),
                ConstValueVisit::Choice {
                    nominal_path: visited_path,
                    tag,
                    fields,
                } => {
                    assert_eq!(*visited_path, nominal_path);
                    assert_eq!(fields.len(), 1);
                    let child = fields.into_iter().next().expect("choice field").value;
                    ("choice", format!("choice{tag}({child})"))
                }
                ConstValueVisit::Collection(children) => {
                    ("collection", format!("[{}]", children.join(",")))
                }
                _ => panic!("unexpected mixed graph payload"),
            };
            callback_order.push(kind);
            Ok(mapped)
        })
        .expect("mixed visitor");
    assert_eq!(mapped, "[p,choice3(some(cast(a..z))),[]]");
    assert_eq!(
        callback_order,
        vec![
            "char",
            "char",
            "char",
            "range",
            "coerced",
            "optional",
            "choice",
            "collection",
            "collection"
        ]
    );
}

#[test]
fn fold_value_callback_failure_precedes_later_field_lookup() {
    #[derive(Debug)]
    enum VisitFailure {
        Callback,
        Invariant,
    }

    impl From<CompilerError> for VisitFailure {
        fn from(_error: CompilerError) -> Self {
            Self::Invariant
        }
    }

    let mut strings = StringTable::new();
    let mut paths = PathInternerFork::empty();
    let first = paths
        .try_intern_portable_path("first", &mut strings)
        .expect("first field");
    let later = paths
        .try_intern_portable_path("later", &mut strings)
        .expect("later field");
    for is_choice in [false, true] {
        let mut store = ConstValueStore::default();
        let leaf = store
            .insert_resolved(builtin_type_ids::BOOL, ConstValuePayload::Bool(true), None)
            .expect("first child");
        let fields = vec![
            ConstValueField {
                name: first,
                value: leaf,
            },
            ConstValueField {
                name: later,
                value: leaf,
            },
        ];
        let payload = if is_choice {
            ConstValuePayload::Choice {
                nominal_path: first,
                tag: 0,
                fields,
            }
        } else {
            ConstValuePayload::Record(fields)
        };
        let root = store
            .insert_resolved(builtin_type_ids::BOOL, payload, None)
            .expect("valid field edges");
        // Corrupt a private row after construction to exercise the field lookup invariant.
        let fields = match &mut store.values[root.index()].payload {
            ConstValuePayload::Record(fields) | ConstValuePayload::Choice { fields, .. } => fields,
            _ => panic!("record or choice root"),
        };
        fields[1].value = ConstValueId(u32::MAX);

        let mut callbacks = 0;
        let failure = store.fold_value(root, &mut |_, visit| -> Result<(), VisitFailure> {
            assert!(matches!(visit, ConstValueVisit::Bool(true)));
            callbacks += 1;
            Err(VisitFailure::Callback)
        });
        assert!(matches!(failure, Err(VisitFailure::Callback)));
        assert_eq!(callbacks, 1);

        let mut callbacks = 0;
        let failure = store.fold_value(root, &mut |_, visit| -> Result<(), VisitFailure> {
            assert!(matches!(visit, ConstValueVisit::Bool(true)));
            callbacks += 1;
            Ok(())
        });
        assert!(matches!(failure, Err(VisitFailure::Invariant)));
        assert_eq!(
            callbacks, 1,
            "the earlier field completes before the lookup error"
        );
    }
}
