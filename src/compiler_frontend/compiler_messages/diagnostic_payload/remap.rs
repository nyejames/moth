//! String-table remapping for diagnostic payload facts.
//!
//! WHAT: walks payload variants and updates every interned string-bearing field after
//! string tables are merged.
//! WHY: keeping this traversal outside the payload declarations makes the diagnostic data
//! model easier to scan while preserving one canonical remap implementation.

use super::*;

impl DiagnosticPayload {
    pub(crate) fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        match self {
            DiagnosticPayload::None
            | DiagnosticPayload::UnexpectedTrailingComma
            | DiagnosticPayload::UnescapedImplicitTemplateClose { .. }
            | DiagnosticPayload::TypeMismatch { .. }
            | DiagnosticPayload::UnreachableMatchArm => {}

            DiagnosticPayload::ExpectedToken { expected, found } => {
                expected.remap_string_ids(remap);

                if let Some(found) = found {
                    found.remap_string_ids(remap);
                }
            }

            DiagnosticPayload::UnexpectedToken { found } => {
                found.remap_string_ids(remap);
            }

            DiagnosticPayload::UnknownName { name, .. }
            | DiagnosticPayload::UnusedName { name }
            | DiagnosticPayload::MothFilePathInTemplateOutput { path: name }
            | DiagnosticPayload::LargeTrackedAsset { path: name, .. }
            | DiagnosticPayload::IdentifierNamingConvention { name, .. }
            | DiagnosticPayload::MalformedTemplate { message: name } => {
                *name = remap.get(*name);
            }

            DiagnosticPayload::DuplicateDeclaration {
                name,
                first_location,
            } => {
                *name = remap.get(*name);
                if let Some(location) = first_location {
                    location.remap_string_ids(remap);
                }
            }

            DiagnosticPayload::MissingImportTarget { path }
            | DiagnosticPayload::AmbiguousImportTarget { path }
            | DiagnosticPayload::BareFileImport { path }
            | DiagnosticPayload::DirectSpecialFileImport { path }
            | DiagnosticPayload::NotExportedBySourceFile { symbol_path: path }
            | DiagnosticPayload::NotExportedByPublicSurface {
                requested_path: path,
                ..
            }
            | DiagnosticPayload::MissingModuleRootPublicSurface { symbol_path: path }
            | DiagnosticPayload::CrossModuleImportNotExported { symbol_path: path }
            | DiagnosticPayload::InvalidMothTemplateApiScopeItem { path } => {
                remap_path_import_payload(path, remap);
            }

            DiagnosticPayload::DuplicateMothTemplateInputPath {
                path,
                first_location,
            } => {
                remap_path_import_payload(path, remap);
                first_location.remap_string_ids(remap);
            }

            DiagnosticPayload::InvalidImportPath { path, reason } => {
                remap_invalid_import_path_payload(path, reason, remap);
            }

            DiagnosticPayload::ImportNameCollision {
                name,
                previous_location,
            } => {
                *name = remap.get(*name);
                if let Some(location) = previous_location {
                    location.remap_string_ids(remap);
                }
            }

            DiagnosticPayload::MissingPackageSymbol {
                symbol,
                package_path,
            } => {
                *symbol = remap.get(*symbol);
                *package_path = remap.get(*package_path);
            }

            DiagnosticPayload::BorrowConflict { place, .. }
            | DiagnosticPayload::UseOfUninitializedLocal { place } => {
                remap_single_place_borrow_payload(place, remap);
            }

            DiagnosticPayload::SharedMutableConflict {
                place,
                conflicting_place,
                existing_location,
                ..
            } => {
                remap_shared_mutable_conflict_payload(
                    place,
                    conflicting_place,
                    existing_location,
                    remap,
                );
            }

            DiagnosticPayload::WholeObjectBorrowConflict {
                whole_place,
                part_place,
                part_location,
            } => {
                remap_whole_object_borrow_conflict_payload(
                    whole_place,
                    part_place,
                    part_location,
                    remap,
                );
            }

            DiagnosticPayload::MultipleMutableBorrows {
                place,
                conflicting_place,
                existing_location,
            } => {
                remap_place_with_optional_conflict(place, conflicting_place, remap);
                remap_optional_location(existing_location, remap);
            }

            DiagnosticPayload::UseAfterPossibleMove {
                place,
                move_location,
            } => {
                remap_place_with_optional_location(place, move_location, remap);
            }

            DiagnosticPayload::MoveWhileBorrowed {
                place,
                borrow_location,
                ..
            } => {
                remap_place_with_optional_location(place, borrow_location, remap);
            }

            DiagnosticPayload::InvalidMutableAccess {
                place,
                conflicting_place,
                conflicting_location,
                ..
            } => {
                remap_place_with_optional_conflict(place, conflicting_place, remap);
                remap_optional_location(conflicting_location, remap);
            }

            DiagnosticPayload::InvalidConfig { key, reason } => {
                if let Some(key) = key {
                    *key = remap.get(*key);
                }
                reason.remap_string_ids(remap);
            }

            DiagnosticPayload::DeferredFeature { reason } => {
                reason.remap_string_ids(remap);
            }

            DiagnosticPayload::UnsupportedExternalFunction {
                function_name,
                package_path,
                backend_name,
            } => {
                *function_name = remap.get(*function_name);
                if let Some(package_path) = package_path {
                    *package_path = remap.get(*package_path);
                }
                *backend_name = remap.get(*backend_name);
            }

            DiagnosticPayload::DependencyAliasCaseMismatch { alias, symbol } => {
                *alias = remap.get(*alias);
                *symbol = remap.get(*symbol);
            }

            DiagnosticPayload::InvalidNumberLiteral { literal_text, .. } => {
                *literal_text = remap.get(*literal_text);
            }

            DiagnosticPayload::InvalidStyleDirective {
                directive_name,
                supported_directives,
            } => {
                *directive_name = remap.get(*directive_name);
                *supported_directives = remap.get(*supported_directives);
            }

            DiagnosticPayload::InvalidMapType { reason } => {
                reason.remap_string_ids(remap);
            }

            DiagnosticPayload::InvalidMapLiteral { reason } => {
                reason.remap_string_ids(remap);
            }

            DiagnosticPayload::MissingClosingDelimiter { expected_delimiter } => {
                *expected_delimiter = remap.get(*expected_delimiter);
            }

            DiagnosticPayload::UnexpectedEndOfFile { expected_delimiter } => {
                if let Some(expected_delimiter) = expected_delimiter {
                    *expected_delimiter = remap.get(*expected_delimiter);
                }
            }

            DiagnosticPayload::LegacyDependencyClause { replacement, .. } => {
                if let Some(replacement) = replacement {
                    *replacement = remap.get(*replacement);
                }
            }

            DiagnosticPayload::InvalidCharacter { .. }
            | DiagnosticPayload::InvalidStringEscape { .. }
            | DiagnosticPayload::InvalidGenericApplication { .. }
            | DiagnosticPayload::InvalidPath { .. }
            | DiagnosticPayload::InvalidDependencyClause { .. }
            | DiagnosticPayload::InvalidCollectionType { .. }
            | DiagnosticPayload::InvalidGenericParameter { .. }
            | DiagnosticPayload::InvalidStructDefaultValue => {}

            DiagnosticPayload::InvalidTemplateStructure { .. } => {}

            DiagnosticPayload::InvalidChoiceVariant {
                choice_name,
                variant_name,
                available_variants,
                ..
            } => {
                if let Some(name) = choice_name {
                    *name = remap.get(*name);
                }
                if let Some(name) = variant_name {
                    *name = remap.get(*name);
                }
                for variant in available_variants {
                    *variant = remap.get(*variant);
                }
            }

            DiagnosticPayload::InvalidTypeAnnotation { reason, .. } => {
                if let InvalidTypeAnnotationReason::InvalidTokenAfterName { token }
                | InvalidTypeAnnotationReason::ExpectedTypeAnnotation { found: token } = reason
                {
                    token.remap_string_ids(remap);
                }
            }

            DiagnosticPayload::InvalidTemplateDirective {
                directive_name,
                reason,
            } => {
                if let Some(directive_name) = directive_name {
                    *directive_name = remap.get(*directive_name);
                }
                if let InvalidTemplateDirectiveReason::InvalidArgument { detail } = reason
                    && let Some(detail) = detail
                {
                    *detail = remap.get(*detail);
                }
            }

            DiagnosticPayload::InvalidSignatureMember { .. } => {}

            DiagnosticPayload::InvalidFunctionSignature { reason } => {
                if let InvalidFunctionSignatureReason::MissingArrowOrColon { found }
                | InvalidFunctionSignatureReason::MissingCommaOrColon { found } = reason
                {
                    found.remap_string_ids(remap);
                }
            }

            DiagnosticPayload::MissingDeclarationInitializer { name } => {
                *name = remap.get(*name);
            }

            DiagnosticPayload::CircularDependency { path } => {
                path.remap_string_ids(remap);
            }

            DiagnosticPayload::NamespaceMisuse { name, .. } => {
                *name = remap.get(*name);
            }

            DiagnosticPayload::ShadowedName {
                name,
                first_location,
            } => {
                *name = remap.get(*name);
                first_location.remap_string_ids(remap);
            }

            DiagnosticPayload::ReservedNameCollision { name, .. } => {
                *name = remap.get(*name);
            }

            DiagnosticPayload::InvalidThisUsage { .. }
            | DiagnosticPayload::InvalidTraitKeywordUsage { .. }
            | DiagnosticPayload::InvalidReceiverDeclaration { .. }
            | DiagnosticPayload::InvalidCopyTarget { .. } => {}

            DiagnosticPayload::DuplicatePublicExport {
                name,
                first_location,
            } => {
                *name = remap.get(*name);
                first_location.remap_string_ids(remap);
            }

            DiagnosticPayload::PrivateTypeInExportedApi { exported_name, .. } => {
                *exported_name = remap.get(*exported_name);
            }

            DiagnosticPayload::InvalidControlFlowStatement { .. }
            | DiagnosticPayload::InvalidFallibleHandling { .. } => {}

            DiagnosticPayload::CompileTimeEvaluationError { operation, .. } => {
                if let Some(operation) = operation {
                    *operation = remap.get(*operation);
                }
            }

            DiagnosticPayload::InvalidDeclaration { name, reason } => {
                if let Some(name) = name {
                    *name = remap.get(*name);
                }
                match reason {
                    InvalidDeclarationReason::UnusedGenericParameter { parameter_name }
                    | InvalidDeclarationReason::InvalidGenericParameterName { parameter_name }
                    | InvalidDeclarationReason::DuplicateGenericParameter { parameter_name }
                    | InvalidDeclarationReason::GenericParameterNameCollision { parameter_name }
                    | InvalidDeclarationReason::ReservedGenericParameterName { parameter_name }
                    | InvalidDeclarationReason::ExternalTypeAlias {
                        type_name: parameter_name,
                    } => {
                        *parameter_name = remap.get(*parameter_name);
                    }
                    _ => {}
                }
            }

            DiagnosticPayload::InvalidAssignmentTarget {
                target_name,
                field_name,
                root_binding_name,
                declaration_location,
                ..
            } => {
                if let Some(name) = target_name {
                    *name = remap.get(*name);
                }
                if let Some(name) = field_name {
                    *name = remap.get(*name);
                }
                if let Some(name) = root_binding_name {
                    *name = remap.get(*name);
                }
                remap_optional_location(declaration_location, remap);
            }

            DiagnosticPayload::InvalidMultiBind {
                target_name: name, ..
            }
            | DiagnosticPayload::InvalidBuiltinCall {
                builtin_name: name, ..
            }
            | DiagnosticPayload::InvalidTemplateSlot {
                slot_name: name, ..
            } => {
                if let Some(name) = name {
                    *name = remap.get(*name);
                }
            }

            // InvalidFieldAccess carries both a field_name and a known_fields list,
            // both of which need remapping.
            DiagnosticPayload::InvalidFieldAccess {
                field_name: name,
                known_fields,
                ..
            } => {
                if let Some(name) = name {
                    *name = remap.get(*name);
                }
                for field in known_fields {
                    *field = remap.get(*field);
                }
            }

            DiagnosticPayload::InvalidReceiverCall {
                receiver_type,
                method_name,
                receiver_binding_name,
                ..
            } => {
                if let Some(receiver_type) = receiver_type {
                    *receiver_type = remap.get(*receiver_type);
                }
                if let Some(method_name) = method_name {
                    *method_name = remap.get(*method_name);
                }
                if let Some(receiver_binding_name) = receiver_binding_name {
                    *receiver_binding_name = remap.get(*receiver_binding_name);
                }
            }

            DiagnosticPayload::InvalidMatchPattern {
                variant_name,
                scrutinee_name,
                ..
            } => {
                if let Some(variant_name) = variant_name {
                    *variant_name = remap.get(*variant_name);
                }
                if let Some(scrutinee_name) = scrutinee_name {
                    *scrutinee_name = remap.get(*scrutinee_name);
                }
            }

            DiagnosticPayload::NonExhaustiveMatch {
                missing_variants, ..
            } => {
                for variant in missing_variants {
                    *variant = remap.get(*variant);
                }
            }

            DiagnosticPayload::EmptyCollectionTypeAmbiguity
            | DiagnosticPayload::UnsupportedOperatorTypes { .. }
            | DiagnosticPayload::InvalidFallibleOperand { .. }
            | DiagnosticPayload::InvalidCast { .. }
            | DiagnosticPayload::InvalidReturnShape { .. } => {}

            DiagnosticPayload::InvalidGenericInstantiation { type_name, reason } => {
                if let Some(type_name) = type_name {
                    *type_name = remap.get(*type_name);
                }
                match reason {
                    InvalidGenericInstantiationReason::CannotInferArguments {
                        missing_parameters,
                    }
                    | InvalidGenericInstantiationReason::CannotInferFunctionArguments {
                        missing_parameters,
                    } => {
                        for parameter in missing_parameters {
                            *parameter = remap.get(*parameter);
                        }
                    }
                    InvalidGenericInstantiationReason::ConflictingInference {
                        parameter_name,
                        current_evidence_location,
                        previous_evidence_location,
                        ..
                    } => {
                        *parameter_name = remap.get(*parameter_name);
                        current_evidence_location.remap_string_ids(remap);
                        if let Some(previous_evidence_location) = previous_evidence_location {
                            previous_evidence_location.remap_string_ids(remap);
                        }
                    }
                    InvalidGenericInstantiationReason::MissingTraitEvidence {
                        parameter_name,
                        trait_name,
                        ..
                    }
                    | InvalidGenericInstantiationReason::MissingNominalTraitEvidence {
                        parameter_name,
                        trait_name,
                        ..
                    } => {
                        *parameter_name = remap.get(*parameter_name);
                        *trait_name = remap.get(*trait_name);
                    }
                    InvalidGenericInstantiationReason::WrongArgumentCount { .. }
                    | InvalidGenericInstantiationReason::TypeDoesNotAcceptArguments
                    | InvalidGenericInstantiationReason::OptionTypeSyntaxNotSupported
                    | InvalidGenericInstantiationReason::ResultTypeSyntaxNotSupported
                    | InvalidGenericInstantiationReason::ExternalTypeArgumentsUnsupported
                    | InvalidGenericInstantiationReason::MissingTypeArguments
                    | InvalidGenericInstantiationReason::RecursiveFunctionInstantiation
                    | InvalidGenericInstantiationReason::ExplicitCallTypeArgumentsUnsupported
                    | InvalidGenericInstantiationReason::GenericFunctionValueDeferred => {}
                }
            }

            DiagnosticPayload::IncompatibleChoiceComparison { reason, .. } => {
                if let IncompatibleChoiceComparisonReason::PayloadEqualityNotSupported {
                    field_name,
                    ..
                } = reason
                {
                    *field_name = remap.get(*field_name);
                }
            }

            DiagnosticPayload::InvalidCallShape {
                reason,
                callee_name,
            } => {
                if let Some(callee_name) = callee_name {
                    *callee_name = remap.get(*callee_name);
                }
                match reason {
                    InvalidCallShapeReason::MissingArgument { parameter_name, .. }
                    | InvalidCallShapeReason::DuplicateArgument { parameter_name, .. }
                    | InvalidCallShapeReason::MutableAccessRequired { parameter_name, .. }
                    | InvalidCallShapeReason::MutableAccessNotAllowed { parameter_name, .. }
                    | InvalidCallShapeReason::MutableAccessOnNonPlace { parameter_name, .. }
                    | InvalidCallShapeReason::ReactiveSourceRequired { parameter_name, .. } => {
                        if let Some(parameter_name) = parameter_name {
                            *parameter_name = remap.get(*parameter_name);
                        }
                    }
                    InvalidCallShapeReason::MutableAccessOnImmutablePlace {
                        parameter_name,
                        binding_name,
                        ..
                    }
                    | InvalidCallShapeReason::ImmutablePlaceMutableAccessRequired {
                        parameter_name,
                        binding_name,
                        ..
                    } => {
                        if let Some(parameter_name) = parameter_name {
                            *parameter_name = remap.get(*parameter_name);
                        }
                        if let Some(binding_name) = binding_name {
                            *binding_name = remap.get(*binding_name);
                        }
                    }
                    InvalidCallShapeReason::ExtraPositionalArgument { .. }
                    | InvalidCallShapeReason::PositionalAfterNamed
                    | InvalidCallShapeReason::NamedArgumentsNotSupported => {}
                    InvalidCallShapeReason::NamedArgumentNotFound {
                        name,
                        known_parameters,
                    } => {
                        *name = remap.get(*name);
                        for parameter_name in known_parameters {
                            *parameter_name = remap.get(*parameter_name);
                        }
                    }
                }
            }

            DiagnosticPayload::InvalidRangeOperand { .. } => {}

            DiagnosticPayload::UnsupportedBuilderPackage { package_path } => {
                *package_path = remap.get(*package_path);
            }

            DiagnosticPayload::UnsupportedBackendFeature { backend_name, .. } => {
                *backend_name = remap.get(*backend_name);
            }

            DiagnosticPayload::InvalidPageMetadata { key, .. } => {
                *key = remap.get(*key);
            }

            DiagnosticPayload::InvalidCompileTimePath { path, reason } => {
                path.remap_string_ids(remap);
                reason.remap_string_ids(remap);
            }

            DiagnosticPayload::DirectSymbolPathImport { path }
            | DiagnosticPayload::InvalidNamespaceDefaultName { path }
            | DiagnosticPayload::ExplicitMothExtension { path } => {
                path.remap_string_ids(remap);
            }

            DiagnosticPayload::ExplicitSourceExtension { path, extension }
            | DiagnosticPayload::UnsupportedSourceFileKind { path, extension }
            | DiagnosticPayload::InvalidSourceFileEntry { path, extension }
            | DiagnosticPayload::UnsupportedExternalExtension { path, extension } => {
                path.remap_string_ids(remap);
                *extension = remap.get(*extension);
            }

            DiagnosticPayload::InvalidExternalModule { path, message } => {
                path.remap_string_ids(remap);
                *message = remap.get(*message);
            }

            DiagnosticPayload::DuplicateImportSurfaceMember {
                surface_path,
                member_name,
            } => {
                surface_path.remap_string_ids(remap);
                *member_name = remap.get(*member_name);
            }

            DiagnosticPayload::DependencyNamespaceUsedAsValue { record_name }
            | DiagnosticPayload::ConstRecordUsedAsValue { record_name }
            | DiagnosticPayload::NestedDependencyTraversal { record_name } => {
                *record_name = remap.get(*record_name);
            }
            DiagnosticPayload::NamespaceTypeValueMisuse { name, .. } => {
                *name = remap.get(*name);
            }

            DiagnosticPayload::UnknownTrait { name } => {
                *name = remap.get(*name);
            }

            DiagnosticPayload::DuplicateTraitRequirement {
                trait_name,
                requirement_name,
                first_location,
            } => {
                *trait_name = remap.get(*trait_name);
                *requirement_name = remap.get(*requirement_name);
                first_location.remap_string_ids(remap);
            }

            DiagnosticPayload::TraitPrivateSurfaceLeak { trait_name, .. } => {
                *trait_name = remap.get(*trait_name);
            }

            DiagnosticPayload::GenericBoundPrivateSurfaceLeak {
                function_name,
                trait_name,
            } => {
                *function_name = remap.get(*function_name);
                *trait_name = remap.get(*trait_name);
            }

            DiagnosticPayload::UnsupportedTraitFeature {
                trait_name,
                feature,
            } => {
                *trait_name = remap.get(*trait_name);
                *feature = remap.get(*feature);
            }

            DiagnosticPayload::InvalidTraitConformance {
                target_name,
                trait_name,
                reason,
            } => {
                *target_name = remap.get(*target_name);
                if let Some(trait_name) = trait_name {
                    *trait_name = remap.get(*trait_name);
                }
                reason.remap_string_ids(remap);
            }

            DiagnosticPayload::InvalidTraitIncompatibility {
                subject_name,
                incompatible_trait_name,
                ..
            } => {
                *subject_name = remap.get(*subject_name);
                if let Some(incompatible_trait_name) = incompatible_trait_name {
                    *incompatible_trait_name = remap.get(*incompatible_trait_name);
                }
            }

            DiagnosticPayload::TraitNameUsedAsType { trait_name } => {
                *trait_name = remap.get(*trait_name);
            }

            DiagnosticPayload::InvalidExpression { .. }
            | DiagnosticPayload::ExpectedSymbolStatement
            | DiagnosticPayload::MissingCollectionItem => {}

            DiagnosticPayload::MissingOperatorOperand { operator, .. } => {
                *operator = remap.get(*operator);
            }

            DiagnosticPayload::InvalidStandaloneStatement { .. }
            | DiagnosticPayload::InvalidMatchArm { .. }
            | DiagnosticPayload::InvalidLoopHeader { .. }
            | DiagnosticPayload::InvalidStatementPosition { .. } => {}

            DiagnosticPayload::CommonSyntaxMistake { reason } => {
                reason.remap_string_ids(remap);
            }

            DiagnosticPayload::InfrastructureError { .. } => {
                // Infrastructure payloads carry rendered strings; no interned IDs to remap.
            }
        }
    }

    /// Rebind every source location carried by a diagnostic payload to one final file scope.
    ///
    /// Most payloads carry semantic names only; the variants below retain a secondary authored
    /// span that must follow the diagnostic's primary location through synthetic identity rebinding.
    pub(crate) fn rebind_source_identity(&mut self, logical_path: &InternedPath) {
        match self {
            DiagnosticPayload::DuplicateDeclaration { first_location, .. }
            | DiagnosticPayload::ImportNameCollision {
                previous_location: first_location,
                ..
            } => rebind_optional_location(first_location, logical_path),

            DiagnosticPayload::ShadowedName { first_location, .. }
            | DiagnosticPayload::DuplicatePublicExport { first_location, .. }
            | DiagnosticPayload::DuplicateTraitRequirement { first_location, .. } => {
                first_location.rebind_source_identity(logical_path)
            }

            DiagnosticPayload::DuplicateMothTemplateInputPath { first_location, .. } => {
                first_location.rebind_source_identity(logical_path)
            }

            DiagnosticPayload::MultipleMutableBorrows {
                existing_location, ..
            }
            | DiagnosticPayload::UseAfterPossibleMove {
                move_location: existing_location,
                ..
            }
            | DiagnosticPayload::MoveWhileBorrowed {
                borrow_location: existing_location,
                ..
            }
            | DiagnosticPayload::WholeObjectBorrowConflict {
                part_location: existing_location,
                ..
            }
            | DiagnosticPayload::InvalidMutableAccess {
                conflicting_location: existing_location,
                ..
            } => rebind_optional_location(existing_location, logical_path),

            DiagnosticPayload::SharedMutableConflict {
                existing_location, ..
            } => rebind_optional_location(existing_location, logical_path),

            DiagnosticPayload::InvalidAssignmentTarget {
                declaration_location,
                ..
            } => rebind_optional_location(declaration_location, logical_path),

            DiagnosticPayload::InvalidGenericInstantiation { reason, .. } => {
                reason.rebind_source_identity(logical_path);
            }

            DiagnosticPayload::None
            | DiagnosticPayload::ExpectedToken { .. }
            | DiagnosticPayload::UnexpectedToken { .. }
            | DiagnosticPayload::UnexpectedTrailingComma
            | DiagnosticPayload::UnescapedImplicitTemplateClose { .. }
            | DiagnosticPayload::UnknownName { .. }
            | DiagnosticPayload::TypeMismatch { .. }
            | DiagnosticPayload::MissingImportTarget { .. }
            | DiagnosticPayload::AmbiguousImportTarget { .. }
            | DiagnosticPayload::BareFileImport { .. }
            | DiagnosticPayload::DirectSpecialFileImport { .. }
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
            | DiagnosticPayload::UnsupportedExternalExtension { .. }
            | DiagnosticPayload::InvalidExternalModule { .. }
            | DiagnosticPayload::BorrowConflict { .. }
            | DiagnosticPayload::UseOfUninitializedLocal { .. }
            | DiagnosticPayload::InvalidConfig { .. }
            | DiagnosticPayload::DeferredFeature { .. }
            | DiagnosticPayload::UnsupportedExternalFunction { .. }
            | DiagnosticPayload::UnusedName { .. }
            | DiagnosticPayload::UnreachableMatchArm
            | DiagnosticPayload::MothFilePathInTemplateOutput { .. }
            | DiagnosticPayload::LargeTrackedAsset { .. }
            | DiagnosticPayload::IdentifierNamingConvention { .. }
            | DiagnosticPayload::DependencyAliasCaseMismatch { .. }
            | DiagnosticPayload::MalformedTemplate { .. }
            | DiagnosticPayload::InvalidCharacter { .. }
            | DiagnosticPayload::InvalidStringEscape { .. }
            | DiagnosticPayload::InvalidNumberLiteral { .. }
            | DiagnosticPayload::InvalidStyleDirective { .. }
            | DiagnosticPayload::MissingClosingDelimiter { .. }
            | DiagnosticPayload::InvalidGenericApplication { .. }
            | DiagnosticPayload::UnexpectedEndOfFile { .. }
            | DiagnosticPayload::InvalidPath { .. }
            | DiagnosticPayload::InvalidDependencyClause { .. }
            | DiagnosticPayload::LegacyDependencyClause { .. }
            | DiagnosticPayload::InvalidTypeAnnotation { .. }
            | DiagnosticPayload::InvalidCollectionType { .. }
            | DiagnosticPayload::InvalidMapType { .. }
            | DiagnosticPayload::InvalidMapLiteral { .. }
            | DiagnosticPayload::InvalidGenericParameter { .. }
            | DiagnosticPayload::InvalidTemplateDirective { .. }
            | DiagnosticPayload::InvalidTemplateStructure { .. }
            | DiagnosticPayload::InvalidSignatureMember { .. }
            | DiagnosticPayload::InvalidFunctionSignature { .. }
            | DiagnosticPayload::InvalidChoiceVariant { .. }
            | DiagnosticPayload::InvalidStructDefaultValue
            | DiagnosticPayload::MissingDeclarationInitializer { .. }
            | DiagnosticPayload::CircularDependency { .. }
            | DiagnosticPayload::NamespaceMisuse { .. }
            | DiagnosticPayload::ReservedNameCollision { .. }
            | DiagnosticPayload::InvalidThisUsage { .. }
            | DiagnosticPayload::InvalidReceiverDeclaration { .. }
            | DiagnosticPayload::InvalidControlFlowStatement { .. }
            | DiagnosticPayload::InvalidDeclaration { .. }
            | DiagnosticPayload::InvalidMultiBind { .. }
            | DiagnosticPayload::InvalidBuiltinCall { .. }
            | DiagnosticPayload::InvalidCast { .. }
            | DiagnosticPayload::InvalidReceiverCall { .. }
            | DiagnosticPayload::InvalidCopyTarget { .. }
            | DiagnosticPayload::InvalidFieldAccess { .. }
            | DiagnosticPayload::InvalidMatchPattern { .. }
            | DiagnosticPayload::NonExhaustiveMatch { .. }
            | DiagnosticPayload::InvalidFallibleHandling { .. }
            | DiagnosticPayload::InvalidTemplateSlot { .. }
            | DiagnosticPayload::CompileTimeEvaluationError { .. }
            | DiagnosticPayload::EmptyCollectionTypeAmbiguity
            | DiagnosticPayload::UnsupportedOperatorTypes { .. }
            | DiagnosticPayload::InvalidFallibleOperand { .. }
            | DiagnosticPayload::IncompatibleChoiceComparison { .. }
            | DiagnosticPayload::InvalidCallShape { .. }
            | DiagnosticPayload::InvalidReturnShape { .. }
            | DiagnosticPayload::InvalidRangeOperand { .. }
            | DiagnosticPayload::UnsupportedBuilderPackage { .. }
            | DiagnosticPayload::UnsupportedBackendFeature { .. }
            | DiagnosticPayload::InvalidPageMetadata { .. }
            | DiagnosticPayload::InvalidCompileTimePath { .. }
            | DiagnosticPayload::DependencyNamespaceUsedAsValue { .. }
            | DiagnosticPayload::ConstRecordUsedAsValue { .. }
            | DiagnosticPayload::NestedDependencyTraversal { .. }
            | DiagnosticPayload::NamespaceTypeValueMisuse { .. }
            | DiagnosticPayload::UnknownTrait { .. }
            | DiagnosticPayload::TraitPrivateSurfaceLeak { .. }
            | DiagnosticPayload::GenericBoundPrivateSurfaceLeak { .. }
            | DiagnosticPayload::UnsupportedTraitFeature { .. }
            | DiagnosticPayload::InvalidTraitKeywordUsage { .. }
            | DiagnosticPayload::PrivateTypeInExportedApi { .. }
            | DiagnosticPayload::InvalidTraitConformance { .. }
            | DiagnosticPayload::InvalidTraitIncompatibility { .. }
            | DiagnosticPayload::TraitNameUsedAsType { .. }
            | DiagnosticPayload::InvalidExpression { .. }
            | DiagnosticPayload::MissingOperatorOperand { .. }
            | DiagnosticPayload::InvalidStandaloneStatement { .. }
            | DiagnosticPayload::ExpectedSymbolStatement
            | DiagnosticPayload::MissingCollectionItem
            | DiagnosticPayload::InvalidMatchArm { .. }
            | DiagnosticPayload::InvalidLoopHeader { .. }
            | DiagnosticPayload::InvalidStatementPosition { .. }
            | DiagnosticPayload::CommonSyntaxMistake { .. }
            | DiagnosticPayload::InfrastructureError { .. } => {}
        }
    }
}

