//! Order-independent header symbol collection.
//!
//! WHAT: validates declared names, builds per-file dependency/export maps, records generic
//! declaration kinds, and stages builtin declarations during header parsing.
//! WHY: this work depends only on parsed headers, not dependency order. Keeping it separate lets
//! `prepare_header_syntax` stay orchestration-first and leaves dependency sorting as the owner of
//! declaration ordering.

use crate::compiler_frontend::builtins::casts::traits::is_core_cast_trait_name;
use crate::compiler_frontend::builtins::error_type::{
    is_reserved_builtin_symbol, register_builtin_error_types,
};
use crate::compiler_frontend::compiler_errors::CompilerError;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, DiagnosticBag, ReservedNameOwner,
};
use crate::compiler_frontend::datatypes::generic_parameters::GenericParameterList;
use crate::compiler_frontend::datatypes::parsed::ParsedTypeRef;
use crate::compiler_frontend::declaration_syntax::signature_members::FunctionSignatureSyntax;
use crate::compiler_frontend::headers::module_symbols::{
    GenericDeclarationKind, ModuleSymbols, register_declared_symbol,
};
use crate::compiler_frontend::headers::parse_file_headers::HeaderPreparationFailure;
use crate::compiler_frontend::headers::types::{
    FileFrontendPrepareOutput, FileRole, Header, HeaderExportMode, HeaderKind,
};
use crate::compiler_frontend::source::SourceId;
use crate::compiler_frontend::symbols::identifier_policy::ensure_not_keyword_shadow_identifier;
use crate::compiler_frontend::symbols::path_interner::PathInternerFork;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::projects::settings::IMPLICIT_START_FUNC_NAME;

/// Collect all order-independent top-level symbol metadata from parsed (unsorted) headers.
///
/// WHAT: registers every prepared file path, per-file dependency clauses, and declared symbols.
/// WHY: dependency-only files may produce no declaration headers but still contribute file
/// metadata and clauses to the module symbol package.
pub(super) fn build_module_symbols(
    prepared_files: &mut [FileFrontendPrepareOutput],
    string_table: &mut StringTable,
    capture: &mut impl FnMut(SourceId, &mut CompilerDiagnostic) -> Result<(), CompilerError>,
    path_fork: &mut PathInternerFork,
) -> Result<ModuleSymbols, HeaderPreparationFailure> {
    let mut module_symbols = ModuleSymbols::empty();
    let mut diagnostic_bag = DiagnosticBag::new();

    for file_output in prepared_files.iter() {
        module_symbols
            .module_file_paths
            .insert(file_output.source_file.to_owned());
        module_symbols
            .file_roles_by_source
            .insert(file_output.source_file.to_owned(), file_output.file_role);
        module_symbols
            .source_ids_by_source
            .insert(file_output.source_file.to_owned(), file_output.file_id);

        for header in &file_output.headers {
            if let Some(mut diagnostic) =
                validate_declared_name(header, file_output.file_role, string_table, path_fork)
            {
                capture(file_output.file_id, &mut diagnostic)
                    .map_err(HeaderPreparationFailure::Infrastructure)?;
                diagnostic_bag.push(diagnostic);
                continue;
            }

            // Header source paths are already the compiler's logical source identity. Keep that
            // identity in the module map instead of deriving a second filesystem path spelling.
            module_symbols.canonical_source_by_symbol_path.insert(
                header.tokens.src_path.to_owned(),
                header.source_file.clone(),
            );
            if let Some(name_span) = header.name_span {
                module_symbols
                    .declaration_spans_by_symbol_path
                    .insert(header.tokens.src_path.to_owned(), name_span);
            }

            register_header_symbol(&mut module_symbols, header, string_table, path_fork);
        }
    }

    if diagnostic_bag.has_errors() {
        return Err(HeaderPreparationFailure::Diagnosed(diagnostic_bag));
    }

    // The retained clause and selection tables have completed validation above. Move them into
    // the module-owned package now so binding reads one canonical table without cloning every
    // prepared file's selections before the file output is consumed.
    for file_output in prepared_files.iter_mut() {
        if !file_output.file_dependency_clauses.is_empty() {
            module_symbols.file_dependency_clauses_by_source.insert(
                file_output.source_file.to_owned(),
                std::mem::take(&mut file_output.file_dependency_clauses),
            );
        }
        if !file_output.dependency_selections.is_empty() {
            module_symbols.dependency_selections_by_source.insert(
                file_output.source_file.to_owned(),
                std::mem::take(&mut file_output.dependency_selections),
            );
        }
    }

    register_builtin_symbols(&mut module_symbols, string_table, path_fork);

    Ok(module_symbols)
}

