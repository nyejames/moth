//! String-table remapping for diagnostic payload facts.
//!
//! WHAT: walks payload variants and updates every interned string-bearing field after
//! string tables are merged.
//! WHY: keeping this traversal outside the payload declarations makes the diagnostic data
//! model easier to scan while preserving one canonical remap implementation.

use super::*;

macro_rules! emit_reasoned_payload_remap {
    (
        $remap_label:ident: $remap_name:ident;
        $(
            $category:ident::$kind:ident => {
                payload: $payload:ident;
                fields: { $( $field:ident : $field_type:ty ),* $(,)? }
                bindings: { $( $binding:ident ),* $(,)? }
                remap: { $($remap:tt)* }
                descriptor: $descriptor:tt
            },
        )*
    ) => {
        impl DiagnosticPayload {
            fn remap_reasoned_string_ids(&mut self, $remap_name: &StringIdRemap) -> bool {
                match self {
                    $(
                        DiagnosticPayload::$payload { $( $binding, )* .. } => {
                            $($remap)*
                            true
                        }
                    )*
                    _ => false,
                }
            }
        }
    };
}

crate::define_reasoned_diagnostic_registry!(emit_reasoned_payload_remap);

impl DiagnosticPayload {
    pub(crate) fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        if self.remap_reasoned_string_ids(remap) {
            return;
        }
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
            | DiagnosticPayload::ReservedBuiltinName { name } => {
                *name = remap.get(*name);
            }

            DiagnosticPayload::DuplicateDeclaration { name } => {
                *name = remap.get(*name);
            }
            DiagnosticPayload::IdentifierNamingConvention { name, .. } => {
                *name = remap.get(*name);
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
            | DiagnosticPayload::CrossModuleImportNotExported { symbol_path: path } => {
                remap_path_import_payload(path, remap);
            }

            DiagnosticPayload::DuplicateMothTemplateInputPath { path } => {
                remap_path_import_payload(path, remap);
            }
            DiagnosticPayload::MothTemplateInputsShareNoCommonAncestor {
                first_path,
                second_path,
            } => {
                remap_path_import_payload(first_path, remap);
                remap_path_import_payload(second_path, remap);
            }

            DiagnosticPayload::ImportNameCollision { name } => {
                *name = remap.get(*name);
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
                ..
            } => {
                remap_shared_mutable_conflict_payload(place, conflicting_place, remap);
            }

            DiagnosticPayload::WholeObjectBorrowConflict {
                whole_place,
                part_place,
            } => {
                whole_place.remap_string_ids(remap);
                part_place.remap_string_ids(remap);
            }

            DiagnosticPayload::MultipleMutableBorrows {
                place,
                conflicting_place,
            } => {
                remap_place_with_optional_conflict(place, conflicting_place, remap);
            }

            DiagnosticPayload::UseAfterPossibleMove { place } => {
                remap_single_place_borrow_payload(place, remap);
            }

            DiagnosticPayload::MoveWhileBorrowed { place, .. } => {
                remap_single_place_borrow_payload(place, remap);
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

            DiagnosticPayload::InvalidStyleDirective {
                directive_name,
                supported_directives,
            } => {
                *directive_name = remap.get(*directive_name);
                *supported_directives = remap.get(*supported_directives);
            }

            DiagnosticPayload::MissingClosingDelimiter { expected_delimiter } => {
                *expected_delimiter = remap.get(*expected_delimiter);
            }

            DiagnosticPayload::UnexpectedEndOfFile { expected_delimiter } => {
                if let Some(expected_delimiter) = expected_delimiter {
                    *expected_delimiter = remap.get(*expected_delimiter);
                }
            }

            DiagnosticPayload::SourceSpanCapacity { .. }
            | DiagnosticPayload::InvalidCharacter { .. }
            | DiagnosticPayload::InvalidPath { .. }
            | DiagnosticPayload::InvalidStructDefaultValue => {}

            DiagnosticPayload::MissingDeclarationInitializer { name } => {
                *name = remap.get(*name);
            }

            DiagnosticPayload::CircularDependency { path } => {
                path.remap_string_ids(remap);
            }

            DiagnosticPayload::NamespaceMisuse { name, .. } => {
                *name = remap.get(*name);
            }

            DiagnosticPayload::ShadowedName { name } => {
                *name = remap.get(*name);
            }

            DiagnosticPayload::ReservedNameCollision { name, .. } => {
                *name = remap.get(*name);
            }

            DiagnosticPayload::DuplicatePublicExport { name } => {
                *name = remap.get(*name);
            }

            DiagnosticPayload::PrivateTypeInExportedApi { exported_name, .. } => {
                *exported_name = remap.get(*exported_name);
            }

            // InvalidFieldAccess carries both a field_name and a known_fields list,
            // both of which need remapping.
            DiagnosticPayload::EmptyCollectionTypeAmbiguity
            | DiagnosticPayload::UnsupportedOperatorTypes { .. } => {}

            DiagnosticPayload::InvalidRangeOperand { .. } => {}

            DiagnosticPayload::UnsupportedBuilderPackage { package_path } => {
                *package_path = remap.get(*package_path);
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
            } => {
                *trait_name = remap.get(*trait_name);
                *requirement_name = remap.get(*requirement_name);
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

            DiagnosticPayload::TraitNameUsedAsType { trait_name } => {
                *trait_name = remap.get(*trait_name);
            }

            DiagnosticPayload::ExpectedSymbolStatement
            | DiagnosticPayload::MissingCollectionItem => {}

            DiagnosticPayload::MissingOperatorOperand { operator, .. } => {
                *operator = remap.get(*operator);
            }

            _ => unreachable!("reasoned payloads are handled by the central remap registry"),
        }
    }
}

fn remap_path_import_payload(path: &mut InternedPath, remap: &StringIdRemap) {
    path.remap_string_ids(remap);
}

fn remap_single_place_borrow_payload(place: &mut DiagnosticPlace, remap: &StringIdRemap) {
    place.remap_string_ids(remap);
}

fn remap_shared_mutable_conflict_payload(
    place: &mut DiagnosticPlace,
    conflicting_place: &mut Option<DiagnosticPlace>,
    remap: &StringIdRemap,
) {
    place.remap_string_ids(remap);
    remap_optional_place(conflicting_place, remap);
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
