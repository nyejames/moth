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

pub(super) fn descriptor_for_kind(kind: DiagnosticKind) -> DiagnosticDescriptor {
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
        SyntaxDiagnosticKind::InvalidNumberLiteral => DiagnosticDescriptor::new(
            "MOTH-SYNTAX-0008",
            "Invalid number literal",
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
        SyntaxDiagnosticKind::InvalidTypeAnnotation => DiagnosticDescriptor::new(
            "MOTH-SYNTAX-0014",
            "Invalid type annotation",
            DiagnosticSeverity::Error,
        ),
        SyntaxDiagnosticKind::InvalidGenericApplication => DiagnosticDescriptor::new(
            "MOTH-SYNTAX-0015",
            "Invalid generic application",
            DiagnosticSeverity::Error,
        ),
        SyntaxDiagnosticKind::InvalidCollectionType => DiagnosticDescriptor::new(
            "MOTH-SYNTAX-0016",
            "Invalid collection type",
            DiagnosticSeverity::Error,
        ),
        SyntaxDiagnosticKind::InvalidMapType => DiagnosticDescriptor::new(
            "MOTH-SYNTAX-0016-MAP",
            "Invalid map type",
            DiagnosticSeverity::Error,
        ),
        SyntaxDiagnosticKind::InvalidMapLiteral => DiagnosticDescriptor::new(
            "MOTH-SYNTAX-0033",
            "Invalid map literal",
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
        SyntaxDiagnosticKind::InvalidDependencyClause => DiagnosticDescriptor::new(
            "MOTH-SYNTAX-0019",
            "Invalid dependency clause",
            DiagnosticSeverity::Error,
        ),
        SyntaxDiagnosticKind::LegacyDependencyClause => DiagnosticDescriptor::new(
            "MOTH-SYNTAX-0035",
            "Legacy dependency clause syntax",
            DiagnosticSeverity::Error,
        ),
        SyntaxDiagnosticKind::InvalidGenericParameter => DiagnosticDescriptor::new(
            "MOTH-SYNTAX-0020",
            "Invalid generic parameter",
            DiagnosticSeverity::Error,
        ),
        SyntaxDiagnosticKind::InvalidTemplateDirective => DiagnosticDescriptor::new(
            "MOTH-SYNTAX-0021",
            "Invalid template directive",
            DiagnosticSeverity::Error,
        ),
        SyntaxDiagnosticKind::InvalidTemplateStructure => DiagnosticDescriptor::new(
            "MOTH-SYNTAX-0022",
            "Invalid template structure",
            DiagnosticSeverity::Error,
        ),
        SyntaxDiagnosticKind::InvalidExpression => DiagnosticDescriptor::new(
            "MOTH-SYNTAX-0023",
            "Invalid expression",
            DiagnosticSeverity::Error,
        ),
        SyntaxDiagnosticKind::MissingOperatorOperand => DiagnosticDescriptor::new(
            "MOTH-SYNTAX-0024",
            "Missing operator operand",
            DiagnosticSeverity::Error,
        ),
        SyntaxDiagnosticKind::InvalidStandaloneStatement => DiagnosticDescriptor::new(
            "MOTH-SYNTAX-0025",
            "Invalid standalone statement",
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
        SyntaxDiagnosticKind::InvalidMatchArm => DiagnosticDescriptor::new(
            "MOTH-SYNTAX-0028",
            "Invalid match arm",
            DiagnosticSeverity::Error,
        ),
        SyntaxDiagnosticKind::InvalidLoopHeader => DiagnosticDescriptor::new(
            "MOTH-SYNTAX-0029",
            "Invalid loop header",
            DiagnosticSeverity::Error,
        ),
        SyntaxDiagnosticKind::InvalidStatementPosition => DiagnosticDescriptor::new(
            "MOTH-SYNTAX-0030",
            "Invalid statement position",
            DiagnosticSeverity::Error,
        ),
        SyntaxDiagnosticKind::CommonSyntaxMistake => DiagnosticDescriptor::new(
            "MOTH-SYNTAX-0031",
            "Common syntax mistake",
            DiagnosticSeverity::Error,
        ),
        SyntaxDiagnosticKind::UnescapedImplicitTemplateClose => DiagnosticDescriptor::new(
            "MOTH-SYNTAX-0032",
            "Unescaped implicit template close",
            DiagnosticSeverity::Error,
        ),
        SyntaxDiagnosticKind::InvalidStringEscape => DiagnosticDescriptor::new(
            "MOTH-SYNTAX-0034",
            "Invalid string escape",
            DiagnosticSeverity::Error,
        ),
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
        TypeDiagnosticKind::InvalidFallibleOperand => DiagnosticDescriptor::new(
            "MOTH-TYPE-0004",
            "Unhandled fallible operand",
            DiagnosticSeverity::Error,
        ),
        TypeDiagnosticKind::IncompatibleChoiceComparison => DiagnosticDescriptor::new(
            "MOTH-TYPE-0005",
            "Incompatible choice comparison",
            DiagnosticSeverity::Error,
        ),
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
        RuleDiagnosticKind::UnusedVariable => DiagnosticDescriptor::new(
            "MOTH-RULE-0010",
            "Unused variable",
            DiagnosticSeverity::Warning,
        ),
        RuleDiagnosticKind::UnusedFunction => DiagnosticDescriptor::new(
            "MOTH-RULE-0011",
            "Unused function",
            DiagnosticSeverity::Warning,
        ),
        RuleDiagnosticKind::UnusedType => {
            DiagnosticDescriptor::new("MOTH-RULE-0012", "Unused type", DiagnosticSeverity::Warning)
        }
        RuleDiagnosticKind::UnusedConstant => DiagnosticDescriptor::new(
            "MOTH-RULE-0013",
            "Unused constant",
            DiagnosticSeverity::Warning,
        ),
        RuleDiagnosticKind::UnusedFunctionArgument => DiagnosticDescriptor::new(
            "MOTH-RULE-0014",
            "Unused function argument",
            DiagnosticSeverity::Warning,
        ),
        RuleDiagnosticKind::UnusedFunctionReturnValue => DiagnosticDescriptor::new(
            "MOTH-RULE-0015",
            "Unused function return value",
            DiagnosticSeverity::Warning,
        ),
        RuleDiagnosticKind::UnusedFunctionParameter => DiagnosticDescriptor::new(
            "MOTH-RULE-0016",
            "Unused function parameter",
            DiagnosticSeverity::Warning,
        ),
        RuleDiagnosticKind::UnusedFunctionParameterDefaultValue => DiagnosticDescriptor::new(
            "MOTH-RULE-0017",
            "Unused function parameter default value",
            DiagnosticSeverity::Warning,
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
        RuleDiagnosticKind::InvalidSignatureMember => DiagnosticDescriptor::new(
            "MOTH-RULE-0028",
            "Invalid signature member",
            DiagnosticSeverity::Error,
        ),
        RuleDiagnosticKind::InvalidChoiceVariant => DiagnosticDescriptor::new(
            "MOTH-RULE-0029",
            "Invalid choice variant",
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
        RuleDiagnosticKind::InvalidThisUsage => DiagnosticDescriptor::new(
            "MOTH-RULE-0040",
            "Invalid this usage",
            DiagnosticSeverity::Error,
        ),
        RuleDiagnosticKind::InvalidReceiverDeclaration => DiagnosticDescriptor::new(
            "MOTH-RULE-0041",
            "Invalid receiver declaration",
            DiagnosticSeverity::Error,
        ),
        RuleDiagnosticKind::InvalidControlFlowStatement => DiagnosticDescriptor::new(
            "MOTH-RULE-0042",
            "Invalid control flow statement",
            DiagnosticSeverity::Error,
        ),
        RuleDiagnosticKind::InvalidDeclaration => DiagnosticDescriptor::new(
            "MOTH-RULE-0043",
            "Invalid declaration",
            DiagnosticSeverity::Error,
        ),
        RuleDiagnosticKind::InvalidAssignmentTarget => DiagnosticDescriptor::new(
            "MOTH-RULE-0044",
            "Invalid assignment target",
            DiagnosticSeverity::Error,
        ),
        RuleDiagnosticKind::InvalidMultiBind => DiagnosticDescriptor::new(
            "MOTH-RULE-0045",
            "Invalid multi-bind",
            DiagnosticSeverity::Error,
        ),
        RuleDiagnosticKind::InvalidBuiltinCall => DiagnosticDescriptor::new(
            "MOTH-RULE-0046",
            "Invalid builtin call",
            DiagnosticSeverity::Error,
        ),
        RuleDiagnosticKind::InvalidCast => {
            DiagnosticDescriptor::new("MOTH-RULE-0083", "Invalid cast", DiagnosticSeverity::Error)
        }
        RuleDiagnosticKind::InvalidReceiverCall => DiagnosticDescriptor::new(
            "MOTH-RULE-0047",
            "Invalid receiver call",
            DiagnosticSeverity::Error,
        ),
        RuleDiagnosticKind::InvalidCopyTarget => DiagnosticDescriptor::new(
            "MOTH-RULE-0056",
            "Invalid copy target",
            DiagnosticSeverity::Error,
        ),
        RuleDiagnosticKind::InvalidFieldAccess => DiagnosticDescriptor::new(
            "MOTH-RULE-0048",
            "Invalid field access",
            DiagnosticSeverity::Error,
        ),
        RuleDiagnosticKind::InvalidMatchPattern => DiagnosticDescriptor::new(
            "MOTH-RULE-0049",
            "Invalid match pattern",
            DiagnosticSeverity::Error,
        ),
        RuleDiagnosticKind::NonExhaustiveMatch => DiagnosticDescriptor::new(
            "MOTH-RULE-0050",
            "Non-exhaustive match",
            DiagnosticSeverity::Error,
        ),
        RuleDiagnosticKind::InvalidFallibleHandling => DiagnosticDescriptor::new(
            "MOTH-RULE-0051",
            "Invalid fallible handling",
            DiagnosticSeverity::Error,
        ),
        RuleDiagnosticKind::InvalidTemplateSlot => DiagnosticDescriptor::new(
            "MOTH-RULE-0052",
            "Invalid template slot",
            DiagnosticSeverity::Error,
        ),
        RuleDiagnosticKind::CompileTimeEvaluationError => DiagnosticDescriptor::new(
            "MOTH-RULE-0053",
            "Compile-time evaluation error",
            DiagnosticSeverity::Error,
        ),
        RuleDiagnosticKind::InvalidCallShape => DiagnosticDescriptor::new(
            "MOTH-RULE-0054",
            "Invalid call shape",
            DiagnosticSeverity::Error,
        ),
        RuleDiagnosticKind::InvalidReturnShape => DiagnosticDescriptor::new(
            "MOTH-RULE-0055",
            "Invalid return shape",
            DiagnosticSeverity::Error,
        ),
        RuleDiagnosticKind::InvalidFunctionSignature => DiagnosticDescriptor::new(
            "MOTH-RULE-0062",
            "Invalid function signature",
            DiagnosticSeverity::Error,
        ),
        RuleDiagnosticKind::InvalidGenericInstantiation => DiagnosticDescriptor::new(
            "MOTH-RULE-0057",
            "Invalid generic instantiation",
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
        RuleDiagnosticKind::UnsupportedBackendFeature => DiagnosticDescriptor::new(
            "MOTH-RULE-0064",
            "Unsupported backend feature",
            DiagnosticSeverity::Error,
        ),
        RuleDiagnosticKind::InvalidPageMetadata => DiagnosticDescriptor::new(
            "MOTH-RULE-0061",
            "Invalid page metadata",
            DiagnosticSeverity::Error,
        ),
        RuleDiagnosticKind::InvalidCompileTimePath => DiagnosticDescriptor::new(
            "MOTH-RULE-0063",
            "Invalid compile-time path",
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
        RuleDiagnosticKind::InvalidTraitConformance => DiagnosticDescriptor::new(
            "MOTH-RULE-0073",
            "Invalid trait conformance",
            DiagnosticSeverity::Error,
        ),
        RuleDiagnosticKind::InvalidTraitIncompatibility => DiagnosticDescriptor::new(
            "MOTH-RULE-0084",
            "Invalid trait incompatibility",
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
        RuleDiagnosticKind::InvalidTraitKeywordUsage => DiagnosticDescriptor::new(
            "MOTH-RULE-0076",
            "Invalid trait keyword usage",
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
        ImportDiagnosticKind::InvalidImportPath => DiagnosticDescriptor::new(
            "MOTH-IMPORT-0016",
            "Invalid dependency path",
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
        ImportDiagnosticKind::InvalidExternalModule => DiagnosticDescriptor::new(
            "MOTH-IMPORT-0022",
            "Invalid external JS module",
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
        ImportDiagnosticKind::InvalidMothTemplateApiScopeItem => DiagnosticDescriptor::new(
            "MOTH-IMPORT-0027",
            "Invalid Moth template API scope item",
            DiagnosticSeverity::Error,
        ),
        ImportDiagnosticKind::DuplicateMothTemplateInputPath => DiagnosticDescriptor::new(
            "MOTH-IMPORT-0028",
            "Duplicate Moth template input path",
            DiagnosticSeverity::Error,
        ),
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
        BorrowDiagnosticKind::InvalidMutableAccess => DiagnosticDescriptor::new(
            "MOTH-BORROW-0007",
            "Invalid mutable access",
            DiagnosticSeverity::Error,
        ),
        BorrowDiagnosticKind::UseOfUninitializedLocal => DiagnosticDescriptor::new(
            "MOTH-BORROW-0009",
            "Use of uninitialized local",
            DiagnosticSeverity::Error,
        ),
    }
}

fn config_descriptor(kind: ConfigDiagnosticKind) -> DiagnosticDescriptor {
    match kind {
        ConfigDiagnosticKind::InvalidConfig => DiagnosticDescriptor::new(
            "MOTH-CONFIG-0001",
            "Invalid config",
            DiagnosticSeverity::Error,
        ),
    }
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

fn deferred_feature_descriptor(kind: DeferredFeatureDiagnosticKind) -> DiagnosticDescriptor {
    match kind {
        DeferredFeatureDiagnosticKind::DeferredFeature => DiagnosticDescriptor::new(
            "MOTH-DEFERRED-0001",
            "Deferred feature",
            DiagnosticSeverity::Error,
        ),
    }
}
