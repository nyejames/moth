//! The first-party package dependency audit.
//!
//! WHAT: walks the explicitly owned first-party package and runtime roots, rejects package-manager
//!       metadata, vendored dependency directories and unapproved bare JavaScript module imports,
//!       and writes the machine-readable result used by local and CI validation.
//! WHY: first-party packages promise zero third-party runtime dependencies. That promise needs one
//!      narrow source owner rather than a repository-wide text search that mistakes documentation,
//!      tests or benchmarks for package implementations.
//!
//! # What this module owns
//! - The scoped walk of first-party package implementation roots.
//! - The single `@moth/runtime` bare-module allowlist and JavaScript import checks.
//! - Inspection of the inline `MOTH_RUNTIME_SOURCE_V1` source owned by the HTML project runtime.
//! - Findings, the atomic JSON report and the `first-party-deps` command result.
//!
//! # What this module does NOT own
//! - User-owned or future dependency packages and their manifests.
//! - Package declarations, aliases, resolution or package-graph design.
//! - Generated HTML runtime glue, documentation, tests, benchmarks or repository-root manifests.
//! - Runtime semantics or the registry's module implementation itself; those remain with the HTML
//!   project runtime module registry.

use crate::report_file::{ReportRunIdentity, write_report_atomically};
use crate::source_tree::{relative_display_path, workspace_root};
use serde::Serialize;
use std::fmt;
use std::fs;
use std::path::Path;

/// Where the first-party dependency report is written, relative to the workspace root.
pub const FIRST_PARTY_DEPS_REPORT_PATH: &str = "target/test-reports/first_party_deps.json";

/// Schema version of the first-party dependency report.
pub const FIRST_PARTY_DEPS_SCHEMA_VERSION: u32 = 1;

/// First-party implementation roots, in deterministic scan order.
///
/// These are deliberately package-owned roots rather than all source, documentation or fixture
/// trees. Keep this list synchronized with the ownership statement in `validation.mtf`.
pub const FIRST_PARTY_SOURCE_ROOTS: &[&str] = &[
    "packages",
    "src/builder_surface/core_packages",
    "src/backends/js/package_bindings",
    "src/projects/html_project/binding_packages",
];

const RUNTIME_REGISTRY_RELATIVE_PATH: &str =
    "src/projects/html_project/external_js/runtime_module_registry.rs";
const RUNTIME_SOURCE_LABEL: &str =
    "src/projects/html_project/external_js/runtime_module_registry.rs::MOTH_RUNTIME_SOURCE_V1";
const RUNTIME_SOURCE_MARKER: &str = "const MOTH_RUNTIME_SOURCE_V1: &str = r#\"";
const RUNTIME_SOURCE_END: &str = "\"#;";

/// Exact package-manager files forbidden inside a first-party implementation root.
const FORBIDDEN_MANIFEST_BASENAMES: &[&str] = &[
    "package.json",
    "package-lock.json",
    "npm-shrinkwrap.json",
    "yarn.lock",
    "pnpm-lock.yaml",
    "bun.lock",
    "bun.lockb",
    "deno.json",
    "deno.jsonc",
    "deno.lock",
];

/// Directory names that identify copied or vendored third-party code.
const FORBIDDEN_VENDOR_DIRECTORY_NAMES: &[&str] = &["node_modules", "vendor", "third_party"];

/// The only bare JavaScript module specifier first-party implementations may import.
const ALLOWED_BARE_MODULES: &[&str] = &["@moth/runtime"];

/// Which first-party dependency rule produced a finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FirstPartyDepsRule {
    /// A package-manager manifest or lockfile was found by exact basename.
    PackageManagerManifest,
    /// A known vendored dependency directory was found by exact directory name.
    VendoredDependencyRoot,
    /// A JavaScript import-like expression named an unapproved bare module.
    UnapprovedBareImport,
    /// A path could not be read or represented, so the audit could not inspect it.
    UnreadablePath,
    /// The explicitly owned inline runtime source could not be extracted safely.
    InvalidRuntimeSource,
}

impl FirstPartyDepsRule {
    const fn label(self) -> &'static str {
        match self {
            Self::PackageManagerManifest => "package-manager-manifest",
            Self::VendoredDependencyRoot => "vendored-dependency-root",
            Self::UnapprovedBareImport => "unapproved-bare-import",
            Self::UnreadablePath => "unreadable-path",
            Self::InvalidRuntimeSource => "invalid-runtime-source",
        }
    }
}

