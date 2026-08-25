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
