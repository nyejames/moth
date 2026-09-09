//! User-facing message display helpers.
//!
//! WHAT: renders compiled diagnostics and errors into human-readable terminal output.
//! WHY: this is the final boundary between internal structured diagnostics and what the user sees.

use crate::backends::error_types::BackendErrorType;
use crate::compiler_frontend::compiler_errors::{
    CompilerError, CompilerErrorMetadataKey, CompilerMessages, ErrorType,
};
use crate::compiler_frontend::compiler_messages::render::relative_display_path_from_root;
use crate::compiler_frontend::symbols::string_interning::StringTable;
use saying::say;

pub fn print_compiler_messages(messages: CompilerMessages) {
    // The outer infrastructure failure prints as a standalone error beside the diagnostics,
    // after every error diagnostic and before warnings/notes — mirroring the severity-bucket
    // display order without fabricating a user diagnostic.
    let mut outer_emitted = messages.infrastructure_error().is_none();
    for diagnostic_index in messages.diagnostic_display_order() {
        let diagnostic = &messages.diagnostic_slice()[diagnostic_index];
        if !outer_emitted
            && diagnostic.severity
                != crate::compiler_frontend::compiler_messages::DiagnosticSeverity::Error
        {
            if let Some(error) = messages.infrastructure_error() {
                print_formatted_error(error.clone(), &messages.string_table);
            }
            outer_emitted = true;
        }
        let render_context = messages.diagnostic_render_context(diagnostic_index);
        crate::compiler_frontend::compiler_messages::render::terminal::print_diagnostic_with_context(
            diagnostic,
            render_context,
        );
    }
    if !outer_emitted {
        if let Some(error) = messages.infrastructure_error() {
            print_formatted_error(error.clone(), &messages.string_table);
        }
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

pub fn print_formatted_error(e: CompilerError, _string_table: &StringTable) {
    // Infrastructure errors carry no diagnostic source frame. Their optional host path is shown
    // only as an explicit path, while message and structured guidance remain the useful detail.
    let display_path = e.host_path.as_deref().map(|host_path| {
        let resolved = std::fs::canonicalize(host_path).unwrap_or_else(|_| host_path.to_path_buf());
        relative_display_path_from_root(&resolved, &std::env::current_dir().unwrap_or_default())
    });

    say!(
        "\n",
        Bright Bold Red error_display_name(&e.error_type),
        Reset " ",
        Reset error_visual(&e.error_type),
    );

    say!(Reset e.msg.as_str());

    if let Some(display_path) = display_path.filter(|path| !path.is_empty()) {
        say!(
            Blue "\n  --> ",
            Reset Magenta display_path.as_str(),
        );
    }

    for guidance_line in format_error_guidance_lines(&e) {
        say!(Bright Blue "  ", guidance_line);
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
