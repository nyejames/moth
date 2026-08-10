//! Config value validation and application helpers for Stage 0 project config loading.
//!
//! WHAT: validates parsed config constants, converts supported value shapes, and applies the
//! results to [`Config`].
//! WHY: separating value semantics from AST construction keeps the Stage 0 pipeline easier to
//! audit and extend.

use crate::build_system::output::{
    ValidatedDirectoryOutputSettings, ValidatedOutputFolder, canonical_output_root_for_identity,
    classify_output_folder, output_path_identity, validate_output_folder_containment,
};
use crate::build_system::project_config::parsing::ParsedConfigFile;

use crate::builder_surface::config_key_registry::{
    ConfigKeyEntry, ConfigKeyOwner, ConfigValueShape, ProjectConfigKeyRegistry,
    config_value_shape_name,
};
use crate::compiler_frontend::ast::ast_nodes::{Declaration, NodeKind};
use crate::compiler_frontend::ast::const_values::facts::{AstConstFacts, ConstFactValueKind};
use crate::compiler_frontend::ast::expressions::expression::{Expression, ExpressionKind};
use crate::compiler_frontend::compiler_errors::SourceLocation;
use crate::compiler_frontend::compiler_messages::{
    CompilerDiagnostic, InvalidConfigReason, InvalidOutputFolderReason, InvalidPackageFolderReason,
};
use crate::compiler_frontend::symbols::interned_path::InternedPath;
use crate::compiler_frontend::symbols::string_interning::{StringId, StringTable};
use crate::projects::settings::{
    Config, IMPLICIT_START_FUNC_NAME, MAX_TEMPLATE_CONST_LOOP_ITERATIONS,
    TEMPLATE_CONST_LOOP_ITERATION_LIMIT_KEY,
};

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

// -------------------------
//  Validation Entry Point
// -------------------------

