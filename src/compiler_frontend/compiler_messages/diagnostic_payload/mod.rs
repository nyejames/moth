//! Typed diagnostic payload facts.
//!
//! WHAT: stores structured data needed to render or inspect diagnostics later.
//! WHY: compiler stages should carry stable IDs, source locations, and typed context rather than
//! pre-rendered strings or generic argument maps.

use crate::builder_surface::SourceFileKind;
use crate::compiler_frontend::compiler_messages::DiagnosticToken;
use crate::compiler_frontend::datatypes::ids::{GenericParameterId, TypeId};
use crate::compiler_frontend::symbols::path_interner::{PathId, PathIdRemap};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringIdRemap};

mod reason_keys;
mod remap;
mod types;

pub use types::*;

macro_rules! emit_diagnostic_payload_enum {
    (
        $remap_label:ident: $remap_name:ident;
        $(
            $category:ident::$kind:ident => {
                payload: $payload:ident;
                fields: {
                    $( $field:ident : $field_type:ty ),* $(,)?
                }
                bindings: { $( $binding:ident ),* $(,)? }
                remap: { $($remap:tt)* }
                descriptor: $descriptor:tt
            },
        )*
    ) => {
        #[derive(Clone, Debug, PartialEq)]
        pub enum DiagnosticPayload {
            None,

            ExpectedToken {
                expected: DiagnosticToken,
                found: Option<DiagnosticToken>,
            },
            UnexpectedToken {
                found: DiagnosticToken,
            },
            UnexpectedTrailingComma,
            UnescapedImplicitTemplateClose {
                source_kind: SourceFileKind,
            },
            UnknownName {
                name: StringId,
                namespace: NameNamespace,
            },
            TypeMismatch {
                expected: TypeId,
                found: TypeId,
                context: TypeMismatchContext,
            },
            DuplicateDeclaration {
                name: StringId,
            },
            ReservedBuiltinName {
                name: StringId,
            },

            MissingImportTarget {
                path: PathId,
            },
            AmbiguousImportTarget {
                path: PathId,
            },
            BareFileImport {
                path: PathId,
            },
            DirectSpecialFileImport {
                path: PathId,
            },
            ImportNameCollision {
                name: StringId,
            },
            NotExportedBySourceFile {
                symbol_path: PathId,
            },
            NotExportedByPublicSurface {
                requested_path: PathId,
                public_surface_name: StringId,
                public_surface_type: ImportPublicSurfaceType,
            },
            MissingModuleRootPublicSurface {
                symbol_path: PathId,
            },
            MissingPackageSymbol {
                symbol: StringId,
                package_path: StringId,
            },
            CrossModuleImportNotExported {
                symbol_path: PathId,
            },
            DirectSymbolPathImport {
                path: PathId,
            },
            InvalidNamespaceDefaultName {
                path: PathId,
            },
            DuplicateImportSurfaceMember {
                surface_path: PathId,
                member_name: StringId,
            },
            ExplicitMothExtension {
                path: PathId,
            },
            ExplicitSourceExtension {
                path: PathId,
                extension: StringId,
            },
            UnsupportedSourceFileKind {
                path: PathId,
                extension: StringId,
            },
            InvalidSourceFileEntry {
                path: PathId,
                extension: StringId,
            },
            MothTemplateInputsShareNoCommonAncestor {
                first_path: PathId,
                second_path: PathId,
            },
            DuplicateMothTemplateInputPath {
                path: PathId,
            },
            UnsupportedExternalExtension {
                path: PathId,
                extension: StringId,
            },

            BorrowConflict {
                place: DiagnosticPlace,
                existing_access: BorrowAccessKind,
                requested_access: BorrowAccessKind,
            },
            MultipleMutableBorrows {
                place: DiagnosticPlace,
                conflicting_place: Option<DiagnosticPlace>,
            },
            SharedMutableConflict {
                place: DiagnosticPlace,
                existing_access: BorrowAccessKind,
                requested_access: BorrowAccessKind,
                conflicting_place: Option<DiagnosticPlace>,
            },
            UseAfterPossibleMove {
                place: DiagnosticPlace,
            },
            MoveWhileBorrowed {
                place: DiagnosticPlace,
                existing_access: BorrowAccessKind,
            },
            WholeObjectBorrowConflict {
                whole_place: DiagnosticPlace,
                part_place: DiagnosticPlace,
            },
            UseOfUninitializedLocal {
                place: DiagnosticPlace,
            },

            UnsupportedExternalFunction {
                function_name: StringId,
                package_path: Option<StringId>,
                backend_name: StringId,
            },

            UnreachableMatchArm,
            IdentifierNamingConvention {
                name: StringId,
                expected_style: NamingConvention,
            },
            DependencyAliasCaseMismatch {
                alias: StringId,
                symbol: StringId,
            },

            SourceSpanCapacity {
                start: u32,
                length: u32,
                resource: SourceSpanCapacityResource,
            },
            InvalidCharacter {
                character: char,
            },
            InvalidStyleDirective {
                directive_name: StringId,
                supported_directives: StringId,
            },
            MissingClosingDelimiter {
                expected_delimiter: StringId,
            },
            UnexpectedEndOfFile {
                expected_delimiter: Option<StringId>,
            },
            InvalidPath {
                path_kind: PathKind,
            },

            InvalidStructDefaultValue,
            MissingDeclarationInitializer {
                name: StringId,
            },
            CircularDependency {
                path: PathId,
            },
            NamespaceMisuse {
                name: StringId,
                expected: NameNamespace,
                found: NameNamespace,
            },
            ShadowedName {
                name: StringId,
            },
            ReservedNameCollision {
                name: StringId,
                reserved_by: ReservedNameOwner,
            },
            EmptyCollectionTypeAmbiguity,
            UnsupportedOperatorTypes {
                operator: DiagnosticOperator,
                lhs: TypeId,
                rhs: Option<TypeId>,
            },
            InvalidRangeOperand {
                operand: RangeOperandKind,
                found_type: TypeId,
            },
            UnsupportedBuilderPackage {
                package_path: StringId,
            },
            DependencyNamespaceUsedAsValue {
                record_name: StringId,
            },
            ConstRecordUsedAsValue {
                record_name: StringId,
            },
            NestedDependencyTraversal {
                record_name: StringId,
            },
            NamespaceTypeValueMisuse {
                name: StringId,
                expected: NamespaceTypeValueMisuseKind,
                found: NamespaceTypeValueMisuseKind,
            },
            UnknownTrait {
                name: StringId,
            },
            DuplicateTraitRequirement {
                trait_name: StringId,
                requirement_name: StringId,
            },
            TraitPrivateSurfaceLeak {
                trait_name: StringId,
                surface_type: TypeId,
            },
            GenericBoundPrivateSurfaceLeak {
                function_name: StringId,
                trait_name: StringId,
            },
            UnsupportedTraitFeature {
                trait_name: StringId,
                feature: StringId,
            },
            DuplicatePublicExport {
                name: StringId,
            },
            PrivateTypeInExportedApi {
                exported_name: StringId,
                private_type: TypeId,
            },
            TraitNameUsedAsType {
                trait_name: StringId,
            },
            MissingOperatorOperand {
                operator: StringId,
                position: OperatorOperandPosition,
            },
            ExpectedSymbolStatement,
            MissingCollectionItem,

            $( $payload {
                $( $field: $field_type, )*
            }, )*
        }
    };
}