/// One first-party dependency finding, named by the implementation file or directory involved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FirstPartyDepsFinding {
    /// Workspace-relative path with `/` separators, plus a runtime-source label when applicable.
    pub file: String,
    pub rule: FirstPartyDepsRule,
    pub message: String,
}

impl fmt::Display for FirstPartyDepsFinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "[{}] {}: {}",
            self.rule.label(),
            self.file,
            self.message
        )
    }
}

/// Complete machine-readable first-party dependency audit report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FirstPartyDepsReport {
    pub schema_version: u32,
    pub run: ReportRunIdentity,
    pub audited_roots: Vec<String>,
    pub audited_file_count: usize,
    pub findings: Vec<FirstPartyDepsFinding>,
}

/// Run the scoped first-party dependency audit and write its report.
pub fn run_first_party_deps() -> Result<(), String> {
    let workspace_root = workspace_root()?;
    let report_path = workspace_root.join(FIRST_PARTY_DEPS_REPORT_PATH);
    let run = ReportRunIdentity::started("first-party-deps", None);

    write_first_party_deps_report(&report_path, &started_report(run.clone()))?;

    let (audited_file_count, findings) = audit_first_party_deps(&workspace_root)?;
    let report = FirstPartyDepsReport {
        schema_version: FIRST_PARTY_DEPS_SCHEMA_VERSION,
        run: run.completed(),
        audited_roots: audited_roots(),
        audited_file_count,
        findings,
    };

    write_first_party_deps_report(&report_path, &report)?;

    println!(
        "first-party-deps: {} files audited, {} finding(s)",
        report.audited_file_count,
        report.findings.len()
    );

    if report.findings.is_empty() {
        return Ok(());
    }

    for finding in &report.findings {
        println!("  {finding}");
    }
    Err(format!(
        "first-party dependency audit failed with {} finding(s)",
        report.findings.len()
    ))
}

/// Audit the first-party roots and the one explicitly owned inline runtime source.
///
/// Root traversal errors return `Err`; unreadable files become typed findings so a completed report
/// still records exactly which paths prevented inspection.
pub(crate) fn audit_first_party_deps(
    workspace_root: &Path,
) -> Result<(usize, Vec<FirstPartyDepsFinding>), String> {
    let mut state = ScanState::default();

    for root in FIRST_PARTY_SOURCE_ROOTS {
        scan_first_party_root(workspace_root, root, &mut state)?;
    }

    scan_runtime_source(workspace_root, &mut state);
    Ok((state.audited_file_count, state.findings))
}

fn started_report(run: ReportRunIdentity) -> FirstPartyDepsReport {
    FirstPartyDepsReport {
        schema_version: FIRST_PARTY_DEPS_SCHEMA_VERSION,
        run,
        audited_roots: audited_roots(),
        audited_file_count: 0,
        findings: Vec::new(),
    }
}

fn audited_roots() -> Vec<String> {
    FIRST_PARTY_SOURCE_ROOTS
        .iter()
        .map(|root| (*root).to_owned())
        .chain(std::iter::once(RUNTIME_SOURCE_LABEL.to_owned()))
        .collect()
}

fn write_first_party_deps_report(path: &Path, report: &FirstPartyDepsReport) -> Result<(), String> {
    let json = serde_json::to_string_pretty(report).map_err(|error| {
        format!("failed to serialise the first-party dependency report: {error}")
    })?;
    write_report_atomically(path, json.as_bytes())
}

#[derive(Default)]
struct ScanState {
    audited_file_count: usize,
    findings: Vec<FirstPartyDepsFinding>,
}

fn scan_first_party_root(
    workspace_root: &Path,
    relative_root: &str,
    state: &mut ScanState,
) -> Result<(), String> {
    let root = workspace_root.join(relative_root);
    let metadata = fs::symlink_metadata(&root).map_err(|error| {
        format!(
            "failed to read first-party root '{}': {error}",
            root.display()
        )
    })?;
    if !metadata.is_dir() {
        return Err(format!(
            "first-party root '{}' is not a directory",
            root.display()
        ));
    }
    scan_directory(workspace_root, &root, state)
}

