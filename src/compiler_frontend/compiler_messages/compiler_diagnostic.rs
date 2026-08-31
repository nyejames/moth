//! Structured compiler diagnostic record.
//!
//! WHAT: combines a diagnostic kind, severity, source labels, primary location, and typed payload.
//! WHY: frontend stages should emit facts; renderers at the boundary decide final prose.

use crate::builder_surface::SourceFileKind;
use crate::compiler_frontend::compiler_messages::source_location::SourceLocation;
use crate::compiler_frontend::compiler_messages::{
    BorrowAccessKind, BorrowDiagnosticKind, CommonSyntaxMistakeReason, ConfigDiagnosticKind,
    DeferredFeatureDiagnosticKind, DeferredFeatureReason, DependencyClauseKind, DiagnosticBag,
    DiagnosticIdentity, DiagnosticKind, DiagnosticLabel, DiagnosticLabelMessage,
    DiagnosticOperator, DiagnosticPayload, DiagnosticPlace, DiagnosticSeverity,
    GenericApplicationErrorReason, GenericInferenceSubject, ImportDiagnosticKind,
    ImportPublicSurfaceType, IncompatibleChoiceComparisonReason, InvalidCastReason,
    InvalidChoiceVariantReason, InvalidCollectionTypeReason, InvalidCompileTimePathReason,
    InvalidConfigReason, InvalidDependencyClauseReason, InvalidExpressionReason,
    InvalidFallibleOperandReason, InvalidFunctionSignatureReason, InvalidGenericParameterReason,
    InvalidImportPathReason, InvalidLoopHeaderReason, InvalidMapLiteralReason,
    InvalidMapTypeReason, InvalidMatchArmReason, InvalidMutableAccessReason,
    InvalidPageMetadataReason, InvalidSignatureMemberReason, InvalidStandaloneStatementReason,
    InvalidStatementPositionReason, InvalidStringEscapeReason, InvalidTemplateDirectiveReason,
    InvalidTemplateStructureReason, InvalidTraitConformanceReason,
    InvalidTraitIncompatibilityReason, InvalidTraitKeywordUsageReason, InvalidTypeAnnotationReason,
    LegacyDependencyClauseReason, NameNamespace, NamespaceTypeValueMisuseKind, NamingConvention,
    NumberLiteralErrorReason, OperatorOperandPosition, PathKind, RangeOperandKind,
    RuleDiagnosticKind, SyntaxDiagnosticKind, TypeAnnotationContext, TypeDiagnosticKind,
    TypeMismatchContext, UnsupportedBackendFeatureReason, UnsupportedOperatorCategory,
};
use crate::compiler_frontend::datatypes::generic_bindings::BindingConflict;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringIdRemap};
use crate::compiler_frontend::tokenizer::tokens::TokenKind;
#[derive(Clone, Debug, PartialEq)]
pub struct CompilerDiagnostic {
    pub kind: DiagnosticKind,
    pub severity: DiagnosticSeverity,
    pub primary_location: SourceLocation,
    pub labels: Vec<DiagnosticLabel>,
    pub payload: DiagnosticPayload,
}

impl CompilerDiagnostic {
    // ------------------------------------------------------------------
    //  Basic Constructors
    // ------------------------------------------------------------------

    pub(crate) fn new(
        kind: DiagnosticKind,
        primary_location: SourceLocation,
        payload: DiagnosticPayload,
    ) -> Self {
        Self::with_severity(kind, kind.default_severity(), primary_location, payload)
    }

    pub(crate) fn with_severity(
        kind: DiagnosticKind,
        severity: DiagnosticSeverity,
        primary_location: SourceLocation,
        payload: DiagnosticPayload,
    ) -> Self {
        Self {
            kind,
            severity,
            labels: vec![DiagnosticLabel::primary(primary_location.clone())],
            primary_location,
            payload,
        }
    }

    pub(crate) fn with_labels(mut self, labels: Vec<DiagnosticLabel>) -> Self {
        self.labels = labels;
        self
    }

    // ------------------------------------------------------------------
    //  Syntax Constructors
    // ------------------------------------------------------------------

