//! Parsed type syntax before resolution.
//!
//! WHAT: AST type annotations start as `ParsedTypeRef` and are resolved
//!      into `TypeId` by the type-resolution pass.
//! WHY: unresolved names, inferred positions, and source spelling must not
//!      be confused with resolved semantic type identity.

use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringIdRemap};

// Parsed syntax retains exact global spans when authored source identity is available. Generated
// or synthetic references remain spanless rather than carrying a second line/column provenance.

/// Parsed fixed-capacity syntax before semantic folding.
///
/// WHAT: stores the narrow capacity shape the parser accepts: a positive integer literal
///       or a bare constant name. Arithmetic and other general expression forms are
///       rejected at parse time so type resolution only sees canonical capacity forms.
/// WHY: collection capacity identity requires a compile-time-known value, but the parser
///      enforces the literal-or-bare-const rule directly rather than carrying raw tokens.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ParsedCollectionCapacity {
    /// A positive integer literal such as `64`.
    Literal {
        value: i32,
        span: Option<SourceSpan>,
    },
    /// A bare visible constant name such as `capacity`.
    BareConstant {
        name: StringId,
        span: Option<SourceSpan>,
    },
}

/// Parsed type annotation before resolution.
///
/// Does NOT represent semantic type identity.
#[derive(Debug, Clone, PartialEq)]
pub enum ParsedTypeRef {
    // -----------------
    //  Meta-types
    // -----------------
    Inferred,

    Named {
        name: StringId,
        span: Option<SourceSpan>,
    },

    /// A dotted namespace-qualified type path such as `Canvas.Context` or
    /// `io.input.Input`.
    ///
    /// WHAT: stores the unresolved path components before type resolution walks the
    ///      visible namespace records.
    /// WHY: external package surfaces expose nested namespaces, so type position must
    ///      support arbitrary-depth dotted paths while keeping the syntax representation
    ///      separate from semantic type identity.
    Qualified {
        path: Vec<StringId>,
        span: Option<SourceSpan>,
    },

    Applied {
        base: Box<ParsedTypeRef>,
        arguments: Vec<ParsedTypeRef>,
        span: Option<SourceSpan>,
    },

    // -----------------
    //  Builtin Types
    // -----------------
    BuiltinBool {
        span: Option<SourceSpan>,
    },

    BuiltinInt {
        span: Option<SourceSpan>,
    },

    BuiltinFloat {
        span: Option<SourceSpan>,
    },

    BuiltinString {
        span: Option<SourceSpan>,
    },

    BuiltinChar {
        span: Option<SourceSpan>,
    },

    // -----------------
    //  Trait-local Types
    // -----------------
    This {
        span: Option<SourceSpan>,
    },

    // -----------------
    //  Constructed Types
    // -----------------
    Collection {
        element: Box<ParsedTypeRef>,
        span: Option<SourceSpan>,
        fixed_capacity: Option<ParsedCollectionCapacity>,
    },

    Map {
        key: Box<ParsedTypeRef>,
        value: Box<ParsedTypeRef>,
        span: Option<SourceSpan>,
    },

    Optional {
        inner: Box<ParsedTypeRef>,
        span: Option<SourceSpan>,
    },
}

impl ParsedTypeRef {
    /// Remap all interned string IDs in this parsed type reference into a merged string table.
    ///
    /// WHAT: updates every interned name recursively through `Applied` and constructed types.
    /// WHY: per-file header parsing produces parsed references using local string tables;
    ///      remapping keeps their semantic names valid after merge into the module/global table.
    // Called by per-file frontend output remapping before module-wide dependency sorting.
    pub fn remap_string_ids(&mut self, remap: &StringIdRemap) {
        match self {
            ParsedTypeRef::Inferred => {}

            ParsedTypeRef::Named { name, .. } => {
                *name = remap.get(*name);
            }
            ParsedTypeRef::Qualified { path, .. } => {
                for component in path {
                    *component = remap.get(*component);
                }
            }

            ParsedTypeRef::Applied {
                base, arguments, ..
            } => {
                base.remap_string_ids(remap);
                for argument in arguments {
                    argument.remap_string_ids(remap);
                }
            }

            ParsedTypeRef::BuiltinBool { .. }
            | ParsedTypeRef::BuiltinInt { .. }
            | ParsedTypeRef::BuiltinFloat { .. }
            | ParsedTypeRef::BuiltinString { .. }
            | ParsedTypeRef::BuiltinChar { .. }
            | ParsedTypeRef::This { .. } => {}

            ParsedTypeRef::Collection {
                element,
                fixed_capacity,
                ..
            } => {
                element.remap_string_ids(remap);
                if let Some(ParsedCollectionCapacity::BareConstant { name, .. }) = fixed_capacity {
                    *name = remap.get(*name);
                }
            }

            ParsedTypeRef::Map { key, value, .. } => {
                key.remap_string_ids(remap);
                value.remap_string_ids(remap);
            }

            ParsedTypeRef::Optional { inner, .. } => {
                inner.remap_string_ids(remap);
            }
        }
    }
}
