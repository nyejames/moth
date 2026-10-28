//! Structured payloads and host presentation for proven compiler invariant violations.
//!
//! Compiler bug reports resolve their optional source location into owned display facts before
//! unwinding can discard the compiler state that owns compact identities.

use crate::compiler_frontend::compiler_errors::InfrastructureClass;
use crate::compiler_frontend::compiler_messages::CompilerStage;
use crate::compiler_frontend::compiler_messages::diagnostics::foreign::SiteRef;
use crate::compiler_frontend::compiler_messages::render::context::DiagnosticRenderContext;
use crate::compiler_frontend::compiler_messages::render::infrastructure::ISSUE_URL;
use crate::compiler_frontend::source::{FrozenIdentityContext, SourceSpan};
use std::any::Any;
use std::fmt;
use std::panic::Location;
use std::sync::Once;
static INSTALL_COMPILER_BUG_HOOK: Once = Once::new();

/// Render a structured compiler-bug panic for its owning host boundary.
///
/// The hook suppresses this payload while it unwinds, so exactly one host can render it after the
/// worker has been discarded.
pub fn render_compiler_bug_payload(payload: &(dyn Any + Send)) -> Option<String> {
    payload
        .downcast_ref::<CompilerBugReport>()
        .map(format_compiler_bug_report)
}

/// Install one process-level hook. Structured reports are rendered by the owning host; unknown
/// panics retain the previous hook's output.
pub fn install_compiler_bug_hook() {
    INSTALL_COMPILER_BUG_HOOK.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |panic| {
            if !panic.payload().is::<CompilerBugReport>() {
                previous(panic);
            }
        }));
    });
}

#[allow(
    dead_code,
    reason = "the source-bearing compiler-bug form retains bounded excerpts even before a current invariant site needs one"
)]
const MAX_SOURCE_EXCERPT_CHARS: usize = 160;

/// Resolved source facts that remain safe to render after the compiler worker unwinds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CompilerBugSource {
    pub(crate) logical_display_path: Box<str>,
    pub(crate) byte_range: std::ops::Range<u32>,
    pub(crate) start_line: u32,
    pub(crate) start_column: u32,
    pub(crate) end_line: u32,
    pub(crate) end_column: u32,
    pub(crate) excerpt: Box<str>,
}

impl CompilerBugSource {
    /// Resolve an exact source span through the same frozen source and path owners as diagnostics.
    #[allow(
        dead_code,
        reason = "the source-bearing compiler-bug form resolves spans when a caller has a frozen identity context"
    )]
    pub(crate) fn from_span(span: SourceSpan, frozen: &FrozenIdentityContext) -> Option<Self> {
        let context = DiagnosticRenderContext::new(frozen.strings()).with_frozen_identity(frozen);
        let position = context.site_position(SiteRef::Local(span))?;
        let logical_display_path = position.path.to_str()?.to_owned().into_boxed_str();

        // Reuse the diagnostic renderer's source resolver while its identity context is alive.
        Some(Self {
            logical_display_path,
            byte_range: position.byte_range,
            start_line: position.start.line + 1,
            start_column: position.start.column + 1,
            end_line: position.end.line + 1,
            end_column: position.end.column + 1,
            excerpt: position
                .line
                .chars()
                .take(MAX_SOURCE_EXCERPT_CHARS)
                .collect::<String>()
                .into_boxed_str(),
        })
    }
}

/// Self-contained compiler bug information carried as the panic payload.
#[derive(Debug)]
pub(crate) struct CompilerBugReport {
    pub(crate) stage: CompilerStage,
    pub(crate) location: &'static Location<'static>,
    pub(crate) version: &'static str,
    pub(crate) message: Box<str>,
    pub(crate) source: Option<CompilerBugSource>,
}

/// Render a compiler bug report without consulting compiler-owned tables.
pub(crate) fn format_compiler_bug_report(report: &CompilerBugReport) -> String {
    format_bug_parts(
        report.stage,
        report.location,
        report.version,
        &report.message,
        report.source.as_ref(),
    )
}

fn format_bug_parts(
    stage: CompilerStage,
    location: &Location<'_>,
    version: &str,
    message: &str,
    source: Option<&CompilerBugSource>,
) -> String {
    let mut output = format!(
        "{}\n{}\n\nStage: {}\nRust location: {}:{}:{}\nCompiler version: {}\n\nDetails:\n{}",
        InfrastructureClass::CompilerBug.introduction(),
        ISSUE_URL,
        stage,
        location.file(),
        location.line(),
        location.column(),
        version,
        message,
    );

    if let Some(source) = source {
        use fmt::Write as _;
        let _ = write!(
            output,
            "\n  --> {}:{}:{}\n     | {}",
            source.logical_display_path, source.start_line, source.start_column, source.excerpt,
        );
    }

    output
}

/// Raise a non-recoverable compiler bug with the macro's call site and owned source facts.
#[track_caller]
#[cold]
pub(crate) fn raise_compiler_bug(
    stage: CompilerStage,
    source: Option<CompilerBugSource>,
    message: fmt::Arguments<'_>,
) -> ! {
    std::panic::panic_any(CompilerBugReport {
        stage,
        location: Location::caller(),
        version: env!("CARGO_PKG_VERSION"),
        message: format!("{message}").into_boxed_str(),
        source,
    })
}

/// Raise a structured compiler bug for a proven invariant violation.
#[macro_export]
macro_rules! compiler_bug {
    ($stage:expr, source = $source:expr, $($message:tt)+) => {{
        $crate::compiler_frontend::compiler_messages::compiler_bug::raise_compiler_bug(
            $stage,
            Some($source),
            format_args!($($message)+),
        )
    }};
    ($stage:expr, $($message:tt)+) => {{
        $crate::compiler_frontend::compiler_messages::compiler_bug::raise_compiler_bug(
            $stage,
            None,
            format_args!($($message)+),
        )
    }};
}
