//! Stable descriptor table for diagnostic kinds.
//!
//! WHAT: maps each diagnostic kind to the code, title, and default severity exposed at render
//! boundaries.
//! WHY: keeping the large mapping out of the taxonomy file makes the enum definitions easier to
//! scan while preserving one authoritative descriptor source.

use super::diagnostic_kind::{
    BorrowDiagnosticKind, ConfigDiagnosticKind, DeferredFeatureDiagnosticKind, DiagnosticKind,
    ImportDiagnosticKind, InfrastructureDiagnosticKind, RuleDiagnosticKind, SyntaxDiagnosticKind,
    TypeDiagnosticKind,
};
use crate::compiler_frontend::compiler_messages::{DiagnosticDescriptor, DiagnosticSeverity};

macro_rules! reasoned_diagnostic_kind {
    (Syntax::$kind:ident) => {
        DiagnosticKind::Syntax(SyntaxDiagnosticKind::$kind)
    };
    (Type::$kind:ident) => {
        DiagnosticKind::Type(TypeDiagnosticKind::$kind)
    };
    (Rule::$kind:ident) => {
        DiagnosticKind::Rule(RuleDiagnosticKind::$kind)
    };
    (Import::$kind:ident) => {
        DiagnosticKind::Import(ImportDiagnosticKind::$kind)
    };
    (Borrow::$kind:ident) => {
        DiagnosticKind::Borrow(BorrowDiagnosticKind::$kind)
    };
    (Config::$kind:ident) => {
        DiagnosticKind::Config(ConfigDiagnosticKind::$kind)
    };
    (DeferredFeature::$kind:ident) => {
        DiagnosticKind::DeferredFeature(DeferredFeatureDiagnosticKind::$kind)
    };
}

macro_rules! emit_reasoned_descriptors {
    (
        $remap_label:ident: $remap_name:ident;
        $( $records:tt )*
    ) => {
        emit_reasoned_descriptors!(@collect [] $( $records )*);
    };
    (@collect [$( $arms:tt )*]) => {
        fn reasoned_descriptor_for_kind(kind: DiagnosticKind) -> Option<DiagnosticDescriptor> {
            match kind {
                $( $arms )*
                _ => None,
            }
        }
    };
    (
        @collect [$( $arms:tt )*]
        Shared::$kind:ident => {
            payload: $payload:ident;
            fields: $fields:tt
            bindings: $bindings:tt
            remap: $remap:tt
            descriptor: none
        },
        $( $rest:tt )*
    ) => {
        emit_reasoned_descriptors!(@collect [$( $arms )*] $( $rest )*);
    };
    (
        @collect [$( $arms:tt )*]
        $category:ident::$kind:ident => {
            payload: $payload:ident;
            fields: $fields:tt
            bindings: $bindings:tt
            remap: $remap:tt
            descriptor: {
                $code:literal, $title:literal, $severity:ident
            }
        },
        $( $rest:tt )*
    ) => {
        emit_reasoned_descriptors!(
            @collect [
                $( $arms )*
                reasoned_diagnostic_kind!($category::$kind) => Some(DiagnosticDescriptor::new(
                    $code,
                    $title,
                    DiagnosticSeverity::$severity,
                )),
            ]
            $( $rest )*
        );
    };
}

crate::define_reasoned_diagnostic_registry!(emit_reasoned_descriptors);

pub(super) fn descriptor_for_kind(kind: DiagnosticKind) -> DiagnosticDescriptor {
    if let Some(descriptor) = reasoned_descriptor_for_kind(kind) {
        return descriptor;
    }
    match kind {
        DiagnosticKind::Syntax(kind) => syntax_descriptor(kind),
        DiagnosticKind::Type(kind) => type_descriptor(kind),
        DiagnosticKind::Rule(kind) => rule_descriptor(kind),
        DiagnosticKind::Import(kind) => import_descriptor(kind),
        DiagnosticKind::Borrow(kind) => borrow_descriptor(kind),
        DiagnosticKind::Config(kind) => config_descriptor(kind),
        DiagnosticKind::Infrastructure(kind) => infrastructure_descriptor(kind),
        DiagnosticKind::DeferredFeature(kind) => deferred_feature_descriptor(kind),
    }
}