fn validate_declared_name(
    header: &Header,
    file_role: FileRole,
    string_table: &StringTable,
    path_fork: &PathInternerFork,
) -> Option<CompilerDiagnostic> {
    let symbol_name = path_fork.component(header.tokens.src_path)?;

    let symbol_name_text = string_table.resolve(symbol_name);

    if let Err(diagnostic) =
        ensure_not_keyword_shadow_identifier(symbol_name, header.name_span, string_table)
    {
        return Some(diagnostic);
    }

    if is_reserved_builtin_symbol(symbol_name_text) {
        return Some(CompilerDiagnostic::reserved_name_collision(
            symbol_name,
            ReservedNameOwner::BuiltinType,
            header.name_span,
        ));
    }

    if is_core_cast_trait_name(symbol_name_text) {
        return Some(CompilerDiagnostic::reserved_name_collision(
            symbol_name,
            ReservedNameOwner::CoreTrait,
            header.name_span,
        ));
    }

    if file_role == FileRole::ActiveModuleRoot
        && !matches!(&header.kind, HeaderKind::StartFunction)
        && symbol_name_text == IMPLICIT_START_FUNC_NAME
    {
        return Some(CompilerDiagnostic::reserved_name_collision(
            symbol_name,
            ReservedNameOwner::ImplicitStart,
            header.name_span,
        ));
    }

    None
}

/// Detect whether a parsed function signature is a receiver method candidate.
///
/// WHAT: checks if the first parameter is named `this`.
/// WHY: header stage needs to route receiver methods away from free-function
///      value-member paths without waiting for AST type resolution.
/// NOTE: invalid receiver types (unsupported types, wrong file, etc.) are left
///       for AST validation; this helper only identifies the candidate shape.
pub(super) fn is_receiver_method_candidate(
    signature: &FunctionSignatureSyntax,
    string_table: &StringTable,
    path_fork: &PathInternerFork,
) -> bool {
    let Some(first_parameter) = signature.parameters.first() else {
        return false;
    };

    path_fork
        .component(first_parameter.id)
        .is_some_and(|name| string_table.resolve(name) == "this")
}

/// Extract the parsed receiver type name from a receiver-method candidate.
///
/// WHAT: records `Counter` from `tick |this Counter|` before semantic type resolution.
/// WHY: header binding preparation can then bind only methods attached to a provider
///      nominal type from the same surface instead of every receiver method in that file.
fn receiver_method_receiver_name(
    signature: &FunctionSignatureSyntax,
    string_table: &mut StringTable,
    path_fork: &PathInternerFork,
) -> Option<StringId> {
    let first_parameter = signature.parameters.first()?;

    if !path_fork
        .component(first_parameter.id)
        .is_some_and(|name| string_table.resolve(name) == "this")
    {
        return None;
    }

    match &first_parameter.type_annotation {
        ParsedTypeRef::Named { name, .. } => Some(*name),
        ParsedTypeRef::Qualified { path, .. } => {
            // Receiver type name is the final segment of a namespace-qualified path.
            path.last().copied()
        }
        // Builtin scalar types are parsed directly; map them to their language-visible names.
        ParsedTypeRef::BuiltinInt { .. } => Some(string_table.intern("Int")),
        ParsedTypeRef::BuiltinFloat { .. } => Some(string_table.intern("Float")),
        ParsedTypeRef::BuiltinBool { .. } => Some(string_table.intern("Bool")),
        ParsedTypeRef::BuiltinString { .. } => Some(string_table.intern("String")),
        ParsedTypeRef::BuiltinChar { .. } => Some(string_table.intern("Char")),
        _ => None,
    }
}

fn is_dependency_bindable_for_symbol_collection(header: &Header) -> bool {
    if matches!(
        &header.kind,
        HeaderKind::Constant { declaration }
            if declaration.config_qualifier.is_some()
    ) {
        return false;
    }

    if header.file_role == FileRole::ImportedModuleRoot {
        // Imported roots are compiled only as public module surfaces; private declarations stay
        // available for resolving the root's own exported signatures, not for consumer lookup.
        return header.export_mode == HeaderExportMode::Public;
    }

    // Active roots and ordinary source files expose authored declarations within the module.
    true
}

