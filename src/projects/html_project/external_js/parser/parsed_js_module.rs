//! Parser-owned data model for a parsed JavaScript binding module.
//!
//! WHAT: defines the structures that the `@moth.*` annotation parser produces before
//!       any conversion into compiler frontend types such as `ExternalFunctionDef`.
//! WHY: keeps the JS scanner isolated from compiler-stage boundaries so later phases
//!      (provider wiring, registry insertion) can decide how to map parsed data.

/// A source position inside a JS file.
///
/// WHAT: byte-offset + line/column tracking for every parsed construct so the parser
///       can emit diagnostics that point at exact JS source locations.
/// WHY: parser-local spans avoid needing full `SourceLocation` integration in this slice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsSourceSpan {
    pub byte_start: usize,
    pub byte_end: usize,
    pub line: usize,
    pub column: usize,
}

impl JsSourceSpan {
    /// Creates a zero-width span at the given position.
    pub fn at(byte: usize, line: usize, column: usize) -> Self {
        Self {
            byte_start: byte,
            byte_end: byte,
            line,
            column,
        }
    }

    /// Creates a span covering a byte range.
    pub fn range(byte_start: usize, byte_end: usize, line: usize, column: usize) -> Self {
        Self {
            byte_start,
            byte_end,
            line,
            column,
        }
    }
}

/// Classification for parser-local diagnostics.
///
/// WHAT: each variant maps to a specific user-facing error or warning produced by
///       the JS annotation scanner.
/// WHY: structured kinds let the provider layer convert these into stable
///      `CompilerDiagnostic` codes later.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JsDiagnosticKind {
    UnsupportedPackageTag,
    UnknownMothDirective,
    UnannotatedExport,
    MissingExportAfterSig,
    DuplicateMothName,
    DuplicateJsExportName,
    DefaultExport,
    ReExport,
    CommonJsExport,
    ClassExport,
    ArbitraryImport,
    DynamicImport,
    UnsupportedParameterPattern,
    UnsupportedTypeSyntax,
    GenericExternalFunction,
    GenericExternalType,
    VoidReturn,
    MultiSuccessReturn,
    UnknownExternalType,
    ArityMismatch,
    InvalidReceiverParameter,
    UnsupportedRuntimeImportForm,
    UnknownRuntimeImportName,
    ExpressionBodiedArrowExport,
}

/// A diagnostic emitted by the JS parser before conversion to compiler diagnostics.
///
/// WHAT: carries a human-readable message, the offending span, and a structured kind.
/// WHY: the provider layer can transform these into stable diagnostic codes while
///      the parser itself stays independent of `CompilerDiagnostic` construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsParserDiagnostic {
    pub message: String,
    pub span: JsSourceSpan,
    pub kind: JsDiagnosticKind,
}

/// A single parameter parsed from a `@moth.sig` signature.
///
/// WHAT: describes one Moth-facing parameter. Receiver-shaped signatures are still
///       classified when the first parameter is named `this` so registration boundaries
///       can reject them with a targeted diagnostic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedParameter {
    pub name: String,
    pub type_name: String,
    pub is_receiver: bool,
    pub is_mutable: bool,
}

/// A single success return type parsed from a `@moth.sig` signature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedReturnType {
    pub type_name: String,
}

/// The parsed body of a `@moth.sig` annotation.
///
/// WHAT: represents the Moth-facing parameter list and return types after
///       light-weight signature parsing.
/// WHY: the provider layer will validate type names against known builtins and
///      `@moth.opaque` declarations and then construct `ExternalFunctionDef` values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedSignature {
    pub parameters: Vec<ParsedParameter>,
    pub returns: Vec<ParsedReturnType>,
    pub has_error_return: bool,
    /// Parser-local recovery flag for rejected generic-looking `@moth.sig`
    /// preambles. This is not external package metadata; provider registration
    /// stops before converting a module with parser diagnostics.
    pub has_unsupported_generic_parameters: bool,
}

impl ParsedSignature {
    /// Returns the number of ABI parameters, counting the receiver `this` as one.
    pub fn abi_parameter_count(&self) -> usize {
        self.parameters.len()
    }

    /// Returns true if the first parameter is a receiver (`this`).
    pub fn has_receiver(&self) -> bool {
        self.parameters.first().is_some_and(|p| p.is_receiver)
    }
}

/// A function or method discovered in a JS file and matched to a `@moth.sig` annotation.
///
/// WHAT: binds the Moth-facing name (from `@moth.sig`) to the JS export name
///       and the parsed signature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedJsFunction {
    pub moth_name: String,
    pub js_name: String,
    pub signature: ParsedSignature,
    pub annotation_span: JsSourceSpan,
    pub export_span: JsSourceSpan,
}

/// An opaque external type declared with `@moth.opaque`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedOpaqueType {
    pub name: String,
    pub span: JsSourceSpan,
}

/// A single registered runtime module import discovered in a JS source file.
///
/// WHAT: records that the JS file imports specific symbols from a registered core
///       runtime module such as `@moth/runtime`.
/// WHY: provider and backend emission need actual parsed imports, not fallibility
///      inference, to decide which runtime modules to emit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedRuntimeImport {
    pub module_name: String,
    pub imported_names: Vec<String>,
    pub span: JsSourceSpan,
}

/// Final result of parsing one JS source file.
///
/// WHAT: collects all opaque types, free functions, receiver-shaped signatures, registered
///       runtime imports, and any diagnostics produced while scanning.
/// WHY: this is the complete parser output consumed by the JS import provider and
///      built-in JS-backed package registration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedJsModule {
    pub opaque_types: Vec<ParsedOpaqueType>,
    pub free_functions: Vec<ParsedJsFunction>,
    pub receiver_methods: Vec<ParsedJsFunction>,
    pub runtime_imports: Vec<ParsedRuntimeImport>,
    pub diagnostics: Vec<JsParserDiagnostic>,
}

impl ParsedJsModule {
    /// Creates an empty parsed module with no symbols and no diagnostics.
    pub fn empty() -> Self {
        Self {
            opaque_types: Vec::new(),
            free_functions: Vec::new(),
            receiver_methods: Vec::new(),
            runtime_imports: Vec::new(),
            diagnostics: Vec::new(),
        }
    }
}
