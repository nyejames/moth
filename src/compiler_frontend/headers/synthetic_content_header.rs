//! Shared helper for compiler-generated `content #String` headers.
//!
//! WHAT: builds ordinary private `content #String` headers for compiler-generated source assets
//!       such as Moth template `.mtf` and plain Markdown `.md`.
//! WHY: `.mtf` and `.md` both expose a single generated content constant but differ in how their
//!      initializer tokens are produced. This helper removes that duplication without changing
//!      either source kind's output shape.
//! MUST NOT: render Markdown, tokenize source, parse dependency clauses, own source-kind decisions, or
//!           construct source-location facts from filesystem paths.

use crate::compiler_frontend::datatypes::parsed::ParsedTypeRef;
use crate::compiler_frontend::declaration_syntax::binding_mode::BindingMode;
use crate::compiler_frontend::declaration_syntax::declaration_shell::DeclarationSyntax;
use crate::compiler_frontend::headers::types::{FileRole, Header, HeaderExportMode, HeaderKind};
use crate::compiler_frontend::source::SourceId;
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use crate::compiler_frontend::tokenizer::tokens::{FileTokens, SourceLocation, Token};
use crate::compiler_frontend::utilities::token_scan::InitializerReference;
use std::collections::HashSet;
use std::path::PathBuf;

const SYNTHETIC_CONTENT_NAME: &str = "content";

/// Inputs needed to build one synthetic `content #String` header.
///
/// WHY: grouping these fields avoids a long argument list and keeps the caller responsible for
///      source-identity and initializer facts while this helper owns the repetitive header shape.
pub(crate) struct SyntheticContentHeaderInput {
    pub(crate) source_file: InternedPath,
    pub(crate) file_id: Option<SourceId>,
    pub(crate) canonical_os_path: Option<PathBuf>,
    pub(crate) location: SourceLocation,
    pub(crate) initializer_tokens: Vec<Token>,
    pub(crate) initializer_references: Vec<InitializerReference>,
}

/// The synthetic content constant's graph path for one content source.
///
/// WHAT: derives `<content source>/content` — the exact path `synthetic_content_header` owns — so
///       ordering facts can target the generated constant without a second content name.
/// WHY: declaration ordering and the header stage must agree on the content constant identity from
///       one owner of the synthetic name.
pub(crate) fn content_constant_path(
    content_source: &InternedPath,
    string_table: &mut StringTable,
) -> InternedPath {
    content_source.append(string_table.intern(SYNTHETIC_CONTENT_NAME))
}

/// Build a private `content #String` constant header from generated initializer tokens.
///
/// WHAT: builds the header path through `content_constant_path` and packages the supplied
///       initializer tokens into a normal constant header.
/// WHY: later frontend stages should see an ordinary private constant, not a source-kind-specific
///      AST/HIR path.
pub(crate) fn synthetic_content_header(
    input: SyntheticContentHeaderInput,
    string_table: &mut StringTable,
) -> Header {
    let header_path = content_constant_path(&input.source_file, string_table);

    let header_tokens = FileTokens::new_deferred_with_identity(
        header_path.clone(),
        input.file_id,
        input.canonical_os_path,
        Vec::new(),
    );

    let declaration = DeclarationSyntax {
        binding_mode: BindingMode::CompileTimeConstant,
        type_annotation: ParsedTypeRef::BuiltinString {
            location: input.location.clone(),
        },
        config_qualifier: None,
        initializer_tokens: input.initializer_tokens,
        initializer_references: input.initializer_references,
        location: input.location.clone(),
    };

    Header {
        kind: HeaderKind::Constant { declaration },
        file_role: FileRole::Normal,
        export_mode: HeaderExportMode::Private,
        local_ordering_hints: HashSet::new(),
        name_location: input.location.clone(),
        tokens: header_tokens,
        source_file: input.source_file,
        capacity_references: Vec::new(),
    }
}
