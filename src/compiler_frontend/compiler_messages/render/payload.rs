//! Shared diagnostic payload prose.
//!
//! WHAT: owns the primary message and optional guidance for every structured diagnostic payload.
//! WHY: terminal, terse, and dev-server renderers should differ only in output format. The
//! payload facts themselves must become user-facing text through one dispatch path.

use super::*;
use crate::builder_surface::SourceFileKind;
use crate::compiler_frontend::compiler_messages::{
    DiagnosticKind, DiagnosticPayload, InvalidTraitConformanceReason,
    InvalidTraitIncompatibilityReason, InvalidTraitKeywordUsageReason, NamingConvention,
    ReservedNameOwner,
};

pub(crate) struct RenderedPayload {
    pub(crate) message: String,
    pub(crate) guidance: Vec<String>,
}

pub(crate) fn render_payload(
    payload: &DiagnosticPayload,
    context: DiagnosticRenderContext<'_>,
) -> RenderedPayload {
    let message = render_payload_message(payload, context);
    let guidance = match payload {
        DiagnosticPayload::CommonSyntaxMistake { reason } => {
            vec![format!(
                "Suggestion: {}",
                common_syntax_mistake_suggestion(reason)
            )]
        }
        DiagnosticPayload::UnescapedImplicitTemplateClose { .. } => vec![
            "Insert a literal closing bracket through a nested string expression such as `[\"]\"]`.".to_owned(),
        ],
        DiagnosticPayload::TypeMismatch {
            expected, found, ..
        } => vec![
            format!("Expected: {}", diagnostic_type_name(*expected, context)),
            format!("Found: {}", diagnostic_type_name(*found, context)),
        ],
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
                "Add a type annotation to the receiving declaration, for example `value Int = ...`."
                    .to_owned(),
            ]
        }
        DiagnosticPayload::EmptyCollectionTypeAmbiguity => {
            vec![
                "Add an explicit type annotation, for example `scores {String = Int} = {}` for an empty map or `items {Int} = {}` for an empty collection.".to_owned(),
            ]
        }
        DiagnosticPayload::InvalidTemplateDirective {
            reason: crate::compiler_frontend::compiler_messages::InvalidTemplateDirectiveReason::UnknownDirective,
            ..
        } => {
            vec![
                "Known template directives include `$md`, `$raw`, `$children`, `$slot`, `$insert`, `$fresh`, `$html`, `$css`, `$escape_html`, `$code`, `$note`, `$todo`, and `$doc`.".to_owned(),
            ]
        }
        _ => Vec::new(),
    };

    RenderedPayload { message, guidance }
}

