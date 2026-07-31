//! Config, import, and path diagnostic prose.
//!
//! WHAT: renders diagnostics tied to project configuration, source imports, and compile-time paths.
//! WHY: these messages share path/string-table formatting concerns and are separate from
//! expression/type/rule diagnostic rendering.

use super::*;

pub(crate) fn invalid_config_message(
    key: Option<StringId>,
    reason: &InvalidConfigReason,
    string_table: &StringTable,
) -> String {
    let key_name = key.map(|key| string_table.resolve(key).to_owned());
    let key_label = key_name.as_deref().unwrap_or("config");

    match reason {
        InvalidConfigReason::MissingKey => "Config constant is missing a key name.".to_owned(),
        InvalidConfigReason::DuplicateKey => {
            if let Some(key_name) = key_name {
                format!("Duplicate config key '{key_name}' found. Each config key must be unique.")
            } else {
                "Duplicate config key found. Each config key must be unique.".to_owned()
            }
        }
        InvalidConfigReason::ConfigImportUnsupported => {
            "`config.moth` is self-contained and does not support imports.".to_owned()
        }
        InvalidConfigReason::FunctionUnsupported => {
            "`config.moth` does not support user-defined functions. Use earlier private helper constants for reusable folded values.".to_owned()
        }
        InvalidConfigReason::TraitDeclarationUnsupported => {
            "`config.moth` does not support trait declarations. Use ordinary source files for trait contracts.".to_owned()
        }
        InvalidConfigReason::TraitConformanceUnsupported => {
            "`config.moth` does not support trait conformance declarations. Use ordinary source files for reusable trait evidence.".to_owned()
        }
        InvalidConfigReason::TraitIncompatibilityUnsupported => {
            "`config.moth` does not support trait incompatibility declarations. Use ordinary source files for trait metadata.".to_owned()
        }
        InvalidConfigReason::MutableBindingUnsupported => {
            "`config.moth` settings must be immutable constant declarations. Use `name #= value`.".to_owned()
        }
        InvalidConfigReason::PlainBindingUnsupported => {
            format!(
                "Config key '{key_label}' must be a top-level compile-time constant. Write `{key_label} #= value` instead of a runtime binding."
            )
        }
        InvalidConfigReason::UnsupportedStatement => {
            "`config.moth` supports known setting declarations plus import/type support declarations only.".to_owned()
        }
        InvalidConfigReason::StandaloneTemplateUnsupported => {
            "`config.moth` does not support standalone templates or page fragments. Assign a folded template to a known setting instead.".to_owned()
        }
        InvalidConfigReason::MissingValue => {
            format!("Missing value for config constant '{key_label}'.")
        }
        InvalidConfigReason::UnsupportedScalarValue => {
            format!("Unsupported value for config constant '{key_label}'.")
        }
        InvalidConfigReason::NotCompileTimeConstant => {
            format!(
                "Config value '{key_label}' must be a compile-time constant value. Config declarations cannot use runtime expressions, function calls, host calls, or references to non-constant bindings."
            )
        }
        InvalidConfigReason::ValueCouldNotFold => {
            format!(
                "Config value '{key_label}' could not be fully evaluated at compile time. Config declarations cannot depend on runtime evaluation."
            )
        }
        InvalidConfigReason::UnsupportedPackageFoldersValue => {
            "Unsupported value in 'package_folders'. Use a string folder name or a collection of string folder names.".to_owned()
        }
        InvalidConfigReason::DuplicatePackageFolder { folder } => format!(
            "Duplicate 'package_folders' entries are not allowed: {}",
            string_table.resolve(*folder)
        ),
        InvalidConfigReason::InvalidPackageFolder { folder, reason } => {
            invalid_package_folder_message(*folder, *reason, string_table)
        }
        InvalidConfigReason::EmptyProjectSetting => {
            format!("Config setting '{key_label}' cannot be empty.")
        }
        InvalidConfigReason::UnknownKey { key } => format!(
            "Unknown config key '{}'. `config.moth` currently accepts only known project config keys. Helper declarations are not supported yet.",
            string_table.resolve(*key)
        ),
        InvalidConfigReason::InvalidConfigValueShape { expected } => format!(
            "Invalid value shape for config constant '{key_label}'. Expected {}.",
            string_table.resolve(*expected)
        ),
        InvalidConfigReason::InvalidProjectSettingValue { value, expected } => format!(
            "Invalid value '{}' for config setting '{key_label}'. Expected {}.",
            string_table.resolve(*value),
            string_table.resolve(*expected)
        ),
        InvalidConfigReason::MissingHtmlHomepage { entry_root } => format!(
            "HTML project builds require an artifact-producing module root at the configured entry root '{}'.",
            string_table.resolve(*entry_root),
        ),
        InvalidConfigReason::DuplicateHtmlOutputPath {
            output_path,
            entry_point,
            existing_entry_point,
        } => format!(
            "HTML builder produced duplicate output path '{}'. Entry '{}' conflicts with already-mapped entry '{}'. Ensure each '@*.moth' entry maps to a unique page output.",
            string_table.resolve(*output_path),
            string_table.resolve(*entry_point),
            string_table.resolve(*existing_entry_point),
        ),
        InvalidConfigReason::TrackedAssetOutputConflict {
            asset_path,
            output_path,
            existing_owner,
        } => format!(
            "Tracked asset '{}' would emit to '{}', but that output path is already claimed by '{}'.",
            string_table.resolve(*asset_path),
            string_table.resolve(*output_path),
            string_table.resolve(*existing_owner),
        ),
        InvalidConfigReason::TrackedAssetBuilderOutputConflict {
            asset_path,
            output_path,
        } => format!(
            "Tracked asset '{}' would emit to '{}', but that output path is already claimed by another emitted HTML builder artifact.",
            string_table.resolve(*asset_path),
            string_table.resolve(*output_path),
        ),
        InvalidConfigReason::ConfiguredEntryRootMissing { entry_root } => format!(
            "Configured entry root '{}' does not exist.",
            string_table.resolve(*entry_root),
        ),
        InvalidConfigReason::ConfiguredPackageFolderMissing { folder } => format!(
            "Configured package folder '{}' does not exist.",
            string_table.resolve(*folder),
        ),
        InvalidConfigReason::ConfiguredPackageFolderNotDirectory { folder } => format!(
            "Configured package folder '{}' is not a directory.",
            string_table.resolve(*folder),
        ),
        InvalidConfigReason::SourcePackagePrefixCollision {
            prefix,
            first_root,
            second_root,
        } => format!(
            "Configured package folder collision: source-backed package prefix '@{}' is defined by both '{}' and '{}'.",
            string_table.resolve(*prefix),
            string_table.resolve(*first_root),
            string_table.resolve(*second_root),
        ),
        InvalidConfigReason::SourcePackageBuilderPrefixCollision {
            prefixes,
            package_folders,
        } => format!(
            "Project-local package prefixes collide with Builder package prefixes: {}. Rename or remove the conflicting project-local package prefix, or update 'package_folders' (currently: {}).",
            string_table.resolve(*prefixes),
            string_table.resolve(*package_folders),
        ),
        InvalidConfigReason::EntryRootPackagePrefixCollision {
            prefix,
            entry_folder,
        } => format!(
            "Entry-root folder '{}' collides with source-backed package prefix '@{}'. Ambiguous imports are disallowed.",
            string_table.resolve(*entry_folder),
            string_table.resolve(*prefix),
        ),
        InvalidConfigReason::SourcePackageMissingRoot { prefix, root } => format!(
            "Source-backed package '@{}' at '{}' is missing a direct-child normal module root file. Every source-backed package must contain exactly one non-empty filename matching '@*.moth'.",
            string_table.resolve(*prefix),
            string_table.resolve(*root),
        ),
        InvalidConfigReason::SourcePackageMultipleRoots {
            prefix,
            root,
            candidates,
        } => {
            let candidates = candidates
                .iter()
                .map(|candidate| format!("'{}'", string_table.resolve(*candidate)))
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "Source-backed package '@{}' at '{}' has multiple direct-child normal module root files: {}. Every source-backed package must contain exactly one non-empty filename matching '@*.moth'.",
                string_table.resolve(*prefix),
                string_table.resolve(*root),
                candidates,
            )
        }
        InvalidConfigReason::NoRootModuleEntries { entry_root } => format!(
            "No root module entries were found under '{}'. Expected at least one '@*.moth' file under the configured entry root.",
            string_table.resolve(*entry_root),
        ),
        InvalidConfigReason::MultipleModuleRootFiles {
            directory,
            candidates,
        } => {
            let candidates = candidates
                .iter()
                .map(|candidate| format!("'{}'", string_table.resolve(*candidate)))
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "Module directory '{}' contains multiple normal module root files: {}. Every module directory must contain exactly one non-config '@*.moth' root file.",
                string_table.resolve(*directory),
                candidates,
            )
        }
        InvalidConfigReason::SourceFileFolderCollision {
            file_name,
            folder_name,
            directory,
        } => format!(
            "Project structure collision: '{}' and folder '{}' share the same import name in '{}'. Compiler-recognized source files and folders in the same directory must have unique import names. Rename one of them to keep import paths unambiguous.",
            string_table.resolve(*file_name),
            string_table.resolve(*folder_name),
            string_table.resolve(*directory),
        ),
        InvalidConfigReason::LegacyModuleRootFileName {
            file_name,
            directory,
        } => {
            let name = string_table.resolve(*file_name);
            let replacement = name.replacen('#', "@", 1);
            format!(
                "Legacy module root filename '{}' found in '{}'. The '#' prefix has been replaced with '@'. Rename it to '{}'.",
                name,
                string_table.resolve(*directory),
                replacement,
            )
        }
    }
}

