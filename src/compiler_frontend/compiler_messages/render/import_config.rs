//! Config, import, and path diagnostic prose.
//!
//! WHAT: renders diagnostics tied to project configuration, source dependencies, and compile-time paths.
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
            "`config.moth` is self-contained and does not support dependency clauses.".to_owned()
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
            "`config.moth` supports known setting declarations plus `#Import`/type support declarations only.".to_owned()
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
        InvalidConfigReason::EntryRootPackagePrefixCollision {
            prefix,
            entry_folder,
        } => format!(
            "Entry-root folder '{}' collides with source-backed package prefix '@{}'. Ambiguous dependencies are disallowed.",
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
            "Project structure collision: '{}' and folder '{}' share the same dependency name in '{}'. Compiler-recognized source files and folders in the same directory must have unique dependency names. Rename one of them to keep dependency paths unambiguous.",
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
        InvalidConfigReason::InvalidOutputFolder { folder, reason } => {
            invalid_output_folder_message(key_label, *folder, *reason, string_table)
        }
        InvalidConfigReason::OutputFoldersNotDistinct {
            dev_folder,
            release_folder,
        } => format!(
            "Development and release output folders '{}' and '{}' resolve to the same output root and must be distinct.",
            string_table.resolve(*dev_folder),
            string_table.resolve(*release_folder),
        ),
        InvalidConfigReason::OutputManifestOwnerConflict {
            output_root,
            existing_builder,
            existing_profile,
            active_builder,
            active_profile,
        } => format!(
            "Build output root '{}' is already owned by builder '{}' in profile '{}', but the active build is builder '{}' in profile '{}'. Choose a different output folder or remove the output only after verifying its owner.",
            string_table.resolve(*output_root),
            string_table.resolve(*existing_builder),
            string_table.resolve(*existing_profile),
            string_table.resolve(*active_builder),
            string_table.resolve(*active_profile),
        ),
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

fn invalid_output_folder_message(
    key_label: &str,
    folder: Option<StringId>,
    reason: InvalidOutputFolderReason,
    string_table: &StringTable,
) -> String {
    let folder_name = folder.map(|folder| string_table.resolve(folder).to_owned());

    match reason {
        InvalidOutputFolderReason::Empty => {
            format!(
                "'{key_label}' cannot be empty. Configure a project-relative output folder in config.moth."
            )
        }
        InvalidOutputFolderReason::NonUtf8 => {
            format!("'{key_label}' must use valid UTF-8 portable path components.")
        }
        InvalidOutputFolderReason::AbsolutePath => {
            let name = folder_name.unwrap_or_else(|| "<empty>".to_owned());
            format!("'{key_label}' '{name}' must be relative to the project root, not absolute.")
        }
        InvalidOutputFolderReason::RootOrPrefix => {
            format!(
                "'{key_label}' must be a project-relative path, not a root or platform-prefix path."
            )
        }
        InvalidOutputFolderReason::ParentDirectorySegment => {
            let name = folder_name.unwrap_or_else(|| "<empty>".to_owned());
            format!("'{key_label}' '{name}' must not contain parent-directory segments ('..').")
        }
        InvalidOutputFolderReason::CurrentDirectory => {
            format!(
                "'{key_label}' must not be '.'. Configure a named project-relative output folder."
            )
        }
        InvalidOutputFolderReason::InvalidPathComponent => {
            let name = folder_name.unwrap_or_else(|| "<empty>".to_owned());
            format!(
                "'{key_label}' '{name}' contains a Windows-incompatible or otherwise invalid path component."
            )
        }
        InvalidOutputFolderReason::InsideOrEqualToEntryRoot => {
            let name = folder_name.unwrap_or_else(|| "<empty>".to_owned());
            format!(
                "'{key_label}' '{name}' must be outside the source entry root. Configure a distinct output folder."
            )
        }
        InvalidOutputFolderReason::ResolvesOutsideProjectRoot => {
            let name = folder_name.unwrap_or_else(|| "<empty>".to_owned());
            format!(
                "'{key_label}' '{name}' must resolve strictly inside the project root and may not escape through a symlink."
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
        InvalidImportPathReason::PublicRoot => {
            "Dependency paths must name a provider under the owning module root; '@/' cannot be used as a dependency.".to_owned()
        }
        InvalidImportPathReason::CurrentDirectorySegment => format!(
            "Dependency paths resolve from the owning module root, so '@./{}' is not supported. Remove './'.",
            path.to_portable_string(string_table)
                .trim_start_matches("./")
        ),
        InvalidImportPathReason::ParentDirectorySegment => format!(
            "Dependency paths containing '..' are not supported: '{}'",
            path.to_portable_string(string_table)
        ),
        InvalidImportPathReason::EscapesProjectRoot => format!(
            "Dependency path escapes the project root and is not allowed: '{}'",
            path.to_portable_string(string_table)
        ),
        InvalidImportPathReason::EscapesSourcePackageRoot => format!(
            "Dependency path escapes the source-backed package root and is not allowed: '{}'",
            path.to_portable_string(string_table)
        ),
        InvalidImportPathReason::CaseMismatch { provided, expected } => format!(
            "Dependency path case mismatch: '{}' should be '{}'.",
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
        PathKind::OnlyRootSlashSupported => {
            "Only exact \"@/\" is supported as the public root path. Use '@name/...' for rooted paths."
        }
        PathKind::EmptyComponent => "Empty path component. Consecutive separators are not allowed.",
        PathKind::WhitespaceMustBeQuoted => {
            "Path components with whitespace must be quoted. Wrap the component in double quotes."
        }
        PathKind::MissingSeparator => {
            "Missing path separator. Path components must be separated by '/'."
        }
        PathKind::MissingClosingQuote => {
            "Unclosed quoted path component. Quoted components must end with a double quote."
        }
        PathKind::InvalidEscape => {
            "Invalid escape in quoted path component. Only '\"' and '\\' are supported."
        }
        PathKind::LeadingAtInPathComponent => {
            "The leading '@' starts a dependency path and is not part of the module name.\n\
             Depend on the module directory, for example `@pages`.\n\
             Normal module-root filenames such as `@page.moth` are not referenced directly."
        }
    }
}

pub(crate) fn direct_symbol_path_import_message(
    path: &InternedPath,
    string_table: &StringTable,
) -> String {
    let path_text = path.to_portable_string(string_table);
    format!(
        "Direct symbol dependency paths are not supported: `@{path_text}`.\n\
         Select the symbol from its containing surface, such as `@path/to/file symbol`, \
         or bind the containing namespace and access `namespace.symbol`.",
    )
}

pub(crate) fn invalid_namespace_default_name_message(
    path: &InternedPath,
    string_table: &StringTable,
) -> String {
    let path_text = path.to_portable_string(string_table);
    let stem = path.name().map(|n| string_table.resolve(n)).unwrap_or("");
    // Ensure the rendered example includes the @ prefix that dependency paths require.
    let at_prefix = if path_text.starts_with('@') { "" } else { "@" };
    format!(
        "Cannot derive a dependency namespace name from `{stem}`.\n\
         Use an explicit alias, for example `{at_prefix}{path_text} as my_name`.",
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
        "Dependency surface `{path_text}` exposes more than one member named `{member}`.\n\
         Moth dependency namespace records require unique member names, even across value and type contexts.\n\
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
        "Dependency paths must not include the `.moth` extension: `@{path_text}`.\n\
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
        "Dependency paths must not include the `.{extension}` source-file extension: `@{path_text}`.\n\
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
        "Dependency `{path}` resolves to a recognized source file kind `.{extension}`, but this builder does not support it.\n\
         Use a builder that supports `.{extension}` files or depend on a Moth source file instead.",
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
         Depend on this file from a `.moth` entry file using extensionless dependency syntax, or use a `.moth`/`@page.moth` file as the build entry.",
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
         Register an external import provider for `.{ext}` or depend on a Moth source file instead.",
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

pub(crate) fn dependency_namespace_used_as_value_message(
    record_name: StringId,
    string_table: &StringTable,
) -> String {
    let name = string_table.resolve(record_name);
    format!(
        "`{name}` is a dependency namespace binding, not a value.\n\
         Use `{name}.member` for bound values or `{name}.Type` in type position.\n\
         For Moth template and Markdown content files, the generated string is always `{name}.content`.\n\
         Alternative: `@path content as {name}`",
    )
}

pub(crate) fn const_record_used_as_value_message(
    record_name: StringId,
    string_table: &StringTable,
) -> String {
    let name = string_table.resolve(record_name);
    format!(
        "Records are compile-time field records and cannot be used as values.\n\
         They are used to group named fields, module dependencies, and compile-time members.\n\
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
            format!(
                "`{name}` is a value member of the dependency namespace and cannot be used as a type."
            )
        }
        (NamespaceTypeValueMisuseKind::Value, NamespaceTypeValueMisuseKind::Type) => {
            format!(
                "`{name}` is a type member of the dependency namespace and cannot be used as a value."
            )
        }
        (NamespaceTypeValueMisuseKind::Value, NamespaceTypeValueMisuseKind::Namespace) => {
            format!(
                "`{name}` is a namespace member of the dependency namespace and cannot be used as a value or type."
            )
        }
        (NamespaceTypeValueMisuseKind::Type, NamespaceTypeValueMisuseKind::Namespace) => {
            format!(
                "`{name}` is a namespace member of the dependency namespace and cannot be used as a type."
            )
        }
        (NamespaceTypeValueMisuseKind::Namespace, NamespaceTypeValueMisuseKind::Value) => {
            format!(
                "`{name}` is a value member of the dependency namespace and cannot be used as a namespace."
            )
        }
        (NamespaceTypeValueMisuseKind::Namespace, NamespaceTypeValueMisuseKind::Type) => {
            format!(
                "`{name}` is a type member of the dependency namespace and cannot be used as a namespace."
            )
        }
        _ => format!("`{name}` cannot be used in this context."),
    }
}

pub(crate) fn nested_dependency_traversal_message(
    _record_name: StringId,
    _string_table: &StringTable,
) -> String {
    String::from(
        "Dependency namespace records do not expose nested filesystem paths as fields.\n\
         Bind the child path directly as a separate clause, for example `@child/path as child`.",
    )
}

pub(crate) fn invalid_dependency_clause_message(
    reason: InvalidDependencyClauseReason,
) -> &'static str {
    match reason {
        InvalidDependencyClauseReason::MissingPath => {
            "Expected a dependency path beginning with `@`."
        }
        InvalidDependencyClauseReason::ExpectedPath => {
            "Expected a dependency path beginning with `@`, found something else."
        }
        InvalidDependencyClauseReason::ExpectedSelectionName => {
            "Expected a direct selected name. Dependency selections are flat identifiers separated by commas."
        }
        InvalidDependencyClauseReason::MissingAlias => {
            "Expected an alias after `as`.\nWrite `@path as local_name` or `@path symbol as local_name`."
        }
        InvalidDependencyClauseReason::ExpectedAliasName => "Expected alias name after `as`.",
        InvalidDependencyClauseReason::DuplicateSelectionName => {
            "A dependency clause cannot select the same surface member more than once. Keep one selection and use its local alias."
        }
        InvalidDependencyClauseReason::DuplicateSelectionLocalName => {
            "A dependency clause cannot bind two selections to the same local name. Rename or alias one selection."
        }
        InvalidDependencyClauseReason::LegacyBraceSelections => {
            "Dependency selections no longer use braces. Write selected names directly after the path, separated by commas."
        }
        InvalidDependencyClauseReason::MissingSelectionAfterComma => {
            "A dependency-clause comma must be followed by another selected name. Remove the comma to end the clause, or add the promised selection. Trailing commas are not allowed."
        }
        InvalidDependencyClauseReason::MissingCommaBetweenSelections => {
            "Dependency selections must be separated by commas. After a continued line, remove the previous comma to end the clause or complete the promised selection."
        }
        InvalidDependencyClauseReason::NamespaceAliasWithSelections => {
            "A namespace alias cannot be followed by direct selections. Use either `@path as namespace` or `@path name, other`."
        }
        InvalidDependencyClauseReason::InvalidSelectionDelimiter => {
            "Dependency selections are delimiter-free. Parentheses, pipes, colon blocks and other selection delimiters are not allowed."
        }
        InvalidDependencyClauseReason::DependencyClauseNotAllowed => {
            "Dependency clauses are not allowed in this file kind."
        }
        InvalidDependencyClauseReason::ProviderRequiresBinding => {
            "An explicit-extension provider clause requires a namespace alias or at least one direct selection."
        }
        InvalidDependencyClauseReason::ContinuationEnteredStatement => {
            "The comma continued the dependency clause, so this name was consumed as the next selected dependency name. Remove the comma to end the clause and start the following statement."
        }
    }
}