/// WHAT: guarantees every terse record carries a non-empty message.
/// WHY: descriptor-only diagnostics use `DiagnosticPayload::None`, which renders an empty
/// payload message. The descriptor title is always present and provides useful context.
pub(crate) fn terse_payload_message(
    payload: &DiagnosticPayload,
    kind: DiagnosticKind,
    context: DiagnosticRenderContext<'_>,
) -> String {
    let rendered = render_payload(payload, context);
    if rendered.message.is_empty() {
        kind.descriptor().title.to_owned()
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
        DiagnosticPayload::InfrastructureError { msg, .. } => msg.clone(),
        DiagnosticPayload::ExpectedToken { expected, found } => {
            expected_token_message(expected, found.as_ref(), string_table)
        }
        DiagnosticPayload::UnexpectedToken { found } => {
            unexpected_token_message(found, string_table)
        }
        DiagnosticPayload::UnexpectedTrailingComma => "Unexpected trailing comma".to_owned(),
        DiagnosticPayload::UnescapedImplicitTemplateClose { source_kind } => {
            format!(
                "{} starts inside an implicit template body, so an unescaped `]` would close a template that is not written in the source file.",
                source_kind_name(*source_kind)
            )
        }
        DiagnosticPayload::UnknownName { name, namespace } => {
            unknown_name_message(*name, *namespace, string_table)
        }
        DiagnosticPayload::TypeMismatch {
            expected,
            found,
            context: mismatch_context,
        } => format!(
            "Type mismatch in {}: expected {}, found {}",
            type_mismatch_context_name(*mismatch_context),
            diagnostic_type_name(*expected, context),
            diagnostic_type_name(*found, context)
        ),
        DiagnosticPayload::DuplicateDeclaration { name, .. } => {
            duplicate_declaration_message(*name, string_table)
        }
        DiagnosticPayload::MissingImportTarget { .. }
        | DiagnosticPayload::AmbiguousImportTarget { .. }
        | DiagnosticPayload::BareFileImport { .. }
        | DiagnosticPayload::DirectSpecialFileImport { .. }
        | DiagnosticPayload::ImportNameCollision { .. }
        | DiagnosticPayload::NotExportedBySourceFile { .. }
        | DiagnosticPayload::NotExportedByPublicSurface { .. }
        | DiagnosticPayload::MissingModuleRootPublicSurface { .. }
        | DiagnosticPayload::MissingPackageSymbol { .. }
        | DiagnosticPayload::CrossModuleImportNotExported { .. }
        | DiagnosticPayload::InvalidImportPath { .. }
        | DiagnosticPayload::DirectSymbolPathImport { .. }
        | DiagnosticPayload::InvalidNamespaceDefaultName { .. }
        | DiagnosticPayload::DuplicateImportSurfaceMember { .. }
        | DiagnosticPayload::ExplicitMothExtension { .. }
        | DiagnosticPayload::ExplicitSourceExtension { .. }
        | DiagnosticPayload::UnsupportedSourceFileKind { .. }
        | DiagnosticPayload::InvalidSourceFileEntry { .. }
        | DiagnosticPayload::InvalidMothTemplateApiScopeItem { .. }
        | DiagnosticPayload::DuplicateMothTemplateInputPath { .. }
        | DiagnosticPayload::UnsupportedExternalExtension { .. }
        | DiagnosticPayload::InvalidExternalModule { .. } => {
            import_payload_message(payload, string_table)
        }
        DiagnosticPayload::BorrowConflict { .. }
        | DiagnosticPayload::MultipleMutableBorrows { .. }
        | DiagnosticPayload::SharedMutableConflict { .. }
        | DiagnosticPayload::UseAfterPossibleMove { .. }
        | DiagnosticPayload::MoveWhileBorrowed { .. }
        | DiagnosticPayload::WholeObjectBorrowConflict { .. }
        | DiagnosticPayload::InvalidMutableAccess { .. }
        | DiagnosticPayload::UseOfUninitializedLocal { .. } => {
            borrow_payload_message(payload, string_table)
        }
        DiagnosticPayload::InvalidConfig { key, reason } => {
            invalid_config_message(*key, reason, string_table)
        }
        DiagnosticPayload::DeferredFeature { reason } => {
            deferred_feature_message(reason, string_table)
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
        DiagnosticPayload::UnusedName { name } => {
            format!("Unused name '{}'", string_table.resolve(*name))
        }
        DiagnosticPayload::UnreachableMatchArm => "Unreachable match arm".to_owned(),
        DiagnosticPayload::MothFilePathInTemplateOutput { path } => format!(
            "Moth source path '{}' is being inserted into template output",
            string_table.resolve(*path)
        ),
        DiagnosticPayload::LargeTrackedAsset { path, byte_size } => {
            let mib = *byte_size as f64 / (1024.0 * 1024.0);
            format!(
                "Large tracked asset '{}' ({mib:.1} MiB)",
                string_table.resolve(*path)
            )
        }
        DiagnosticPayload::IdentifierNamingConvention {
            name,
            expected_style,
        } => {
            let style_name = match expected_style {
                NamingConvention::CamelCase => "CamelCase",
                NamingConvention::LowercaseWithUnderscores => "lowercase_with_underscores",
                NamingConvention::UppercaseWithUnderscores => "UPPER_CASE_WITH_UNDERSCORES",
                NamingConvention::LowercaseOrUppercaseWithUnderscores => {
                    "lowercase_with_underscores or UPPER_CASE_WITH_UNDERSCORES"
                }
            };
            format!(
                "Identifier '{}' should use {}",
                string_table.resolve(*name),
                style_name
            )
        }
        DiagnosticPayload::DependencyAliasCaseMismatch { alias, symbol } => format!(
            "Dependency alias '{}' case mismatch with symbol '{}'",
            string_table.resolve(*alias),
            string_table.resolve(*symbol)
        ),
        DiagnosticPayload::MalformedTemplate { message } => {
            format!("Malformed template: {}", string_table.resolve(*message))
        }
        DiagnosticPayload::InvalidCharacter { character } => {
            format!("Invalid character: '{character}'")
        }
        DiagnosticPayload::InvalidStringEscape { reason } => invalid_string_escape_message(*reason),
        DiagnosticPayload::InvalidNumberLiteral {
            literal_text,
            reason,
        } => invalid_number_literal_message(*literal_text, *reason, string_table),
        DiagnosticPayload::InvalidStyleDirective {
            directive_name,
            supported_directives,
        } => invalid_style_directive_message(*directive_name, *supported_directives, string_table),
        DiagnosticPayload::MissingClosingDelimiter { expected_delimiter } => {
            format!(
                "Missing closing delimiter '{}'",
                string_table.resolve(*expected_delimiter)
            )
        }
        DiagnosticPayload::InvalidGenericApplication { reason } => {
            invalid_generic_application_message(*reason).to_owned()
        }
        DiagnosticPayload::UnexpectedEndOfFile { expected_delimiter } => {
            if let Some(expected_delimiter) = expected_delimiter {
                format!(
                    "Unexpected end of file, expected '{}'",
                    string_table.resolve(*expected_delimiter)
                )
            } else {
                "Unexpected end of file".to_owned()
            }
        }
        DiagnosticPayload::InvalidPath { path_kind } => invalid_path_message(*path_kind).to_owned(),
        DiagnosticPayload::InvalidDependencyClause { reason, .. } => {
            invalid_dependency_clause_message(*reason).to_owned()
        }
        DiagnosticPayload::LegacyDependencyClause { replacement, .. } => match replacement {
            Some(replacement) => format!(
                "Source dependency clauses no longer use `import`. Write `{}`.",
                string_table.resolve(*replacement)
            ),
            None => "Source dependency clauses no longer use `import`, and this clause has no automatic delimiter-free replacement. Choose either a namespace alias or direct flat selections beginning with `@`.".to_owned(),
        },
        DiagnosticPayload::InvalidTypeAnnotation { reason, .. } => {
            invalid_type_annotation_message(reason, string_table)
        }
        DiagnosticPayload::InvalidCollectionType { reason } => {
            invalid_collection_type_message(*reason).to_owned()
        }
        DiagnosticPayload::InvalidMapType { reason } => invalid_map_type_message(*reason, context),
        DiagnosticPayload::InvalidMapLiteral { reason } => {
            invalid_map_literal_message(*reason).to_owned()
        }
        DiagnosticPayload::InvalidGenericParameter { reason } => {
            invalid_generic_parameter_message(reason, string_table)
        }
        DiagnosticPayload::InvalidTemplateDirective {
            directive_name,
            reason,
        } => invalid_template_directive_message(*directive_name, *reason, string_table),
        DiagnosticPayload::InvalidTemplateStructure { reason } => {
            invalid_template_structure_message(*reason, context)
        }
        DiagnosticPayload::InvalidSignatureMember { reason } => {
            invalid_signature_member_message(*reason)
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
        DiagnosticPayload::InvalidStructDefaultValue => "Invalid struct default value".to_owned(),
        DiagnosticPayload::MissingDeclarationInitializer { name } => {
            format!(
                "Declaration '{}' requires '=' followed by an initializer expression.",
                string_table.resolve(*name)
            )
        }
        DiagnosticPayload::CircularDependency { path } => {
            format!(
                "Circular dependency at '{}'",
                path.to_portable_string(string_table)
            )
        }
        DiagnosticPayload::NamespaceMisuse {
            name,
            expected,
            found,
        } => namespace_misuse_message(*name, *expected, *found, string_table),
        DiagnosticPayload::DependencyNamespaceUsedAsValue { record_name } => {
            dependency_namespace_used_as_value_message(*record_name, string_table)
        }
        DiagnosticPayload::ConstRecordUsedAsValue { record_name } => {
            const_record_used_as_value_message(*record_name, string_table)
        }
        DiagnosticPayload::NestedDependencyTraversal { record_name } => {
            nested_dependency_traversal_message(*record_name, string_table)
        }
        DiagnosticPayload::NamespaceTypeValueMisuse {
            name,
            expected,
            found,
        } => namespace_type_value_misuse_message(*name, *expected, *found, string_table),
        DiagnosticPayload::UnknownTrait { name } => {
            format!("Unknown trait '{}'.", string_table.resolve(*name))
        }
        DiagnosticPayload::DuplicateTraitRequirement {
            trait_name,
            requirement_name,
            ..
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
            function_name,
            trait_name,
        } => format!(
            "Public generic function '{}' exposes private trait bound '{}'. Export the trait through the same public surface or keep the function private.",
            string_table.resolve(*function_name),
            string_table.resolve(*trait_name)
        ),
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
        DiagnosticPayload::DuplicatePublicExport { name, .. } => format!(
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
        DiagnosticPayload::ShadowedName { name, .. } => {
            format!("Shadowed name '{}'", string_table.resolve(*name))
        }
        DiagnosticPayload::ReservedNameCollision { name, reserved_by } => {
            let owner = match reserved_by {
                ReservedNameOwner::BuiltinType => "builtin type",
                ReservedNameOwner::Keyword => "keyword",
                ReservedNameOwner::CoreTrait => "core trait",
            };
            format!(
                "Reserved name collision: '{}' is a reserved {}",
                string_table.resolve(*name),
                owner
            )
        }
        DiagnosticPayload::InvalidThisUsage { reason } => {
            invalid_this_usage_message(*reason, string_table)
        }
        DiagnosticPayload::InvalidReceiverDeclaration { reason } => {
            invalid_receiver_declaration_message(*reason, string_table)
        }
        DiagnosticPayload::InvalidControlFlowStatement { reason } => {
            invalid_control_flow_statement_message(*reason)
        }
        DiagnosticPayload::InvalidDeclaration { reason, name } => {
            invalid_declaration_message(reason.clone(), *name, string_table)
        }
        DiagnosticPayload::InvalidAssignmentTarget {
            reason,
            target_name,
            target_type,
            field_name,
            root_binding_name,
            declaration_location: _,
        } => invalid_assignment_target_message(
            *reason,
            *target_name,
            *target_type,
            *field_name,
            *root_binding_name,
            context,
        ),
        DiagnosticPayload::InvalidMultiBind {
            reason,
            target_name,
        } => invalid_multi_bind_message(*reason, *target_name, string_table),
        DiagnosticPayload::InvalidBuiltinCall {
            reason,
            builtin_name,
        } => invalid_builtin_call_message(*reason, *builtin_name, string_table),
        DiagnosticPayload::InvalidCast {
            reason,
            source_type,
            target_type,
        } => invalid_cast_message(*reason, *source_type, *target_type, context),
        DiagnosticPayload::InvalidReceiverCall {
            reason,
            receiver_type,
            method_name,
            receiver_kind,
            receiver_binding_name,
        } => invalid_receiver_call_message(
            *reason,
            *receiver_type,
            *method_name,
            *receiver_kind,
            *receiver_binding_name,
            string_table,
        ),
        DiagnosticPayload::InvalidCopyTarget { reason } => invalid_copy_target_message(*reason),
        DiagnosticPayload::InvalidFieldAccess {
            reason,
            field_name,
            receiver_type,
            known_fields,
        } => invalid_field_access_message(
            *reason,
            *field_name,
            *receiver_type,
            known_fields,
            context,
        ),
        DiagnosticPayload::InvalidMatchPattern {
            reason,
            variant_name,
            ..
        } => invalid_match_pattern_message(*reason, *variant_name, string_table),
        DiagnosticPayload::NonExhaustiveMatch {
            reason,
            missing_variants,
            ..
        } => non_exhaustive_match_message(*reason, missing_variants, string_table),
        DiagnosticPayload::InvalidFallibleHandling { reason } => reason.message().to_owned(),
        DiagnosticPayload::InvalidTemplateSlot { reason, slot_name } => {
            invalid_template_slot_message(*reason, *slot_name, string_table)
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
        DiagnosticPayload::InvalidFallibleOperand {
            reason,
            category,
            operand_type,
        } => invalid_fallible_operand_message(*reason, *category, *operand_type, context),
        DiagnosticPayload::IncompatibleChoiceComparison { reason, lhs, rhs } => {
            incompatible_choice_comparison_message(reason, *lhs, *rhs, context)
        }
        DiagnosticPayload::InvalidCallShape {
            reason,
            callee_name,
        } => invalid_call_shape_message(reason.clone(), *callee_name, string_table),
        DiagnosticPayload::InvalidReturnShape { reason } => invalid_return_shape_message(*reason),
        DiagnosticPayload::InvalidGenericInstantiation { type_name, reason } => {
            invalid_generic_instantiation_message(*type_name, reason, context)
        }
        DiagnosticPayload::InvalidRangeOperand {
            operand,
            found_type,
        } => invalid_range_operand_message(*operand, *found_type, context),
        DiagnosticPayload::UnsupportedBuilderPackage { package_path } => {
            unsupported_builder_package_message(*package_path, string_table)
        }
        DiagnosticPayload::UnsupportedBackendFeature {
            backend_name,
            reason,
        } => format!(
            "Backend '{}' does not support {} yet.",
            string_table.resolve(*backend_name),
            reason.feature_name()
        ),
        DiagnosticPayload::InvalidPageMetadata { key, reason } => {
            invalid_page_metadata_message(*key, *reason, string_table)
        }
        DiagnosticPayload::InvalidCompileTimePath { path, reason } => {
            invalid_compile_time_path_message(path, *reason, string_table)
        }
        DiagnosticPayload::InvalidExpression { reason } => invalid_expression_message(*reason),
        DiagnosticPayload::CommonSyntaxMistake { reason } => {
            common_syntax_mistake_message(reason, string_table)
        }
        DiagnosticPayload::MissingOperatorOperand { operator, position } => {
            missing_operator_operand_message(*operator, *position, string_table)
        }
        DiagnosticPayload::InvalidStandaloneStatement { reason } => {
            invalid_standalone_statement_message(*reason)
        }
        DiagnosticPayload::ExpectedSymbolStatement => expected_symbol_statement_message(),
        DiagnosticPayload::MissingCollectionItem => missing_collection_item_message(),
        DiagnosticPayload::InvalidMatchArm { reason } => invalid_match_arm_message(*reason),
        DiagnosticPayload::InvalidLoopHeader { reason } => {
            invalid_loop_header_message(*reason, context)
        }
        DiagnosticPayload::InvalidStatementPosition { reason } => {
            invalid_statement_position_message(*reason)
        }
        DiagnosticPayload::None => String::new(),
    }
}

fn source_kind_name(source_kind: SourceFileKind) -> &'static str {
    match source_kind {
        SourceFileKind::Moth => "Moth `.moth` source",
        SourceFileKind::MothTemplate => "Moth template `.mtf` source",
        SourceFileKind::PlainMarkdown => "plain Markdown `.md` source",
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

fn import_payload_message(payload: &DiagnosticPayload, string_table: &StringTable) -> String {
    match payload {
        DiagnosticPayload::MissingImportTarget { path } => {
            format!(
                "Cannot resolve dependency '{}'.",
                path.to_portable_string(string_table)
            )
        }
        DiagnosticPayload::AmbiguousImportTarget { path } => format!(
            "Ambiguous dependency target '{}'. Use a more specific path.",
            path.to_portable_string(string_table)
        ),
        DiagnosticPayload::BareFileImport { path } => format!(
            "Bare file dependency clauses are not supported; select an exported symbol from the file '{}'.",
            path.to_portable_string(string_table)
        ),
        DiagnosticPayload::DirectSpecialFileImport { path } => {
            let special_file = special_file_name_from_path(path, string_table);
            let path_text = path.to_portable_string(string_table);
            // Guide the author toward the support package directory dependency rather than the root filename.
            let suggestion = support_root_import_suggestion(path, string_table);
            format!(
                "Cannot depend directly on '{special_file}' via '{path_text}'. Support roots are referenced through their package directory, not by filename.{suggestion}"
            )
        }
        DiagnosticPayload::ImportNameCollision { name, .. } => {
            format!(
                "Dependency binding name collision: '{}' is already visible in this file.",
                string_table.resolve(*name)
            )
        }
        DiagnosticPayload::NotExportedBySourceFile { symbol_path } => {
            format!(
                "Cannot bind '{}' because it is not exported.",
                symbol_path.to_portable_string(string_table)
            )
        }
        DiagnosticPayload::NotExportedByPublicSurface {
            requested_path,
            public_surface_name,
            public_surface_type,
        } => {
            let path_text = requested_path.to_portable_string(string_table);
            let public_surface_name = string_table.resolve(*public_surface_name);
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
        DiagnosticPayload::MissingModuleRootPublicSurface { symbol_path } => format!(
            "Cannot bind '{}' because the target module has no public export surface. Depend on a concrete file from inside the same module, or add a module root file with an `export:` block to define the module's public dependency surface.",
            symbol_path.to_portable_string(string_table)
        ),
        DiagnosticPayload::MissingPackageSymbol {
            symbol,
            package_path,
        } => format!(
            "Cannot bind '{}' from package '{}': symbol not found.",
            string_table.resolve(*symbol),
            string_table.resolve(*package_path)
        ),
        DiagnosticPayload::CrossModuleImportNotExported { symbol_path } => format!(
            "Cannot bind '{}' because it is not exported by the target module's public surface.",
            symbol_path.to_portable_string(string_table)
        ),
        DiagnosticPayload::InvalidImportPath { path, reason } => {
            invalid_import_path_message(path, *reason, string_table)
        }
        DiagnosticPayload::DirectSymbolPathImport { path } => {
            direct_symbol_path_import_message(path, string_table)
        }
        DiagnosticPayload::InvalidNamespaceDefaultName { path } => {
            invalid_namespace_default_name_message(path, string_table)
        }
        DiagnosticPayload::DuplicateImportSurfaceMember {
            surface_path,
            member_name,
        } => duplicate_import_surface_member_message(surface_path, *member_name, string_table),
        DiagnosticPayload::ExplicitMothExtension { path } => {
            explicit_moth_extension_message(path, string_table)
        }
        DiagnosticPayload::ExplicitSourceExtension { path, extension } => {
            explicit_source_extension_message(path, *extension, string_table)
        }
        DiagnosticPayload::UnsupportedSourceFileKind { path, extension } => {
            unsupported_source_file_kind_message(path, *extension, string_table)
        }
        DiagnosticPayload::InvalidSourceFileEntry { path, extension } => {
            invalid_source_file_entry_message(path, *extension, string_table)
        }
        DiagnosticPayload::InvalidMothTemplateApiScopeItem { path } => {
            invalid_moth_template_api_scope_item_message(path, string_table)
        }
        DiagnosticPayload::DuplicateMothTemplateInputPath { path, .. } => {
            duplicate_moth_template_input_path_message(path, string_table)
        }
        DiagnosticPayload::UnsupportedExternalExtension { path, extension } => {
            unsupported_external_extension_message(path, *extension, string_table)
        }
        DiagnosticPayload::InvalidExternalModule { path, message } => {
            invalid_external_module_message(path, *message, string_table)
        }
        _ => String::new(),
    }
}

fn borrow_payload_message(payload: &DiagnosticPayload, string_table: &StringTable) -> String {
    match payload {
        DiagnosticPayload::BorrowConflict {
            place,
            existing_access,
            requested_access,
            ..
        } => borrow_conflict_message(place, *existing_access, *requested_access, string_table),
        DiagnosticPayload::MultipleMutableBorrows {
            place,
            conflicting_place,
            ..
        } => multiple_mutable_borrows_message(place, conflicting_place.as_ref(), string_table),
        DiagnosticPayload::SharedMutableConflict {
            place,
            existing_access,
            requested_access,
            conflicting_place,
            ..
        } => shared_mutable_conflict_message(
            place,
            *existing_access,
            *requested_access,
            conflicting_place.as_ref(),
            string_table,
        ),
        DiagnosticPayload::UseAfterPossibleMove { place, .. } => {
            use_after_possible_move_message(place, string_table)
        }
        DiagnosticPayload::MoveWhileBorrowed {
            place,
            existing_access,
            ..
        } => move_while_borrowed_message(place, *existing_access, string_table),
        DiagnosticPayload::WholeObjectBorrowConflict {
            whole_place,
            part_place,
            ..
        } => whole_object_borrow_conflict_message(whole_place, part_place, string_table),
        DiagnosticPayload::InvalidMutableAccess {
            place,
            reason,
            conflicting_place,
            ..
        } => {
            invalid_mutable_access_message(place, *reason, conflicting_place.as_ref(), string_table)
        }
        DiagnosticPayload::UseOfUninitializedLocal { place } => {
            use_of_uninitialized_local_message(place, string_table)
        }
        _ => String::new(),
    }
}