fn invalid_package_folder_message(
    folder: Option<StringId>,
    reason: InvalidPackageFolderReason,
    string_table: &StringTable,
) -> String {
    let folder_name = folder.map(|folder| string_table.resolve(folder).to_owned());

    match reason {
        InvalidPackageFolderReason::Empty => {
            "Invalid 'package_folders' entry. Package folders cannot be empty.".to_owned()
        }
        InvalidPackageFolderReason::AbsolutePath => {
            let folder_name = folder_name.unwrap_or_else(|| "<empty>".to_owned());
            format!(
                "Invalid 'package_folders' entry '{folder_name}'. Package folders must be relative to the project root."
            )
        }
        InvalidPackageFolderReason::ParentDirectorySegment => {
            let folder_name = folder_name.unwrap_or_else(|| "<empty>".to_owned());
            format!(
                "Invalid 'package_folders' entry '{folder_name}'. Parent-directory segments ('..') are not allowed."
            )
        }
        InvalidPackageFolderReason::NestedPath => {
            let folder_name = folder_name.unwrap_or_else(|| "<empty>".to_owned());
            format!(
                "Invalid 'package_folders' entry '{folder_name}'. Package folders must be a single top-level folder name such as \"lib\"."
            )
        }
    }
}

pub(crate) fn invalid_import_path_message(
    path: &InternedPath,
    reason: InvalidImportPathReason,
    string_table: &StringTable,
) -> String {
    match reason {
        InvalidImportPathReason::ParentDirectorySegment => format!(
            "Import paths containing '..' are not supported: '{}'",
            path.to_portable_string(string_table)
        ),
        InvalidImportPathReason::EscapesProjectRoot => format!(
            "Import escapes the project root and is not allowed: '{}'",
            path.to_portable_string(string_table)
        ),
        InvalidImportPathReason::EscapesSourcePackageRoot => format!(
            "Import escapes the source-backed package root and is not allowed: '{}'",
            path.to_portable_string(string_table)
        ),
        InvalidImportPathReason::CaseMismatch { provided, expected } => format!(
            "Import path case mismatch: '{}' should be '{}'.",
            string_table.resolve(provided),
            string_table.resolve(expected),
        ),
    }
}