crate::define_reasoned_diagnostic_registry!(emit_diagnostic_payload_enum);

macro_rules! emit_reasoned_payload_stable_key {
    (
        $remap_label:ident: $remap_name:ident;
        $(
            $category:ident::$kind:ident => {
                payload: $payload:ident;
                fields: {
                    $( $field:ident : $field_type:ty ),* $(,)?
                }
                bindings: { $( $binding:ident ),* $(,)? }
                remap: { $($remap:tt)* }
                descriptor: $descriptor:tt
            },
        )*
    ) => {
        impl DiagnosticPayload {
            fn reasoned_stable_reason_key(&self) -> Option<&'static str> {
                match self {
                    $(
                        DiagnosticPayload::$payload { reason, .. } => {
                            Some(reason.stable_reason_key())
                        }
                    )*
                    _ => None,
                }
            }
        }
    };
}

crate::define_reasoned_diagnostic_registry!(emit_reasoned_payload_stable_key);

#[cfg(test)]
pub(super) fn stable_reason_keys_for_tests() -> &'static [&'static str] {
    reason_keys::stable_reason_keys_for_tests()
}

impl DiagnosticPayload {
    /// Return the stable, qualified key for a typed reason payload.
    ///
    /// Reasonless payloads deliberately return `None`; every reasoned variant is dispatched by
    /// the central registry above.
    pub(super) fn stable_reason_key(&self) -> Option<&'static str> {
        self.reasoned_stable_reason_key()
    }
}