fn syntax_descriptor(kind: SyntaxDiagnosticKind) -> DiagnosticDescriptor {
    match kind {
        SyntaxDiagnosticKind::ExpectedToken => DiagnosticDescriptor::new(
            "MOTH-SYNTAX-0001",
            "Expected token",
            DiagnosticSeverity::Error,
        ),
        SyntaxDiagnosticKind::UnexpectedToken => DiagnosticDescriptor::new(
            "MOTH-SYNTAX-0002",
            "Unexpected token",
            DiagnosticSeverity::Error,
        ),
        SyntaxDiagnosticKind::UnexpectedTrailingComma => DiagnosticDescriptor::new(
            "MOTH-SYNTAX-0003",
            "Unexpected trailing comma",
            DiagnosticSeverity::Error,
        ),
        SyntaxDiagnosticKind::MalformedCssTemplate => DiagnosticDescriptor::new(
            "MOTH-SYNTAX-0004",
            "Malformed CSS template",
            DiagnosticSeverity::Warning,
        ),
        SyntaxDiagnosticKind::MalformedHtmlTemplate => DiagnosticDescriptor::new(
            "MOTH-SYNTAX-0005",
            "Malformed HTML template",
            DiagnosticSeverity::Warning,
        ),
        SyntaxDiagnosticKind::UnterminatedStringLiteral => DiagnosticDescriptor::new(
            "MOTH-SYNTAX-0006",
            "Unterminated string literal",
            DiagnosticSeverity::Error,
        ),
        SyntaxDiagnosticKind::InvalidCharacter => DiagnosticDescriptor::new(
            "MOTH-SYNTAX-0007",
            "Invalid character",
            DiagnosticSeverity::Error,
        ),
        SyntaxDiagnosticKind::InvalidCharLiteral => DiagnosticDescriptor::new(
            "MOTH-SYNTAX-0009",
            "Invalid character literal",
            DiagnosticSeverity::Error,
        ),
        SyntaxDiagnosticKind::InvalidStyleDirective => DiagnosticDescriptor::new(
            "MOTH-SYNTAX-0010",
            "Invalid style directive",
            DiagnosticSeverity::Error,
        ),
        SyntaxDiagnosticKind::InvalidIdentifier => DiagnosticDescriptor::new(
            "MOTH-SYNTAX-0011",
            "Invalid identifier",
            DiagnosticSeverity::Error,
        ),
        SyntaxDiagnosticKind::MissingClosingDelimiter => DiagnosticDescriptor::new(
            "MOTH-SYNTAX-0012",
            "Missing closing delimiter",
            DiagnosticSeverity::Error,
        ),
        SyntaxDiagnosticKind::UnexpectedTokenInDeclaration => DiagnosticDescriptor::new(
            "MOTH-SYNTAX-0013",
            "Unexpected token in declaration",
            DiagnosticSeverity::Error,
        ),
        SyntaxDiagnosticKind::UnexpectedEndOfFile => DiagnosticDescriptor::new(
            "MOTH-SYNTAX-0017",
            "Unexpected end of file",
            DiagnosticSeverity::Error,
        ),
        SyntaxDiagnosticKind::InvalidPath => DiagnosticDescriptor::new(
            "MOTH-SYNTAX-0018",
            "Invalid path",
            DiagnosticSeverity::Error,
        ),
        SyntaxDiagnosticKind::MissingOperatorOperand => DiagnosticDescriptor::new(
            "MOTH-SYNTAX-0024",
            "Missing operator operand",
            DiagnosticSeverity::Error,
        ),
        SyntaxDiagnosticKind::ExpectedSymbolStatement => DiagnosticDescriptor::new(
            "MOTH-SYNTAX-0026",
            "Expected symbol statement",
            DiagnosticSeverity::Error,
        ),
        SyntaxDiagnosticKind::MissingCollectionItem => DiagnosticDescriptor::new(
            "MOTH-SYNTAX-0027",
            "Missing collection item",
            DiagnosticSeverity::Error,
        ),
        SyntaxDiagnosticKind::UnescapedImplicitTemplateClose => DiagnosticDescriptor::new(
            "MOTH-SYNTAX-0032",
            "Unescaped implicit template close",
            DiagnosticSeverity::Error,
        ),
        SyntaxDiagnosticKind::SourceSpanCapacity => DiagnosticDescriptor::new(
            "MOTH-SYNTAX-0036",
            "Source span capacity exceeded",
            DiagnosticSeverity::Error,
        ),
        _ => unreachable!("reasoned syntax descriptors come from the central registry"),
    }
}

