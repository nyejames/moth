//! Shared receiver-method diagnostics and formatting helpers.
//!
//! WHAT: centralizes receiver-method catalog construction, validation, and error construction.
//! WHY: parser entrypoints report the same receiver-method misuse errors, so one helper keeps
//! diagnostics deterministic and avoids drift in wording/metadata.
//!
//! INVARIANT: every source-authored receiver method must belong to the same file as its
//! user-defined struct or choice declaration. Builtin, imported, and external receiver types use
//! free functions or builder-owned external metadata instead of source-authored receiver methods.

use crate::compiler_frontend::ast::ast_nodes::Declaration;
use crate::compiler_frontend::ast::statements::functions::FunctionSignature;
use crate::compiler_frontend::ast::type_resolution::ResolvedFunctionSignature;
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidReceiverCallReason, InvalidReceiverDeclarationReason,
};
use crate::compiler_frontend::datatypes::ReceiverKey;
use crate::compiler_frontend::headers::parse_file_headers::{Header, HeaderKind};
use crate::compiler_frontend::source::SourceSpan;
use crate::compiler_frontend::symbols::path_interner::{PathId, PathInternerFork};
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use rustc_hash::FxHashMap;

pub(crate) enum ReceiverMethodCatalogError {
    Diagnostic(CompilerDiagnostic),
    Infrastructure(Box<CompilerError>),
}

impl From<CompilerDiagnostic> for ReceiverMethodCatalogError {
    fn from(diagnostic: CompilerDiagnostic) -> Self {
        ReceiverMethodCatalogError::Diagnostic(diagnostic)
    }
}

impl From<CompilerError> for ReceiverMethodCatalogError {
    fn from(error: CompilerError) -> Self {
        ReceiverMethodCatalogError::Infrastructure(Box::new(error))
    }
}

/// Canonical metadata for one receiver method declaration.
#[derive(Clone)]
pub(crate) struct ReceiverMethodEntry {
    pub(crate) function_path: PathId,
    pub(crate) receiver: ReceiverKey,
    pub(crate) source_file: PathId,
    pub(crate) receiver_mutable: bool,
    pub(crate) signature: FunctionSignature,
}

/// Receiver-method lookup tables used by parser and diagnostics.
#[derive(Clone, Default)]
pub(crate) struct ReceiverMethodCatalog {
    pub(crate) by_receiver_and_name: FxHashMap<(ReceiverKey, StringId), Vec<ReceiverMethodEntry>>,
    /// Index by bare method name for "called as free function" diagnostics.
    ///
    /// WHAT: maps a method name to all receiver methods with that name, regardless of receiver
    /// type, so that `method(value)` can produce a targeted diagnostic.
    /// WHY: this is diagnostic-only quality; it does not drive semantic dispatch.
    pub(crate) by_method_name: FxHashMap<StringId, Vec<ReceiverMethodEntry>>,
    /// Maps canonical function path to its receiver-method entry.
    /// WHY: file visibility resolves source receiver methods by function path, so a path-keyed
    /// index lets lookups avoid scanning the whole catalog.
    pub(crate) by_function_path: FxHashMap<PathId, ReceiverMethodEntry>,
}

/// Inputs required to build the receiver-method catalog.
///
/// WHAT: groups the resolved signature and source ownership side tables used while building the
/// receiver method catalog.
/// WHY: catalog construction sits at the join between header discovery and AST semantic typing;
/// keeping the inputs named makes that boundary easier to audit than a long positional list.
pub(crate) struct BuildReceiverMethodCatalogInput<'a> {
    pub(crate) sorted_headers: &'a [Header],
    pub(crate) resolved_function_signatures_by_path:
        &'a FxHashMap<PathId, ResolvedFunctionSignature>,
    pub(crate) struct_fields_by_path: &'a FxHashMap<PathId, Vec<Declaration>>,
    pub(crate) struct_source_by_path: &'a FxHashMap<PathId, PathId>,
    pub(crate) choice_source_by_path: &'a FxHashMap<PathId, PathId>,
    pub(crate) source_file_by_symbol_path: &'a FxHashMap<PathId, PathId>,
    pub(crate) path_fork: &'a PathInternerFork,
    pub(crate) string_table: &'a StringTable,
}

/// Render a human-readable receiver name for diagnostics.
pub(crate) fn receiver_kind_label(receiver: &ReceiverKey, _string_table: &StringTable) -> String {
    match receiver {
        ReceiverKey::Struct(path) | ReceiverKey::Choice(path) => format!("{path:?}"),
        ReceiverKey::External(type_id) => format!("External({})", type_id.0),
        ReceiverKey::BuiltinScalar(builtin) => format!("{builtin:?}"),
    }
}

/// Build a diagnostic for free-function style calls to receiver-only methods.
pub(crate) fn free_function_receiver_method_call_error(
    method_name: StringId,
    method_entry: &ReceiverMethodEntry,
    span: Option<SourceSpan>,
    string_table: &mut StringTable,
) -> CompilerDiagnostic {
    let receiver_label = receiver_kind_label(&method_entry.receiver, string_table);
    let receiver_type_id = string_table.intern(&receiver_label);

    CompilerDiagnostic::invalid_receiver_call(
        InvalidReceiverCallReason::CalledAsFreeFunction,
        Some(receiver_type_id),
        Some(method_name),
        None,
        None,
        span,
    )
}