impl InvalidGenericInstantiationReason {
    fn rebind_source_identity(&mut self, logical_path: &InternedPath) {
        if let Self::ConflictingInference {
            current_evidence_location,
            previous_evidence_location,
            ..
        } = self
        {
            current_evidence_location.rebind_source_identity(logical_path);
            if let Some(previous_evidence_location) = previous_evidence_location {
                previous_evidence_location.rebind_source_identity(logical_path);
            }
        }
    }
}

fn remap_path_import_payload(path: &mut InternedPath, remap: &StringIdRemap) {
    path.remap_string_ids(remap);
}

fn remap_invalid_import_path_payload(
    path: &mut InternedPath,
    reason: &mut InvalidImportPathReason,
    remap: &StringIdRemap,
) {
    path.remap_string_ids(remap);
    reason.remap_string_ids(remap);
}

fn remap_single_place_borrow_payload(place: &mut DiagnosticPlace, remap: &StringIdRemap) {
    place.remap_string_ids(remap);
}

fn remap_shared_mutable_conflict_payload(
    place: &mut DiagnosticPlace,
    conflicting_place: &mut Option<DiagnosticPlace>,
    existing_location: &mut Option<SourceLocation>,
    remap: &StringIdRemap,
) {
    place.remap_string_ids(remap);
    remap_optional_place(conflicting_place, remap);
    remap_optional_location(existing_location, remap);
}

