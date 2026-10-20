//! Shared diagnostic payload prose.
//!
//! WHAT: owns the primary message and optional guidance for every structured diagnostic payload.
//! WHY: terminal, terse, and dev-server renderers should differ only in output format. The
//! payload facts themselves must become user-facing text through one dispatch path.

use super::subject::DiagnosticSubject;
use super::*;
use crate::builder_surface::SourceFileKind;
use crate::compiler_frontend::compiler_messages::diagnostics::families::calls_data::InvalidValueReceiverView;
use crate::compiler_frontend::compiler_messages::diagnostics::families::imports::{
    AmbiguousImportTargetView, BareFileImportView, CrossModuleImportNotExportedView,
    DependencyAliasCaseMismatchView, DirectSpecialFileImportView, ImportNameCollisionView,
    MissingModuleRootPublicSurfaceView, MissingPackageSymbolView, NotExportedByPublicSurfaceView,
    NotExportedBySourceFileView,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::project_config::CircularDependencyView;
use crate::compiler_frontend::compiler_messages::diagnostics::families::project_config::LegacyDependencyClauseView;
use crate::compiler_frontend::compiler_messages::diagnostics::families::project_config::ProjectContextEscapeView;
use crate::compiler_frontend::compiler_messages::diagnostics::families::templates::UnescapedImplicitTemplateCloseView;
use crate::compiler_frontend::compiler_messages::diagnostics::families::token_lexical::CompilerCapacityExceededView;
use crate::compiler_frontend::compiler_messages::diagnostics::families::token_lexical::{
    InvalidCharacterView, MissingClosingDelimiterView, UnexpectedTrailingCommaView,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::token_lexical::{
    SourceSpanCapacityView, UnexpectedEndOfFileView,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::types_borrows_targets::TypeMismatchView;
use crate::compiler_frontend::compiler_messages::diagnostics::view::DiagnosticView;
use crate::compiler_frontend::compiler_messages::{
    DiagnosticCapacityResource, DiagnosticPayload, InvalidTraitConformanceReason,
    InvalidTraitIncompatibilityReason, InvalidTraitKeywordUsageReason, ProjectContextEscapeReason,
    PublicGenericOwnerKind, TypeMismatchContext, ValueReceiverContext,
};

pub(crate) struct RenderedPayload {
    pub(crate) message: String,
    pub(crate) guidance: Vec<String>,
}

fn invalid_value_receiver_message(
    receiver: ValueReceiverContext,
    target_count: usize,
    produced_count: usize,
) -> String {
    let receiver_name = value_receiver_context_name(receiver);
    format!(
        "Invalid {receiver_name} receiver: expected {target_count} success slot(s), but this expression produces {produced_count}."
    )
}

/// WHAT: renders a value-receiver arity error and its actionable guidance.
/// WHY: the compact facts retain the receiving context and both success-slot counts.
pub(crate) fn render_invalid_value_receiver(
    _diagnostic: &DiagnosticView<'_>,
    view: &InvalidValueReceiverView,
    _context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    let guidance = if view.produced_count > 1 {
        "Use a multi-bind with one target per success slot."
    } else {
        "Use an expression that produces the required success values."
    };
    RenderedPayload {
        message: invalid_value_receiver_message(
            view.receiver,
            view.target_count,
            view.produced_count,
        ),
        guidance: vec![guidance.to_owned()],
    }
}

pub(crate) fn render_payload(
    payload: &DiagnosticPayload,
    context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    let message = render_payload_message(payload, context);
    let guidance = match payload {
        DiagnosticPayload::CompileTimeEvaluationError { reason, .. } => {
            vec![compile_time_evaluation_error_suggestion(*reason).to_owned()]
        }
        DiagnosticPayload::InvalidGenericInstantiation {
            reason:
                crate::compiler_frontend::compiler_messages::InvalidGenericInstantiationReason::CannotInferFunctionArguments {
                    ..
                },
            ..
        } => {
            vec![
                "Add a type annotation to the receiving declaration, for example `value I32 = ...`."
                    .to_owned(),
            ]
        }
        DiagnosticPayload::EmptyCollectionTypeAmbiguity => {
            vec![
                "Add an explicit type annotation, for example `scores {String = I32} = {}` for an empty map or `items {I32} = {}` for an empty collection.".to_owned(),
            ]
        }
        _ => Vec::new(),
    };

    RenderedPayload { message, guidance }
}

/// WHAT: renders a capacity diagnostic from its compact facts.
/// WHY: the record carries the resource, which is everything the message needs.
pub(crate) fn render_compiler_capacity_exceeded(
    _diagnostic: &DiagnosticView<'_>,
    facts: &CompilerCapacityExceededView,
    _context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    RenderedPayload {
        message: capacity_message(facts.resource),
        guidance: Vec::new(),
    }
}

/// WHAT: renders the fixed unexpected-trailing-comma message.
/// WHY: the row carries no facts, so the message is constant.
pub(crate) fn render_unexpected_trailing_comma(
    _diagnostic: &DiagnosticView<'_>,
    _view: &UnexpectedTrailingCommaView,
    _context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    RenderedPayload {
        message: "Unexpected trailing comma".to_owned(),
        guidance: Vec::new(),
    }
}

/// WHAT: renders an invalid character from its compact fact.
/// WHY: the character is the whole message.
pub(crate) fn render_invalid_character(
    _diagnostic: &DiagnosticView<'_>,
    view: &InvalidCharacterView,
    _context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    RenderedPayload {
        message: format!("Invalid character: '{}'", view.character),
        guidance: Vec::new(),
    }
}

/// WHAT: renders a missing closing delimiter from its compact fact.
/// WHY: the delimiter text is the only variable part of the message.
pub(crate) fn render_missing_closing_delimiter(
    _diagnostic: &DiagnosticView<'_>,
    view: &MissingClosingDelimiterView,
    context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    RenderedPayload {
        message: format!(
            "Missing closing delimiter '{}'",
            context.string_table.resolve(view.expected_delimiter)
        ),
        guidance: Vec::new(),
    }
}

/// WHAT: renders a type mismatch from its compact facts.
/// WHY: each type is spelled through the snapshot's display rules. A type prints as a raw
/// `TypeId` unless both the live type environment and the path table are available.
pub(crate) fn render_type_mismatch(
    diagnostic: &DiagnosticView<'_>,
    facts: &TypeMismatchView,
    context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    let expected = diagnostic.types().display_for_render(
        facts.expected,
        context.string_table,
        context.path_table(),
    );
    let found = diagnostic.types().display_for_render(
        facts.found,
        context.string_table,
        context.path_table(),
    );
    RenderedPayload {
        message: type_mismatch_message(facts.context, &expected, &found),
        guidance: type_mismatch_guidance(&expected, &found),
    }
}

fn type_mismatch_message(
    mismatch_context: TypeMismatchContext,
    expected: &str,
    found: &str,
) -> String {
    format!(
        "Type mismatch in {}: expected {}, found {}",
        type_mismatch_context_name(mismatch_context),
        expected,
        found
    )
}

fn type_mismatch_guidance(expected: &str, found: &str) -> Vec<String> {
    vec![format!("Expected: {expected}"), format!("Found: {found}")]
}

/// WHAT: guarantees every terse record carries a non-empty message.
/// WHY: descriptor-only diagnostics use `DiagnosticPayload::None`, which renders an empty
/// payload message. The descriptor title is always present and provides useful context.
pub(crate) fn terse_diagnostic_message(
    subject: &DiagnosticSubject<'_>,
    context: DiagnosticRenderContext<'_>,
) -> String {
    let rendered = subject.payload(context);
    if rendered.message.is_empty() {
        subject.descriptor().title.to_owned()
    } else {
        rendered.message
    }
}

fn render_payload_message(
    payload: &DiagnosticPayload,
    context: DiagnosticRenderContext<'_>,
) -> String {
    let string_table = context.string_table;

    match payload {
        DiagnosticPayload::UnknownName { name, namespace } => {
            unknown_name_message(*name, *namespace, string_table)
        }
        DiagnosticPayload::DirectSymbolPathImport { .. }
        | DiagnosticPayload::InvalidNamespaceDefaultName { .. }
        | DiagnosticPayload::DuplicateImportSurfaceMember { .. }
        | DiagnosticPayload::ExplicitMothExtension { .. }
        | DiagnosticPayload::ExplicitSourceExtension { .. }
        | DiagnosticPayload::UnsupportedSourceFileKind { .. }
        | DiagnosticPayload::InvalidSourceFileEntry { .. }
        | DiagnosticPayload::MothTemplateInputsShareNoCommonAncestor { .. }
        | DiagnosticPayload::DuplicateMothTemplateInputPath { .. }
        | DiagnosticPayload::UnsupportedExternalExtension { .. }
        | DiagnosticPayload::InvalidExternalModule { .. } => {
            import_payload_message(payload, context)
        }
        DiagnosticPayload::BorrowConflict { .. }
        | DiagnosticPayload::MultipleMutableBorrows { .. }
        | DiagnosticPayload::SharedMutableConflict { .. }
        | DiagnosticPayload::UseAfterPossibleMove { .. }
        | DiagnosticPayload::MoveWhileBorrowed { .. }
        | DiagnosticPayload::WholeObjectBorrowConflict { .. }
        | DiagnosticPayload::InvalidMutableAccess { .. } => {
            borrow_payload_message(payload, context)
        }
        DiagnosticPayload::UnsupportedExternalFunction {
            function_name,
            package_path,
            backend_name,
        } => unsupported_external_function_message(
            *function_name,
            *package_path,
            *backend_name,
            string_table,
        ),
        DiagnosticPayload::InvalidGenericApplication { reason } => {
            invalid_generic_application_message(*reason).to_owned()
        }
        DiagnosticPayload::InvalidParameterContract { reason } => {
            invalid_parameter_contract_message(*reason)
        }
        DiagnosticPayload::InvalidGenericParameter { reason } => {
            invalid_generic_parameter_message(reason, string_table)
        }
        DiagnosticPayload::InvalidFunctionSignature { reason } => {
            invalid_function_signature_message(reason, string_table)
        }
        DiagnosticPayload::InvalidChoiceVariant {
            reason,
            choice_name,
            variant_name,
            available_variants,
        } => invalid_choice_variant_message(
            *reason,
            *choice_name,
            *variant_name,
            available_variants,
            string_table,
        ),
        DiagnosticPayload::UnknownTrait { name } => {
            format!("Unknown trait '{}'.", string_table.resolve(*name))
        }
        DiagnosticPayload::DuplicateTraitRequirement {
            trait_name,
            requirement_name,
        } => format!(
            "Trait '{}' declares duplicate requirement '{}'. Trait requirements cannot be overloaded in v1.",
            string_table.resolve(*trait_name),
            string_table.resolve(*requirement_name)
        ),
        DiagnosticPayload::TraitPrivateSurfaceLeak {
            trait_name,
            surface_type,
        } => format!(
            "Exported trait '{}' exposes private type {} in its requirement surface.",
            string_table.resolve(*trait_name),
            diagnostic_type_name(*surface_type, context)
        ),
        DiagnosticPayload::GenericBoundPrivateSurfaceLeak {
            owner_kind,
            owner_name,
            trait_name,
        } => {
            let (owner_label, kept_private) = match owner_kind {
                PublicGenericOwnerKind::Function => ("function", "the function"),
                PublicGenericOwnerKind::ReceiverMethod => ("receiver method", "the receiver type"),
                PublicGenericOwnerKind::Struct => ("struct", "the struct"),
                PublicGenericOwnerKind::Choice => ("choice", "the choice"),
            };
            format!(
                "Public generic {} '{}' exposes private trait bound '{}'. Make the trait public in the module that declares it, or keep {} private.",
                owner_label,
                string_table.resolve(*owner_name),
                string_table.resolve(*trait_name),
                kept_private
            )
        }
        DiagnosticPayload::PrivateTypeInExportedApi {
            exported_name,
            private_type,
        } => format!(
            "Exported declaration '{}' exposes private type {}. Export that type through the same public surface or hide it behind a public wrapper.",
            string_table.resolve(*exported_name),
            diagnostic_type_name(*private_type, context)
        ),
        DiagnosticPayload::UnsupportedTraitFeature {
            trait_name,
            feature,
        } => format!(
            "Trait '{}' uses unsupported feature '{}'.",
            string_table.resolve(*trait_name),
            string_table.resolve(*feature)
        ),
        DiagnosticPayload::InvalidTraitKeywordUsage { reason } => {
            invalid_trait_keyword_usage_message(*reason).to_owned()
        }
        DiagnosticPayload::DuplicatePublicExport { name } => format!(
            "Duplicate public export '{}' in module public surface. Each exported name must be unique.",
            string_table.resolve(*name)
        ),
        DiagnosticPayload::InvalidTraitConformance {
            target_name,
            trait_name,
            reason,
        } => invalid_trait_conformance_message(*target_name, *trait_name, reason, context),
        DiagnosticPayload::InvalidTraitIncompatibility {
            subject_name,
            incompatible_trait_name,
            reason,
        } => invalid_trait_incompatibility_message(
            *subject_name,
            *incompatible_trait_name,
            reason,
            context,
        ),
        DiagnosticPayload::TraitNameUsedAsType { trait_name } => {
            trait_name_used_as_type_message(*trait_name, context)
        }
        DiagnosticPayload::InvalidThisUsage { reason } => {
            invalid_this_usage_message(*reason, string_table)
        }
        DiagnosticPayload::InvalidReceiverDeclaration { reason } => {
            invalid_receiver_declaration_message(*reason, context)
        }
        DiagnosticPayload::InvalidControlFlowStatement { reason } => {
            invalid_control_flow_statement_message(*reason)
        }
        DiagnosticPayload::InvalidMatchPattern {
            reason,
            variant_name,
            ..
        } => invalid_match_pattern_message(*reason, *variant_name, string_table),
        DiagnosticPayload::InvalidFallibleHandling { reason } => {
            invalid_fallible_handling_message(reason, context)
        }
        DiagnosticPayload::CompileTimeEvaluationError { reason, operation } => {
            compile_time_evaluation_error_message(*reason, *operation, string_table)
        }
        DiagnosticPayload::EmptyCollectionTypeAmbiguity => {
            "Cannot infer the type of an empty `{}` literal. Add an explicit type annotation."
                .to_owned()
        }
        DiagnosticPayload::UnsupportedOperatorTypes { operator, lhs, rhs } => {
            unsupported_operator_types_message(*operator, *lhs, *rhs, context)
        }
        DiagnosticPayload::IncompatibleChoiceComparison { reason, lhs, rhs } => {
            incompatible_choice_comparison_message(reason, *lhs, *rhs, context)
        }
        DiagnosticPayload::InvalidRouteOutput { reason, output } => {
            invalid_route_output_message(reason, *output, context)
        }
        DiagnosticPayload::InvalidGenericInstantiation { type_name, reason } => {
            invalid_generic_instantiation_message(*type_name, reason, context)
        }
        DiagnosticPayload::UnsupportedBackendFeature {
            backend_name,
            reason,
        } => format!(
            "Backend '{}' does not support {} yet.",
            string_table.resolve(*backend_name),
            reason.feature_name()
        ),
        DiagnosticPayload::InvalidStandaloneStatement { reason } => {
            invalid_standalone_statement_message(*reason)
        }
        DiagnosticPayload::InvalidMatchArm { reason } => invalid_match_arm_message(*reason),
        DiagnosticPayload::InvalidLoopHeader { reason } => {
            invalid_loop_header_message(*reason, context)
        }
        DiagnosticPayload::InvalidStatementPosition { reason } => {
            invalid_statement_position_message(*reason)
        }
        DiagnosticPayload::None => String::new(),
        _ => unreachable!(
            "migrated families render through the compact path; P4.6 deletes this function"
        ),
    }
}
fn circular_dependency_message(path: PathId, context: DiagnosticRenderContext<'_>) -> String {
    format!("Circular dependency at '{}'", context.render_path(path))
}

/// WHAT: renders a circular dependency from its compact path fact.
/// WHY: the path is the only variable part of this diagnostic's message.
pub(crate) fn render_circular_dependency(
    _diagnostic: &DiagnosticView<'_>,
    facts: &CircularDependencyView,
    context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    RenderedPayload {
        message: circular_dependency_message(facts.path, context),
        guidance: Vec::new(),
    }
}
fn legacy_dependency_clause_message(
    replacement: Option<StringId>,
    string_table: &dyn StringTableResolver,
) -> String {
    match replacement {
        Some(replacement) => format!(
            "Source dependency clauses no longer use `import`. Write `{}`.",
            string_table.resolve(replacement)
        ),
        None => "Source dependency clauses no longer use `import`, and this clause has no automatic delimiter-free replacement. Choose either a namespace alias or direct flat selections beginning with `@`.".to_owned(),
    }
}

/// WHAT: renders a legacy dependency clause from its compact replacement fact.
/// WHY: the legacy reason is fixed to the import keyword, and its replacement is the only message fact.
pub(crate) fn render_legacy_dependency_clause(
    _diagnostic: &DiagnosticView<'_>,
    facts: &LegacyDependencyClauseView,
    context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    let _ = facts.reason;
    RenderedPayload {
        message: legacy_dependency_clause_message(facts.replacement, context.string_table),
        guidance: Vec::new(),
    }
}

fn source_span_capacity_message(
    start: u32,
    length: u32,
    resource: SourceSpanCapacityResource,
) -> String {
    match resource {
        SourceSpanCapacityResource::ExtendedSpan => format!(
            "This source needs an exact span at byte offset {start} with length {length}, but its \
             extended span table cannot hold another long or late range."
        ),
        SourceSpanCapacityResource::LogicalPath => format!(
            "This project needs more than {length} logical path entries in its compact path \
             identity table; the four-byte path table cannot address another entry."
        ),
        SourceSpanCapacityResource::SourceIdentity => format!(
            "This project needs more than {length} source files in its compact source identity \
             table; the four-byte identity table cannot address another source."
        ),
        SourceSpanCapacityResource::PathSyntax => format!(
            "This source contains more than {length} authored path syntax rows; the compact \
             source-owned path table cannot address another row."
        ),
        SourceSpanCapacityResource::NumericLiteral => format!(
            "This source contains more than {length} retained numeric literals; the compact \
             numeric side store cannot address another row."
        ),
        SourceSpanCapacityResource::Token => format!(
            "This source contains more than {length} tokens; the compact \
             source-owned token store cannot address another token."
        ),
    }
}

/// WHAT: renders source-span capacity facts without widening the durable record.
/// WHY: the rejected source range and exhausted resource are the complete legacy payload.
pub(crate) fn render_source_span_capacity(
    _diagnostic: &DiagnosticView<'_>,
    facts: &SourceSpanCapacityView,
    _context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    let start = u32::try_from(facts.start)
        .expect("stored source-span capacity starts originated as u32 values");
    let length = u32::try_from(facts.length)
        .expect("stored source-span capacity lengths originated as u32 values");
    RenderedPayload {
        message: source_span_capacity_message(start, length, facts.resource),
        guidance: Vec::new(),
    }
}

fn unexpected_end_of_file_message(
    expected_delimiter: Option<StringId>,
    string_table: &dyn StringTableResolver,
) -> String {
    if let Some(expected_delimiter) = expected_delimiter {
        format!(
            "Unexpected end of file, expected '{}'",
            string_table.resolve(expected_delimiter)
        )
    } else {
        "Unexpected end of file".to_owned()
    }
}

/// WHAT: renders an optional expected delimiter from the compact EOF row.
/// WHY: the reason text is the same regardless of whether the parser named a delimiter.
pub(crate) fn render_unexpected_end_of_file(
    _diagnostic: &DiagnosticView<'_>,
    facts: &UnexpectedEndOfFileView,
    context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    RenderedPayload {
        message: unexpected_end_of_file_message(facts.expected_delimiter, context.string_table),
        guidance: Vec::new(),
    }
}

fn capacity_message(resource: CapacityResource) -> String {
    match resource {
        CapacityResource::Hir(resource) => hir_capacity_message(resource),
        CapacityResource::Ast(resource) => ast_capacity_message(resource),
        CapacityResource::Store(resource) => store_capacity_message(resource),
    }
}

fn hir_capacity_message(resource: HirCapacityResource) -> String {
    let resource_name = match resource {
        HirCapacityResource::ExpressionRows => "expression rows",
        HirCapacityResource::ValueEdges => "value edges",
        HirCapacityResource::StructFields => "struct fields",
        HirCapacityResource::VariantFields => "variant fields",
        HirCapacityResource::MapEntries => "map entries",
        HirCapacityResource::StringPieces => "string pieces",
        HirCapacityResource::PlaceProjections => "place projections",
    };
    format!("This source exceeds the compiler's capacity for HIR {resource_name}.")
}

fn store_capacity_message(resource: DiagnosticCapacityResource) -> String {
    let resource_name = match resource {
        DiagnosticCapacityResource::Records => "records",
        DiagnosticCapacityResource::Extras => "extra data rows",
        DiagnosticCapacityResource::Labels => "secondary labels",
        DiagnosticCapacityResource::LabelData => "label data rows",
        DiagnosticCapacityResource::StringIds => "string id lists",
        DiagnosticCapacityResource::Substitutions => "generic substitution rows",
        DiagnosticCapacityResource::ForeignSites => "foreign diagnostic sites",
        DiagnosticCapacityResource::IdentityContexts => "identity contexts",
        DiagnosticCapacityResource::FactValue => "fact values",
    };
    format!("This source exceeds the compiler's capacity for diagnostic {resource_name}.")
}

fn ast_capacity_message(resource: AstCapacityResource) -> String {
    match resource {
        AstCapacityResource::ExpressionRows => {
            "This source exceeds the compiler's capacity for AST expression rows.".into()
        }
        AstCapacityResource::ExpressionSideEntries => {
            "This source exceeds the compiler's capacity for AST expression side entries.".into()
        }
        AstCapacityResource::ConstValues => {
            "This source exceeds the compiler's capacity for constant values.".into()
        }
        AstCapacityResource::ChoiceTags => {
            "This source exceeds the compiler's capacity for choice variant tags.".into()
        }
        AstCapacityResource::GroupDistances => {
            "This source exceeds the compiler's capacity for expression group distances.".into()
        }
    }
}

fn source_kind_name(source_kind: SourceFileKind) -> &'static str {
    match source_kind {
        SourceFileKind::Moth => "Moth `.moth` source",
        SourceFileKind::MothTemplate => "Moth template `.mtf` source",
        SourceFileKind::PlainMarkdown => "plain Markdown `.md` source",
    }
}

/// WHAT: renders the implicit-template-close diagnostic from its compact source-kind fact.
/// WHY: its message and guidance are the legacy payload's complete output.
pub(crate) fn render_unescaped_implicit_template_close(
    _diagnostic: &DiagnosticView<'_>,
    facts: &UnescapedImplicitTemplateCloseView,
    _context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    RenderedPayload {
        message: format!(
            "{} starts inside an implicit template body, so an unescaped `]` would close a template that is not written in the source file.",
            source_kind_name(facts.source_kind)
        ),
        guidance: vec![
            "Insert a literal closing bracket through a nested string expression such as `[\"]\"]`."
                .to_owned(),
        ],
    }
}

fn project_context_escape_message(reason: ProjectContextEscapeReason) -> String {
    match reason {
        ProjectContextEscapeReason::ExportedDeclaration => {
            "ProjectContext-dependent declaration cannot be exported through a package facade."
                .to_owned()
        }
        ProjectContextEscapeReason::ReachableExecutable => {
            "ProjectContext-dependent executable function cannot be reached through a package facade."
                .to_owned()
        }
    }
}
/// WHAT: renders a project-context escape from its compact reason code.
/// WHY: the reason selects one of the legacy fixed messages.
pub(crate) fn render_project_context_escape(
    _diagnostic: &DiagnosticView<'_>,
    facts: &ProjectContextEscapeView,
    _context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    RenderedPayload {
        message: project_context_escape_message(facts.reason),
        guidance: Vec::new(),
    }
}

fn invalid_trait_conformance_message(
    target_name: StringId,
    trait_name: Option<StringId>,
    reason: &InvalidTraitConformanceReason,
    context: DiagnosticRenderContext<'_>,
) -> String {
    let string_table = context.string_table;
    let target = string_table.resolve(target_name);
    let trait_text = trait_name
        .map(|name| format!(" to '{}'", string_table.resolve(name)))
        .unwrap_or_default();

    match reason {
        InvalidTraitConformanceReason::ImportedModuleRoot => {
            "Trait conformance declarations are not allowed in imported module root files."
                .to_owned()
        }
        InvalidTraitConformanceReason::AliasTarget => {
            format!(
                "Type alias '{target}' cannot declare trait conformance. Use the underlying nominal type."
            )
        }
        InvalidTraitConformanceReason::NonCanonicalTarget => {
            format!(
                "'{target}' cannot produce trait evidence{trait_text}. User-authored conformance targets must be a user-defined struct, choice, or generic type constructor declared in the same file."
            )
        }
        InvalidTraitConformanceReason::NonlocalSourceTarget => {
            format!(
                "User-authored trait conformance for '{target}'{trait_text} is allowed only when the type is declared in the same file. Use a local wrapper choice or struct when adapting an imported type."
            )
        }
        InvalidTraitConformanceReason::BuiltinTarget => {
            format!(
                "Builtin type '{target}' cannot declare user-authored trait conformance{trait_text}. Use a local wrapper choice or struct when adapting a builtin value."
            )
        }
        InvalidTraitConformanceReason::ExternalOpaqueTarget => {
            format!(
                "External opaque type '{target}' cannot declare user-authored trait conformance{trait_text}. Use a local wrapper choice or struct when adapting an external type."
            )
        }
        InvalidTraitConformanceReason::DuplicateCanonicalEvidence => {
            format!("Duplicate canonical trait evidence for '{target}'{trait_text}.")
        }
        InvalidTraitConformanceReason::BuiltinEvidenceOverride => {
            format!(
                "User-authored trait evidence for '{target}'{trait_text} cannot override compiler-owned builtin evidence."
            )
        }
        InvalidTraitConformanceReason::IncompatibleTraitEvidence {
            incompatible_trait_name,
        } => {
            format!(
                "Type '{target}' cannot conform{trait_text} because it is incompatible with existing evidence for trait '{}'.",
                string_table.resolve(*incompatible_trait_name)
            )
        }
        InvalidTraitConformanceReason::MissingMethod { requirement_name } => {
            format!(
                "'{target}' cannot conform{trait_text} because same-file receiver method '{}' is missing.",
                string_table.resolve(*requirement_name)
            )
        }
        InvalidTraitConformanceReason::ReceiverMutabilityMismatch { requirement_name } => {
            format!(
                "'{target}' cannot conform{trait_text} because receiver mutability for '{}' does not match the trait requirement.",
                string_table.resolve(*requirement_name)
            )
        }
        InvalidTraitConformanceReason::ParameterCountMismatch {
            requirement_name,
            expected,
            found,
        } => {
            format!(
                "'{target}' cannot conform{trait_text} because '{}' expects {expected} non-receiver parameter(s) but the method has {found}.",
                string_table.resolve(*requirement_name)
            )
        }
        InvalidTraitConformanceReason::ParameterModeMismatch {
            requirement_name,
            parameter_index,
        } => {
            format!(
                "'{target}' cannot conform{trait_text} because parameter {parameter_index} of '{}' has a different value mode.",
                string_table.resolve(*requirement_name)
            )
        }
        InvalidTraitConformanceReason::ParameterContractMismatch {
            requirement_name,
            parameter_index,
        } => {
            format!(
                "'{target}' cannot conform{trait_text} because parameter {parameter_index} of '{}' uses a different parameter contract; a Wire, Route or value parameter must match the trait requirement exactly.",
                string_table.resolve(*requirement_name)
            )
        }
        InvalidTraitConformanceReason::ParameterTypeMismatch {
            requirement_name,
            parameter_index,
            expected_type,
            found_type,
        } => {
            format!(
                "'{target}' cannot conform{trait_text} because parameter {parameter_index} of '{}' has type {}, expected {}.",
                string_table.resolve(*requirement_name),
                diagnostic_type_name(*found_type, context),
                diagnostic_type_name(*expected_type, context)
            )
        }
        InvalidTraitConformanceReason::ReturnCountMismatch {
            requirement_name,
            expected,
            found,
        } => {
            format!(
                "'{target}' cannot conform{trait_text} because '{}' expects {expected} return slot(s) but the method has {found}.",
                string_table.resolve(*requirement_name)
            )
        }
        InvalidTraitConformanceReason::ReturnTypeMismatch {
            requirement_name,
            return_index,
            expected_type,
            found_type,
        } => {
            format!(
                "'{target}' cannot conform{trait_text} because return {return_index} of '{}' has type {}, expected {}.",
                string_table.resolve(*requirement_name),
                diagnostic_type_name(*found_type, context),
                diagnostic_type_name(*expected_type, context)
            )
        }
        InvalidTraitConformanceReason::ReturnChannelMismatch {
            requirement_name,
            return_index,
        } => {
            format!(
                "'{target}' cannot conform{trait_text} because return {return_index} of '{}' uses a different return channel.",
                string_table.resolve(*requirement_name)
            )
        }
        InvalidTraitConformanceReason::InferredFailureInInfallibleCast { requirement_name } => {
            format!(
                "'{target}' cannot conform{trait_text} because method '{}' has escaping inferred failure, which violates the infallible cast evidence contract. Handle the failure locally or use the corresponding TRY_CASTABLE_TO_* trait with its try_to_* method and declared Error! return slot.",
                string_table.resolve(*requirement_name)
            )
        }
    }
}

fn invalid_trait_keyword_usage_message(reason: InvalidTraitKeywordUsageReason) -> &'static str {
    match reason {
        InvalidTraitKeywordUsageReason::MustOutsideTraitSyntax => {
            "Keyword 'must' is trait-only syntax and cannot be used here."
        }
        InvalidTraitKeywordUsageReason::ThisOutsideTraitSyntax => {
            "Keyword 'This' is trait-local syntax and cannot be used here."
        }
    }
}

fn invalid_trait_incompatibility_message(
    subject_name: StringId,
    incompatible_trait_name: Option<StringId>,
    reason: &InvalidTraitIncompatibilityReason,
    context: DiagnosticRenderContext<'_>,
) -> String {
    let string_table = context.string_table;
    let subject = string_table.resolve(subject_name);
    let trait_text = incompatible_trait_name
        .map(|name| format!("'{}'", string_table.resolve(name)))
        .unwrap_or_else(|| "a trait".to_string());

    match reason {
        InvalidTraitIncompatibilityReason::SelfIncompatible => {
            format!("Trait '{subject}' cannot be declared incompatible with itself.")
        }
        InvalidTraitIncompatibilityReason::UnknownTrait => {
            format!(
                "Trait incompatibility declaration for '{subject}' references unknown trait {trait_text}. Both traits must be visible and declared before the relation."
            )
        }
        InvalidTraitIncompatibilityReason::DuplicateRelation => {
            format!(
                "Duplicate incompatibility relation between '{subject}' and {trait_text}. The same pair of traits only needs to be declared incompatible once."
            )
        }
        InvalidTraitIncompatibilityReason::PrivateTraitSurfaceLeak => {
            format!(
                "Exported trait incompatibility relation between '{subject}' and {trait_text} exposes a private trait through the public export surface. Both sides of an exported relation must be public from that surface."
            )
        }
    }
}

fn trait_name_used_as_type_message(
    trait_name: StringId,
    context: DiagnosticRenderContext<'_>,
) -> String {
    let trait_text = context.string_table.resolve(trait_name);
    format!(
        "Trait '{trait_text}' is a static contract and cannot be used as a value type. Use `type T is {trait_text}` for static reuse or a choice for runtime heterogeneity."
    )
}

fn import_payload_message(
    payload: &DiagnosticPayload,
    context: DiagnosticRenderContext<'_>,
) -> String {
    match payload {
        DiagnosticPayload::DirectSymbolPathImport { path } => {
            direct_symbol_path_import_message(*path, context)
        }
        DiagnosticPayload::InvalidNamespaceDefaultName { path } => {
            invalid_namespace_default_name_message(*path, context)
        }
        DiagnosticPayload::DuplicateImportSurfaceMember {
            surface_path,
            member_name,
        } => duplicate_import_surface_member_message(*surface_path, *member_name, context),
        DiagnosticPayload::ExplicitMothExtension { path } => {
            explicit_moth_extension_message(*path, context)
        }
        DiagnosticPayload::ExplicitSourceExtension { path, extension } => {
            explicit_source_extension_message(*path, *extension, context)
        }
        DiagnosticPayload::UnsupportedSourceFileKind { path, extension } => {
            unsupported_source_file_kind_message(*path, *extension, context)
        }
        DiagnosticPayload::InvalidSourceFileEntry { path, extension } => {
            invalid_source_file_entry_message(*path, *extension, context)
        }
        DiagnosticPayload::DuplicateMothTemplateInputPath { path, .. } => {
            duplicate_moth_template_input_path_message(*path, context)
        }
        DiagnosticPayload::MothTemplateInputsShareNoCommonAncestor {
            first_path,
            second_path,
        } => moth_template_inputs_share_no_common_ancestor_message(
            *first_path,
            *second_path,
            context,
        ),
        DiagnosticPayload::UnsupportedExternalExtension { path, extension } => {
            unsupported_external_extension_message(*path, *extension, context)
        }
        DiagnosticPayload::InvalidExternalModule { path, reason } => {
            invalid_external_module_message(*path, reason, context)
        }
        _ => unreachable!(
            "migrated families render through the compact path; P4.6 deletes this function"
        ),
    }
}

fn missing_package_symbol_message(
    symbol: StringId,
    package_path: StringId,
    context: DiagnosticRenderContext<'_>,
) -> String {
    format!(
        "Cannot bind '{}' from package '{}': symbol not found.",
        context.string_table.resolve(symbol),
        context.string_table.resolve(package_path)
    )
}

/// Renders a missing package symbol from its compact string facts.
pub(crate) fn render_missing_package_symbol(
    _diagnostic: &DiagnosticView<'_>,
    view: &MissingPackageSymbolView,
    context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    RenderedPayload {
        message: missing_package_symbol_message(view.symbol, view.package_path, context),
        guidance: Vec::new(),
    }
}

fn cross_module_import_not_exported_message(
    symbol_path: PathId,
    context: DiagnosticRenderContext<'_>,
) -> String {
    format!(
        "Cannot bind '{}' because it is not exported by the target module's public surface.",
        context.render_path(symbol_path)
    )
}

/// Renders a cross-module export failure from its compact path fact.
pub(crate) fn render_cross_module_import_not_exported(
    _diagnostic: &DiagnosticView<'_>,
    view: &CrossModuleImportNotExportedView,
    context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    RenderedPayload {
        message: cross_module_import_not_exported_message(view.symbol_path, context),
        guidance: Vec::new(),
    }
}

fn not_exported_by_source_file_message(
    symbol_path: PathId,
    context: DiagnosticRenderContext<'_>,
) -> String {
    format!(
        "Cannot bind '{}' because it is not exported.",
        context.render_path(symbol_path)
    )
}

/// Renders an unexported source-file symbol from its compact path fact.
pub(crate) fn render_not_exported_by_source_file(
    _diagnostic: &DiagnosticView<'_>,
    view: &NotExportedBySourceFileView,
    context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    RenderedPayload {
        message: not_exported_by_source_file_message(view.symbol_path, context),
        guidance: Vec::new(),
    }
}

fn not_exported_by_public_surface_message(
    requested_path: PathId,
    public_surface_name: StringId,
    public_surface_type: crate::compiler_frontend::compiler_messages::ImportPublicSurfaceType,
    context: DiagnosticRenderContext<'_>,
) -> String {
    let path_text = context.render_path(requested_path);
    let public_surface_name = context.string_table.resolve(public_surface_name);
    match public_surface_type {
        crate::compiler_frontend::compiler_messages::ImportPublicSurfaceType::SourcePackage => {
            format!(
                "Cannot bind '{path_text}' from source-backed package '@{public_surface_name}' because it is not exported by the package public surface."
            )
        }
        crate::compiler_frontend::compiler_messages::ImportPublicSurfaceType::ModuleRoot => {
            format!(
                "Cannot bind '{path_text}' from module '{public_surface_name}' because it is not exported by the module's public surface."
            )
        }
    }
}

/// Renders a public-surface export failure from its path, name and typed surface facts.
pub(crate) fn render_not_exported_by_public_surface(
    _diagnostic: &DiagnosticView<'_>,
    view: &NotExportedByPublicSurfaceView,
    context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    RenderedPayload {
        message: not_exported_by_public_surface_message(
            view.requested_path,
            view.public_surface_name,
            view.public_surface_type,
            context,
        ),
        guidance: Vec::new(),
    }
}

fn missing_module_root_public_surface_message(
    symbol_path: PathId,
    context: DiagnosticRenderContext<'_>,
) -> String {
    format!(
        "Cannot bind '{}' because the target module has no public export surface. Depend on a concrete file from inside the same module, or add a module root file with an `export:` block to define the module's public dependency surface.",
        context.render_path(symbol_path)
    )
}

/// Renders the missing module-root surface message from its compact path fact.
pub(crate) fn render_missing_module_root_public_surface(
    _diagnostic: &DiagnosticView<'_>,
    view: &MissingModuleRootPublicSurfaceView,
    context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    RenderedPayload {
        message: missing_module_root_public_surface_message(view.symbol_path, context),
        guidance: Vec::new(),
    }
}

fn dependency_alias_case_mismatch_message(
    alias: StringId,
    symbol: StringId,
    context: DiagnosticRenderContext<'_>,
) -> String {
    format!(
        "Dependency alias '{}' case mismatch with symbol '{}'",
        context.string_table.resolve(alias),
        context.string_table.resolve(symbol)
    )
}

/// Renders the dependency alias's case mismatch from its compact name facts.
pub(crate) fn render_dependency_alias_case_mismatch(
    _diagnostic: &DiagnosticView<'_>,
    view: &DependencyAliasCaseMismatchView,
    context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    RenderedPayload {
        message: dependency_alias_case_mismatch_message(view.alias, view.symbol, context),
        guidance: Vec::new(),
    }
}

fn import_name_collision_message(name: StringId, context: DiagnosticRenderContext<'_>) -> String {
    format!(
        "Dependency binding name collision: '{}' is already visible in this file.",
        context.string_table.resolve(name)
    )
}

/// Renders a colliding import binding from its compact name fact.
pub(crate) fn render_import_name_collision(
    _diagnostic: &DiagnosticView<'_>,
    view: &ImportNameCollisionView,
    context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    RenderedPayload {
        message: import_name_collision_message(view.name, context),
        guidance: Vec::new(),
    }
}
fn ambiguous_import_target_message(path: PathId, context: DiagnosticRenderContext<'_>) -> String {
    format!(
        "Ambiguous dependency target '{}'. Use a more specific path.",
        context.render_path(path)
    )
}

/// Renders an ambiguous dependency target from its compact path fact.
pub(crate) fn render_ambiguous_import_target(
    _diagnostic: &DiagnosticView<'_>,
    view: &AmbiguousImportTargetView,
    context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    RenderedPayload {
        message: ambiguous_import_target_message(view.path, context),
        guidance: Vec::new(),
    }
}

fn direct_special_file_import_message(
    path: PathId,
    context: DiagnosticRenderContext<'_>,
) -> String {
    let special_file = special_file_name_from_path(path, context);
    let path_text = context.render_path(path);
    let suggestion = support_root_import_suggestion(path, context);
    format!(
        "Cannot depend directly on '{special_file}' via '{path_text}'. Support roots are referenced through their package directory, not by filename.{suggestion}"
    )
}

/// Renders a direct special-file dependency from its compact path fact.
pub(crate) fn render_direct_special_file_import(
    _diagnostic: &DiagnosticView<'_>,
    view: &DirectSpecialFileImportView,
    context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    RenderedPayload {
        message: direct_special_file_import_message(view.path, context),
        guidance: Vec::new(),
    }
}
fn bare_file_import_message(path: PathId, context: DiagnosticRenderContext<'_>) -> String {
    format!(
        "Bare file dependency clauses are not supported; select an exported symbol from the file '{}'.",
        context.render_path(path)
    )
}

/// Renders the legacy bare-file import message from the compact path fact.
pub(crate) fn render_bare_file_import(
    _diagnostic: &DiagnosticView<'_>,
    view: &BareFileImportView,
    context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    RenderedPayload {
        message: bare_file_import_message(view.path, context),
        guidance: Vec::new(),
    }
}

fn borrow_payload_message(
    payload: &DiagnosticPayload,
    context: DiagnosticRenderContext<'_>,
) -> String {
    match payload {
        DiagnosticPayload::BorrowConflict {
            place,
            existing_access,
            requested_access,
            ..
        } => borrow_conflict_message(place, *existing_access, *requested_access, context),
        DiagnosticPayload::MultipleMutableBorrows {
            place,
            conflicting_place,
        } => multiple_mutable_borrows_message(place, conflicting_place.as_ref(), context),
        DiagnosticPayload::SharedMutableConflict {
            place,
            existing_access,
            requested_access,
            conflicting_place,
        } => shared_mutable_conflict_message(
            place,
            *existing_access,
            *requested_access,
            conflicting_place.as_ref(),
            context,
        ),
        DiagnosticPayload::UseAfterPossibleMove { place, .. } => {
            use_after_possible_move_message(place, context)
        }
        DiagnosticPayload::MoveWhileBorrowed {
            place,
            existing_access,
            ..
        } => move_while_borrowed_message(place, *existing_access, context),
        DiagnosticPayload::WholeObjectBorrowConflict {
            whole_place,
            part_place,
            ..
        } => whole_object_borrow_conflict_message(whole_place, part_place, context),
        DiagnosticPayload::InvalidMutableAccess {
            place,
            reason,
            conflicting_place,
        } => invalid_mutable_access_message(place, *reason, conflicting_place.as_ref(), context),
        _ => unreachable!(
            "migrated families render through the compact path; P4.6 deletes this function"
        ),
    }
}

/// WHAT: renders a family whose legacy payload carried no message.
/// WHY: an empty message leaves the descriptor title as the only text, as the `None` payload did.
pub(crate) fn render_title_only<V>(
    _diagnostic: &DiagnosticView<'_>,
    _view: &V,
    _context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    RenderedPayload {
        message: String::new(),
        guidance: Vec::new(),
    }
}