/// Validate that a source-authored receiver method belongs to the same file as its receiver type.
///
/// WHAT: enforces the same-file nominal rule: receiver methods are valid only for user-defined
/// structs or choices declared in the same source file as the method.
/// WHY: receiver methods travel with the type that owns them, so source files cannot attach local
/// receiver methods to values owned by another file, package, or builtin surface.
fn validate_source_receiver_method_declaration(
    receiver: &ReceiverKey,
    method_source_file: &PathId,
    struct_source_by_path: &FxHashMap<PathId, PathId>,
    choice_source_by_path: &FxHashMap<PathId, PathId>,
    span: Option<SourceSpan>,
) -> Result<(), CompilerDiagnostic> {
    match receiver {
        ReceiverKey::Struct(struct_path) => {
            let Some(struct_source_file) = struct_source_by_path.get(struct_path) else {
                return Err(CompilerDiagnostic::invalid_receiver_declaration(
                    InvalidReceiverDeclarationReason::UnknownStructTarget,
                    span,
                ));
            };

            if struct_source_file != method_source_file {
                return Err(CompilerDiagnostic::invalid_receiver_declaration(
                    InvalidReceiverDeclarationReason::NonlocalSourceType,
                    span,
                ));
            }
        }

        ReceiverKey::Choice(choice_path) => {
            let Some(choice_source_file) = choice_source_by_path.get(choice_path) else {
                return Err(CompilerDiagnostic::invalid_receiver_declaration(
                    InvalidReceiverDeclarationReason::UnknownStructTarget,
                    span,
                ));
            };

            if choice_source_file != method_source_file {
                return Err(CompilerDiagnostic::invalid_receiver_declaration(
                    InvalidReceiverDeclarationReason::NonlocalSourceType,
                    span,
                ));
            }
        }

        ReceiverKey::External(_) => {
            return Err(CompilerDiagnostic::invalid_receiver_declaration(
                InvalidReceiverDeclarationReason::ExternalOpaqueType,
                span,
            ));
        }

        ReceiverKey::BuiltinScalar(_) => {
            return Err(CompilerDiagnostic::invalid_receiver_declaration(
                InvalidReceiverDeclarationReason::BuiltinScalarType,
                span,
            ));
        }
    }

    Ok(())
}

/// Collect receiver methods and validate receiver/member naming constraints.
pub(crate) fn build_receiver_method_catalog(
    input: BuildReceiverMethodCatalogInput<'_>,
) -> Result<ReceiverMethodCatalog, ReceiverMethodCatalogError> {
    // WHAT: materializes receiver methods into lookup tables keyed by receiver/name and by name.
    // WHY: parser diagnostics and dot-call lowering both need stable, deterministic method lookup
    // without scanning declaration vectors at call sites.
    let mut catalog = ReceiverMethodCatalog::default();

    // ----------------------------
    //  Filter function headers with receivers
    // ----------------------------
    for header in input.sorted_headers {
        let HeaderKind::Function { .. } = &header.kind else {
            continue;
        };

        let Some(resolved_signature) = input
            .resolved_function_signatures_by_path
            .get(&header.tokens.src_path)
        else {
            continue;
        };

        let Some(receiver) = resolved_signature.receiver.as_ref() else {
            continue;
        };

        let Some(method_name) = input.path_fork.component(header.tokens.src_path) else {
            continue;
        };
        let Some(method_source_file) = input
            .source_file_by_symbol_path
            .get(&header.tokens.src_path)
            .copied()
        else {
            return Err(CompilerError::compiler_error(format!(
                "Receiver method '{}' is missing canonical source-file metadata.",
                input.path_fork.render_portable(
                    header.tokens.src_path,
                    input.string_table,
                    &mut Vec::new(),
                )
            ))
            .into());
        };

        validate_source_receiver_method_declaration(
            receiver,
            &method_source_file,
            input.struct_source_by_path,
            input.choice_source_by_path,
            header.name_span,
        )?;

        // ----------------------------
        //  Validate canonical struct receiver constraints
        // ----------------------------
        if let ReceiverKey::Struct(struct_path) = receiver
            && input
                .struct_fields_by_path
                .get(struct_path)
                .is_some_and(|fields| {
                    fields
                        .iter()
                        .any(|field| input.path_fork.component(field.id) == Some(method_name))
                })
        {
            return Err(CompilerDiagnostic::invalid_receiver_declaration(
                InvalidReceiverDeclarationReason::FieldNameConflict,
                header.name_span,
            )
            .into());
        }

        // ----------------------------
        //  Check for duplicate receiver methods
        // ----------------------------
        let key = (receiver.to_owned(), method_name);
        if let Some(existing_entries) = catalog.by_receiver_and_name.get(&key) {
            for existing_entry in existing_entries {
                if existing_entry.source_file == method_source_file {
                    return Err(CompilerDiagnostic::invalid_receiver_declaration(
                        InvalidReceiverDeclarationReason::DuplicateMethod,
                        header.name_span,
                    )
                    .into());
                }
            }
        }

        let receiver_mutable = resolved_signature
            .signature
            .parameters
            .first()
            .is_some_and(|parameter| parameter.value.value_mode.is_mutable());

        let entry = ReceiverMethodEntry {
            function_path: header.tokens.src_path,
            receiver: receiver.to_owned(),
            source_file: method_source_file,
            receiver_mutable,
            signature: resolved_signature.signature.to_owned(),
        };

        catalog
            .by_receiver_and_name
            .entry(key)
            .or_default()
            .push(entry.to_owned());
        catalog
            .by_method_name
            .entry(method_name)
            .or_default()
            .push(entry.to_owned());
        catalog
            .by_function_path
            .insert(header.tokens.src_path.to_owned(), entry);
    }

    for entries in catalog.by_method_name.values_mut() {
        entries.sort_by(|left, right| {
            input
                .path_fork
                .render_portable(left.function_path, input.string_table, &mut Vec::new())
                .cmp(&input.path_fork.render_portable(
                    right.function_path,
                    input.string_table,
                    &mut Vec::new(),
                ))
        });
    }

    Ok(catalog)
}