pub(crate) fn invalid_compile_time_path_message(
    path: &InternedPath,
    reason: InvalidCompileTimePathReason,
    string_table: &StringTable,
) -> String {
    let path_text = path.to_portable_string(string_table);

    match reason {
        InvalidCompileTimePathReason::MissingTarget => format!(
            "Compile-time path '{path_text}' does not exist. Check that the file or directory exists relative to the configured path base."
        ),
        InvalidCompileTimePathReason::EscapesProjectRoot => format!(
            "Compile-time path '{path_text}' escapes the project root. Use a path inside the project root or move the target into the project."
        ),
    }
}

pub(crate) fn invalid_path_message(path_kind: PathKind) -> &'static str {
    match path_kind {
        PathKind::Empty => {
            "Path cannot be empty. Paths must start with a valid prefix such as './', '../', or '@name/'."
        }
        PathKind::TrailingSeparator => {
            "Path cannot end with a trailing separator. Remove the final '/'."
        }
        PathKind::InvalidRoot => {
            "Invalid path root. Paths must start with './', '../', '@name/', or '@/'."
        }
        PathKind::InvalidComponent => {
            "Invalid path component. Use path components without syntax delimiters or cross-platform reserved filename characters."
        }
        PathKind::InvalidGroupedSyntax => "Invalid grouped path syntax.",
        PathKind::OnlyRootSlashSupported => {
            "Only exact \"@/\" is supported as the public root path. Use '@name/...' for rooted paths."
        }
        PathKind::SlashBeforeGroup => {
            "Slash-before-group syntax is not supported. Use 'base { ... }'."
        }
        PathKind::EmptyComponent => "Empty path component. Consecutive separators are not allowed.",
        PathKind::WhitespaceMustBeQuoted => {
            "Path components with whitespace must be quoted. Wrap the component in double quotes."
        }
        PathKind::MissingSeparator => {
            "Missing path separator. Path components must be separated by '/'."
        }
        PathKind::MissingClosingBrace => "Grouped path is missing a closing '}'.",
        PathKind::MissingClosingQuote => {
            "Unclosed quoted path component. Quoted components must end with a double quote."
        }
        PathKind::InvalidEscape => {
            "Invalid escape in quoted path component. Only '\"' and '\\' are supported."
        }
        PathKind::EmptyGroupedBlock => "Grouped path requires at least one entry.",
        PathKind::EntriesNeedCommas => "Grouped path entries must be separated by commas.",
        PathKind::MultipleCommas => "Consecutive commas are not allowed in grouped paths.",
        PathKind::AliasOnlyOnLeaf => "Path aliases are only valid on leaf entries.",
        PathKind::NestedGroupNeedsPrefix => "Nested groups require a non-empty prefix.",
        PathKind::GroupedEntryEmpty => "Grouped path entry cannot be empty.",
        PathKind::GroupedPrefixTrailingSeparator => {
            "Grouped path prefix cannot end with a separator."
        }
    }
}