fn scan_directory(
    workspace_root: &Path,
    directory: &Path,
    state: &mut ScanState,
) -> Result<(), String> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(directory).map_err(|error| {
        format!(
            "failed to read first-party directory '{}': {error}",
            directory.display()
        )
    })? {
        entries.push(entry.map_err(|error| {
            format!(
                "failed to read an entry of first-party directory '{}': {error}",
                directory.display()
            )
        })?);
    }
    entries.sort_by_key(|entry| entry.path());

    for entry in entries {
        let path = entry.path();
        let relative = relative_display_path(workspace_root, &path)?;
        let file_name = entry.file_name();
        let name = file_name.to_str().ok_or_else(|| {
            format!(
                "path '{}' has a file name that is not valid UTF-8",
                path.display()
            )
        })?;

        if FORBIDDEN_MANIFEST_BASENAMES.contains(&name) {
            state.findings.push(FirstPartyDepsFinding {
                file: relative.clone(),
                rule: FirstPartyDepsRule::PackageManagerManifest,
                message: format!("forbidden package-manager file '{name}'"),
            });
        }

        let metadata = fs::symlink_metadata(&path).map_err(|error| {
            format!(
                "failed to stat first-party path '{}': {error}",
                path.display()
            )
        })?;
        if metadata.is_dir() {
            if FORBIDDEN_VENDOR_DIRECTORY_NAMES.contains(&name) {
                state.findings.push(FirstPartyDepsFinding {
                    file: relative,
                    rule: FirstPartyDepsRule::VendoredDependencyRoot,
                    message: format!("forbidden vendored dependency directory '{name}'"),
                });
            } else {
                scan_directory(workspace_root, &path, state)?;
            }
            continue;
        }

        if metadata.is_file() {
            state.audited_file_count += 1;
            let bytes = match fs::read(&path) {
                Ok(bytes) => bytes,
                Err(error) => {
                    state.findings.push(FirstPartyDepsFinding {
                        file: relative,
                        rule: FirstPartyDepsRule::UnreadablePath,
                        message: format!("unreadable file ({error})"),
                    });
                    continue;
                }
            };

            if is_javascript_file(&path) {
                match String::from_utf8(bytes) {
                    Ok(source) => state
                        .findings
                        .extend(audit_javascript_source(&relative, &source)),
                    Err(error) => state.findings.push(FirstPartyDepsFinding {
                        file: relative,
                        rule: FirstPartyDepsRule::UnreadablePath,
                        message: format!("JavaScript source is not valid UTF-8 ({error})"),
                    }),
                }
            }
            continue;
        }

        state.audited_file_count += 1;
        state.findings.push(FirstPartyDepsFinding {
            file: relative,
            rule: FirstPartyDepsRule::UnreadablePath,
            message: "path is neither a regular file nor a directory".to_owned(),
        });
    }

    Ok(())
}

fn scan_runtime_source(workspace_root: &Path, state: &mut ScanState) {
    state.audited_file_count += 1;
    let registry_path = workspace_root.join(RUNTIME_REGISTRY_RELATIVE_PATH);
    let registry_source = match fs::read_to_string(&registry_path) {
        Ok(source) => source,
        Err(error) => {
            state.findings.push(FirstPartyDepsFinding {
                file: RUNTIME_SOURCE_LABEL.to_owned(),
                rule: FirstPartyDepsRule::UnreadablePath,
                message: format!("unreadable runtime registry ({error})"),
            });
            return;
        }
    };

    let runtime_source = match extract_runtime_source(&registry_source) {
        Ok(source) => source,
        Err(error) => {
            state.findings.push(FirstPartyDepsFinding {
                file: RUNTIME_SOURCE_LABEL.to_owned(),
                rule: FirstPartyDepsRule::InvalidRuntimeSource,
                message: error,
            });
            return;
        }
    };
    state.findings.extend(audit_javascript_source(
        RUNTIME_SOURCE_LABEL,
        runtime_source,
    ));
}

fn extract_runtime_source(registry_source: &str) -> Result<&str, String> {
    let marker = registry_source
        .find(RUNTIME_SOURCE_MARKER)
        .ok_or_else(|| "MOTH_RUNTIME_SOURCE_V1 marker was not found".to_owned())?;
    let source_start = marker + RUNTIME_SOURCE_MARKER.len();
    let source_end = registry_source[source_start..]
        .find(RUNTIME_SOURCE_END)
        .map(|offset| source_start + offset)
        .ok_or_else(|| "MOTH_RUNTIME_SOURCE_V1 raw string was not terminated".to_owned())?;
    Ok(&registry_source[source_start..source_end])
}

fn is_javascript_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| matches!(extension, "js" | "mjs" | "cjs"))
}

