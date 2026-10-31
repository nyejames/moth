//! Shared diagnostic payload prose.
//!
//! WHAT: owns the primary message and optional guidance for every structured diagnostic payload.
//! WHY: terminal, terse, and dev-server renderers should differ only in output format. The
//! payload facts themselves must become user-facing text through one dispatch path.

use super::subject::DiagnosticSubject;
use super::*;
use crate::builder_surface::SourceFileKind;
use crate::compiler_frontend::compiler_messages::diagnostics::families::calls_data::InvalidValueReceiverView;
use crate::compiler_frontend::compiler_messages::diagnostics::families::generics_traits_api::{
    DuplicatePublicExportView, DuplicateTraitRequirementView, GenericBoundPrivateSurfaceLeakView,
    InvalidTraitConformanceNames, InvalidTraitConformanceShape, InvalidTraitConformanceView,
    InvalidTraitIncompatibilityView, InvalidTraitKeywordUsageView, PrivateTypeInExportedApiView,
    TraitNameUsedAsTypeView, TraitPrivateSurfaceLeakView, UnknownTraitView,
    UnsupportedTraitFeatureView, decode_invalid_trait_conformance_names,
};
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
    NonUtf8SourceView, SourceSpanCapacityView, UnexpectedEndOfFileView,
};
use crate::compiler_frontend::compiler_messages::diagnostics::families::types_borrows_targets::{
    EmptyCollectionTypeAmbiguityView, TypeMismatchView,
};
use crate::compiler_frontend::compiler_messages::diagnostics::view::DiagnosticView;
use crate::compiler_frontend::compiler_messages::{
    DiagnosticCapacityResource, InvalidTraitIncompatibilityReason, InvalidTraitKeywordUsageReason,
    ProjectContextEscapeReason, PublicGenericOwnerKind, TypeMismatchContext, ValueReceiverContext,
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
/// WHY: type spellings come from the owning site's captured snapshot; unresolved roots retain
/// their stable `TypeId(N)` spelling.
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
        guidance: type_mismatch_guidance(facts.context, &expected, &found),
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

fn type_mismatch_guidance(
    mismatch_context: TypeMismatchContext,
    expected: &str,
    found: &str,
) -> Vec<String> {
    let mut guidance = vec![format!("Expected: {expected}"), format!("Found: {found}")];
    if mismatch_context == TypeMismatchContext::CatchRecovery {
        guidance.push("Each `then` value must match the corresponding success slot.".to_owned());
    }
    guidance
}

/// WHAT: guarantees every terse record carries a non-empty message.
/// WHY: rows without message facts render empty prose, so the descriptor title supplies context.
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

fn unknown_trait_message(name: StringId, string_table: &dyn StringTableResolver) -> String {
    format!("Unknown trait '{}'.", string_table.resolve(name))
}

/// WHAT: renders an unknown trait from its compact name fact.
/// WHY: the name is the only variable part of this message.
pub(crate) fn render_unknown_trait(
    _diagnostic: &DiagnosticView<'_>,
    facts: &UnknownTraitView,
    context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    RenderedPayload {
        message: unknown_trait_message(facts.name, context.string_table),
        guidance: Vec::new(),
    }
}

fn duplicate_trait_requirement_message(
    trait_name: StringId,
    requirement_name: StringId,
    string_table: &dyn StringTableResolver,
) -> String {
    format!(
        "Trait '{}' declares duplicate requirement '{}'. Trait requirements cannot be overloaded in v1.",
        string_table.resolve(trait_name),
        string_table.resolve(requirement_name)
    )
}

/// WHAT: renders a duplicate trait requirement from its two names.
/// WHY: its label is stored separately from its message facts.
pub(crate) fn render_duplicate_trait_requirement(
    _diagnostic: &DiagnosticView<'_>,
    facts: &DuplicateTraitRequirementView,
    context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    RenderedPayload {
        message: duplicate_trait_requirement_message(
            facts.trait_name,
            facts.requirement_name,
            context.string_table,
        ),
        guidance: Vec::new(),
    }
}

fn trait_private_surface_leak_message(
    trait_name: StringId,
    surface_type: &str,
    string_table: &dyn StringTableResolver,
) -> String {
    format!(
        "Exported trait '{}' exposes private type {} in its requirement surface.",
        string_table.resolve(trait_name),
        surface_type
    )
}

/// WHAT: renders a trait surface leak from its trait name and type display.
/// WHY: compaction stores a `TypeDisplayId`, not a raw `TypeId`.
pub(crate) fn render_trait_private_surface_leak(
    diagnostic: &DiagnosticView<'_>,
    facts: &TraitPrivateSurfaceLeakView,
    context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    let surface_type = diagnostic.types().display_for_render(
        facts.surface_type,
        context.string_table,
        context.path_table(),
    );
    RenderedPayload {
        message: trait_private_surface_leak_message(
            facts.trait_name,
            &surface_type,
            context.string_table,
        ),
        guidance: Vec::new(),
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

fn non_utf8_source_message(path: PathId, context: DiagnosticRenderContext<'_>) -> String {
    format!("Source '{}' is not valid UTF-8", context.render_path(path))
}

/// WHAT: names the interned source path that failed UTF-8 decoding.
/// WHY: a title-only row cannot tell two invalid files apart.
pub(crate) fn render_non_utf8_source(
    _diagnostic: &DiagnosticView<'_>,
    facts: &NonUtf8SourceView,
    context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    RenderedPayload {
        message: non_utf8_source_message(facts.path, context),
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
    path: Option<&str>,
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
        SourceSpanCapacityResource::SourceSnapshot => match path {
            Some(path) => format!(
                "Source '{path}' is at least {length} bytes; a source must be shorter than {length} bytes \
                 so u32 offsets can address it."
            ),
            None => format!(
                "This source is at least {length} bytes; a source must be shorter than {length} bytes \
                 so u32 offsets can address it."
            ),
        },
    }
}

/// WHAT: renders source-span capacity facts without widening the durable record.
/// WHY: the rejected range and exhausted resource are the message facts that
/// `source_span_capacity_message` takes.
pub(crate) fn render_source_span_capacity(
    _diagnostic: &DiagnosticView<'_>,
    facts: &SourceSpanCapacityView,
    context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    let start = u32::try_from(facts.start)
        .expect("stored source-span capacity starts originated as u32 values");
    let length = u32::try_from(facts.length)
        .expect("stored source-span capacity lengths originated as u32 values");
    let path = facts.path.map(|id| context.render_path(id));
    RenderedPayload {
        message: source_span_capacity_message(start, length, facts.resource, path.as_deref()),
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
        CapacityResource::PathNodes => {
            "This source exceeds the compiler's capacity for path nodes.".into()
        }
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
        DiagnosticCapacityResource::TypeDisplays => "type displays",
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
/// WHY: the source kind is the only message fact; the guidance is fixed text.
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
/// WHY: the reason selects one of the fixed messages in `project_context_escape_message`.
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
    shape: &InvalidTraitConformanceShape<TypeDisplayId>,
    names: InvalidTraitConformanceNames,
    types: &TypeDisplaySnapshot,
    context: DiagnosticRenderContext<'_>,
) -> String {
    use InvalidTraitConformanceShape as Shape;
    let string_table = context.string_table;
    let target = string_table.resolve(names.target_name);
    let trait_text = names
        .trait_name
        .map(|name| format!(" to '{}'", string_table.resolve(name)))
        .unwrap_or_default();
    let reason_name = || {
        string_table.resolve(
            names
                .reason_name
                .expect("reason-shaped conformance facts carry their name"),
        )
    };

    match shape {
        Shape::ImportedModuleRoot => {
            "Trait conformance declarations are not allowed in imported module root files."
                .to_owned()
        }
        Shape::AliasTarget => {
            format!(
                "Type alias '{target}' cannot declare trait conformance. Use the underlying nominal type."
            )
        }
        Shape::NonCanonicalTarget => {
            format!(
                "'{target}' cannot produce trait evidence{trait_text}. User-authored conformance targets must be a user-defined struct, choice, or generic type constructor declared in the same file."
            )
        }
        Shape::NonlocalSourceTarget => {
            format!(
                "User-authored trait conformance for '{target}'{trait_text} is allowed only when the type is declared in the same file. Use a local wrapper choice or struct when adapting an imported type."
            )
        }
        Shape::BuiltinTarget => {
            format!(
                "Builtin type '{target}' cannot declare user-authored trait conformance{trait_text}. Use a local wrapper choice or struct when adapting a builtin value."
            )
        }
        Shape::ExternalOpaqueTarget => {
            format!(
                "External opaque type '{target}' cannot declare user-authored trait conformance{trait_text}. Use a local wrapper choice or struct when adapting an external type."
            )
        }
        Shape::DuplicateCanonicalEvidence => {
            format!("Duplicate canonical trait evidence for '{target}'{trait_text}.")
        }
        Shape::BuiltinEvidenceOverride => {
            format!(
                "User-authored trait evidence for '{target}'{trait_text} cannot override compiler-owned builtin evidence."
            )
        }
        Shape::IncompatibleTraitEvidence => {
            format!(
                "Type '{target}' cannot conform{trait_text} because it is incompatible with existing evidence for trait '{}'.",
                reason_name()
            )
        }
        Shape::MissingMethod => format!(
            "'{target}' cannot conform{trait_text} because same-file receiver method '{}' is missing.",
            reason_name()
        ),
        Shape::ReceiverMutabilityMismatch => format!(
            "'{target}' cannot conform{trait_text} because receiver mutability for '{}' does not match the trait requirement.",
            reason_name()
        ),
        Shape::ParameterCountMismatch { expected, found } => format!(
            "'{target}' cannot conform{trait_text} because '{}' expects {expected} non-receiver parameter(s) but the method has {found}.",
            reason_name()
        ),
        Shape::ParameterModeMismatch { parameter_index } => format!(
            "'{target}' cannot conform{trait_text} because parameter {parameter_index} of '{}' has a different value mode.",
            reason_name()
        ),
        Shape::ParameterContractMismatch { parameter_index } => format!(
            "'{target}' cannot conform{trait_text} because parameter {parameter_index} of '{}' uses a different parameter contract; a Wire, Route or value parameter must match the trait requirement exactly.",
            reason_name()
        ),
        Shape::ParameterTypeMismatch {
            parameter_index,
            expected_type,
            found_type,
        } => format!(
            "'{target}' cannot conform{trait_text} because parameter {parameter_index} of '{}' has type {}, expected {}.",
            reason_name(),
            diagnostic_type_display_name(types, *found_type, context),
            diagnostic_type_display_name(types, *expected_type, context)
        ),
        Shape::ReturnCountMismatch { expected, found } => format!(
            "'{target}' cannot conform{trait_text} because '{}' expects {expected} return slot(s) but the method has {found}.",
            reason_name()
        ),
        Shape::ReturnTypeMismatch {
            return_index,
            expected_type,
            found_type,
        } => format!(
            "'{target}' cannot conform{trait_text} because return {return_index} of '{}' has type {}, expected {}.",
            reason_name(),
            diagnostic_type_display_name(types, *found_type, context),
            diagnostic_type_display_name(types, *expected_type, context)
        ),
        Shape::ReturnChannelMismatch { return_index } => format!(
            "'{target}' cannot conform{trait_text} because return {return_index} of '{}' uses a different return channel.",
            reason_name()
        ),
        Shape::InferredFailureInInfallibleCast => format!(
            "'{target}' cannot conform{trait_text} because method '{}' has escaping inferred failure, which violates the infallible cast evidence contract. Handle the failure locally or use the corresponding TRY_CASTABLE_TO_* trait with its try_to_* method and declared Error! return slot.",
            reason_name()
        ),
    }
}

/// WHAT: renders trait conformance facts from the compact reason shape and its ordered names.
/// WHY: the shared message helper is the single owner of the prose for each conformance reason.
pub(crate) fn render_invalid_trait_conformance(
    diagnostic: &DiagnosticView<'_>,
    facts: &InvalidTraitConformanceView,
    context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    let names = decode_invalid_trait_conformance_names(facts.shape.code(), diagnostic.string_ids())
        .expect("trait-conformance shape code and StringIds agree");
    RenderedPayload {
        message: invalid_trait_conformance_message(
            &facts.shape,
            names,
            diagnostic.types(),
            context,
        ),
        guidance: Vec::new(),
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
/// WHAT: renders a trait-keyword misuse from its compact reason code.
/// WHY: each keyword's message is fixed text selected by its compact reason code.
pub(crate) fn render_invalid_trait_keyword_usage(
    _diagnostic: &DiagnosticView<'_>,
    view: &InvalidTraitKeywordUsageView,
    _context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    RenderedPayload {
        message: invalid_trait_keyword_usage_message(view.reason).to_owned(),
        guidance: Vec::new(),
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
/// WHAT: renders trait incompatibility facts through `invalid_trait_incompatibility_message`.
/// WHY: the reason and names are the complete message input.
pub(crate) fn render_invalid_trait_incompatibility(
    _diagnostic: &DiagnosticView<'_>,
    view: &InvalidTraitIncompatibilityView,
    context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    RenderedPayload {
        message: invalid_trait_incompatibility_message(
            view.subject_name,
            view.incompatible_trait_name,
            &view.reason,
            context,
        ),
        guidance: Vec::new(),
    }
}

fn generic_bound_private_surface_leak_message(
    owner_kind: PublicGenericOwnerKind,
    owner_name: StringId,
    trait_name: StringId,
    context: DiagnosticRenderContext<'_>,
) -> String {
    let (owner_label, kept_private) = match owner_kind {
        PublicGenericOwnerKind::Function => ("function", "the function"),
        PublicGenericOwnerKind::ReceiverMethod => ("receiver method", "the receiver type"),
        PublicGenericOwnerKind::Struct => ("struct", "the struct"),
        PublicGenericOwnerKind::Choice => ("choice", "the choice"),
    };
    format!(
        "Public generic {} '{}' exposes private trait bound '{}'. Make the trait public in the module that declares it, or keep {} private.",
        owner_label,
        context.string_table.resolve(owner_name),
        context.string_table.resolve(trait_name),
        kept_private
    )
}

/// WHAT: renders a generic-bound visibility leak from its compact owner and name facts.
/// WHY: the owner kind selects the wording in `generic_bound_private_surface_leak_message`.
pub(crate) fn render_generic_bound_private_surface_leak(
    _diagnostic: &DiagnosticView<'_>,
    view: &GenericBoundPrivateSurfaceLeakView,
    context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    RenderedPayload {
        message: generic_bound_private_surface_leak_message(
            view.owner_kind,
            view.owner_name,
            view.trait_name,
            context,
        ),
        guidance: Vec::new(),
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
/// WHAT: renders a trait-as-type diagnostic from its compact trait-name fact.
/// WHY: `trait_name_used_as_type_message` owns the message text.
pub(crate) fn render_trait_name_used_as_type(
    _diagnostic: &DiagnosticView<'_>,
    view: &TraitNameUsedAsTypeView,
    context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    RenderedPayload {
        message: trait_name_used_as_type_message(view.trait_name, context),
        guidance: Vec::new(),
    }
}

fn unsupported_trait_feature_message(
    trait_name: StringId,
    feature: StringId,
    context: DiagnosticRenderContext<'_>,
) -> String {
    format!(
        "Trait '{}' uses unsupported feature '{}'.",
        context.string_table.resolve(trait_name),
        context.string_table.resolve(feature)
    )
}

/// WHAT: renders an unsupported trait feature from its compact string facts.
/// WHY: both names are the complete input to `unsupported_trait_feature_message`.
pub(crate) fn render_unsupported_trait_feature(
    _diagnostic: &DiagnosticView<'_>,
    view: &UnsupportedTraitFeatureView,
    context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    RenderedPayload {
        message: unsupported_trait_feature_message(view.trait_name, view.feature, context),
        guidance: Vec::new(),
    }
}
fn duplicate_public_export_message(name: StringId, context: DiagnosticRenderContext<'_>) -> String {
    format!(
        "Duplicate public export '{}' in module public surface. Each exported name must be unique.",
        context.string_table.resolve(name)
    )
}

/// WHAT: renders duplicate public exports from the compact exported-name fact.
/// WHY: `duplicate_public_export_message` builds the message from the exported name.
pub(crate) fn render_duplicate_public_export(
    _diagnostic: &DiagnosticView<'_>,
    view: &DuplicatePublicExportView,
    context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    RenderedPayload {
        message: duplicate_public_export_message(view.name, context),
        guidance: Vec::new(),
    }
}

fn private_type_in_exported_api_message(exported_name: &str, private_type: &str) -> String {
    format!(
        "Exported declaration '{}' exposes private type {}. Export that type through the same public surface or hide it behind a public wrapper.",
        exported_name, private_type
    )
}

/// WHAT: renders an exported-API visibility leak from its name and type facts.
/// WHY: the diagnostic type snapshot spells the type after the live type environment is gone.
pub(crate) fn render_private_type_in_exported_api(
    diagnostic: &DiagnosticView<'_>,
    view: &PrivateTypeInExportedApiView,
    context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    let private_type = diagnostic.types().display_for_render(
        view.private_type,
        context.string_table,
        context.path_table(),
    );
    let exported_name = context.string_table.resolve(view.exported_name);
    RenderedPayload {
        message: private_type_in_exported_api_message(exported_name, &private_type),
        guidance: Vec::new(),
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

/// Renders a missing package symbol from its compact string facts and direct-export candidates.
pub(crate) fn render_missing_package_symbol(
    diagnostic: &DiagnosticView<'_>,
    view: &MissingPackageSymbolView,
    context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    let mut message = missing_package_symbol_message(view.symbol, view.package_path, context);
    let symbol = context.string_table.resolve(view.symbol);
    if let Some(suggested) =
        closest_name_suggestion(symbol, diagnostic.string_ids(), context.string_table)
    {
        message.push_str(&format!(" Did you mean '{suggested}'?"));
    }
    RenderedPayload {
        message,
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

/// Renders the bare-file import message from the compact path fact.
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

/// WHAT: renders a family that carries no message text.
/// WHY: an empty message leaves the descriptor title as the only text.
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

fn empty_collection_type_ambiguity_message() -> String {
    "Cannot infer the type of an empty `{}` literal. Add an explicit type annotation.".to_owned()
}

/// Renders the fixed empty-collection message and its explicit-type guidance.
pub(crate) fn render_empty_collection_type_ambiguity(
    _diagnostic: &DiagnosticView<'_>,
    _view: &EmptyCollectionTypeAmbiguityView,
    _context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    RenderedPayload {
        message: empty_collection_type_ambiguity_message(),
        guidance: vec![
            "Add an explicit type annotation, for example `scores {String = I32} = {}` for an empty map or `items {I32} = {}` for an empty collection.".to_owned(),
        ],
    }
}