fn type_descriptor(kind: TypeDiagnosticKind) -> DiagnosticDescriptor {
    match kind {
        TypeDiagnosticKind::TypeMismatch => {
            DiagnosticDescriptor::new("MOTH-TYPE-0001", "Type mismatch", DiagnosticSeverity::Error)
        }
        TypeDiagnosticKind::EmptyCollectionTypeAmbiguity => DiagnosticDescriptor::new(
            "MOTH-TYPE-0002",
            "Empty collection type ambiguity",
            DiagnosticSeverity::Error,
        ),
        TypeDiagnosticKind::UnsupportedOperatorTypes => DiagnosticDescriptor::new(
            "MOTH-TYPE-0003",
            "Unsupported operator types",
            DiagnosticSeverity::Error,
        ),
        _ => unreachable!("reasoned type descriptors come from the central registry"),
    }
}

fn rule_descriptor(kind: RuleDiagnosticKind) -> DiagnosticDescriptor {
    match kind {
        RuleDiagnosticKind::UnknownName => {
            DiagnosticDescriptor::new("MOTH-RULE-0001", "Unknown name", DiagnosticSeverity::Error)
        }
        RuleDiagnosticKind::DuplicateDeclaration => DiagnosticDescriptor::new(
            "MOTH-RULE-0002",
            "Duplicate declaration",
            DiagnosticSeverity::Error,
        ),
        RuleDiagnosticKind::IdentifierNamingConvention => DiagnosticDescriptor::new(
            "MOTH-RULE-0021",
            "Identifier naming convention",
            DiagnosticSeverity::Warning,
        ),
        RuleDiagnosticKind::UnreachableMatchArm => DiagnosticDescriptor::new(
            "MOTH-RULE-0022",
            "Unreachable match arm",
            DiagnosticSeverity::Warning,
        ),
        RuleDiagnosticKind::InvalidTopLevelRuntimeStatement => DiagnosticDescriptor::new(
            "MOTH-RULE-0023",
            "Invalid top-level runtime statement",
            DiagnosticSeverity::Error,
        ),
        RuleDiagnosticKind::ReservedBuiltinName => DiagnosticDescriptor::new(
            "MOTH-RULE-0027",
            "Reserved builtin name",
            DiagnosticSeverity::Error,
        ),
        RuleDiagnosticKind::InvalidStructDefaultValue => DiagnosticDescriptor::new(
            "MOTH-RULE-0030",
            "Invalid struct default value",
            DiagnosticSeverity::Error,
        ),
        RuleDiagnosticKind::MissingDeclarationInitializer => DiagnosticDescriptor::new(
            "MOTH-RULE-0031",
            "Missing declaration initializer",
            DiagnosticSeverity::Error,
        ),
        RuleDiagnosticKind::CircularDependency => DiagnosticDescriptor::new(
            "MOTH-RULE-0033",
            "Circular dependency",
            DiagnosticSeverity::Error,
        ),
        RuleDiagnosticKind::UnknownValueName => DiagnosticDescriptor::new(
            "MOTH-RULE-0034",
            "Unknown value name",
            DiagnosticSeverity::Error,
        ),
        RuleDiagnosticKind::UnknownTypeName => DiagnosticDescriptor::new(
            "MOTH-RULE-0035",
            "Unknown type name",
            DiagnosticSeverity::Error,
        ),
        RuleDiagnosticKind::ValueUsedAsType => DiagnosticDescriptor::new(
            "MOTH-RULE-0036",
            "Value used as type",
            DiagnosticSeverity::Error,
        ),
        RuleDiagnosticKind::TypeUsedAsValue => DiagnosticDescriptor::new(
            "MOTH-RULE-0037",
            "Type used as value",
            DiagnosticSeverity::Error,
        ),
        RuleDiagnosticKind::ShadowedName => {
            DiagnosticDescriptor::new("MOTH-RULE-0038", "Shadowed name", DiagnosticSeverity::Error)
        }
        RuleDiagnosticKind::ReservedNameCollision => DiagnosticDescriptor::new(
            "MOTH-RULE-0039",
            "Reserved name collision",
            DiagnosticSeverity::Error,
        ),
        RuleDiagnosticKind::UnsupportedExternalFunction => DiagnosticDescriptor::new(
            "MOTH-RULE-0058",
            "Unsupported external function",
            DiagnosticSeverity::Error,
        ),
        RuleDiagnosticKind::InvalidRangeOperand => DiagnosticDescriptor::new(
            "MOTH-RULE-0059",
            "Invalid range operand",
            DiagnosticSeverity::Error,
        ),
        RuleDiagnosticKind::UnsupportedBuilderPackage => DiagnosticDescriptor::new(
            "MOTH-RULE-0060",
            "Unsupported builder package",
            DiagnosticSeverity::Error,
        ),
        RuleDiagnosticKind::DependencyNamespaceUsedAsValue => DiagnosticDescriptor::new(
            "MOTH-RULE-0065",
            "Dependency namespace used as value",
            DiagnosticSeverity::Error,
        ),
        RuleDiagnosticKind::ConstRecordUsedAsValue => DiagnosticDescriptor::new(
            "MOTH-RULE-0068",
            "Const record used as value",
            DiagnosticSeverity::Error,
        ),
        RuleDiagnosticKind::NestedDependencyTraversal => DiagnosticDescriptor::new(
            "MOTH-RULE-0066",
            "Nested dependency-namespace traversal",
            DiagnosticSeverity::Error,
        ),
        RuleDiagnosticKind::NamespaceTypeValueMisuse => DiagnosticDescriptor::new(
            "MOTH-RULE-0067",
            "Namespace type/value misuse",
            DiagnosticSeverity::Error,
        ),
        RuleDiagnosticKind::UnknownTrait => {
            DiagnosticDescriptor::new("MOTH-RULE-0069", "Unknown trait", DiagnosticSeverity::Error)
        }
        RuleDiagnosticKind::DuplicateTraitRequirement => DiagnosticDescriptor::new(
            "MOTH-RULE-0070",
            "Duplicate trait requirement",
            DiagnosticSeverity::Error,
        ),
        RuleDiagnosticKind::TraitPrivateSurfaceLeak => DiagnosticDescriptor::new(
            "MOTH-RULE-0071",
            "Private type exposed by trait",
            DiagnosticSeverity::Error,
        ),
        RuleDiagnosticKind::UnsupportedTraitFeature => DiagnosticDescriptor::new(
            "MOTH-RULE-0072",
            "Unsupported trait feature",
            DiagnosticSeverity::Error,
        ),
        RuleDiagnosticKind::GenericBoundPrivateSurfaceLeak => DiagnosticDescriptor::new(
            "MOTH-RULE-0074",
            "Private trait exposed by generic bound",
            DiagnosticSeverity::Error,
        ),
        RuleDiagnosticKind::TraitNameUsedAsType => DiagnosticDescriptor::new(
            "MOTH-RULE-0075",
            "Trait name used as value type",
            DiagnosticSeverity::Error,
        ),
        RuleDiagnosticKind::ExportOutsideModuleRoot => DiagnosticDescriptor::new(
            "MOTH-RULE-0077",
            "`export:` is only valid in a module root file",
            DiagnosticSeverity::Error,
        ),
        RuleDiagnosticKind::InvalidExportTarget => DiagnosticDescriptor::new(
            "MOTH-RULE-0080",
            "`export:` contains an invalid module API item",
            DiagnosticSeverity::Error,
        ),
        RuleDiagnosticKind::DuplicatePublicExport => DiagnosticDescriptor::new(
            "MOTH-RULE-0081",
            "Duplicate public export",
            DiagnosticSeverity::Error,
        ),
        RuleDiagnosticKind::DuplicateExportBlock => DiagnosticDescriptor::new(
            "MOTH-RULE-0085",
            "Duplicate export block",
            DiagnosticSeverity::Error,
        ),
        RuleDiagnosticKind::PrivateTypeInExportedApi => DiagnosticDescriptor::new(
            "MOTH-RULE-0082",
            "Private type exposed by exported API",
            DiagnosticSeverity::Error,
        ),
        _ => unreachable!("reasoned rule descriptors come from the central registry"),
    }
}