/// Apply import rules to one JavaScript source fragment.
///
/// Kept separate from filesystem traversal so the import contract can be proved with small fixture
/// strings without turning a source-text test into the owner of the repository-wide ban.
fn audit_javascript_source(file: &str, source: &str) -> Vec<FirstPartyDepsFinding> {
    let tokens = tokenize_javascript(source);
    let mut findings = Vec::new();

    for index in 0..tokens.len() {
        let Token::Identifier(keyword) = &tokens[index] else {
            continue;
        };

        let specifier = match keyword.as_str() {
            "import" => {
                if matches!(tokens.get(index + 1), Some(Token::Punctuation('('))) {
                    string_after_open_paren(&tokens, index + 1, "dynamic import")
                } else if let Some(Token::String(specifier)) = tokens.get(index + 1) {
                    Some((specifier.as_str(), "static import"))
                } else {
                    static_import_specifier(&tokens, index)
                }
            }
            "export"
                if matches!(tokens.get(index + 1), Some(Token::Punctuation('{')))
                    || matches!(tokens.get(index + 1), Some(Token::Punctuation('*'))) =>
            {
                re_export_specifier(&tokens, index)
            }
            "require" if matches!(tokens.get(index + 1), Some(Token::Punctuation('('))) => {
                string_after_open_paren(&tokens, index + 1, "require")
            }
            _ => None,
        };

        if let Some((specifier, form)) = specifier
            && is_unapproved_bare_module(specifier)
        {
            findings.push(FirstPartyDepsFinding {
                file: file.to_owned(),
                rule: FirstPartyDepsRule::UnapprovedBareImport,
                message: format!("{form} uses unapproved bare module '{specifier}'"),
            });
        }
    }

    findings
}

fn string_after_open_paren<'a>(
    tokens: &'a [Token],
    open_paren_index: usize,
    form: &'static str,
) -> Option<(&'a str, &'static str)> {
    match tokens.get(open_paren_index + 1) {
        Some(Token::String(specifier)) => Some((specifier.as_str(), form)),
        _ => None,
    }
}

fn static_import_specifier(tokens: &[Token], import_index: usize) -> Option<(&str, &'static str)> {
    from_specifier(tokens, import_index, "static import")
}

fn re_export_specifier(tokens: &[Token], export_index: usize) -> Option<(&str, &'static str)> {
    from_specifier(tokens, export_index, "re-export")
}

fn from_specifier<'a>(
    tokens: &'a [Token],
    keyword_index: usize,
    form: &'static str,
) -> Option<(&'a str, &'static str)> {
    for index in keyword_index + 1..tokens.len() {
        match &tokens[index] {
            Token::Identifier(identifier) if identifier == "from" => {
                if let Some(Token::String(specifier)) = tokens.get(index + 1) {
                    return Some((specifier.as_str(), form));
                }
            }
            Token::Identifier(_) | Token::String(_) => {}
            Token::Punctuation('{' | '}' | '*' | ',') => {}
            _ => break,
        }
    }

    None
}

fn is_unapproved_bare_module(specifier: &str) -> bool {
    let is_relative = specifier.starts_with("./") || specifier.starts_with("../");
    let is_absolute = specifier.starts_with('/');
    let is_url = ["http:", "https:", "data:"]
        .iter()
        .any(|prefix| specifier.to_ascii_lowercase().starts_with(prefix));

    !is_relative && !is_absolute && !is_url && !ALLOWED_BARE_MODULES.contains(&specifier)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Identifier(String),
    String(String),
    Punctuation(char),
}