pub(crate) fn direct_symbol_path_import_message(
    path: &InternedPath,
    string_table: &StringTable,
) -> String {
    let path_text = path.to_portable_string(string_table);
    format!(
        "Direct symbol-path imports are not supported: `@{path_text}`.\n\
         Import from the containing surface with grouped syntax, such as `import @path/to/file {{ symbol }}`, \
         or import the containing namespace and access a member with `namespace.symbol`.",
    )
}

pub(crate) fn invalid_namespace_default_name_message(
    path: &InternedPath,
    string_table: &StringTable,
) -> String {
    let path_text = path.to_portable_string(string_table);
    let stem = path.name().map(|n| string_table.resolve(n)).unwrap_or("");
    // Ensure the rendered example includes the @ prefix that import paths require.
    let at_prefix = if path_text.starts_with('@') { "" } else { "@" };
    format!(
        "Cannot derive an import namespace name from `{stem}`.\n\
         Use an explicit alias, for example `import {at_prefix}{path_text} as my_name`.",
    )
}

pub(crate) fn duplicate_import_surface_member_message(
    surface_path: &InternedPath,
    member_name: StringId,
    string_table: &StringTable,
) -> String {
    let path_text = surface_path.to_portable_string(string_table);
    let member = string_table.resolve(member_name);
    format!(
        "Import surface `{path_text}` exposes more than one member named `{member}`.\n\
         Moth import records require unique member names, even across value and type contexts.\n\
         Rename or alias one of the exported members.",
    )
}

pub(crate) fn explicit_moth_extension_message(
    path: &InternedPath,
    string_table: &StringTable,
) -> String {
    let path_text = path.to_portable_string(string_table);
    let extensionless_path = path_text.strip_suffix(".moth").unwrap_or(&path_text);
    format!(
        "Import paths must not include the `.moth` extension: `@{path_text}`.\n\
         Use `@{extensionless_path}` instead.",
    )
}

pub(crate) fn explicit_source_extension_message(
    path: &InternedPath,
    extension: StringId,
    string_table: &StringTable,
) -> String {
    let path_text = path.to_portable_string(string_table);
    let extension = string_table.resolve(extension);
    let suffix = format!(".{extension}");
    let extensionless_path = path_text.strip_suffix(&suffix).unwrap_or(&path_text);
    format!(
        "Import paths must not include the `.{extension}` source-file extension: `@{path_text}`.\n\
         Use `@{extensionless_path}` instead.",
    )
}