/// Validate AST-extracted config declarations and apply accepted values to the runtime config.
///
/// WHY: this keeps duplicate detection and value semantics in one place after AST construction
/// has produced folded expressions.
pub(super) fn validate_and_apply_config_ast(
    config: &mut Config,
    parsed_config: &ParsedConfigFile,
    config_keys: &ProjectConfigKeyRegistry,
    string_table: &mut StringTable,
) -> Result<(), Vec<CompilerDiagnostic>> {
    let mut errors = Vec::new();
    let mut seen_config_keys = HashSet::new();

    // Only top-level compile-time constant declarations authored in `config.moth` are config keys.
    // Config files are self-contained: imported declarations are never config entries. The
    // authored scope is the exact interned identity the parser used for tokenization, so
    // membership is checked by direct interned equality rather than by converting paths back to
    // `PathBuf`.
    let authored_scope = &parsed_config.authored_scope;

    // 1. Extract authored top-level compile-time constants.
    for declaration in &parsed_config.ast.module_constants {
        // A module constant's source file is the parent of its symbol path.
        // WHY: the value expression's location scope may be normalized to an imported
        // file when the initializer references an imported constant, so the declaration id
        // is the reliable source-of-authority for which file owns the constant.
        if declaration.id.parent().as_ref() != Some(authored_scope) {
            continue;
        }

        let key = declaration
            .id
            .name_str(string_table)
            .unwrap_or("")
            .to_string();
        if !seen_config_keys.insert(key.clone()) {
            errors.push(config_diagnostic(
                Some(string_table.intern(&key)),
                InvalidConfigReason::DuplicateKey,
                key_identity_location(declaration, &parsed_config.authored_key_name_locations)
                    .clone(),
            ));
            continue;
        }

        if let Err(mut decl_errors) = extract_config_declaration(
            config,
            declaration,
            config_keys,
            &parsed_config.ast.const_facts,
            &parsed_config.authored_key_name_locations,
            string_table,
        ) {
            errors.append(&mut decl_errors);
        }
    }

    // 2. Reject authored start-body statements in `config.moth`.
    // Only top-level compile-time constants are config entries. Plain bindings and runtime
    // statements are not.
    for node in &parsed_config.ast.nodes {
        let NodeKind::Function(path, _, body) = &node.kind else {
            continue;
        };

        if path.name_str(string_table) != Some(IMPLICIT_START_FUNC_NAME) {
            continue;
        }

        for body_node in body {
            // Only consider statements authored in the config file itself.
            if body_node.scope != *authored_scope {
                continue;
            }

            match &body_node.kind {
                NodeKind::VariableDeclaration(declaration) => {
                    let key = declaration
                        .id
                        .name_str(string_table)
                        .unwrap_or("")
                        .to_string();
                    errors.push(config_diagnostic(
                        Some(string_table.intern(&key)),
                        InvalidConfigReason::PlainBindingUnsupported,
                        declaration.value.location.clone(),
                    ));
                }
                NodeKind::PushStartRuntimeFragment(_) => errors.push(config_diagnostic(
                    None,
                    InvalidConfigReason::StandaloneTemplateUnsupported,
                    body_node.location.clone(),
                )),
                _ => errors.push(config_diagnostic(
                    None,
                    InvalidConfigReason::UnsupportedStatement,
                    body_node.location.clone(),
                )),
            }
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

// -------------------------
//  AST Declaration Extraction
// -------------------------

/// Extract one config key-value pair from a folded top-level compile-time constant declaration.
///
/// WHY: top-level compile-time constants in the authored config file are the only source of
/// config entries.
fn extract_config_declaration(
    config: &mut Config,
    declaration: &Declaration,
    config_keys: &ProjectConfigKeyRegistry,
    const_facts: &AstConstFacts,
    authored_key_name_locations: &HashMap<InternedPath, SourceLocation>,
    string_table: &mut StringTable,
) -> Result<(), Vec<CompilerDiagnostic>> {
    let key = declaration
        .id
        .name_str(string_table)
        .unwrap_or("")
        .to_string();
    let location = declaration.value.location.clone();

    if declaration.value.value_mode.is_mutable() {
        return Err(vec![config_diagnostic(
            declaration.id.name(),
            InvalidConfigReason::MutableBindingUnsupported,
            location,
        )]);
    }

    config
        .setting_locations
        .insert(key.clone(), location.clone());

    // Every config declaration must be a known key before Stage 0 stores it. All unregistered
    // names flow through the ordinary `UnknownKey` reason.
    let Some(config_key) = config_keys.lookup(&key) else {
        let key_id = string_table.intern(&key);

        return Err(vec![config_diagnostic(
            Some(key_id),
            InvalidConfigReason::UnknownKey { key: key_id },
            key_identity_location(declaration, authored_key_name_locations).clone(),
        )]);
    };

    // Look up the const fact for this declaration. Config values must resolve to
    // compile-time constants through the shared const resolver.
    let fact = const_facts.declarations.get(&declaration.id);

    let resolved_expression = match fact {
        Some(fact) if fact.value_kind != ConstFactValueKind::NonConst => &fact.resolved_expression,
        _ => {
            return Err(vec![config_diagnostic(
                Some(string_table.intern(&key)),
                InvalidConfigReason::NotCompileTimeConstant,
                declaration.value.location.clone(),
            )]);
        }
    };

    // Enforce the registered value shape before applying or storing the config value.
    // WHY: the registry declares broad shapes so Stage 0 can reject clearly wrong values
    // before they reach backend-specific validation.
    let validated =
        match extract_config_value_for_shape(resolved_expression, config_key.shape, string_table) {
            Ok(value) => value,
            Err(reason) => {
                return Err(vec![config_diagnostic(
                    Some(string_table.intern(&key)),
                    reason,
                    declaration.value.location.clone(),
                )]);
            }
        };

    apply_validated_config_value(
        config,
        config_key,
        &key,
        validated,
        &declaration.value.location,
        string_table,
    )?;
    Ok(())
}

// -------------------------
//  Shape Extraction
// -------------------------

/// A config value that has been validated against its registered [`ConfigValueShape`].
///
/// WHY: carrying the validated shape lets `apply_validated_config_value` dispatch cleanly
/// without re-inspecting the AST expression.
enum ValidatedConfigValue {
    String(String),
    Int(i32),
    Bool(bool),
    StringCollection(Vec<ValidatedConfigString>),
}

struct ValidatedConfigString {
    value: String,
    location: SourceLocation,
}

/// Extract a validated config value from a folded AST expression according to its shape.
///
/// WHY: centralizes shape enforcement so every registered key is validated through one path.
fn extract_config_value_for_shape(
    expression: &Expression,
    shape: ConfigValueShape,
    string_table: &mut StringTable,
) -> Result<ValidatedConfigValue, InvalidConfigReason> {
    match shape {
        ConfigValueShape::String => extract_string_value(expression, string_table)
            .map(ValidatedConfigValue::String)
            .ok_or_else(|| invalid_shape_reason(shape, string_table)),

        ConfigValueShape::Int => extract_int_value(expression)
            .map(ValidatedConfigValue::Int)
            .ok_or_else(|| invalid_shape_reason(shape, string_table)),

        ConfigValueShape::ClosedStringSet { allowed } => {
            let value = extract_string_value(expression, string_table)
                .ok_or_else(|| invalid_shape_reason(shape, string_table))?;

            if allowed.contains(&value.as_str()) {
                Ok(ValidatedConfigValue::String(value))
            } else {
                Err(invalid_shape_reason(shape, string_table))
            }
        }

        ConfigValueShape::Bool => extract_bool_value(expression)
            .map(ValidatedConfigValue::Bool)
            .ok_or_else(|| invalid_shape_reason(shape, string_table)),

        ConfigValueShape::StringCollection => {
            extract_string_collection_value(expression, string_table)
                .map(ValidatedConfigValue::StringCollection)
                .ok_or(InvalidConfigReason::UnsupportedPackageFoldersValue)
        }
    }
}

fn invalid_shape_reason(
    shape: ConfigValueShape,
    string_table: &mut StringTable,
) -> InvalidConfigReason {
    let expected = match shape {
        ConfigValueShape::ClosedStringSet { allowed } => {
            string_table.intern(&format_closed_string_set_expected(allowed))
        }
        _ => string_table.intern(config_value_shape_name(shape)),
    };

    InvalidConfigReason::InvalidConfigValueShape { expected }
}

/// Extract a string value: string slices and folded templates.
///
/// WHY: core path/metadata keys and backend string keys must not accept bool/int/float/char
/// by accidental stringification.
fn extract_string_value(expression: &Expression, string_table: &StringTable) -> Option<String> {
    match &expression.kind {
        ExpressionKind::StringSlice(value) => Some(string_table.resolve(*value).to_string()),
        ExpressionKind::Coerced { value, .. } => extract_string_value(value, string_table),
        _ => None,
    }
}

/// Format a human-readable expected-value description for a closed string set.
///
/// WHY: the diagnostic renderer needs a concrete message that lists the allowed strings
/// so users know exactly which values are accepted.
fn format_closed_string_set_expected(allowed: &[&str]) -> String {
    if allowed.len() == 1 {
        format!("\"{}\"", allowed[0])
    } else {
        let quoted: Vec<String> = allowed.iter().map(|s| format!("\"{}\"", s)).collect();
        format!("one of: {}", quoted.join(", "))
    }
}

/// Extract an integer value.
///
/// WHY: numeric config keys must not accept floats, bools, or strings through coercion or
/// stringification. Config validation consumes the AST-folded scalar directly.
fn extract_int_value(expression: &Expression) -> Option<i32> {
    match &expression.kind {
        ExpressionKind::Int(value) => Some(*value),
        ExpressionKind::Coerced { value, .. } => extract_int_value(value),
        _ => None,
    }
}

/// Extract a boolean value.
///
/// WHY: backend bool keys require actual boolean literals, not string representations.
fn extract_bool_value(expression: &Expression) -> Option<bool> {
    match &expression.kind {
        ExpressionKind::Bool(value) => Some(*value),
        ExpressionKind::Coerced { value, .. } => extract_bool_value(value),
        _ => None,
    }
}

/// Extract a string collection value.
///
/// WHY: `package_folders` accepts either a single string literal or a `{ ... }` collection
/// of string literals. Each item must be a string-like value, not a bool/int/float/char.
fn extract_string_collection_value(
    expression: &Expression,
    string_table: &StringTable,
) -> Option<Vec<ValidatedConfigString>> {
    match &expression.kind {
        ExpressionKind::Collection(items) => {
            let mut values = Vec::with_capacity(items.len());
            for item in items {
                values.push(ValidatedConfigString {
                    value: extract_string_value(item, string_table)?,
                    location: item.location.clone(),
                });
            }
            Some(values)
        }
        _ => {
            // Single string is accepted as a collection of one.
            Some(vec![ValidatedConfigString {
                value: extract_string_value(expression, string_table)?,
                location: expression.location.clone(),
            }])
        }
    }
}

// -------------------------
//  Package Folders Application
// -------------------------

/// Apply validated `package_folders` string collection values to the config.
///
/// WHY: path validation for package folders stays separate from broad shape extraction
/// so folder-specific rules (no absolute paths, no parent-dir segments, etc.) are still enforced.
fn apply_package_folders(
    config: &mut Config,
    folder_strings: Vec<ValidatedConfigString>,
    location: &SourceLocation,
    string_table: &mut StringTable,
) -> Result<(), Vec<CompilerDiagnostic>> {
    let mut folders = Vec::new();
    let mut errors = Vec::new();

    for folder_string in folder_strings {
        let folder = PathBuf::from(folder_string.value);
        let validated_folder = validate_package_folder_path(
            folder,
            &folder_string.location,
            string_table,
            &mut errors,
        );

        if let Some(folder) = validated_folder {
            folders.push(folder);
        }
    }

    reject_duplicate_folder_entries(&folders, location, string_table, &mut errors);

    if !errors.is_empty() {
        return Err(errors);
    }

    config.package_folders = folders;
    config.has_explicit_package_folders = true;
    Ok(())
}

// -------------------------
//  Folder Entry Validation
// -------------------------

fn reject_duplicate_folder_entries(
    folders: &[PathBuf],
    location: &SourceLocation,
    string_table: &mut StringTable,
    errors: &mut Vec<CompilerDiagnostic>,
) {
    let mut seen = HashSet::new();
    let mut duplicates = Vec::new();

    for folder in folders {
        if !seen.insert(folder.clone()) {
            duplicates.push(folder.display().to_string());
        }
    }

    if duplicates.is_empty() {
        return;
    }

    duplicates.sort();
    duplicates.dedup();
    let duplicate_list = string_table.intern(&duplicates.join(", "));

    errors.push(config_diagnostic(
        None,
        InvalidConfigReason::DuplicatePackageFolder {
            folder: duplicate_list,
        },
        location.clone(),
    ));
}

/// Validate one `package_folders` entry and normalize it to the stored path form.
///
/// WHY: this transitional parser/storage field remains project-relative until the queued Project Config
/// migration removes it; canonical Stage 0 discovers only structural support roots and registered packages.
fn validate_package_folder_path(
    package_folder: PathBuf,
    location: &SourceLocation,
    string_table: &mut StringTable,
    errors: &mut Vec<CompilerDiagnostic>,
) -> Option<PathBuf> {
    if package_folder.as_os_str().is_empty() {
        errors.push(config_diagnostic(
            None,
            InvalidConfigReason::InvalidPackageFolder {
                folder: None,
                reason: InvalidPackageFolderReason::Empty,
            },
            location.clone(),
        ));
        return None;
    }

    let folder_name = package_folder.display().to_string();
    let folder_id = string_table.intern(&folder_name);

    if package_folder.is_absolute()
        || package_folder
            .as_os_str()
            .to_string_lossy()
            .starts_with('/')
    {
        errors.push(config_diagnostic(
            None,
            InvalidConfigReason::InvalidPackageFolder {
                folder: Some(folder_id),
                reason: InvalidPackageFolderReason::AbsolutePath,
            },
            location.clone(),
        ));
        return None;
    }

    if package_folder
        .components()
        .any(|component| component == std::path::Component::ParentDir)
    {
        errors.push(config_diagnostic(
            None,
            InvalidConfigReason::InvalidPackageFolder {
                folder: Some(folder_id),
                reason: InvalidPackageFolderReason::ParentDirectorySegment,
            },
            location.clone(),
        ));
        return None;
    }

    let mut components = package_folder.components();
    let Some(first) = components.next() else {
        errors.push(config_diagnostic(
            None,
            InvalidConfigReason::InvalidPackageFolder {
                folder: None,
                reason: InvalidPackageFolderReason::Empty,
            },
            location.clone(),
        ));
        return None;
    };

    if !matches!(first, std::path::Component::Normal(_)) || components.next().is_some() {
        errors.push(config_diagnostic(
            None,
            InvalidConfigReason::InvalidPackageFolder {
                folder: Some(folder_id),
                reason: InvalidPackageFolderReason::NestedPath,
            },
            location.clone(),
        ));
        return None;
    }

    Some(package_folder)
}

// -------------------------
//  Value Application
// -------------------------

/// Apply a validated config value to the runtime config.
///
/// WHY: separates shape-validated extraction from the storage policy (typed field vs settings map).
fn apply_validated_config_value(
    config: &mut Config,
    config_key: &ConfigKeyEntry,
    key: &str,
    validated: ValidatedConfigValue,
    location: &SourceLocation,
    string_table: &mut StringTable,
) -> Result<(), Vec<CompilerDiagnostic>> {
    match (config_key.owner, validated) {
        (ConfigKeyOwner::Core, ValidatedConfigValue::String(value)) => {
            apply_core_string_config_entry(config, key, &value);
            Ok(())
        }

        (ConfigKeyOwner::Core, ValidatedConfigValue::Int(value)) => {
            apply_core_int_config_entry(config, key, value, location, string_table)
        }

        (ConfigKeyOwner::Core, ValidatedConfigValue::StringCollection(values)) => {
            if key == "package_folders" {
                apply_package_folders(config, values, location, string_table)
            } else {
                // Defensive fallback for future core keys that have not yet grown a typed field.
                // Unknown user keys cannot reach this branch because the registry check runs first.
                let value = values
                    .into_iter()
                    .map(|item| item.value)
                    .collect::<Vec<_>>()
                    .join(", ");
                config.settings.insert(key.to_string(), value);
                Ok(())
            }
        }

        (ConfigKeyOwner::Backend, ValidatedConfigValue::String(value)) => {
            config.settings.insert(key.to_string(), value);
            Ok(())
        }

        (ConfigKeyOwner::Backend, ValidatedConfigValue::Int(value)) => {
            config.settings.insert(key.to_string(), value.to_string());
            Ok(())
        }

        (ConfigKeyOwner::Backend, ValidatedConfigValue::Bool(value)) => {
            config.settings.insert(key.to_string(), value.to_string());
            Ok(())
        }

        (ConfigKeyOwner::Backend, ValidatedConfigValue::StringCollection(values)) => {
            let value = values
                .into_iter()
                .map(|item| item.value)
                .collect::<Vec<_>>()
                .join(", ");
            config.settings.insert(key.to_string(), value);
            Ok(())
        }

        // Shape mismatch between core owner and validated value should not happen
        // if extraction is correct, but handle defensively by storing as string.
        (ConfigKeyOwner::Core, ValidatedConfigValue::Bool(value)) => {
            apply_core_string_config_entry(config, key, &value.to_string());
            Ok(())
        }
    }
}

fn apply_core_int_config_entry(
    config: &mut Config,
    key: &str,
    value: i32,
    location: &SourceLocation,
    string_table: &mut StringTable,
) -> Result<(), Vec<CompilerDiagnostic>> {
    match key {
        TEMPLATE_CONST_LOOP_ITERATION_LIMIT_KEY => {
            let limit =
                validate_template_const_loop_iteration_limit(value, location, string_table)?;
            config.template_const_loop_iteration_limit = limit;
            Ok(())
        }

        _ => {
            // Defensive fallback for future core integer keys without a typed field.
            config.settings.insert(key.to_string(), value.to_string());
            Ok(())
        }
    }
}

fn validate_template_const_loop_iteration_limit(
    value: i32,
    location: &SourceLocation,
    string_table: &mut StringTable,
) -> Result<usize, Vec<CompilerDiagnostic>> {
    if value <= 0 {
        return Err(vec![config_diagnostic(
            Some(string_table.intern(TEMPLATE_CONST_LOOP_ITERATION_LIMIT_KEY)),
            InvalidConfigReason::InvalidProjectSettingValue {
                value: string_table.intern(&value.to_string()),
                expected: string_table.intern("a positive integer"),
            },
            location.clone(),
        )]);
    }

    if value > MAX_TEMPLATE_CONST_LOOP_ITERATIONS as i32 {
        return Err(vec![config_diagnostic(
            Some(string_table.intern(TEMPLATE_CONST_LOOP_ITERATION_LIMIT_KEY)),
            InvalidConfigReason::InvalidProjectSettingValue {
                value: string_table.intern(&value.to_string()),
                expected: string_table.intern("an integer no greater than 1000000"),
            },
            location.clone(),
        )]);
    }

    Ok(value as usize)
}

fn apply_core_string_config_entry(config: &mut Config, key: &str, value: &str) {
    match key {
        "entry_root" => config.entry_root = PathBuf::from(value),
        "output_folder" => config.release_folder = PathBuf::from(value),
        "dev_folder" => config.dev_folder = PathBuf::from(value),

        // `project` remains a core-owned selector setting while the current Config shape has no
        // dedicated field for it. Keeping it in settings preserves the existing build-selection
        // boundary without making backend builders declare the core key.
        "project" => {
            config
                .settings
                .insert("project".to_string(), value.to_string());
        }

        "project_name" | "name" => config.project_name = value.to_string(),
        "version" => config.version = value.to_string(),
        "author" => config.author = value.to_string(),
        "license" => config.license = value.to_string(),

        _ => {
            // Defensive fallback for future core keys that have not yet grown a typed field.
            config.settings.insert(key.to_string(), value.to_string());
        }
    }
}

fn config_diagnostic(
    key: Option<StringId>,
    reason: InvalidConfigReason,
    location: SourceLocation,
) -> CompilerDiagnostic {
    CompilerDiagnostic::invalid_config_reason(key, reason, location)
}

// -------------------------
//  Key-Identity Location Lookup
// -------------------------

/// Resolve the source location for a config key-identity diagnostic.
///
/// Authored name spans are keyed by `Declaration::id`. The declaration value remains a defensive
/// fallback for declarations without a preserved header span.
fn key_identity_location<'a>(
    declaration: &'a Declaration,
    authored_key_name_locations: &'a HashMap<InternedPath, SourceLocation>,
) -> &'a SourceLocation {
    authored_key_name_locations
        .get(&declaration.id)
        .unwrap_or(&declaration.value.location)
}

// -------------------------
//  Directory Output Setting Validation
// -------------------------

/// Validate directory output settings after config values are applied.
///
/// WHAT: rejects development and release output folders that are empty, absolute,
/// parent-traversing, current-directory, equal to the project root, inside `entry_root`,
/// or equal to each other.
/// WHY: directory output roots must be safe and distinct before any output writing or
/// cleanup runs. Single-file output stays separate because it writes to the command
/// working directory.
pub(crate) fn validate_directory_output_settings(
    config: &Config,
    string_table: &mut StringTable,
) -> Result<ValidatedDirectoryOutputSettings, Vec<CompilerDiagnostic>> {
    let mut errors = Vec::new();

    let project_root = &config.entry_dir;
    // The transitional empty or "." entry root form means the entry root covers the whole
    // project, so output containment is validated against the project root only. When
    // entry_root is a strict subdirectory, enforce that the output is outside it.
    let resolved_entry_root =
        if config.entry_root.as_os_str().is_empty() || config.entry_root == Path::new(".") {
            None
        } else {
            Some(project_root.join(&config.entry_root))
        };

    let dev = validate_one_output_folder(
        "dev_folder",
        &config.dev_folder,
        project_root,
        resolved_entry_root.as_deref(),
        config,
        string_table,
        &mut errors,
    );
    let release = validate_one_output_folder(
        "output_folder",
        &config.release_folder,
        project_root,
        resolved_entry_root.as_deref(),
        config,
        string_table,
        &mut errors,
    );

    // Only check distinctness when both folders individually passed validation.
    if let (Some(dev), Some(release)) = (&dev, &release) {
        validate_output_folders_distinct(dev, release, config, string_table, &mut errors);
    }

    let (Some(dev), Some(release)) = (dev, release) else {
        return Err(errors);
    };

    if errors.is_empty() {
        Ok(ValidatedDirectoryOutputSettings { dev, release })
    } else {
        Err(errors)
    }
}

/// Validate one output folder, returning the validated folder when it is valid.
fn validate_one_output_folder(
    key: &str,
    folder: &Path,
    project_root: &Path,
    resolved_entry_root: Option<&Path>,
    config: &Config,
    string_table: &mut StringTable,
    errors: &mut Vec<CompilerDiagnostic>,
) -> Option<ValidatedOutputFolder> {
    let location = config.setting_location_or_config_file(key, string_table);

    match classify_output_folder(folder, project_root, resolved_entry_root) {
        Ok(mut valid) => {
            if let Err(reason) =
                validate_output_folder_containment(&valid, project_root, resolved_entry_root)
            {
                let folder_id = Some(string_table.intern(&folder.to_string_lossy()));
                errors.push(output_folder_diagnostic(
                    key,
                    folder_id,
                    reason,
                    location,
                    string_table,
                ));
                return None;
            }

            valid.location = location;
            Some(valid)
        }
        Err(reason) => {
            let folder_id = (!matches!(reason, InvalidOutputFolderReason::Empty))
                .then(|| string_table.intern(&folder.to_string_lossy()));
            errors.push(output_folder_diagnostic(
                key,
                folder_id,
                reason,
                location,
                string_table,
            ));
            None
        }
    }
}

/// Reject development and release output roots that share one output identity.
fn validate_output_folders_distinct(
    dev: &ValidatedOutputFolder,
    release: &ValidatedOutputFolder,
    config: &Config,
    string_table: &mut StringTable,
    errors: &mut Vec<CompilerDiagnostic>,
) {
    // Both folders passed classification, so their relative paths are guaranteed valid.
    let dev_identity = output_path_identity(&dev.relative_path)
        .expect("dev folder was already validated as a relative output path");
    let release_identity = output_path_identity(&release.relative_path)
        .expect("release folder was already validated as a relative output path");

    let canonical_roots_match = canonical_output_root_for_identity(&dev.resolved_path)
        .ok()
        .zip(canonical_output_root_for_identity(&release.resolved_path).ok())
        .is_some_and(|(dev_root, release_root)| dev_root == release_root);

    if dev_identity == release_identity || canonical_roots_match {
        let location = config.setting_location_or_config_file("dev_folder", string_table);
        errors.push(CompilerDiagnostic::invalid_config_reason(
            Some(string_table.intern("dev_folder")),
            InvalidConfigReason::OutputFoldersNotDistinct {
                dev_folder: string_table.intern(&config.dev_folder.to_string_lossy()),
                release_folder: string_table.intern(&config.release_folder.to_string_lossy()),
            },
            location,
        ));
    }
}

fn output_folder_diagnostic(
    key: &str,
    folder: Option<StringId>,
    reason: InvalidOutputFolderReason,
    location: SourceLocation,
    string_table: &mut StringTable,
) -> CompilerDiagnostic {
    CompilerDiagnostic::invalid_config_reason(
        Some(string_table.intern(key)),
        InvalidConfigReason::InvalidOutputFolder { folder, reason },
        location,
    )
}