fn tokenize_javascript(source: &str) -> Vec<Token> {
    let bytes = source.as_bytes();
    let mut index = 0;
    tokenize_javascript_from(bytes, &mut index, TokenizeStop::End)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TokenizeStop {
    End,
    Interpolation,
}

fn tokenize_javascript_from(bytes: &[u8], index: &mut usize, stop: TokenizeStop) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut brace_depth: u32 = 0;

    while *index < bytes.len() {
        let byte = bytes[*index];
        if stop == TokenizeStop::Interpolation && byte == b'}' && brace_depth == 0 {
            *index += 1;
            break;
        }

        if byte.is_ascii_whitespace() {
            *index += 1;
            continue;
        }

        if byte == b'/' && bytes.get(*index + 1) == Some(&b'/') {
            *index += 2;
            while *index < bytes.len() && bytes[*index] != b'\n' {
                *index += 1;
            }
            continue;
        }
        if byte == b'/' && bytes.get(*index + 1) == Some(&b'*') {
            *index += 2;
            while *index + 1 < bytes.len() && !(bytes[*index] == b'*' && bytes[*index + 1] == b'/')
            {
                *index += 1;
            }
            *index = (*index + 2).min(bytes.len());
            continue;
        }
        if byte == b'/' {
            if slash_starts_regular_expression(&tokens) {
                skip_regular_expression(bytes, index);
            } else {
                *index += 1;
            }
            continue;
        }

        if byte == b'\'' || byte == b'"' {
            tokens.push(Token::String(read_quoted_string(bytes, index)));
            continue;
        }

        if byte == b'`' {
            scan_template_literal(bytes, index, &mut tokens);
            continue;
        }

        if byte.is_ascii_alphabetic() || byte == b'_' || byte == b'$' {
            let start = *index;
            *index += 1;
            while *index < bytes.len()
                && (bytes[*index].is_ascii_alphanumeric()
                    || bytes[*index] == b'_'
                    || bytes[*index] == b'$')
            {
                *index += 1;
            }
            tokens.push(Token::Identifier(
                String::from_utf8_lossy(&bytes[start..*index]).into_owned(),
            ));
            continue;
        }

        if matches!(
            byte,
            b'(' | b')'
                | b'{'
                | b'}'
                | b'['
                | b']'
                | b';'
                | b'*'
                | b','
                | b'.'
                | b'='
                | b'+'
                | b'-'
                | b'!'
                | b'?'
                | b':'
                | b'&'
                | b'|'
                | b'<'
                | b'>'
                | b'%'
                | b'^'
                | b'~'
        ) {
            if byte == b'{' {
                brace_depth += 1;
            } else if byte == b'}' {
                brace_depth = brace_depth.saturating_sub(1);
            }
            tokens.push(Token::Punctuation(byte as char));
        }
        *index += 1;
    }

    tokens
}

fn slash_starts_regular_expression(tokens: &[Token]) -> bool {
    match tokens.last() {
        None => true,
        Some(Token::String(_)) => false,
        Some(Token::Punctuation(punctuation)) => !matches!(punctuation, ')' | ']' | '}'),
        Some(Token::Identifier(identifier)) => matches!(
            identifier.as_str(),
            "return"
                | "throw"
                | "case"
                | "in"
                | "of"
                | "new"
                | "typeof"
                | "void"
                | "delete"
                | "await"
                | "yield"
        ),
    }
}

fn skip_regular_expression(bytes: &[u8], index: &mut usize) {
    *index += 1;
    let mut in_character_class = false;

    while *index < bytes.len() {
        let byte = bytes[*index];
        if byte == b'\\' {
            *index = (*index + 2).min(bytes.len());
            continue;
        }
        if byte == b'\n' {
            return;
        }
        if byte == b'[' {
            in_character_class = true;
        } else if byte == b']' {
            in_character_class = false;
        } else if byte == b'/' && !in_character_class {
            *index += 1;
            while *index < bytes.len() && bytes[*index].is_ascii_alphabetic() {
                *index += 1;
            }
            return;
        }
        *index += 1;
    }
}

fn read_quoted_string(bytes: &[u8], index: &mut usize) -> String {
    let quote = bytes[*index];
    *index += 1;
    let mut value = String::new();

    while *index < bytes.len() {
        let byte = bytes[*index];
        *index += 1;
        if byte == quote {
            break;
        }
        if byte == b'\\' {
            if let Some(escaped) = bytes.get(*index).copied() {
                *index += 1;
                value.push(match escaped {
                    b'n' => '\n',
                    b'r' => '\r',
                    b't' => '\t',
                    b'0' => '\0',
                    other => other as char,
                });
            }
        } else {
            value.push(byte as char);
        }
    }

    value
}

fn scan_template_literal(bytes: &[u8], index: &mut usize, tokens: &mut Vec<Token>) {
    *index += 1;
    let mut literal = String::new();
    let mut interpolated = false;

    while *index < bytes.len() {
        let byte = bytes[*index];
        *index += 1;
        if byte == b'\\' {
            if let Some(escaped) = bytes.get(*index).copied() {
                *index += 1;
                if !interpolated {
                    literal.push(match escaped {
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'0' => '\0',
                        b'`' => '`',
                        b'$' => '$',
                        b'\\' => '\\',
                        other => other as char,
                    });
                }
            }
            continue;
        }
        if byte == b'`' {
            if !interpolated {
                tokens.push(Token::String(literal));
            }
            return;
        }
        if byte == b'$' && bytes.get(*index) == Some(&b'{') {
            interpolated = true;
            *index += 1;
            tokens.extend(tokenize_javascript_from(
                bytes,
                index,
                TokenizeStop::Interpolation,
            ));
            continue;
        }
        if !interpolated {
            literal.push(byte as char);
        }
    }
}

#[cfg(test)]
mod tests;