fn register_header_symbol(
    module_symbols: &mut ModuleSymbols,
    header: &Header,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) {
    match &header.kind {
        HeaderKind::Function {
            generic_parameters,
            signature,
        } => {
            register_declared_symbol(
                module_symbols,
                &header.tokens.src_path,
                &header.source_file,
                is_dependency_bindable_for_symbol_collection(header),
                path_fork,
            );
            register_generic_declaration_kind(
                module_symbols,
                header,
                generic_parameters,
                GenericDeclarationKind::Function,
            );
            if is_receiver_method_candidate(signature, string_table, &*path_fork) {
                module_symbols
                    .receiver_method_paths
                    .insert(header.tokens.src_path.to_owned());

                if let Some(receiver_name) =
                    receiver_method_receiver_name(signature, string_table, &*path_fork)
                {
                    module_symbols
                        .receiver_method_receiver_names
                        .insert(header.tokens.src_path.to_owned(), receiver_name);
                }
            }
        }

        HeaderKind::Struct {
            generic_parameters, ..
        } => {
            register_declared_symbol(
                module_symbols,
                &header.tokens.src_path,
                &header.source_file,
                is_dependency_bindable_for_symbol_collection(header),
                path_fork,
            );
            module_symbols
                .nominal_type_paths
                .insert(header.tokens.src_path.to_owned());
            register_generic_declaration_kind(
                module_symbols,
                header,
                generic_parameters,
                GenericDeclarationKind::Struct,
            );
        }

        HeaderKind::Choice {
            generic_parameters, ..
        } => {
            register_declared_symbol(
                module_symbols,
                &header.tokens.src_path,
                &header.source_file,
                is_dependency_bindable_for_symbol_collection(header),
                path_fork,
            );
            module_symbols
                .nominal_type_paths
                .insert(header.tokens.src_path.to_owned());
            register_generic_declaration_kind(
                module_symbols,
                header,
                generic_parameters,
                GenericDeclarationKind::Choice,
            );
        }

        HeaderKind::StartFunction => {
            // Register the compiler-owned implicit start function under its entry source file.
            let start_name = path_fork
                .try_intern_child(
                    header.source_file,
                    string_table.intern(IMPLICIT_START_FUNC_NAME),
                )
                .expect("path table exhausted while interning implicit start path");
            register_declared_symbol(module_symbols, &start_name, &header.source_file, false, path_fork);
        }

        HeaderKind::Constant { .. } => {
            register_declared_symbol(
                module_symbols,
                &header.tokens.src_path,
                &header.source_file,
                is_dependency_bindable_for_symbol_collection(header),
                path_fork,
            );
            module_symbols
                .constant_paths
                .insert(header.tokens.src_path.to_owned());
        }

        HeaderKind::TypeAlias { .. } => {
            register_declared_symbol(
                module_symbols,
                &header.tokens.src_path,
                &header.source_file,
                is_dependency_bindable_for_symbol_collection(header),
                path_fork,
            );
            module_symbols
                .type_alias_paths
                .insert(header.tokens.src_path.to_owned());
        }

        HeaderKind::ConstTemplate { .. } => {}

        HeaderKind::Trait { .. } => {
            register_declared_symbol(
                module_symbols,
                &header.tokens.src_path,
                &header.source_file,
                is_dependency_bindable_for_symbol_collection(header),
                path_fork,
            );
            module_symbols
                .trait_paths
                .insert(header.tokens.src_path.clone());
        }

        HeaderKind::TraitConformance { .. } => {
            // Conformance declarations are compile-time metadata. They do not introduce a new
            // dependency-bindable symbol; AST validates and indexes evidence later.
        }

        HeaderKind::TraitIncompatibility { .. } => {
            // Incompatibility declarations are compile-time metadata. They do not introduce a new
            // dependency-bindable symbol; AST validates and records the relation after trait registration.
        }
    }
}

fn register_builtin_symbols(
    module_symbols: &mut ModuleSymbols,
    string_table: &mut StringTable,
    path_fork: &mut PathInternerFork,
) {
    // Builtins are merged once here so AST passes see them without a separate absorption step.
    // Mutation: builtin error types register compiler-owned fixed symbols into the table.
    let builtin_manifest = register_builtin_error_types(path_fork, string_table);
    module_symbols
        .builtin_visible_symbol_paths
        .extend(builtin_manifest.visible_symbol_paths.iter().cloned());
    module_symbols.builtin_declarations = builtin_manifest.declarations;
    module_symbols
        .resolved_struct_fields_by_path
        .extend(builtin_manifest.resolved_struct_fields_by_path);
    module_symbols
        .struct_source_by_path
        .extend(builtin_manifest.struct_source_by_path);
    module_symbols
        .builtin_struct_ast_nodes
        .extend(builtin_manifest.ast_struct_nodes);
}

fn register_generic_declaration_kind(
    module_symbols: &mut ModuleSymbols,
    header: &Header,
    generic_parameters: &GenericParameterList,
    kind: GenericDeclarationKind,
) {
    if generic_parameters.is_empty() {
        return;
    }

    // Header classification records only the declaration kind. Canonical parameter names and
    // arity are owned by TypeEnvironment once AST registers the declaration.
    module_symbols
        .generic_declarations_by_path
        .insert(header.tokens.src_path.to_owned(), kind);
}