fn import_descriptor(kind: ImportDiagnosticKind) -> DiagnosticDescriptor {
    match kind {
        ImportDiagnosticKind::UnusedImport => DiagnosticDescriptor::new(
            "MOTH-IMPORT-0002",
            "Unused dependency binding",
            DiagnosticSeverity::Warning,
        ),
        ImportDiagnosticKind::DependencyAliasCaseMismatch => DiagnosticDescriptor::new(
            "MOTH-IMPORT-0003",
            "Dependency alias case mismatch",
            DiagnosticSeverity::Warning,
        ),

        ImportDiagnosticKind::MissingImportTarget => DiagnosticDescriptor::new(
            "MOTH-IMPORT-0005",
            "Missing dependency target",
            DiagnosticSeverity::Error,
        ),
        ImportDiagnosticKind::AmbiguousImportTarget => DiagnosticDescriptor::new(
            "MOTH-IMPORT-0006",
            "Ambiguous dependency target",
            DiagnosticSeverity::Error,
        ),
        ImportDiagnosticKind::BareFileImport => DiagnosticDescriptor::new(
            "MOTH-IMPORT-0007",
            "Bare file dependency",
            DiagnosticSeverity::Error,
        ),
        ImportDiagnosticKind::DirectSpecialFileImport => DiagnosticDescriptor::new(
            "MOTH-IMPORT-0008",
            "Direct special-file dependency",
            DiagnosticSeverity::Error,
        ),
        ImportDiagnosticKind::ImportNameCollision => DiagnosticDescriptor::new(
            "MOTH-IMPORT-0009",
            "Dependency binding name collision",
            DiagnosticSeverity::Error,
        ),
        ImportDiagnosticKind::NotExportedBySourceFile => DiagnosticDescriptor::new(
            "MOTH-IMPORT-0010",
            "Not exported by source file",
            DiagnosticSeverity::Error,
        ),
        ImportDiagnosticKind::NotExportedByPublicSurface => DiagnosticDescriptor::new(
            "MOTH-IMPORT-0011",
            "Not exported by public surface",
            DiagnosticSeverity::Error,
        ),
        ImportDiagnosticKind::MissingModuleRootPublicSurface => DiagnosticDescriptor::new(
            "MOTH-IMPORT-0012",
            "Missing module-root public surface",
            DiagnosticSeverity::Error,
        ),
        ImportDiagnosticKind::MissingPackageSymbol => DiagnosticDescriptor::new(
            "MOTH-IMPORT-0013",
            "Missing package symbol",
            DiagnosticSeverity::Error,
        ),
        ImportDiagnosticKind::CrossModuleImportNotExported => DiagnosticDescriptor::new(
            "MOTH-IMPORT-0015",
            "Cross-module dependency not exported",
            DiagnosticSeverity::Error,
        ),
        ImportDiagnosticKind::DirectSymbolPathImport => DiagnosticDescriptor::new(
            "MOTH-IMPORT-0017",
            "Direct symbol dependency path",
            DiagnosticSeverity::Error,
        ),
        ImportDiagnosticKind::InvalidNamespaceDefaultName => DiagnosticDescriptor::new(
            "MOTH-IMPORT-0018",
            "Invalid namespace default name",
            DiagnosticSeverity::Error,
        ),
        ImportDiagnosticKind::DuplicateImportSurfaceMember => DiagnosticDescriptor::new(
            "MOTH-IMPORT-0019",
            "Duplicate dependency surface member",
            DiagnosticSeverity::Error,
        ),
        ImportDiagnosticKind::ExplicitMothExtension => DiagnosticDescriptor::new(
            "MOTH-IMPORT-0020",
            "Explicit .moth extension in dependency",
            DiagnosticSeverity::Error,
        ),
        ImportDiagnosticKind::UnsupportedExternalExtension => DiagnosticDescriptor::new(
            "MOTH-IMPORT-0021",
            "Unsupported external file dependency extension",
            DiagnosticSeverity::Error,
        ),
        ImportDiagnosticKind::ExplicitSourceExtension => DiagnosticDescriptor::new(
            "MOTH-IMPORT-0024",
            "Explicit source extension in dependency",
            DiagnosticSeverity::Error,
        ),
        ImportDiagnosticKind::UnsupportedSourceFileKind => DiagnosticDescriptor::new(
            "MOTH-IMPORT-0025",
            "Unsupported source file kind",
            DiagnosticSeverity::Error,
        ),
        ImportDiagnosticKind::InvalidSourceFileEntry => DiagnosticDescriptor::new(
            "MOTH-IMPORT-0026",
            "Source file kind cannot be used as an entry",
            DiagnosticSeverity::Error,
        ),
        ImportDiagnosticKind::MothTemplateInputsShareNoCommonAncestor => DiagnosticDescriptor::new(
            "MOTH-IMPORT-0029",
            "Moth template inputs share no common ancestor",
            DiagnosticSeverity::Error,
        ),
        ImportDiagnosticKind::DuplicateMothTemplateInputPath => DiagnosticDescriptor::new(
            "MOTH-IMPORT-0028",
            "Duplicate Moth template input path",
            DiagnosticSeverity::Error,
        ),
        _ => unreachable!("reasoned import descriptors come from the central registry"),
    }
}

