//! Structured compiler diagnostic record.
//!
//! WHAT: combines a diagnostic kind, severity, exact source spans, and typed payload.
//! WHY: frontend stages should emit facts; renderers at the boundary decide final prose.

use crate::builder_surface::SourceFileKind;
use crate::compiler_frontend::compiler_errors::{CompilerError, ErrorType};
use crate::compiler_frontend::compiler_messages::{
    BorrowAccessKind, BorrowDiagnosticKind, CommonSyntaxMistakeReason, ConfigDiagnosticKind,
    DeferredFeatureDiagnosticKind, DeferredFeatureReason, DependencyClauseKind, DiagnosticBag,
    DiagnosticIdentity, DiagnosticKind, DiagnosticLabel, DiagnosticLabelMessage,
    DiagnosticOperator, DiagnosticPayload, DiagnosticPlace, DiagnosticSeverity, DiagnosticToken,
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
    NumberLiteralErrorReason, OperatorOperandPosition, PathKind, ProjectContextEscapeReason,
    RangeOperandKind, RuleDiagnosticKind, SourceSpanCapacityResource, SyntaxDiagnosticKind,
    TypeAnnotationContext, TypeDiagnosticKind, TypeMismatchContext,
    UnsupportedBackendFeatureReason, UnsupportedOperatorCategory,
};
use crate::compiler_frontend::datatypes::generic_bindings::BindingConflict;
use crate::compiler_frontend::datatypes::ids::TypeId;
use crate::compiler_frontend::source::{
    FrozenIdentityHandle, SourceId, SourceSpan, SpanCapacityError, SpanCapacityReason,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringIdRemap};
use crate::compiler_frontend::tokenizer::tokens::TokenKind;
#[derive(Clone, Debug, PartialEq)]
pub struct CompilerDiagnostic {
    pub kind: DiagnosticKind,
    pub severity: DiagnosticSeverity,
    pub(crate) primary_span: Option<SourceSpan>,
    /// Owner for a primary span that belongs to a materialised donor context.
    pub(crate) primary_frozen_identity_handle: Option<FrozenIdentityHandle>,
    pub labels: Vec<DiagnosticLabel>,
    pub payload: DiagnosticPayload,
}

const _: () = assert!(std::mem::size_of::<CompilerDiagnostic>() <= 128);

impl CompilerDiagnostic {
    // ------------------------------------------------------------------
    //  Basic Constructors
    // ------------------------------------------------------------------

    pub(crate) fn new(
        kind: DiagnosticKind,
        primary_span: Option<SourceSpan>,
        payload: DiagnosticPayload,
    ) -> Self {
        Self::with_severity(kind, kind.default_severity(), primary_span, payload)
    }

    pub(crate) fn with_severity(
        kind: DiagnosticKind,
        severity: DiagnosticSeverity,
        primary_span: Option<SourceSpan>,
        payload: DiagnosticPayload,
    ) -> Self {
        Self {
            kind,
            severity,
            primary_span,
            primary_frozen_identity_handle: None,
            labels: Vec::new(),
            payload,
        }
    }

    pub(crate) fn with_labels(mut self, labels: Vec<DiagnosticLabel>) -> Self {
        self.labels = labels;
        self
    }

    pub(crate) fn set_primary_frozen_identity_handle_if_missing(
        &mut self,
        frozen_identity_handle: FrozenIdentityHandle,
    ) {
        if self.primary_frozen_identity_handle.is_none() {
            self.primary_frozen_identity_handle = Some(frozen_identity_handle);
        }
    }
    pub(crate) fn with_primary_frozen_identity_handle(
        mut self,
        frozen_identity_handle: FrozenIdentityHandle,
    ) -> Self {
        self.primary_frozen_identity_handle = Some(frozen_identity_handle);
        self
    }

    // ------------------------------------------------------------------
    //  Syntax Constructors
    // ------------------------------------------------------------------