fn remap_whole_object_borrow_conflict_payload(
    whole_place: &mut DiagnosticPlace,
    part_place: &mut DiagnosticPlace,
    part_location: &mut Option<SourceLocation>,
    remap: &StringIdRemap,
) {
    whole_place.remap_string_ids(remap);
    part_place.remap_string_ids(remap);
    remap_optional_location(part_location, remap);
}

fn remap_place_with_optional_location(
    place: &mut DiagnosticPlace,
    location: &mut Option<SourceLocation>,
    remap: &StringIdRemap,
) {
    place.remap_string_ids(remap);
    remap_optional_location(location, remap);
}

fn remap_place_with_optional_conflict(
    place: &mut DiagnosticPlace,
    conflicting_place: &mut Option<DiagnosticPlace>,
    remap: &StringIdRemap,
) {
    place.remap_string_ids(remap);
    remap_optional_place(conflicting_place, remap);
}

fn remap_optional_place(place: &mut Option<DiagnosticPlace>, remap: &StringIdRemap) {
    if let Some(place) = place {
        place.remap_string_ids(remap);
    }
}

fn remap_optional_location(location: &mut Option<SourceLocation>, remap: &StringIdRemap) {
    if let Some(location) = location {
        location.remap_string_ids(remap);
    }
}

fn rebind_optional_location(location: &mut Option<SourceLocation>, logical_path: &InternedPath) {
    if let Some(location) = location {
        location.rebind_source_identity(logical_path);
    }
}
