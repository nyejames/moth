//! Config import diagnostic prose tests.
//!
//! WHAT: verifies config and import diagnostic messages render correctly.
//! WHY: these messages share path/string-table formatting concerns.

use crate::compiler_frontend::compiler_messages::InvalidConfigReason;
use crate::compiler_frontend::compiler_messages::render::import_config::{
    dependency_namespace_used_as_value_message, invalid_config_message,
};
use crate::compiler_frontend::symbols::string_interning::StringTable;

#[test]
fn dependency_namespace_used_as_value_message_contains_name_and_content_hint() {
    let mut string_table = StringTable::new();
    let record_name = string_table.intern("intro");
    let message = dependency_namespace_used_as_value_message(record_name, &string_table);

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
        "message should mention the `content as ...` dependency selection: {message}"
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

#[test]
fn resource_output_conflicts_render_typed_facts() {
    let mut string_table = StringTable::new();
    let output_path = string_table.intern("assets/logo.svg");
    let existing_origin = string_table.intern("module origin one/assets/logo.svg");
    let conflicting_origin = string_table.intern("module origin two/assets/logo.svg");
    let collision_message = invalid_config_message(
        None,
        &InvalidConfigReason::ResourceOutputPathCollision {
            output_path,
            existing_origin,
            conflicting_origin,
        },
        &string_table,
    );
    assert_eq!(
        collision_message,
        "HTML resource output path 'assets/logo.svg' is claimed by distinct origins 'module origin one/assets/logo.svg' and 'module origin two/assets/logo.svg'. Ensure each resource origin maps to a unique output path.",
    );

    let origin = string_table.intern("module origin/assets/index.html");
    let artefact_kind = string_table.intern("HTML page");
    let reserved_message = invalid_config_message(
        None,
        &InvalidConfigReason::ResourceOutputPathReserved {
            output_path,
            origin,
            artefact_kind,
        },
        &string_table,
    );
    assert_eq!(
        reserved_message,
        "HTML resource origin 'module origin/assets/index.html' conflicts with HTML page output path 'assets/logo.svg'. Choose a different resource path or builder output destination.",
    );
}