fn borrow_descriptor(kind: BorrowDiagnosticKind) -> DiagnosticDescriptor {
    match kind {
        BorrowDiagnosticKind::BorrowConflict => DiagnosticDescriptor::new(
            "MOTH-BORROW-0001",
            "Access conflict",
            DiagnosticSeverity::Error,
        ),
        BorrowDiagnosticKind::MultipleMutableBorrows => DiagnosticDescriptor::new(
            "MOTH-BORROW-0002",
            "Conflicting mutable access",
            DiagnosticSeverity::Error,
        ),
        BorrowDiagnosticKind::SharedMutableConflict => DiagnosticDescriptor::new(
            "MOTH-BORROW-0003",
            "Shared and mutable access conflict",
            DiagnosticSeverity::Error,
        ),
        BorrowDiagnosticKind::UseAfterPossibleMove => DiagnosticDescriptor::new(
            "MOTH-BORROW-0004",
            "Use after possible move",
            DiagnosticSeverity::Error,
        ),
        BorrowDiagnosticKind::MoveWhileBorrowed => DiagnosticDescriptor::new(
            "MOTH-BORROW-0005",
            "Ownership transfer conflicts with active access",
            DiagnosticSeverity::Error,
        ),
        BorrowDiagnosticKind::WholeObjectBorrowConflict => DiagnosticDescriptor::new(
            "MOTH-BORROW-0006",
            "Whole-value access conflict",
            DiagnosticSeverity::Error,
        ),
        BorrowDiagnosticKind::UseOfUninitializedLocal => DiagnosticDescriptor::new(
            "MOTH-BORROW-0009",
            "Use of uninitialized local",
            DiagnosticSeverity::Error,
        ),
        _ => {
            unreachable!("every remaining borrow descriptor is generated from the reason registry")
        }
    }
}

fn config_descriptor(_kind: ConfigDiagnosticKind) -> DiagnosticDescriptor {
    unreachable!("every config descriptor is generated from the reason registry")
}
fn infrastructure_descriptor(kind: InfrastructureDiagnosticKind) -> DiagnosticDescriptor {
    match kind {
        InfrastructureDiagnosticKind::InfrastructureFailure => DiagnosticDescriptor::new(
            "MOTH-INFRA-0001",
            "Infrastructure failure",
            DiagnosticSeverity::Error,
        ),
    }
}

fn deferred_feature_descriptor(_kind: DeferredFeatureDiagnosticKind) -> DiagnosticDescriptor {
    unreachable!("every deferred-feature descriptor is generated from the reason registry")
}
