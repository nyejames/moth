//! User-facing message display helpers.
//!
//! WHAT: renders compiled diagnostics and errors into human-readable terminal output.
//! WHY: this is the final boundary between internal structured diagnostics and what the user sees.

use crate::backends::error_types::BackendErrorType;
use crate::compiler_frontend::compiler_errors::{
    CompilerError, CompilerErrorMetadataKey, CompilerMessages, ErrorType,
};
use crate::compiler_frontend::compiler_messages::render::resolved_display_path;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use saying::say;

pub fn print_compiler_messages(messages: CompilerMessages) {
    let display_order = messages.diagnostic_display_order();
    for diagnostic_index in display_order {
        let diagnostic = &messages.diagnostic_slice()[diagnostic_index];
        let render_context = messages.diagnostic_render_context(diagnostic_index);
        crate::compiler_frontend::compiler_messages::render::terminal::print_diagnostic_with_context(
            diagnostic,
            render_context,
        );
    }
}

pub fn print_terse_compiler_messages(messages: &CompilerMessages) {
    for line in format_terse_compiler_messages(messages) {
        println!("{line}");
    }
}

pub fn format_terse_compiler_messages(messages: &CompilerMessages) -> Vec<String> {
    crate::compiler_frontend::compiler_messages::render::terse::format_terse_compiler_messages(
        messages,
    )
}

pub fn print_formatted_error(e: CompilerError, string_table: &StringTable) {
    // Resolve synthetic header scopes back to source files before choosing a human-readable path.
    let relative_dir = resolved_display_path(&e.location.scope, string_table);
    let display_line = display_line_number(e.location.start_pos.line_number);
    let display_column = display_column_number(e.location.start_pos.char_column);

    // This renders a standalone `CompilerError` from the output-plan lane, which reaches the
    // terminal without a `CompilerMessages` and therefore without the retained source database.
    // Source excerpts belong to the diagnostic renderers that have one; this lane prints the
    // location and guidance only rather than reopening the filesystem.

    say!(
        "\n",
        Bright Bold Red error_display_name(&e.error_type),
        Reset " ",
        Reset error_visual(&e.error_type),
    );

    say!(Reset e.msg.as_str());

    if !relative_dir.is_empty() {
        say!(
            Blue "\n  --> ",
            Reset Magenta relative_dir.as_str(),
            Dark Magenta ":",
            Reset Bold Blue display_line,
            Reset Grey ":",
            Reset Magenta display_column
        );
    } else {
        say!(
            Blue "\n   --> ",
            Reset Magenta display_line,
            Dark Magenta ":",
            Reset Magenta display_column
        );
    }

    for guidance_line in format_error_guidance_lines(&e) {
        say!(Bright Blue "  ", guidance_line);
    }

    if e.location.scope.as_components().is_empty() {
        say!(Dark "     No source location available.");
    }
}

fn error_display_name(error_type: &ErrorType) -> &'static str {
    match error_type {
        ErrorType::Compiler => "Compiler Bug",
        ErrorType::Config => "Malformed Config",
        ErrorType::File => "Missing File or Directory",
        ErrorType::DevServer => "Dev Server Issue",
        ErrorType::HirTransformation => "HIR Transformation Error",
        ErrorType::Backend(BackendErrorType::LirTransformation) => "LIR Transformation Bug",
        ErrorType::Backend(BackendErrorType::WasmGeneration) => "WASM Generation Bug",
    }
}

fn error_visual(error_type: &ErrorType) -> &'static str {
    match error_type {
        ErrorType::Compiler => "🔥 ヽ༼☉ ‿ ⚆༽ﾉ 🔥",
        ErrorType::Config => "🔥📄🔥",
        ErrorType::File => "🔥📁🔥",
        ErrorType::DevServer => "(ﾉ☉_⚆)ﾉ 🔥🖥️🔥",
        ErrorType::HirTransformation => "(☉_☉) 🔥",
        ErrorType::Backend(BackendErrorType::LirTransformation) => "ヽ(°〇°)ﾉ 🔥",
        ErrorType::Backend(BackendErrorType::WasmGeneration) => "(° O °) 🔥",
    }
}

fn display_line_number(raw_line: i32) -> i32 {
    raw_line.saturating_add(1).max(1)
}

fn display_column_number(raw_column: i32) -> i32 {
    raw_column.saturating_add(1).max(1)
}

pub(crate) fn format_error_guidance_lines(error: &CompilerError) -> Vec<String> {
    let mut lines = Vec::new();

    if let Some(stage) = error
        .metadata
        .get(&CompilerErrorMetadataKey::CompilationStage)
        && error.error_type == ErrorType::Compiler
    {
        lines.push(format!("Stage: {stage}"));
    }

    if let Some(suggestion) = error
        .metadata
        .get(&CompilerErrorMetadataKey::PrimarySuggestion)
    {
        lines.push(suggestion.to_owned());
    }

    if let Some(alternative) = error
        .metadata
        .get(&CompilerErrorMetadataKey::AlternativeSuggestion)
    {
        lines.push(format!("Alternative: {alternative}"));
    }

    if let Some(replacement) = error
        .metadata
        .get(&CompilerErrorMetadataKey::SuggestedReplacement)
    {
        lines.push(format!("Suggested replacement: {replacement}"));
    }

    match (
        error
            .metadata
            .get(&CompilerErrorMetadataKey::SuggestedInsertion),
        error
            .metadata
            .get(&CompilerErrorMetadataKey::SuggestedLocation),
    ) {
        (Some(insertion), Some(location)) => {
            lines.push(format!("Suggested insertion: '{insertion}' {location}"))
        }
        (Some(insertion), None) => lines.push(format!("Suggested insertion: '{insertion}'")),
        (None, Some(location)) => lines.push(format!("Suggested location: {location}")),
        (None, None) => {}
    }

    lines
}

#[cfg(test)]
#[path = "tests/display_messages_tests.rs"]
mod display_messages_tests;