    pub(crate) fn expected_token(
        expected: TokenKind,
        found: Option<TokenKind>,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::ExpectedToken),
            location,
            DiagnosticPayload::ExpectedToken { expected, found },
        )
    }

    pub(crate) fn unexpected_token(found: TokenKind, location: SourceLocation) -> Self {
        Self::new(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::UnexpectedToken),
            location,
            DiagnosticPayload::UnexpectedToken { found },
        )
    }

    pub(crate) fn unexpected_trailing_comma(location: SourceLocation) -> Self {
        Self::new(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::UnexpectedTrailingComma),
            location,
            DiagnosticPayload::UnexpectedTrailingComma,
        )
    }

    pub(crate) fn unescaped_implicit_template_close(
        source_kind: SourceFileKind,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::UnescapedImplicitTemplateClose),
            location,
            DiagnosticPayload::UnescapedImplicitTemplateClose { source_kind },
        )
    }

    // ------------------------------------------------------------------
    //  Import Constructors
    // ------------------------------------------------------------------

    pub(crate) fn missing_import_target(path: InternedPath, location: SourceLocation) -> Self {
        Self::new(
            DiagnosticKind::Import(ImportDiagnosticKind::MissingImportTarget),
            location,
            DiagnosticPayload::MissingImportTarget { path },
        )
    }

    pub(crate) fn ambiguous_import_target(path: InternedPath, location: SourceLocation) -> Self {
        Self::new(
            DiagnosticKind::Import(ImportDiagnosticKind::AmbiguousImportTarget),
            location,
            DiagnosticPayload::AmbiguousImportTarget { path },
        )
    }

    pub(crate) fn bare_file_import(path: InternedPath, location: SourceLocation) -> Self {
        Self::new(
            DiagnosticKind::Import(ImportDiagnosticKind::BareFileImport),
            location,
            DiagnosticPayload::BareFileImport { path },
        )
    }

    pub(crate) fn direct_special_file_import(path: InternedPath, location: SourceLocation) -> Self {
        Self::new(
            DiagnosticKind::Import(ImportDiagnosticKind::DirectSpecialFileImport),
            location,
            DiagnosticPayload::DirectSpecialFileImport { path },
        )
    }

    pub(crate) fn import_name_collision(
        name: StringId,
        previous_location: Option<SourceLocation>,
        location: SourceLocation,
    ) -> Self {
        let mut labels = vec![DiagnosticLabel::primary(location.clone())];
        if let Some(ref prev) = previous_location {
            labels.push(DiagnosticLabel::secondary(
                prev.clone(),
                Some(DiagnosticLabelMessage::PreviousDeclaration),
            ));
        }
        Self::new(
            DiagnosticKind::Import(ImportDiagnosticKind::ImportNameCollision),
            location,
            DiagnosticPayload::ImportNameCollision {
                name,
                previous_location,
            },
        )
        .with_labels(labels)
    }

    pub(crate) fn not_exported_by_source_file(
        symbol_path: InternedPath,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Import(ImportDiagnosticKind::NotExportedBySourceFile),
            location,
            DiagnosticPayload::NotExportedBySourceFile { symbol_path },
        )
    }

    pub(crate) fn not_exported_by_public_surface(
        requested_path: InternedPath,
        public_surface_name: StringId,
        public_surface_type: ImportPublicSurfaceType,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Import(ImportDiagnosticKind::NotExportedByPublicSurface),
            location,
            DiagnosticPayload::NotExportedByPublicSurface {
                requested_path,
                public_surface_name,
                public_surface_type,
            },
        )
    }

    pub(crate) fn missing_module_root_public_surface(
        symbol_path: InternedPath,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Import(ImportDiagnosticKind::MissingModuleRootPublicSurface),
            location,
            DiagnosticPayload::MissingModuleRootPublicSurface { symbol_path },
        )
    }

    pub(crate) fn missing_package_symbol(
        symbol: StringId,
        package_path: StringId,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Import(ImportDiagnosticKind::MissingPackageSymbol),
            location,
            DiagnosticPayload::MissingPackageSymbol {
                symbol,
                package_path,
            },
        )
    }

    pub(crate) fn cross_module_import_not_exported(
        symbol_path: InternedPath,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Import(ImportDiagnosticKind::CrossModuleImportNotExported),
            location,
            DiagnosticPayload::CrossModuleImportNotExported { symbol_path },
        )
    }

    pub(crate) fn invalid_import_path(
        path: InternedPath,
        reason: InvalidImportPathReason,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Import(ImportDiagnosticKind::InvalidImportPath),
            location,
            DiagnosticPayload::InvalidImportPath { path, reason },
        )
    }

    pub(crate) fn direct_symbol_path_import(path: InternedPath, location: SourceLocation) -> Self {
        Self::new(
            DiagnosticKind::Import(ImportDiagnosticKind::DirectSymbolPathImport),
            location,
            DiagnosticPayload::DirectSymbolPathImport { path },
        )
    }

    pub(crate) fn invalid_namespace_default_name(
        path: InternedPath,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Import(ImportDiagnosticKind::InvalidNamespaceDefaultName),
            location,
            DiagnosticPayload::InvalidNamespaceDefaultName { path },
        )
    }

    pub(crate) fn duplicate_import_surface_member(
        surface_path: InternedPath,
        member_name: StringId,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Import(ImportDiagnosticKind::DuplicateImportSurfaceMember),
            location,
            DiagnosticPayload::DuplicateImportSurfaceMember {
                surface_path,
                member_name,
            },
        )
    }

    pub(crate) fn explicit_moth_extension(path: InternedPath, location: SourceLocation) -> Self {
        Self::new(
            DiagnosticKind::Import(ImportDiagnosticKind::ExplicitMothExtension),
            location,
            DiagnosticPayload::ExplicitMothExtension { path },
        )
    }

    pub(crate) fn explicit_source_extension(
        path: InternedPath,
        extension: StringId,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Import(ImportDiagnosticKind::ExplicitSourceExtension),
            location,
            DiagnosticPayload::ExplicitSourceExtension { path, extension },
        )
    }

    pub(crate) fn unsupported_source_file_kind(
        path: InternedPath,
        extension: StringId,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Import(ImportDiagnosticKind::UnsupportedSourceFileKind),
            location,
            DiagnosticPayload::UnsupportedSourceFileKind { path, extension },
        )
    }

    pub(crate) fn invalid_source_file_entry(
        path: InternedPath,
        extension: StringId,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Import(ImportDiagnosticKind::InvalidSourceFileEntry),
            location,
            DiagnosticPayload::InvalidSourceFileEntry { path, extension },
        )
    }

    pub(crate) fn invalid_moth_template_api_scope_item(
        path: InternedPath,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Import(ImportDiagnosticKind::InvalidMothTemplateApiScopeItem),
            location,
            DiagnosticPayload::InvalidMothTemplateApiScopeItem { path },
        )
    }

    pub(crate) fn duplicate_moth_template_input_path(
        path: InternedPath,
        first_location: SourceLocation,
        duplicate_location: SourceLocation,
    ) -> Self {
        let payload_first_location = first_location.clone();
        Self::new(
            DiagnosticKind::Import(ImportDiagnosticKind::DuplicateMothTemplateInputPath),
            duplicate_location.clone(),
            DiagnosticPayload::DuplicateMothTemplateInputPath {
                path,
                first_location: payload_first_location,
            },
        )
        .with_labels(vec![
            DiagnosticLabel::primary(duplicate_location),
            DiagnosticLabel::secondary(first_location, None),
        ])
    }

    pub(crate) fn unsupported_external_extension(
        path: InternedPath,
        extension: StringId,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Import(ImportDiagnosticKind::UnsupportedExternalExtension),
            location,
            DiagnosticPayload::UnsupportedExternalExtension { path, extension },
        )
    }

    pub(crate) fn invalid_external_module(
        path: InternedPath,
        message: StringId,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Import(ImportDiagnosticKind::InvalidExternalModule),
            location,
            DiagnosticPayload::InvalidExternalModule { path, message },
        )
    }

    // ------------------------------------------------------------------
    //  Borrow Constructors
    // ------------------------------------------------------------------

    pub(crate) fn multiple_mutable_borrows(
        place: DiagnosticPlace,
        conflicting_place: Option<DiagnosticPlace>,
        existing_location: Option<SourceLocation>,
        location: SourceLocation,
    ) -> Self {
        let mut labels = vec![DiagnosticLabel::primary(location.clone())];
        if let Some(existing_location) = existing_location.clone() {
            labels.push(DiagnosticLabel::secondary(
                existing_location,
                Some(DiagnosticLabelMessage::ConflictingAccess),
            ));
        }

        Self::new(
            DiagnosticKind::Borrow(BorrowDiagnosticKind::MultipleMutableBorrows),
            location,
            DiagnosticPayload::MultipleMutableBorrows {
                place,
                conflicting_place,
                existing_location,
            },
        )
        .with_labels(labels)
    }

    pub(crate) fn shared_mutable_conflict(
        place: DiagnosticPlace,
        existing_access: BorrowAccessKind,
        requested_access: BorrowAccessKind,
        conflicting_place: Option<DiagnosticPlace>,
        existing_location: Option<SourceLocation>,
        location: SourceLocation,
    ) -> Self {
        let mut labels = vec![DiagnosticLabel::primary(location.clone())];
        if let Some(existing_location) = existing_location.clone() {
            labels.push(DiagnosticLabel::secondary(
                existing_location,
                Some(DiagnosticLabelMessage::ConflictingAccess),
            ));
        }

        Self::new(
            DiagnosticKind::Borrow(BorrowDiagnosticKind::SharedMutableConflict),
            location,
            DiagnosticPayload::SharedMutableConflict {
                place,
                existing_access,
                requested_access,
                conflicting_place,
                existing_location,
            },
        )
        .with_labels(labels)
    }

    // Reserved diagnostic constructor; the payload/renderers remain wired for move-specific
    // borrow reports.
    #[allow(dead_code)]
    pub(crate) fn use_after_possible_move(
        place: DiagnosticPlace,
        move_location: Option<SourceLocation>,
        location: SourceLocation,
    ) -> Self {
        let mut labels = vec![DiagnosticLabel::primary(location.clone())];
        if let Some(move_location) = move_location.clone() {
            labels.push(DiagnosticLabel::secondary(
                move_location,
                Some(DiagnosticLabelMessage::ValueMovedHere),
            ));
        }

        Self::new(
            DiagnosticKind::Borrow(BorrowDiagnosticKind::UseAfterPossibleMove),
            location,
            DiagnosticPayload::UseAfterPossibleMove {
                place,
                move_location,
            },
        )
        .with_labels(labels)
    }

    pub(crate) fn invalid_mutable_access(
        place: DiagnosticPlace,
        reason: InvalidMutableAccessReason,
        conflicting_place: Option<DiagnosticPlace>,
        conflicting_location: Option<SourceLocation>,
        location: SourceLocation,
    ) -> Self {
        let mut labels = vec![DiagnosticLabel::primary(location.clone())];
        if let Some(conflicting_location) = conflicting_location.clone() {
            labels.push(DiagnosticLabel::secondary(
                conflicting_location,
                Some(DiagnosticLabelMessage::ConflictingAccess),
            ));
        }

        Self::new(
            DiagnosticKind::Borrow(BorrowDiagnosticKind::InvalidMutableAccess),
            location,
            DiagnosticPayload::InvalidMutableAccess {
                place,
                reason,
                conflicting_place,
                conflicting_location,
            },
        )
        .with_labels(labels)
    }

    pub(crate) fn use_of_uninitialized_local(
        place: DiagnosticPlace,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Borrow(BorrowDiagnosticKind::UseOfUninitializedLocal),
            location,
            DiagnosticPayload::UseOfUninitializedLocal { place },
        )
    }

    // ------------------------------------------------------------------
    //  Rule Constructors
    // ------------------------------------------------------------------

    pub(crate) fn duplicate_declaration(
        name: StringId,
        first_location: Option<SourceLocation>,
        duplicate_location: SourceLocation,
    ) -> Self {
        // Prelude-injected symbols have no authored previous location. Omit the secondary
        // label in that case so no fabricated empty location enters a user-facing label.
        let mut labels = vec![DiagnosticLabel::primary(duplicate_location.clone())];
        if let Some(location) = &first_location {
            labels.push(DiagnosticLabel::secondary(
                location.clone(),
                Some(DiagnosticLabelMessage::PreviousDeclaration),
            ));
        }
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::DuplicateDeclaration),
            duplicate_location,
            DiagnosticPayload::DuplicateDeclaration {
                name,
                first_location,
            },
        )
        .with_labels(labels)
    }

    pub(crate) fn invalid_compile_time_path(
        path: InternedPath,
        reason: InvalidCompileTimePathReason,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidCompileTimePath),
            location,
            DiagnosticPayload::InvalidCompileTimePath { path, reason },
        )
    }

    pub(crate) fn dependency_namespace_used_as_value(
        record_name: StringId,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::DependencyNamespaceUsedAsValue),
            location,
            DiagnosticPayload::DependencyNamespaceUsedAsValue { record_name },
        )
    }

    pub(crate) fn const_record_used_as_value(
        record_name: StringId,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::ConstRecordUsedAsValue),
            location,
            DiagnosticPayload::ConstRecordUsedAsValue { record_name },
        )
    }

    pub(crate) fn nested_dependency_traversal(
        record_name: StringId,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::NestedDependencyTraversal),
            location,
            DiagnosticPayload::NestedDependencyTraversal { record_name },
        )
    }

    pub(crate) fn namespace_type_value_misuse(
        name: StringId,
        expected: NamespaceTypeValueMisuseKind,
        found: NamespaceTypeValueMisuseKind,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::NamespaceTypeValueMisuse),
            location,
            DiagnosticPayload::NamespaceTypeValueMisuse {
                name,
                expected,
                found,
            },
        )
    }

    pub(crate) fn unsupported_external_function(
        function_name: StringId,
        package_path: Option<StringId>,
        backend_name: StringId,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::UnsupportedExternalFunction),
            location,
            DiagnosticPayload::UnsupportedExternalFunction {
                function_name,
                package_path,
                backend_name,
            },
        )
    }

    pub(crate) fn invalid_range_operand(
        operand: RangeOperandKind,
        found_type: TypeId,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidRangeOperand),
            location,
            DiagnosticPayload::InvalidRangeOperand {
                operand,
                found_type,
            },
        )
    }

    pub(crate) fn unsupported_builder_package(
        package_path: StringId,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::UnsupportedBuilderPackage),
            location,
            DiagnosticPayload::UnsupportedBuilderPackage { package_path },
        )
    }

    pub(crate) fn unsupported_backend_feature(
        backend_name: StringId,
        reason: UnsupportedBackendFeatureReason,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::UnsupportedBackendFeature),
            location,
            DiagnosticPayload::UnsupportedBackendFeature {
                backend_name,
                reason,
            },
        )
    }

    pub(crate) fn invalid_page_metadata(
        key: StringId,
        reason: InvalidPageMetadataReason,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidPageMetadata),
            location,
            DiagnosticPayload::InvalidPageMetadata { key, reason },
        )
    }

    // ------------------------------------------------------------------
    //  Config Constructors
    // ------------------------------------------------------------------

    pub(crate) fn invalid_config_reason(
        key: Option<StringId>,
        reason: InvalidConfigReason,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Config(ConfigDiagnosticKind::InvalidConfig),
            location,
            DiagnosticPayload::InvalidConfig { key, reason },
        )
    }

    pub(crate) fn deferred_feature(feature: StringId, location: SourceLocation) -> Self {
        Self::deferred_feature_reason(DeferredFeatureReason::NamedFeature { feature }, location)
    }

    pub(crate) fn deferred_feature_reason(
        reason: DeferredFeatureReason,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::DeferredFeature(DeferredFeatureDiagnosticKind::DeferredFeature),
            location,
            DiagnosticPayload::DeferredFeature { reason },
        )
    }

    // ------------------------------------------------------------------
    //  Warning Constructors
    // ------------------------------------------------------------------

    pub(crate) fn identifier_naming_convention(
        name: StringId,
        expected_style: NamingConvention,
        location: SourceLocation,
    ) -> Self {
        Self::with_severity(
            DiagnosticKind::Rule(RuleDiagnosticKind::IdentifierNamingConvention),
            DiagnosticSeverity::Warning,
            location,
            DiagnosticPayload::IdentifierNamingConvention {
                name,
                expected_style,
            },
        )
    }

    pub(crate) fn unreachable_match_arm(location: SourceLocation) -> Self {
        Self::with_severity(
            DiagnosticKind::Rule(RuleDiagnosticKind::UnreachableMatchArm),
            DiagnosticSeverity::Warning,
            location,
            DiagnosticPayload::UnreachableMatchArm,
        )
    }

    pub(crate) fn dependency_alias_case_mismatch(
        alias: StringId,
        symbol: StringId,
        location: SourceLocation,
    ) -> Self {
        Self::with_severity(
            DiagnosticKind::Import(ImportDiagnosticKind::DependencyAliasCaseMismatch),
            DiagnosticSeverity::Warning,
            location,
            DiagnosticPayload::DependencyAliasCaseMismatch { alias, symbol },
        )
    }

    pub(crate) fn malformed_css_template(message: StringId, location: SourceLocation) -> Self {
        Self::with_severity(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::MalformedCssTemplate),
            DiagnosticSeverity::Warning,
            location,
            DiagnosticPayload::MalformedTemplate { message },
        )
    }

    pub(crate) fn malformed_html_template(message: StringId, location: SourceLocation) -> Self {
        Self::with_severity(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::MalformedHtmlTemplate),
            DiagnosticSeverity::Warning,
            location,
            DiagnosticPayload::MalformedTemplate { message },
        )
    }

    // ------------------------------------------------------------------
    //  Syntax Constructors (Continued)
    // ------------------------------------------------------------------

    pub(crate) fn invalid_character(character: char, location: SourceLocation) -> Self {
        Self::new(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::InvalidCharacter),
            location,
            DiagnosticPayload::InvalidCharacter { character },
        )
    }

    pub(crate) fn invalid_number_literal(
        literal_text: StringId,
        reason: NumberLiteralErrorReason,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::InvalidNumberLiteral),
            location,
            DiagnosticPayload::InvalidNumberLiteral {
                literal_text,
                reason,
            },
        )
    }

    pub(crate) fn invalid_style_directive(
        directive_name: StringId,
        supported_directives: StringId,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::InvalidStyleDirective),
            location,
            DiagnosticPayload::InvalidStyleDirective {
                directive_name,
                supported_directives,
            },
        )
    }

    pub(crate) fn missing_closing_delimiter(
        expected_delimiter: StringId,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::MissingClosingDelimiter),
            location,
            DiagnosticPayload::MissingClosingDelimiter { expected_delimiter },
        )
    }

    pub(crate) fn invalid_generic_application(
        reason: GenericApplicationErrorReason,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::InvalidGenericApplication),
            location,
            DiagnosticPayload::InvalidGenericApplication { reason },
        )
    }

    pub(crate) fn unexpected_end_of_file(
        expected_delimiter: Option<StringId>,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::UnexpectedEndOfFile),
            location,
            DiagnosticPayload::UnexpectedEndOfFile { expected_delimiter },
        )
    }

    pub(crate) fn invalid_path(path_kind: PathKind, location: SourceLocation) -> Self {
        Self::new(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::InvalidPath),
            location,
            DiagnosticPayload::InvalidPath { path_kind },
        )
    }

    pub(crate) fn invalid_dependency_clause(
        clause_kind: DependencyClauseKind,
        reason: InvalidDependencyClauseReason,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::InvalidDependencyClause),
            location,
            DiagnosticPayload::InvalidDependencyClause {
                clause_kind,
                reason,
            },
        )
    }

    pub(crate) fn legacy_dependency_clause(
        replacement: Option<StringId>,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::LegacyDependencyClause),
            location,
            DiagnosticPayload::LegacyDependencyClause {
                reason: LegacyDependencyClauseReason::ImportKeyword,
                replacement,
            },
        )
    }

    pub(crate) fn unterminated_string_literal(location: SourceLocation) -> Self {
        Self::new(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::UnterminatedStringLiteral),
            location,
            DiagnosticPayload::None,
        )
    }

    pub(crate) fn invalid_string_escape(
        reason: InvalidStringEscapeReason,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::InvalidStringEscape),
            location,
            DiagnosticPayload::InvalidStringEscape { reason },
        )
    }

    pub(crate) fn invalid_char_literal(location: SourceLocation) -> Self {
        Self::new(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::InvalidCharLiteral),
            location,
            DiagnosticPayload::None,
        )
    }

    pub(crate) fn unexpected_token_in_declaration(location: SourceLocation) -> Self {
        Self::new(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::UnexpectedTokenInDeclaration),
            location,
            DiagnosticPayload::None,
        )
    }

    pub(crate) fn invalid_identifier(location: SourceLocation) -> Self {
        Self::new(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::InvalidIdentifier),
            location,
            DiagnosticPayload::None,
        )
    }

    pub(crate) fn invalid_type_annotation(
        context: TypeAnnotationContext,
        reason: InvalidTypeAnnotationReason,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::InvalidTypeAnnotation),
            location,
            DiagnosticPayload::InvalidTypeAnnotation { context, reason },
        )
    }

    pub(crate) fn invalid_collection_type(
        reason: InvalidCollectionTypeReason,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::InvalidCollectionType),
            location,
            DiagnosticPayload::InvalidCollectionType { reason },
        )
    }

    pub(crate) fn invalid_map_type(reason: InvalidMapTypeReason, location: SourceLocation) -> Self {
        Self::new(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::InvalidMapType),
            location,
            DiagnosticPayload::InvalidMapType { reason },
        )
    }

    pub(crate) fn invalid_map_literal(
        reason: InvalidMapLiteralReason,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::InvalidMapLiteral),
            location,
            DiagnosticPayload::InvalidMapLiteral { reason },
        )
    }

    pub(crate) fn invalid_generic_parameter(
        reason: InvalidGenericParameterReason,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::InvalidGenericParameter),
            location,
            DiagnosticPayload::InvalidGenericParameter { reason },
        )
    }

    pub(crate) fn invalid_template_directive(
        directive_name: Option<StringId>,
        reason: InvalidTemplateDirectiveReason,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::InvalidTemplateDirective),
            location,
            DiagnosticPayload::InvalidTemplateDirective {
                directive_name,
                reason,
            },
        )
    }

    pub(crate) fn invalid_template_structure(
        reason: InvalidTemplateStructureReason,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::InvalidTemplateStructure),
            location,
            DiagnosticPayload::InvalidTemplateStructure { reason },
        )
    }

    pub(crate) fn invalid_expression(
        reason: InvalidExpressionReason,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::InvalidExpression),
            location,
            DiagnosticPayload::InvalidExpression { reason },
        )
    }

    pub(crate) fn missing_operator_operand(
        operator: StringId,
        position: OperatorOperandPosition,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::MissingOperatorOperand),
            location,
            DiagnosticPayload::MissingOperatorOperand { operator, position },
        )
    }

    pub(crate) fn invalid_standalone_statement(
        reason: InvalidStandaloneStatementReason,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::InvalidStandaloneStatement),
            location,
            DiagnosticPayload::InvalidStandaloneStatement { reason },
        )
    }

    pub(crate) fn expected_symbol_statement(location: SourceLocation) -> Self {
        Self::new(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::ExpectedSymbolStatement),
            location,
            DiagnosticPayload::ExpectedSymbolStatement,
        )
    }

    pub(crate) fn missing_collection_item(location: SourceLocation) -> Self {
        Self::new(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::MissingCollectionItem),
            location,
            DiagnosticPayload::MissingCollectionItem,
        )
    }

    pub(crate) fn invalid_match_arm(
        reason: InvalidMatchArmReason,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::InvalidMatchArm),
            location,
            DiagnosticPayload::InvalidMatchArm { reason },
        )
    }

    pub(crate) fn invalid_loop_header(
        reason: InvalidLoopHeaderReason,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::InvalidLoopHeader),
            location,
            DiagnosticPayload::InvalidLoopHeader { reason },
        )
    }

    pub(crate) fn invalid_statement_position(
        reason: InvalidStatementPositionReason,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::InvalidStatementPosition),
            location,
            DiagnosticPayload::InvalidStatementPosition { reason },
        )
    }

    pub(crate) fn common_syntax_mistake(
        reason: CommonSyntaxMistakeReason,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::CommonSyntaxMistake),
            location,
            DiagnosticPayload::CommonSyntaxMistake { reason },
        )
    }

    // ------------------------------------------------------------------
    //  Rule Constructors (Continued)
    // ------------------------------------------------------------------

    pub(crate) fn invalid_signature_member(
        reason: InvalidSignatureMemberReason,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidSignatureMember),
            location,
            DiagnosticPayload::InvalidSignatureMember { reason },
        )
    }

    pub(crate) fn invalid_function_signature(
        reason: InvalidFunctionSignatureReason,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidFunctionSignature),
            location,
            DiagnosticPayload::InvalidFunctionSignature { reason },
        )
    }

    pub(crate) fn invalid_choice_variant(
        reason: InvalidChoiceVariantReason,
        choice_name: Option<StringId>,
        variant_name: Option<StringId>,
        available_variants: Vec<StringId>,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidChoiceVariant),
            location,
            DiagnosticPayload::InvalidChoiceVariant {
                reason,
                choice_name,
                variant_name,
                available_variants,
            },
        )
    }

    pub(crate) fn invalid_struct_default_value(location: SourceLocation) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidStructDefaultValue),
            location,
            DiagnosticPayload::InvalidStructDefaultValue,
        )
    }

    pub(crate) fn missing_declaration_initializer(
        name: StringId,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::MissingDeclarationInitializer),
            location,
            DiagnosticPayload::MissingDeclarationInitializer { name },
        )
    }

    pub(crate) fn circular_dependency(path: InternedPath, location: SourceLocation) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::CircularDependency),
            location,
            DiagnosticPayload::CircularDependency { path },
        )
    }

    pub(crate) fn unknown_value_name(name: StringId, location: SourceLocation) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::UnknownValueName),
            location,
            DiagnosticPayload::UnknownName {
                name,
                namespace: NameNamespace::Value,
            },
        )
    }

    pub(crate) fn unknown_type_name(name: StringId, location: SourceLocation) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::UnknownTypeName),
            location,
            DiagnosticPayload::UnknownName {
                name,
                namespace: NameNamespace::Type,
            },
        )
    }

    pub(crate) fn unknown_trait_name(name: StringId, location: SourceLocation) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::UnknownTrait),
            location,
            DiagnosticPayload::UnknownTrait { name },
        )
    }

    pub(crate) fn duplicate_trait_requirement(
        trait_name: StringId,
        requirement_name: StringId,
        first_location: SourceLocation,
        duplicate_location: SourceLocation,
    ) -> Self {
        let payload_first_location = first_location.clone();
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::DuplicateTraitRequirement),
            duplicate_location.clone(),
            DiagnosticPayload::DuplicateTraitRequirement {
                trait_name,
                requirement_name,
                first_location: payload_first_location,
            },
        )
        .with_labels(vec![
            DiagnosticLabel::primary(duplicate_location),
            DiagnosticLabel::secondary(
                first_location,
                Some(DiagnosticLabelMessage::PreviousDeclaration),
            ),
        ])
    }

    pub(crate) fn trait_private_surface_leak(
        trait_name: StringId,
        surface_type: TypeId,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::TraitPrivateSurfaceLeak),
            location,
            DiagnosticPayload::TraitPrivateSurfaceLeak {
                trait_name,
                surface_type,
            },
        )
    }

    pub(crate) fn generic_bound_private_surface_leak(
        function_name: StringId,
        trait_name: StringId,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::GenericBoundPrivateSurfaceLeak),
            location,
            DiagnosticPayload::GenericBoundPrivateSurfaceLeak {
                function_name,
                trait_name,
            },
        )
    }

    pub(crate) fn unsupported_trait_feature(
        trait_name: StringId,
        feature: StringId,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::UnsupportedTraitFeature),
            location,
            DiagnosticPayload::UnsupportedTraitFeature {
                trait_name,
                feature,
            },
        )
    }

    pub(crate) fn invalid_trait_keyword_usage(
        reason: InvalidTraitKeywordUsageReason,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidTraitKeywordUsage),
            location,
            DiagnosticPayload::InvalidTraitKeywordUsage { reason },
        )
    }

    pub(crate) fn export_outside_module_root(location: SourceLocation) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::ExportOutsideModuleRoot),
            location,
            DiagnosticPayload::None,
        )
    }

    pub(crate) fn invalid_export_target(location: SourceLocation) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidExportTarget),
            location,
            DiagnosticPayload::None,
        )
    }

    pub(crate) fn duplicate_public_export(
        name: StringId,
        first_location: SourceLocation,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::DuplicatePublicExport),
            location.clone(),
            DiagnosticPayload::DuplicatePublicExport {
                name,
                first_location: first_location.clone(),
            },
        )
        .with_labels(vec![
            DiagnosticLabel::primary(location),
            DiagnosticLabel::secondary(first_location, None),
        ])
    }

    pub(crate) fn duplicate_export_block(location: SourceLocation) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::DuplicateExportBlock),
            location,
            DiagnosticPayload::None,
        )
    }

    pub(crate) fn private_type_in_exported_api(
        exported_name: StringId,
        private_type: TypeId,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::PrivateTypeInExportedApi),
            location,
            DiagnosticPayload::PrivateTypeInExportedApi {
                exported_name,
                private_type,
            },
        )
    }

    pub(crate) fn invalid_trait_conformance(
        target_name: StringId,
        trait_name: Option<StringId>,
        reason: InvalidTraitConformanceReason,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidTraitConformance),
            location,
            DiagnosticPayload::InvalidTraitConformance {
                target_name,
                trait_name,
                reason,
            },
        )
    }

    pub(crate) fn invalid_trait_incompatibility(
        subject_name: StringId,
        incompatible_trait_name: Option<StringId>,
        reason: InvalidTraitIncompatibilityReason,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidTraitIncompatibility),
            location,
            DiagnosticPayload::InvalidTraitIncompatibility {
                subject_name,
                incompatible_trait_name,
                reason,
            },
        )
    }

    pub(crate) fn trait_name_used_as_type(trait_name: StringId, location: SourceLocation) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::TraitNameUsedAsType),
            location,
            DiagnosticPayload::TraitNameUsedAsType { trait_name },
        )
    }

    pub(crate) fn namespace_misuse(
        name: StringId,
        expected: NameNamespace,
        found: NameNamespace,
        location: SourceLocation,
    ) -> Self {
        let kind = match (expected, found) {
            (NameNamespace::Type, NameNamespace::Value) => {
                DiagnosticKind::Rule(RuleDiagnosticKind::ValueUsedAsType)
            }
            (NameNamespace::Value, NameNamespace::Type) => {
                DiagnosticKind::Rule(RuleDiagnosticKind::TypeUsedAsValue)
            }
            _ => DiagnosticKind::Rule(RuleDiagnosticKind::UnknownName),
        };
        Self::new(
            kind,
            location,
            DiagnosticPayload::NamespaceMisuse {
                name,
                expected,
                found,
            },
        )
    }

    pub(crate) fn shadowed_name(
        name: StringId,
        first_location: SourceLocation,
        duplicate_location: SourceLocation,
    ) -> Self {
        let payload_first_location = first_location.clone();
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::ShadowedName),
            duplicate_location.clone(),
            DiagnosticPayload::ShadowedName {
                name,
                first_location: payload_first_location,
            },
        )
        .with_labels(vec![
            DiagnosticLabel::primary(duplicate_location),
            DiagnosticLabel::secondary(
                first_location,
                Some(DiagnosticLabelMessage::PreviousDeclaration),
            ),
        ])
    }

    pub(crate) fn reserved_name_collision(
        name: StringId,
        reserved_by: crate::compiler_frontend::compiler_messages::ReservedNameOwner,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::ReservedNameCollision),
            location,
            DiagnosticPayload::ReservedNameCollision { name, reserved_by },
        )
    }

    pub(crate) fn invalid_this_usage(
        reason: crate::compiler_frontend::compiler_messages::InvalidThisUsageReason,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidThisUsage),
            location,
            DiagnosticPayload::InvalidThisUsage { reason },
        )
    }

    pub(crate) fn invalid_receiver_declaration(
        reason: crate::compiler_frontend::compiler_messages::InvalidReceiverDeclarationReason,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidReceiverDeclaration),
            location,
            DiagnosticPayload::InvalidReceiverDeclaration { reason },
        )
    }

    pub(crate) fn invalid_top_level_runtime_statement(location: SourceLocation) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidTopLevelRuntimeStatement),
            location,
            DiagnosticPayload::None,
        )
    }

    pub(crate) fn reserved_builtin_name(name: StringId, location: SourceLocation) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::ReservedBuiltinName),
            location,
            DiagnosticPayload::UnusedName { name },
        )
    }

    pub(crate) fn invalid_control_flow_statement(
        reason: crate::compiler_frontend::compiler_messages::InvalidControlFlowStatementReason,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidControlFlowStatement),
            location,
            DiagnosticPayload::InvalidControlFlowStatement { reason },
        )
    }

    pub(crate) fn invalid_declaration(
        reason: crate::compiler_frontend::compiler_messages::InvalidDeclarationReason,
        name: Option<StringId>,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidDeclaration),
            location,
            DiagnosticPayload::InvalidDeclaration { reason, name },
        )
    }

    pub(crate) fn invalid_generic_instantiation(
        type_name: Option<StringId>,
        reason: crate::compiler_frontend::compiler_messages::InvalidGenericInstantiationReason,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidGenericInstantiation),
            location,
            DiagnosticPayload::InvalidGenericInstantiation { type_name, reason },
        )
    }

    pub(crate) fn conflicting_generic_inference(
        type_name: Option<StringId>,
        subject: GenericInferenceSubject,
        conflict: BindingConflict,
        parameter_name: StringId,
        current_evidence_location: SourceLocation,
        previous_evidence_location: Option<SourceLocation>,
    ) -> Self {
        let mut diagnostic = Self::invalid_generic_instantiation(
            type_name,
            crate::compiler_frontend::compiler_messages::InvalidGenericInstantiationReason::ConflictingInference {
                subject,
                parameter_id: conflict.parameter_id,
                parameter_name,
                existing_type_id: conflict.existing_type_id,
                replacement_type_id: conflict.replacement_type_id,
                current_evidence_location: current_evidence_location.clone(),
                previous_evidence_location: previous_evidence_location.clone(),
            },
            current_evidence_location.clone(),
        );

        if let Some(previous_evidence_location) = previous_evidence_location {
            diagnostic = diagnostic.with_labels(vec![
                DiagnosticLabel::primary(current_evidence_location),
                DiagnosticLabel::secondary(
                    previous_evidence_location,
                    Some(DiagnosticLabelMessage::GenericInferencePreviousEvidence),
                ),
            ]);
        }

        diagnostic
    }

    pub(crate) fn invalid_assignment_target(
        reason: crate::compiler_frontend::compiler_messages::InvalidAssignmentTargetReason,
        target_name: Option<StringId>,
        target_type: Option<crate::compiler_frontend::datatypes::ids::TypeId>,
        field_name: Option<StringId>,
        root_binding_name: Option<StringId>,
        declaration_location: Option<SourceLocation>,
        location: SourceLocation,
    ) -> Self {
        let mut labels = vec![DiagnosticLabel::primary(location.clone())];
        if let Some(ref declaration_location) = declaration_location {
            labels.push(DiagnosticLabel::secondary(
                declaration_location.clone(),
                Some(DiagnosticLabelMessage::ImmutableBindingDeclaration),
            ));
        }
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidAssignmentTarget),
            location,
            DiagnosticPayload::InvalidAssignmentTarget {
                reason,
                target_name,
                target_type,
                field_name,
                root_binding_name,
                declaration_location,
            },
        )
        .with_labels(labels)
    }

    pub(crate) fn invalid_multi_bind(
        reason: crate::compiler_frontend::compiler_messages::InvalidMultiBindReason,
        target_name: Option<StringId>,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidMultiBind),
            location,
            DiagnosticPayload::InvalidMultiBind {
                reason,
                target_name,
            },
        )
    }

    pub(crate) fn invalid_multi_bind_syntax(
        reason: crate::compiler_frontend::compiler_messages::InvalidMultiBindReason,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::UnexpectedToken),
            location,
            DiagnosticPayload::InvalidMultiBind {
                reason,
                target_name: None,
            },
        )
    }

    pub(crate) fn invalid_builtin_call(
        reason: crate::compiler_frontend::compiler_messages::InvalidBuiltinCallReason,
        builtin_name: Option<StringId>,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidBuiltinCall),
            location,
            DiagnosticPayload::InvalidBuiltinCall {
                reason,
                builtin_name,
            },
        )
    }

    pub(crate) fn invalid_cast(
        reason: InvalidCastReason,
        source_type: Option<crate::compiler_frontend::datatypes::ids::TypeId>,
        target_type: Option<crate::compiler_frontend::datatypes::ids::TypeId>,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidCast),
            location,
            DiagnosticPayload::InvalidCast {
                reason,
                source_type,
                target_type,
            },
        )
    }

    pub(crate) fn invalid_receiver_call(
        reason: crate::compiler_frontend::compiler_messages::InvalidReceiverCallReason,
        receiver_type: Option<StringId>,
        method_name: Option<StringId>,
        receiver_kind: Option<crate::compiler_frontend::compiler_messages::ReceiverCallKind>,
        receiver_binding_name: Option<StringId>,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidReceiverCall),
            location,
            DiagnosticPayload::InvalidReceiverCall {
                reason,
                receiver_type,
                method_name,
                receiver_kind,
                receiver_binding_name,
            },
        )
    }

    pub(crate) fn invalid_copy_target(
        reason: crate::compiler_frontend::compiler_messages::InvalidCopyTargetReason,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidCopyTarget),
            location,
            DiagnosticPayload::InvalidCopyTarget { reason },
        )
    }

    pub(crate) fn invalid_field_access(
        reason: crate::compiler_frontend::compiler_messages::InvalidFieldAccessReason,
        field_name: Option<StringId>,
        receiver_type: Option<crate::compiler_frontend::datatypes::ids::TypeId>,
        known_fields: Vec<StringId>,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidFieldAccess),
            location,
            DiagnosticPayload::InvalidFieldAccess {
                reason,
                field_name,
                receiver_type,
                known_fields,
            },
        )
    }

    pub(crate) fn invalid_match_pattern(
        reason: crate::compiler_frontend::compiler_messages::InvalidMatchPatternReason,
        variant_name: Option<StringId>,
        scrutinee_name: Option<StringId>,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidMatchPattern),
            location,
            DiagnosticPayload::InvalidMatchPattern {
                reason,
                variant_name,
                scrutinee_name,
            },
        )
    }

    pub(crate) fn non_exhaustive_match(
        reason: crate::compiler_frontend::compiler_messages::NonExhaustiveMatchReason,
        missing_variants: Vec<StringId>,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::NonExhaustiveMatch),
            location,
            DiagnosticPayload::NonExhaustiveMatch {
                reason,
                missing_variants,
            },
        )
    }

    pub(crate) fn invalid_fallible_handling(
        reason: crate::compiler_frontend::compiler_messages::InvalidFallibleHandlingReason,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidFallibleHandling),
            location,
            DiagnosticPayload::InvalidFallibleHandling { reason },
        )
    }

    pub(crate) fn invalid_template_slot(
        reason: crate::compiler_frontend::compiler_messages::InvalidTemplateSlotReason,
        slot_name: Option<StringId>,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidTemplateSlot),
            location,
            DiagnosticPayload::InvalidTemplateSlot { reason, slot_name },
        )
    }

    pub(crate) fn compile_time_evaluation_error(
        reason: crate::compiler_frontend::compiler_messages::CompileTimeEvaluationErrorReason,
        operation: Option<StringId>,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::CompileTimeEvaluationError),
            location,
            DiagnosticPayload::CompileTimeEvaluationError { reason, operation },
        )
    }

    pub(crate) fn empty_collection_type_ambiguity(location: SourceLocation) -> Self {
        Self::new(
            DiagnosticKind::Type(TypeDiagnosticKind::EmptyCollectionTypeAmbiguity),
            location,
            DiagnosticPayload::EmptyCollectionTypeAmbiguity,
        )
    }

    pub(crate) fn unsupported_operator_types(
        operator: DiagnosticOperator,
        lhs: TypeId,
        rhs: Option<TypeId>,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Type(TypeDiagnosticKind::UnsupportedOperatorTypes),
            location,
            DiagnosticPayload::UnsupportedOperatorTypes { operator, lhs, rhs },
        )
    }

    pub(crate) fn invalid_fallible_operand(
        reason: InvalidFallibleOperandReason,
        category: UnsupportedOperatorCategory,
        operand_type: TypeId,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Type(TypeDiagnosticKind::InvalidFallibleOperand),
            location,
            DiagnosticPayload::InvalidFallibleOperand {
                reason,
                category,
                operand_type,
            },
        )
    }

    pub(crate) fn incompatible_choice_comparison(
        reason: IncompatibleChoiceComparisonReason,
        lhs: TypeId,
        rhs: TypeId,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Type(TypeDiagnosticKind::IncompatibleChoiceComparison),
            location,
            DiagnosticPayload::IncompatibleChoiceComparison { reason, lhs, rhs },
        )
    }

    pub(crate) fn invalid_call_shape(
        reason: crate::compiler_frontend::compiler_messages::InvalidCallShapeReason,
        callee_name: Option<StringId>,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidCallShape),
            location,
            DiagnosticPayload::InvalidCallShape {
                reason,
                callee_name,
            },
        )
    }

    pub(crate) fn invalid_return_shape(
        reason: crate::compiler_frontend::compiler_messages::InvalidReturnShapeReason,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidReturnShape),
            location,
            DiagnosticPayload::InvalidReturnShape { reason },
        )
    }

    // ------------------------------------------------------------------
    //  Type Constructors
    // ------------------------------------------------------------------

    pub(crate) fn type_mismatch(
        expected: TypeId,
        found: TypeId,
        context: TypeMismatchContext,
        location: SourceLocation,
    ) -> Self {
        Self::new(
            DiagnosticKind::Type(TypeDiagnosticKind::TypeMismatch),
            location,
            DiagnosticPayload::TypeMismatch {
                expected,
                found,
                context,
            },
        )
    }

    // ------------------------------------------------------------------
    //  Supporting Methods
    // ------------------------------------------------------------------

    /// Return the compiler-owned identity used by tests and tooling.
    ///
    /// The descriptor remains the sole code authority and `severity` is the actual severity on
    /// this diagnostic, including deliberate overrides. The payload owns the optional reason key.
    pub(crate) fn identity(&self) -> DiagnosticIdentity {
        DiagnosticIdentity::new(self.kind.descriptor().code, self.severity, &self.payload)
    }

    pub(crate) fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        self.primary_location.remap_string_ids(remap);

        for label in &mut self.labels {
            label.remap_string_ids(remap);
        }

        self.payload.remap_string_ids(remap);
    }

    pub(crate) fn rebind_source_identity(&mut self, logical_path: &InternedPath) {
        self.primary_location.rebind_source_identity(logical_path);

        for label in &mut self.labels {
            label.rebind_source_identity(logical_path);
        }

        self.payload.rebind_source_identity(logical_path);
    }
}

impl From<DiagnosticBag> for CompilerDiagnostic {
    fn from(bag: DiagnosticBag) -> Self {
        // Header dispatch still has one narrow boundary that collapses a local diagnostic bag
        // into the single-diagnostic API it inherited. An empty bag here means a parser helper
        // returned an error without a diagnostic, which is an internal compiler bug.
        bag.into_diagnostics()
            .into_iter()
            .next()
            .expect("DiagnosticBag conversion requires at least one diagnostic")
    }
}

impl From<crate::compiler_frontend::compiler_errors::CompilerError> for CompilerDiagnostic {
    fn from(error: crate::compiler_frontend::compiler_errors::CompilerError) -> Self {
        crate::compiler_frontend::compiler_errors::compiler_error_to_diagnostic(&error)
    }
}