    pub(crate) fn expected_token(
        expected: TokenKind,
        found: Option<TokenKind>,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::ExpectedToken),
            span,
            DiagnosticPayload::ExpectedToken {
                expected: expected.into(),
                found: found.map(DiagnosticToken::from),
            },
        )
    }

    pub(crate) fn unexpected_token(found: TokenKind, span: Option<SourceSpan>) -> Self {
        Self::new(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::UnexpectedToken),
            span,
            DiagnosticPayload::UnexpectedToken {
                found: found.into(),
            },
        )
    }

    pub(crate) fn unexpected_trailing_comma(span: Option<SourceSpan>) -> Self {
        Self::new(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::UnexpectedTrailingComma),
            span,
            DiagnosticPayload::UnexpectedTrailingComma,
        )
    }

    pub(crate) fn unescaped_implicit_template_close(
        source_kind: SourceFileKind,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::UnescapedImplicitTemplateClose),
            span,
            DiagnosticPayload::UnescapedImplicitTemplateClose { source_kind },
        )
    }

    // ------------------------------------------------------------------
    //  Import Constructors
    // ------------------------------------------------------------------

    pub(crate) fn missing_import_target(path: InternedPath, span: Option<SourceSpan>) -> Self {
        Self::new(
            DiagnosticKind::Import(ImportDiagnosticKind::MissingImportTarget),
            span,
            DiagnosticPayload::MissingImportTarget { path },
        )
    }

    pub(crate) fn ambiguous_import_target(path: InternedPath, span: Option<SourceSpan>) -> Self {
        Self::new(
            DiagnosticKind::Import(ImportDiagnosticKind::AmbiguousImportTarget),
            span,
            DiagnosticPayload::AmbiguousImportTarget { path },
        )
    }

    pub(crate) fn bare_file_import(path: InternedPath, span: Option<SourceSpan>) -> Self {
        Self::new(
            DiagnosticKind::Import(ImportDiagnosticKind::BareFileImport),
            span,
            DiagnosticPayload::BareFileImport { path },
        )
    }

    pub(crate) fn direct_special_file_import(path: InternedPath, span: Option<SourceSpan>) -> Self {
        Self::new(
            DiagnosticKind::Import(ImportDiagnosticKind::DirectSpecialFileImport),
            span,
            DiagnosticPayload::DirectSpecialFileImport { path },
        )
    }

    pub(crate) fn import_name_collision(
        name: StringId,
        previous_span: Option<SourceSpan>,
        span: Option<SourceSpan>,
    ) -> Self {
        let mut labels = Vec::new();
        if let Some(prev) = &previous_span {
            labels.push(DiagnosticLabel::secondary(
                Some(*prev),
                Some(DiagnosticLabelMessage::PreviousDeclaration),
            ));
        }
        Self::new(
            DiagnosticKind::Import(ImportDiagnosticKind::ImportNameCollision),
            span,
            DiagnosticPayload::ImportNameCollision { name },
        )
        .with_labels(labels)
    }

    pub(crate) fn not_exported_by_source_file(
        symbol_path: InternedPath,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Import(ImportDiagnosticKind::NotExportedBySourceFile),
            span,
            DiagnosticPayload::NotExportedBySourceFile { symbol_path },
        )
    }

    pub(crate) fn not_exported_by_public_surface(
        requested_path: InternedPath,
        public_surface_name: StringId,
        public_surface_type: ImportPublicSurfaceType,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Import(ImportDiagnosticKind::NotExportedByPublicSurface),
            span,
            DiagnosticPayload::NotExportedByPublicSurface {
                requested_path,
                public_surface_name,
                public_surface_type,
            },
        )
    }

    pub(crate) fn missing_module_root_public_surface(
        symbol_path: InternedPath,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Import(ImportDiagnosticKind::MissingModuleRootPublicSurface),
            span,
            DiagnosticPayload::MissingModuleRootPublicSurface { symbol_path },
        )
    }

    pub(crate) fn missing_package_symbol(
        symbol: StringId,
        package_path: StringId,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Import(ImportDiagnosticKind::MissingPackageSymbol),
            span,
            DiagnosticPayload::MissingPackageSymbol {
                symbol,
                package_path,
            },
        )
    }

    pub(crate) fn cross_module_import_not_exported(
        symbol_path: InternedPath,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Import(ImportDiagnosticKind::CrossModuleImportNotExported),
            span,
            DiagnosticPayload::CrossModuleImportNotExported { symbol_path },
        )
    }

    pub(crate) fn invalid_import_path(
        path: InternedPath,
        reason: InvalidImportPathReason,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Import(ImportDiagnosticKind::InvalidImportPath),
            span,
            DiagnosticPayload::InvalidImportPath { path, reason },
        )
    }

    pub(crate) fn direct_symbol_path_import(path: InternedPath, span: Option<SourceSpan>) -> Self {
        Self::new(
            DiagnosticKind::Import(ImportDiagnosticKind::DirectSymbolPathImport),
            span,
            DiagnosticPayload::DirectSymbolPathImport { path },
        )
    }

    pub(crate) fn invalid_namespace_default_name(
        path: InternedPath,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Import(ImportDiagnosticKind::InvalidNamespaceDefaultName),
            span,
            DiagnosticPayload::InvalidNamespaceDefaultName { path },
        )
    }

    pub(crate) fn duplicate_import_surface_member(
        surface_path: InternedPath,
        member_name: StringId,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Import(ImportDiagnosticKind::DuplicateImportSurfaceMember),
            span,
            DiagnosticPayload::DuplicateImportSurfaceMember {
                surface_path,
                member_name,
            },
        )
    }

    pub(crate) fn explicit_moth_extension(path: InternedPath, span: Option<SourceSpan>) -> Self {
        Self::new(
            DiagnosticKind::Import(ImportDiagnosticKind::ExplicitMothExtension),
            span,
            DiagnosticPayload::ExplicitMothExtension { path },
        )
    }

    pub(crate) fn explicit_source_extension(
        path: InternedPath,
        extension: StringId,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Import(ImportDiagnosticKind::ExplicitSourceExtension),
            span,
            DiagnosticPayload::ExplicitSourceExtension { path, extension },
        )
    }

    pub(crate) fn unsupported_source_file_kind(
        path: InternedPath,
        extension: StringId,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Import(ImportDiagnosticKind::UnsupportedSourceFileKind),
            span,
            DiagnosticPayload::UnsupportedSourceFileKind { path, extension },
        )
    }

    pub(crate) fn invalid_source_file_entry(
        path: InternedPath,
        extension: StringId,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Import(ImportDiagnosticKind::InvalidSourceFileEntry),
            span,
            DiagnosticPayload::InvalidSourceFileEntry { path, extension },
        )
    }

    pub(crate) fn invalid_moth_template_api_scope_item(
        path: InternedPath,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Import(ImportDiagnosticKind::InvalidMothTemplateApiScopeItem),
            span,
            DiagnosticPayload::InvalidMothTemplateApiScopeItem { path },
        )
    }

    pub(crate) fn moth_template_inputs_share_no_common_ancestor(
        first_path: InternedPath,
        second_path: InternedPath,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Import(ImportDiagnosticKind::MothTemplateInputsShareNoCommonAncestor),
            span,
            DiagnosticPayload::MothTemplateInputsShareNoCommonAncestor {
                first_path,
                second_path,
            },
        )
    }

    pub(crate) fn duplicate_moth_template_input_path(
        path: InternedPath,
        first_span: Option<SourceSpan>,
        duplicate_span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Import(ImportDiagnosticKind::DuplicateMothTemplateInputPath),
            duplicate_span,
            DiagnosticPayload::DuplicateMothTemplateInputPath { path },
        )
        .with_labels(vec![DiagnosticLabel::secondary(first_span, None)])
    }

    pub(crate) fn unsupported_external_extension(
        path: InternedPath,
        extension: StringId,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Import(ImportDiagnosticKind::UnsupportedExternalExtension),
            span,
            DiagnosticPayload::UnsupportedExternalExtension { path, extension },
        )
    }

    pub(crate) fn invalid_external_module(
        path: InternedPath,
        message: StringId,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Import(ImportDiagnosticKind::InvalidExternalModule),
            span,
            DiagnosticPayload::InvalidExternalModule { path, message },
        )
    }

    // ------------------------------------------------------------------
    //  Borrow Constructors
    // ------------------------------------------------------------------

    pub(crate) fn multiple_mutable_borrows(
        place: DiagnosticPlace,
        conflicting_place: Option<DiagnosticPlace>,
        existing_span: Option<SourceSpan>,
        span: Option<SourceSpan>,
    ) -> Self {
        let mut labels = Vec::new();
        if let Some(existing_span) = existing_span {
            labels.push(DiagnosticLabel::secondary(
                Some(existing_span),
                Some(DiagnosticLabelMessage::ConflictingAccess),
            ));
        }

        Self::new(
            DiagnosticKind::Borrow(BorrowDiagnosticKind::MultipleMutableBorrows),
            span,
            DiagnosticPayload::MultipleMutableBorrows {
                place,
                conflicting_place,
            },
        )
        .with_labels(labels)
    }

    pub(crate) fn shared_mutable_conflict(
        place: DiagnosticPlace,
        existing_access: BorrowAccessKind,
        requested_access: BorrowAccessKind,
        conflicting_place: Option<DiagnosticPlace>,
        existing_span: Option<SourceSpan>,
        span: Option<SourceSpan>,
    ) -> Self {
        let mut labels = Vec::new();
        if let Some(existing_span) = existing_span {
            labels.push(DiagnosticLabel::secondary(
                Some(existing_span),
                Some(DiagnosticLabelMessage::ConflictingAccess),
            ));
        }

        Self::new(
            DiagnosticKind::Borrow(BorrowDiagnosticKind::SharedMutableConflict),
            span,
            DiagnosticPayload::SharedMutableConflict {
                place,
                existing_access,
                requested_access,
                conflicting_place,
            },
        )
        .with_labels(labels)
    }

    // Reserved diagnostic constructor; the payload/renderers remain wired for move-specific
    // borrow reports.
    #[allow(dead_code)]
    pub(crate) fn use_after_possible_move(
        place: DiagnosticPlace,
        move_span: Option<SourceSpan>,
        span: Option<SourceSpan>,
    ) -> Self {
        let mut labels = Vec::new();
        if let Some(move_span) = move_span {
            labels.push(DiagnosticLabel::secondary(
                Some(move_span),
                Some(DiagnosticLabelMessage::ValueMovedHere),
            ));
        }

        Self::new(
            DiagnosticKind::Borrow(BorrowDiagnosticKind::UseAfterPossibleMove),
            span,
            DiagnosticPayload::UseAfterPossibleMove { place },
        )
        .with_labels(labels)
    }

    pub(crate) fn invalid_mutable_access(
        place: DiagnosticPlace,
        reason: InvalidMutableAccessReason,
        conflicting_place: Option<DiagnosticPlace>,
        conflicting_span: Option<SourceSpan>,
        span: Option<SourceSpan>,
    ) -> Self {
        let mut labels = Vec::new();
        if let Some(conflicting_span) = conflicting_span {
            labels.push(DiagnosticLabel::secondary(
                Some(conflicting_span),
                Some(DiagnosticLabelMessage::ConflictingAccess),
            ));
        }

        Self::new(
            DiagnosticKind::Borrow(BorrowDiagnosticKind::InvalidMutableAccess),
            span,
            DiagnosticPayload::InvalidMutableAccess {
                place,
                reason,
                conflicting_place,
            },
        )
        .with_labels(labels)
    }

    pub(crate) fn use_of_uninitialized_local(
        place: DiagnosticPlace,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Borrow(BorrowDiagnosticKind::UseOfUninitializedLocal),
            span,
            DiagnosticPayload::UseOfUninitializedLocal { place },
        )
    }

    // ------------------------------------------------------------------
    //  Rule Constructors
    // ------------------------------------------------------------------

    pub(crate) fn duplicate_declaration(
        name: StringId,
        first_span: Option<SourceSpan>,
        duplicate_span: Option<SourceSpan>,
    ) -> Self {
        // Prelude-injected symbols have no authored previous span. Omit the secondary
        // label in that case so no fabricated empty span enters a user-facing label.
        let mut labels = Vec::new();
        if let Some(span) = &first_span {
            labels.push(DiagnosticLabel::secondary(
                Some(*span),
                Some(DiagnosticLabelMessage::PreviousDeclaration),
            ));
        }
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::DuplicateDeclaration),
            duplicate_span,
            DiagnosticPayload::DuplicateDeclaration { name },
        )
        .with_labels(labels)
    }

    pub(crate) fn invalid_compile_time_path(
        path: InternedPath,
        reason: InvalidCompileTimePathReason,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidCompileTimePath),
            span,
            DiagnosticPayload::InvalidCompileTimePath { path, reason },
        )
    }

    pub(crate) fn dependency_namespace_used_as_value(
        record_name: StringId,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::DependencyNamespaceUsedAsValue),
            span,
            DiagnosticPayload::DependencyNamespaceUsedAsValue { record_name },
        )
    }

    pub(crate) fn const_record_used_as_value(
        record_name: StringId,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::ConstRecordUsedAsValue),
            span,
            DiagnosticPayload::ConstRecordUsedAsValue { record_name },
        )
    }

    pub(crate) fn nested_dependency_traversal(
        record_name: StringId,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::NestedDependencyTraversal),
            span,
            DiagnosticPayload::NestedDependencyTraversal { record_name },
        )
    }

    pub(crate) fn namespace_type_value_misuse(
        name: StringId,
        expected: NamespaceTypeValueMisuseKind,
        found: NamespaceTypeValueMisuseKind,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::NamespaceTypeValueMisuse),
            span,
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
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::UnsupportedExternalFunction),
            span,
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
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidRangeOperand),
            span,
            DiagnosticPayload::InvalidRangeOperand {
                operand,
                found_type,
            },
        )
    }

    pub(crate) fn unsupported_builder_package(
        package_path: StringId,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::UnsupportedBuilderPackage),
            span,
            DiagnosticPayload::UnsupportedBuilderPackage { package_path },
        )
    }

    pub(crate) fn unsupported_backend_feature(
        backend_name: StringId,
        reason: UnsupportedBackendFeatureReason,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::UnsupportedBackendFeature),
            span,
            DiagnosticPayload::UnsupportedBackendFeature {
                backend_name,
                reason,
            },
        )
    }

    pub(crate) fn invalid_page_metadata(
        key: StringId,
        reason: InvalidPageMetadataReason,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidPageMetadata),
            span,
            DiagnosticPayload::InvalidPageMetadata { key, reason },
        )
    }

    // ------------------------------------------------------------------
    //  Config Constructors
    // ------------------------------------------------------------------

    pub(crate) fn invalid_config_reason(
        key: Option<StringId>,
        reason: InvalidConfigReason,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Config(ConfigDiagnosticKind::InvalidConfig),
            span,
            DiagnosticPayload::InvalidConfig { key, reason },
        )
    }

    pub(crate) fn deferred_feature(feature: StringId, span: Option<SourceSpan>) -> Self {
        Self::deferred_feature_reason(DeferredFeatureReason::NamedFeature { feature }, span)
    }

    pub(crate) fn deferred_feature_reason(
        reason: DeferredFeatureReason,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::DeferredFeature(DeferredFeatureDiagnosticKind::DeferredFeature),
            span,
            DiagnosticPayload::DeferredFeature { reason },
        )
    }

    // ------------------------------------------------------------------
    //  Warning Constructors
    // ------------------------------------------------------------------

    pub(crate) fn identifier_naming_convention(
        name: StringId,
        expected_style: NamingConvention,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::with_severity(
            DiagnosticKind::Rule(RuleDiagnosticKind::IdentifierNamingConvention),
            DiagnosticSeverity::Warning,
            span,
            DiagnosticPayload::IdentifierNamingConvention {
                name,
                expected_style,
            },
        )
    }

    pub(crate) fn unreachable_match_arm(span: Option<SourceSpan>) -> Self {
        Self::with_severity(
            DiagnosticKind::Rule(RuleDiagnosticKind::UnreachableMatchArm),
            DiagnosticSeverity::Warning,
            span,
            DiagnosticPayload::UnreachableMatchArm,
        )
    }

    pub(crate) fn dependency_alias_case_mismatch(
        alias: StringId,
        symbol: StringId,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::with_severity(
            DiagnosticKind::Import(ImportDiagnosticKind::DependencyAliasCaseMismatch),
            DiagnosticSeverity::Warning,
            span,
            DiagnosticPayload::DependencyAliasCaseMismatch { alias, symbol },
        )
    }

    pub(crate) fn malformed_css_template(message: StringId, span: Option<SourceSpan>) -> Self {
        Self::with_severity(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::MalformedCssTemplate),
            DiagnosticSeverity::Warning,
            span,
            DiagnosticPayload::MalformedTemplate { message },
        )
    }

    pub(crate) fn malformed_html_template(message: StringId, span: Option<SourceSpan>) -> Self {
        Self::with_severity(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::MalformedHtmlTemplate),
            DiagnosticSeverity::Warning,
            span,
            DiagnosticPayload::MalformedTemplate { message },
        )
    }

    // ------------------------------------------------------------------
    //  Syntax Constructors (Continued)
    // ------------------------------------------------------------------

    pub(crate) fn invalid_character(character: char, span: Option<SourceSpan>) -> Self {
        Self::new(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::InvalidCharacter),
            span,
            DiagnosticPayload::InvalidCharacter { character },
        )
    }
    /// Report an authored range that could not receive another extended-span row.
    ///
    /// The primary span is an already representable source-owned anchor. The rejected range is
    /// retained as facts in the payload, so reporting this failure never attempts another
    /// allocation from the exhausted table.
    pub(crate) fn source_span_capacity(
        start: u32,
        length: u32,
        resource: SourceSpanCapacityResource,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::SourceSpanCapacity),
            span,
            DiagnosticPayload::SourceSpanCapacity {
                start,
                length,
                resource,
            },
        )
    }

    pub(crate) fn invalid_number_literal(
        literal_text: StringId,
        reason: NumberLiteralErrorReason,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::InvalidNumberLiteral),
            span,
            DiagnosticPayload::InvalidNumberLiteral {
                literal_text,
                reason,
            },
        )
    }

    pub(crate) fn invalid_style_directive(
        directive_name: StringId,
        supported_directives: StringId,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::InvalidStyleDirective),
            span,
            DiagnosticPayload::InvalidStyleDirective {
                directive_name,
                supported_directives,
            },
        )
    }

    pub(crate) fn missing_closing_delimiter(
        expected_delimiter: StringId,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::MissingClosingDelimiter),
            span,
            DiagnosticPayload::MissingClosingDelimiter { expected_delimiter },
        )
    }

    pub(crate) fn invalid_generic_application(
        reason: GenericApplicationErrorReason,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::InvalidGenericApplication),
            span,
            DiagnosticPayload::InvalidGenericApplication { reason },
        )
    }

    pub(crate) fn unexpected_end_of_file(
        expected_delimiter: Option<StringId>,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::UnexpectedEndOfFile),
            span,
            DiagnosticPayload::UnexpectedEndOfFile { expected_delimiter },
        )
    }

    pub(crate) fn invalid_path(path_kind: PathKind, span: Option<SourceSpan>) -> Self {
        Self::new(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::InvalidPath),
            span,
            DiagnosticPayload::InvalidPath { path_kind },
        )
    }

    pub(crate) fn invalid_dependency_clause(
        clause_kind: DependencyClauseKind,
        reason: InvalidDependencyClauseReason,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::InvalidDependencyClause),
            span,
            DiagnosticPayload::InvalidDependencyClause {
                clause_kind,
                reason,
            },
        )
    }

    pub(crate) fn legacy_dependency_clause(
        replacement: Option<StringId>,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::LegacyDependencyClause),
            span,
            DiagnosticPayload::LegacyDependencyClause {
                reason: LegacyDependencyClauseReason::ImportKeyword,
                replacement,
            },
        )
    }

    pub(crate) fn unterminated_string_literal(span: Option<SourceSpan>) -> Self {
        Self::new(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::UnterminatedStringLiteral),
            span,
            DiagnosticPayload::None,
        )
    }

    pub(crate) fn invalid_string_escape(
        reason: InvalidStringEscapeReason,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::InvalidStringEscape),
            span,
            DiagnosticPayload::InvalidStringEscape { reason },
        )
    }

    pub(crate) fn invalid_char_literal(span: Option<SourceSpan>) -> Self {
        Self::new(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::InvalidCharLiteral),
            span,
            DiagnosticPayload::None,
        )
    }

    pub(crate) fn unexpected_token_in_declaration(span: Option<SourceSpan>) -> Self {
        Self::new(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::UnexpectedTokenInDeclaration),
            span,
            DiagnosticPayload::None,
        )
    }

    pub(crate) fn invalid_identifier(span: Option<SourceSpan>) -> Self {
        Self::new(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::InvalidIdentifier),
            span,
            DiagnosticPayload::None,
        )
    }

    pub(crate) fn invalid_type_annotation(
        context: TypeAnnotationContext,
        reason: InvalidTypeAnnotationReason,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::InvalidTypeAnnotation),
            span,
            DiagnosticPayload::InvalidTypeAnnotation { context, reason },
        )
    }

    pub(crate) fn invalid_collection_type(
        reason: InvalidCollectionTypeReason,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::InvalidCollectionType),
            span,
            DiagnosticPayload::InvalidCollectionType { reason },
        )
    }

    pub(crate) fn invalid_map_type(reason: InvalidMapTypeReason, span: Option<SourceSpan>) -> Self {
        Self::new(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::InvalidMapType),
            span,
            DiagnosticPayload::InvalidMapType { reason },
        )
    }

    pub(crate) fn invalid_map_literal(
        reason: InvalidMapLiteralReason,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::InvalidMapLiteral),
            span,
            DiagnosticPayload::InvalidMapLiteral { reason },
        )
    }

    pub(crate) fn invalid_generic_parameter(
        reason: InvalidGenericParameterReason,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::InvalidGenericParameter),
            span,
            DiagnosticPayload::InvalidGenericParameter { reason },
        )
    }

    pub(crate) fn invalid_template_directive(
        directive_name: Option<StringId>,
        reason: InvalidTemplateDirectiveReason,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::InvalidTemplateDirective),
            span,
            DiagnosticPayload::InvalidTemplateDirective {
                directive_name,
                reason,
            },
        )
    }

    pub(crate) fn invalid_template_structure(
        reason: InvalidTemplateStructureReason,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::InvalidTemplateStructure),
            span,
            DiagnosticPayload::InvalidTemplateStructure { reason },
        )
    }

    pub(crate) fn invalid_expression(
        reason: InvalidExpressionReason,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::InvalidExpression),
            span,
            DiagnosticPayload::InvalidExpression { reason },
        )
    }

    pub(crate) fn missing_operator_operand(
        operator: StringId,
        position: OperatorOperandPosition,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::MissingOperatorOperand),
            span,
            DiagnosticPayload::MissingOperatorOperand { operator, position },
        )
    }

    pub(crate) fn invalid_standalone_statement(
        reason: InvalidStandaloneStatementReason,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::InvalidStandaloneStatement),
            span,
            DiagnosticPayload::InvalidStandaloneStatement { reason },
        )
    }

    pub(crate) fn expected_symbol_statement(span: Option<SourceSpan>) -> Self {
        Self::new(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::ExpectedSymbolStatement),
            span,
            DiagnosticPayload::ExpectedSymbolStatement,
        )
    }

    pub(crate) fn missing_collection_item(span: Option<SourceSpan>) -> Self {
        Self::new(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::MissingCollectionItem),
            span,
            DiagnosticPayload::MissingCollectionItem,
        )
    }

    pub(crate) fn invalid_match_arm(
        reason: InvalidMatchArmReason,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::InvalidMatchArm),
            span,
            DiagnosticPayload::InvalidMatchArm { reason },
        )
    }

    pub(crate) fn invalid_loop_header(
        reason: InvalidLoopHeaderReason,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::InvalidLoopHeader),
            span,
            DiagnosticPayload::InvalidLoopHeader { reason },
        )
    }

    pub(crate) fn invalid_statement_position(
        reason: InvalidStatementPositionReason,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::InvalidStatementPosition),
            span,
            DiagnosticPayload::InvalidStatementPosition { reason },
        )
    }

    pub(crate) fn common_syntax_mistake(
        reason: CommonSyntaxMistakeReason,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::CommonSyntaxMistake),
            span,
            DiagnosticPayload::CommonSyntaxMistake { reason },
        )
    }

    // ------------------------------------------------------------------
    //  Rule Constructors (Continued)
    // ------------------------------------------------------------------

    pub(crate) fn invalid_signature_member(
        reason: InvalidSignatureMemberReason,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidSignatureMember),
            span,
            DiagnosticPayload::InvalidSignatureMember { reason },
        )
    }

    pub(crate) fn invalid_function_signature(
        reason: InvalidFunctionSignatureReason,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidFunctionSignature),
            span,
            DiagnosticPayload::InvalidFunctionSignature { reason },
        )
    }

    pub(crate) fn invalid_choice_variant(
        reason: InvalidChoiceVariantReason,
        choice_name: Option<StringId>,
        variant_name: Option<StringId>,
        available_variants: Vec<StringId>,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidChoiceVariant),
            span,
            DiagnosticPayload::InvalidChoiceVariant {
                reason,
                choice_name,
                variant_name,
                available_variants,
            },
        )
    }

    pub(crate) fn invalid_struct_default_value(span: Option<SourceSpan>) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidStructDefaultValue),
            span,
            DiagnosticPayload::InvalidStructDefaultValue,
        )
    }

    pub(crate) fn missing_declaration_initializer(
        name: StringId,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::MissingDeclarationInitializer),
            span,
            DiagnosticPayload::MissingDeclarationInitializer { name },
        )
    }

    pub(crate) fn circular_dependency(path: InternedPath, span: Option<SourceSpan>) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::CircularDependency),
            span,
            DiagnosticPayload::CircularDependency { path },
        )
    }

    pub(crate) fn unknown_value_name(name: StringId, span: Option<SourceSpan>) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::UnknownValueName),
            span,
            DiagnosticPayload::UnknownName {
                name,
                namespace: NameNamespace::Value,
            },
        )
    }

    pub(crate) fn unknown_type_name(name: StringId, span: Option<SourceSpan>) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::UnknownTypeName),
            span,
            DiagnosticPayload::UnknownName {
                name,
                namespace: NameNamespace::Type,
            },
        )
    }

    pub(crate) fn unknown_trait_name(name: StringId, span: Option<SourceSpan>) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::UnknownTrait),
            span,
            DiagnosticPayload::UnknownTrait { name },
        )
    }

    pub(crate) fn duplicate_trait_requirement(
        trait_name: StringId,
        requirement_name: StringId,
        first_span: Option<SourceSpan>,
        duplicate_span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::DuplicateTraitRequirement),
            duplicate_span,
            DiagnosticPayload::DuplicateTraitRequirement {
                trait_name,
                requirement_name,
            },
        )
        .with_labels(vec![DiagnosticLabel::secondary(
            first_span,
            Some(DiagnosticLabelMessage::PreviousDeclaration),
        )])
    }

    pub(crate) fn trait_private_surface_leak(
        trait_name: StringId,
        surface_type: TypeId,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::TraitPrivateSurfaceLeak),
            span,
            DiagnosticPayload::TraitPrivateSurfaceLeak {
                trait_name,
                surface_type,
            },
        )
    }

    pub(crate) fn generic_bound_private_surface_leak(
        function_name: StringId,
        trait_name: StringId,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::GenericBoundPrivateSurfaceLeak),
            span,
            DiagnosticPayload::GenericBoundPrivateSurfaceLeak {
                function_name,
                trait_name,
            },
        )
    }

    pub(crate) fn unsupported_trait_feature(
        trait_name: StringId,
        feature: StringId,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::UnsupportedTraitFeature),
            span,
            DiagnosticPayload::UnsupportedTraitFeature {
                trait_name,
                feature,
            },
        )
    }

    pub(crate) fn invalid_trait_keyword_usage(
        reason: InvalidTraitKeywordUsageReason,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidTraitKeywordUsage),
            span,
            DiagnosticPayload::InvalidTraitKeywordUsage { reason },
        )
    }

    pub(crate) fn export_outside_module_root(span: Option<SourceSpan>) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::ExportOutsideModuleRoot),
            span,
            DiagnosticPayload::None,
        )
    }

    pub(crate) fn invalid_export_target(span: Option<SourceSpan>) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidExportTarget),
            span,
            DiagnosticPayload::None,
        )
    }

    pub(crate) fn duplicate_public_export(
        name: StringId,
        first_span: Option<SourceSpan>,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::DuplicatePublicExport),
            span,
            DiagnosticPayload::DuplicatePublicExport { name },
        )
        .with_labels(vec![DiagnosticLabel::secondary(first_span, None)])
    }

    pub(crate) fn duplicate_export_block(span: Option<SourceSpan>) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::DuplicateExportBlock),
            span,
            DiagnosticPayload::None,
        )
    }

    pub(crate) fn private_type_in_exported_api(
        exported_name: StringId,
        private_type: TypeId,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::PrivateTypeInExportedApi),
            span,
            DiagnosticPayload::PrivateTypeInExportedApi {
                exported_name,
                private_type,
            },
        )
    }

    pub(crate) fn project_context_escape(
        reason: ProjectContextEscapeReason,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::ProjectContextEscape),
            span,
            DiagnosticPayload::ProjectContextEscape { reason },
        )
    }

    pub(crate) fn invalid_trait_conformance(
        target_name: StringId,
        trait_name: Option<StringId>,
        reason: InvalidTraitConformanceReason,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidTraitConformance),
            span,
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
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidTraitIncompatibility),
            span,
            DiagnosticPayload::InvalidTraitIncompatibility {
                subject_name,
                incompatible_trait_name,
                reason,
            },
        )
    }

    pub(crate) fn trait_name_used_as_type(trait_name: StringId, span: Option<SourceSpan>) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::TraitNameUsedAsType),
            span,
            DiagnosticPayload::TraitNameUsedAsType { trait_name },
        )
    }

    pub(crate) fn namespace_misuse(
        name: StringId,
        expected: NameNamespace,
        found: NameNamespace,
        span: Option<SourceSpan>,
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
            span,
            DiagnosticPayload::NamespaceMisuse {
                name,
                expected,
                found,
            },
        )
    }

    pub(crate) fn shadowed_name(
        name: StringId,
        first_span: Option<SourceSpan>,
        duplicate_span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::ShadowedName),
            duplicate_span,
            DiagnosticPayload::ShadowedName { name },
        )
        .with_labels(vec![DiagnosticLabel::secondary(
            first_span,
            Some(DiagnosticLabelMessage::PreviousDeclaration),
        )])
    }

    pub(crate) fn reserved_name_collision(
        name: StringId,
        reserved_by: crate::compiler_frontend::compiler_messages::ReservedNameOwner,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::ReservedNameCollision),
            span,
            DiagnosticPayload::ReservedNameCollision { name, reserved_by },
        )
    }

    pub(crate) fn invalid_this_usage(
        reason: crate::compiler_frontend::compiler_messages::InvalidThisUsageReason,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidThisUsage),
            span,
            DiagnosticPayload::InvalidThisUsage { reason },
        )
    }

    pub(crate) fn invalid_receiver_declaration(
        reason: crate::compiler_frontend::compiler_messages::InvalidReceiverDeclarationReason,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidReceiverDeclaration),
            span,
            DiagnosticPayload::InvalidReceiverDeclaration { reason },
        )
    }

    pub(crate) fn invalid_top_level_runtime_statement(span: Option<SourceSpan>) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidTopLevelRuntimeStatement),
            span,
            DiagnosticPayload::None,
        )
    }

    pub(crate) fn reserved_builtin_name(name: StringId, span: Option<SourceSpan>) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::ReservedBuiltinName),
            span,
            DiagnosticPayload::UnusedName { name },
        )
    }

    pub(crate) fn invalid_control_flow_statement(
        reason: crate::compiler_frontend::compiler_messages::InvalidControlFlowStatementReason,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidControlFlowStatement),
            span,
            DiagnosticPayload::InvalidControlFlowStatement { reason },
        )
    }

    pub(crate) fn invalid_declaration(
        reason: crate::compiler_frontend::compiler_messages::InvalidDeclarationReason,
        name: Option<StringId>,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidDeclaration),
            span,
            DiagnosticPayload::InvalidDeclaration { reason, name },
        )
    }

    pub(crate) fn invalid_generic_instantiation(
        type_name: Option<StringId>,
        reason: crate::compiler_frontend::compiler_messages::InvalidGenericInstantiationReason,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidGenericInstantiation),
            span,
            DiagnosticPayload::InvalidGenericInstantiation { type_name, reason },
        )
    }

    pub(crate) fn conflicting_generic_inference(
        type_name: Option<StringId>,
        subject: GenericInferenceSubject,
        conflict: BindingConflict,
        parameter_name: StringId,
        current_evidence_span: Option<SourceSpan>,
        previous_evidence_span: Option<SourceSpan>,
    ) -> Self {
        let mut diagnostic = Self::invalid_generic_instantiation(
            type_name,
            crate::compiler_frontend::compiler_messages::InvalidGenericInstantiationReason::ConflictingInference {
                subject,
                parameter_id: conflict.parameter_id,
                parameter_name,
                existing_type_id: conflict.existing_type_id,
                replacement_type_id: conflict.replacement_type_id,
            },
            current_evidence_span,
        );
        if previous_evidence_span.is_some() {
            diagnostic = diagnostic.with_labels(vec![DiagnosticLabel::secondary(
                previous_evidence_span,
                Some(DiagnosticLabelMessage::GenericInferencePreviousEvidence),
            )]);
        }

        diagnostic
    }

    pub(crate) fn invalid_assignment_target(
        reason: crate::compiler_frontend::compiler_messages::InvalidAssignmentTargetReason,
        target_name: Option<StringId>,
        target_type: Option<crate::compiler_frontend::datatypes::ids::TypeId>,
        field_name: Option<StringId>,
        root_binding_name: Option<StringId>,
        declaration_span: Option<SourceSpan>,
        span: Option<SourceSpan>,
    ) -> Self {
        let mut labels = Vec::new();
        if let Some(declaration_span) = &declaration_span {
            labels.push(DiagnosticLabel::secondary(
                Some(*declaration_span),
                Some(DiagnosticLabelMessage::ImmutableBindingDeclaration),
            ));
        }
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidAssignmentTarget),
            span,
            DiagnosticPayload::InvalidAssignmentTarget {
                reason,
                target_name,
                target_type,
                field_name,
                root_binding_name,
            },
        )
        .with_labels(labels)
    }

    pub(crate) fn invalid_multi_bind(
        reason: crate::compiler_frontend::compiler_messages::InvalidMultiBindReason,
        target_name: Option<StringId>,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidMultiBind),
            span,
            DiagnosticPayload::InvalidMultiBind {
                reason,
                target_name,
            },
        )
    }

    pub(crate) fn invalid_multi_bind_syntax(
        reason: crate::compiler_frontend::compiler_messages::InvalidMultiBindReason,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Syntax(SyntaxDiagnosticKind::UnexpectedToken),
            span,
            DiagnosticPayload::InvalidMultiBind {
                reason,
                target_name: None,
            },
        )
    }

    pub(crate) fn invalid_builtin_call(
        reason: crate::compiler_frontend::compiler_messages::InvalidBuiltinCallReason,
        builtin_name: Option<StringId>,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidBuiltinCall),
            span,
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
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidCast),
            span,
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
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidReceiverCall),
            span,
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
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidCopyTarget),
            span,
            DiagnosticPayload::InvalidCopyTarget { reason },
        )
    }

    pub(crate) fn invalid_field_access(
        reason: crate::compiler_frontend::compiler_messages::InvalidFieldAccessReason,
        field_name: Option<StringId>,
        receiver_type: Option<crate::compiler_frontend::datatypes::ids::TypeId>,
        known_fields: Vec<StringId>,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidFieldAccess),
            span,
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
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidMatchPattern),
            span,
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
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::NonExhaustiveMatch),
            span,
            DiagnosticPayload::NonExhaustiveMatch {
                reason,
                missing_variants,
            },
        )
    }

    pub(crate) fn invalid_fallible_handling(
        reason: crate::compiler_frontend::compiler_messages::InvalidFallibleHandlingReason,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidFallibleHandling),
            span,
            DiagnosticPayload::InvalidFallibleHandling { reason },
        )
    }

    pub(crate) fn invalid_template_slot(
        reason: crate::compiler_frontend::compiler_messages::InvalidTemplateSlotReason,
        slot_name: Option<StringId>,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidTemplateSlot),
            span,
            DiagnosticPayload::InvalidTemplateSlot { reason, slot_name },
        )
    }

    pub(crate) fn compile_time_evaluation_error(
        reason: crate::compiler_frontend::compiler_messages::CompileTimeEvaluationErrorReason,
        operation: Option<StringId>,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::CompileTimeEvaluationError),
            span,
            DiagnosticPayload::CompileTimeEvaluationError { reason, operation },
        )
    }

    pub(crate) fn empty_collection_type_ambiguity(span: Option<SourceSpan>) -> Self {
        Self::new(
            DiagnosticKind::Type(TypeDiagnosticKind::EmptyCollectionTypeAmbiguity),
            span,
            DiagnosticPayload::EmptyCollectionTypeAmbiguity,
        )
    }

    pub(crate) fn unsupported_operator_types(
        operator: DiagnosticOperator,
        lhs: TypeId,
        rhs: Option<TypeId>,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Type(TypeDiagnosticKind::UnsupportedOperatorTypes),
            span,
            DiagnosticPayload::UnsupportedOperatorTypes { operator, lhs, rhs },
        )
    }

    pub(crate) fn invalid_fallible_operand(
        reason: InvalidFallibleOperandReason,
        category: UnsupportedOperatorCategory,
        operand_type: TypeId,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Type(TypeDiagnosticKind::InvalidFallibleOperand),
            span,
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
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Type(TypeDiagnosticKind::IncompatibleChoiceComparison),
            span,
            DiagnosticPayload::IncompatibleChoiceComparison { reason, lhs, rhs },
        )
    }

    pub(crate) fn invalid_call_shape(
        reason: crate::compiler_frontend::compiler_messages::InvalidCallShapeReason,
        callee_name: Option<StringId>,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidCallShape),
            span,
            DiagnosticPayload::InvalidCallShape {
                reason,
                callee_name,
            },
        )
    }

    pub(crate) fn invalid_return_shape(
        reason: crate::compiler_frontend::compiler_messages::InvalidReturnShapeReason,
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Rule(RuleDiagnosticKind::InvalidReturnShape),
            span,
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
        span: Option<SourceSpan>,
    ) -> Self {
        Self::new(
            DiagnosticKind::Type(TypeDiagnosticKind::TypeMismatch),
            span,
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

    /// Classify a source-span packing failure at the source-owning boundary.
    ///
    /// Extended-table exhaustion is authored input and therefore remains a typed diagnostic.
    /// An unrepresentable end violates the accepted source-size invariant and stays on the
    /// compiler-invariant error lane.
    pub(crate) fn from_span_capacity_error(
        error: SpanCapacityError,
        span: Option<SourceSpan>,
    ) -> Result<Self, CompilerError> {
        match error.reason() {
            SpanCapacityReason::ExtendedTableFull => Ok(Self::source_span_capacity(
                error.start(),
                error.length(),
                SourceSpanCapacityResource::ExtendedSpanTable,
                span,
            )),
            SpanCapacityReason::EndUnrepresentable => {
                Err(CompilerError::source_span_capacity(error, span))
            }
        }
    }

    /// Validate the exact spans retained by one preparation diagnostic.
    ///
    /// Preparation does not derive or encode ranges: diagnostics without a span remain
    /// unspanned, while every existing span must belong to the source being prepared.
    pub(crate) fn capture_preparation_span(
        &mut self,
        source: SourceId,
    ) -> Result<(), CompilerError> {
        let spans = self
            .primary_span
            .into_iter()
            .chain(self.labels.iter().filter_map(|label| label.span));
        if spans.into_iter().any(|span| span.source() != source) {
            return Err(CompilerError::new(
                "preparation diagnostic changed source domains",
                None,
                ErrorType::Compiler,
            ));
        }
        Ok(())
    }

    /// Return the compiler-owned identity used by tests and tooling.
    ///
    /// The descriptor remains the sole code authority and `severity` is the actual severity on
    /// this diagnostic, including deliberate overrides. The payload owns the optional reason key.
    pub(crate) fn identity(&self) -> DiagnosticIdentity {
        DiagnosticIdentity::new(self.kind.descriptor().code, self.severity, &self.payload)
    }

    pub(crate) fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        for label in &mut self.labels {
            label.remap_string_ids(remap);
        }

        self.payload.remap_string_ids(remap);
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