pub(crate) fn unsupported_source_file_kind_message(
    path: &InternedPath,
    extension: StringId,
    string_table: &StringTable,
) -> String {
    let path = path.to_portable_string(string_table);
    let extension = string_table.resolve(extension);
    format!(
        "Import `{path}` resolves to a recognized source file kind `.{extension}`, but this builder does not support it.\n\
         Use a builder that supports `.{extension}` files or import a Moth source file instead.",
    )
}

pub(crate) fn invalid_source_file_entry_message(
    path: &InternedPath,
    extension: StringId,
    string_table: &StringTable,
) -> String {
    let path = path.to_portable_string(string_table);
    let extension = string_table.resolve(extension);
    format!(
        "Entry file `{path}` uses the `.{extension}` source-file kind, but source assets cannot be compiled as page or module entries.\n\
         Import this file from a `.moth` entry file using extensionless import syntax, or use a `.moth`/`@page.moth` file as the build entry.",
    )
}

pub(crate) fn invalid_moth_template_api_scope_item_message(
    path: &InternedPath,
    string_table: &StringTable,
) -> String {
    let path = path.to_portable_string(string_table);
    format!(
        "Direct Moth template compilation for `{path}` does not support caller-supplied scope constants yet.\n\
         Remove the scope constants from the request, or expose compile-time constants through the compiler-integrated `@html` and same-directory module-root public export paths."
    )
}

pub(crate) fn duplicate_moth_template_input_path_message(
    path: &InternedPath,
    string_table: &StringTable,
) -> String {
    let path = path.to_portable_string(string_table);
    format!(
        "Moth template input path `{path}` was provided more than once. Each file or in-memory display path in one direct compile request must be unique."
    )
}

pub(crate) fn unsupported_external_extension_message(
    path: &InternedPath,
    extension: StringId,
    string_table: &StringTable,
) -> String {
    let path = path.to_portable_string(string_table);
    let ext = string_table.resolve(extension);
    format!(
        "External file import `{path}` uses extension `.{ext}`, which is not supported by this builder.\n\
         Register an external import provider for `.{ext}` or import a Moth source file instead.",
    )
}

pub(crate) fn invalid_external_module_message(
    path: &InternedPath,
    message: StringId,
    string_table: &StringTable,
) -> String {
    let path = path.to_portable_string(string_table);
    let message = string_table.resolve(message);
    format!("External JS module `{path}` is invalid.\n{message}")
}

pub(crate) fn import_record_used_as_value_message(
    record_name: StringId,
    string_table: &StringTable,
) -> String {
    let name = string_table.resolve(record_name);
    format!(
        "`{name}` is an import namespace, not a value.\n\
         Use `{name}.member` for imported values or `{name}.Type` in type position.\n\
         For Moth template and Markdown content files, the generated string is always `{name}.content`.\n\
         Alternative: import @path {{ content as {name} }}",
    )
}

pub(crate) fn const_record_used_as_value_message(
    record_name: StringId,
    string_table: &StringTable,
) -> String {
    let name = string_table.resolve(record_name);
    format!(
        "Records are compile-time field records and cannot be used as values.\n\
         They are used to group named fields, module imports, and compile-time members.\n\
         Access a field instead, for example `{name}.member`.",
    )
}

pub(crate) fn namespace_type_value_misuse_message(
    name: StringId,
    expected: NamespaceTypeValueMisuseKind,
    found: NamespaceTypeValueMisuseKind,
    string_table: &StringTable,
) -> String {
    let name = string_table.resolve(name);
    match (expected, found) {
        (NamespaceTypeValueMisuseKind::Type, NamespaceTypeValueMisuseKind::Value) => {
            format!("`{name}` is a value member of the import record and cannot be used as a type.")
        }
        (NamespaceTypeValueMisuseKind::Value, NamespaceTypeValueMisuseKind::Type) => {
            format!("`{name}` is a type member of the import record and cannot be used as a value.")
        }
        (NamespaceTypeValueMisuseKind::Value, NamespaceTypeValueMisuseKind::Namespace) => {
            format!(
                "`{name}` is a namespace member of the import record and cannot be used as a value or type."
            )
        }
        (NamespaceTypeValueMisuseKind::Type, NamespaceTypeValueMisuseKind::Namespace) => {
            format!(
                "`{name}` is a namespace member of the import record and cannot be used as a type."
            )
        }
        (NamespaceTypeValueMisuseKind::Namespace, NamespaceTypeValueMisuseKind::Value) => {
            format!(
                "`{name}` is a value member of the import record and cannot be used as a namespace."
            )
        }
        (NamespaceTypeValueMisuseKind::Namespace, NamespaceTypeValueMisuseKind::Type) => {
            format!(
                "`{name}` is a type member of the import record and cannot be used as a namespace."
            )
        }
        _ => format!("`{name}` cannot be used in this context."),
    }
}

pub(crate) fn nested_traversal_message(
    _record_name: StringId,
    _string_table: &StringTable,
) -> String {
    String::from(
        "Import records do not expose nested filesystem paths as fields.\n\
         Import the child path directly, for example `import @child/path as child`, or use a nested grouped import.",
    )
}

pub(crate) fn invalid_import_clause_message(reason: InvalidImportClauseReason) -> &'static str {
    match reason {
        InvalidImportClauseReason::MissingPath => "Expected a path after the 'import' keyword.",
        InvalidImportClauseReason::ExpectedPath => {
            "Expected a path after the 'import' keyword, found something else."
        }
        InvalidImportClauseReason::MissingAlias => {
            "Expected an alias after `as`.\nWrite `import @path as local_name` or `import @path { symbol as local_name }`."
        }
        InvalidImportClauseReason::ExpectedAliasName => "Expected alias name after `as`.",
        InvalidImportClauseReason::AliasNotValidIdentifier => {
            "Import alias must be a valid local binding name."
        }
        InvalidImportClauseReason::AliasIsKeyword => "Import alias cannot be a reserved keyword.",
        InvalidImportClauseReason::GroupedWithTrailingAlias => {
            "Grouped imports cannot use one alias for the whole group.\nAlias individual entries instead: `import @path { symbol as local_name }`."
        }
        InvalidImportClauseReason::PerEntryAndTrailingAlias => {
            "Cannot use both per-entry aliases and a group-level alias."
        }
        InvalidImportClauseReason::MultipleTrailingAliases => {
            "Import clauses can only have one alias."
        }
        InvalidImportClauseReason::DoubleAliasInGroupedEntry => {
            "Grouped import entries can only have one alias."
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn import_record_used_as_value_message_contains_record_name_and_content_hint() {
        let mut string_table = StringTable::new();
        let record_name = string_table.intern("intro");
        let message = import_record_used_as_value_message(record_name, &string_table);

        assert!(
            message.contains("`intro`"),
            "message should contain the record name: {message}"
        );
        assert!(
            message.contains("intro.content"),
            "message should mention `intro.content`: {message}"
        );
        assert!(
            message.contains("content as intro"),
            "message should mention grouped `content as ...` import: {message}"
        );
    }

    #[test]
    fn empty_project_setting_renders_authored_key_without_marker() {
        let mut string_table = StringTable::new();
        let key = string_table.intern("html_lang");
        let message = invalid_config_message(
            Some(key),
            &InvalidConfigReason::EmptyProjectSetting,
            &string_table,
        );

        assert_eq!(
            message, "Config setting 'html_lang' cannot be empty.",
            "EmptyProjectSetting should render the exact authored key name"
        );
    }

    #[test]
    fn invalid_project_setting_value_renders_authored_key_without_marker() {
        let mut string_table = StringTable::new();
        let key = string_table.intern("page_url_style");
        let value = string_table.intern("slashy");
        let expected = string_table.intern("'trailing_slash', 'no_trailing_slash', or 'ignore'");
        let reason = InvalidConfigReason::InvalidProjectSettingValue { value, expected };
        let message = invalid_config_message(Some(key), &reason, &string_table);

        assert_eq!(
            message,
            "Invalid value 'slashy' for config setting 'page_url_style'. Expected 'trailing_slash', 'no_trailing_slash', or 'ignore'.",
            "InvalidProjectSettingValue should render exact authored key and value facts"
        );
    }

    #[test]
    fn unknown_key_renders_authored_key() {
        let mut string_table = StringTable::new();
        let key = string_table.intern("custom_key");
        let reason = InvalidConfigReason::UnknownKey { key };
        let message = invalid_config_message(Some(key), &reason, &string_table);

        assert_eq!(
            message,
            "Unknown config key 'custom_key'. `config.moth` currently accepts only known project config keys. Helper declarations are not supported yet.",
            "UnknownKey should render the exact authored key name"
        );
    }
}
