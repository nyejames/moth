//! Strict parser for the Moth benchmark diagnostic-status protocol.
//!
//! WHAT: extracts one exact diagnostic count record from captured compiler output.
//! WHY: benchmark execution must fail closed without interpreting ordinary rendered diagnostics.

use std::fmt::{Display, Formatter};

const STATUS_PREFIX: &str = "MOTH_BENCH status";
const STATUS_FIELDS_PREFIX: &str = "MOTH_BENCH status errors=";
const WARNING_FIELD_SEPARATOR: &str = " warnings=";

/// Diagnostic counts reported by one completed Moth benchmark command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BenchmarkDiagnosticStatus {
    pub(crate) error_count: usize,
    pub(crate) warning_count: usize,
}

/// Failure to extract one exact benchmark diagnostic-status record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BenchmarkStatusError {
    Missing,
    Duplicate { count: usize },
    Malformed { line: String },
}

impl Display for BenchmarkStatusError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing => write!(formatter, "missing MOTH_BENCH status record"),
            Self::Duplicate { count } => {
                write!(formatter, "found {count} MOTH_BENCH status records")
            }
            Self::Malformed { line } => {
                write!(formatter, "malformed MOTH_BENCH status record: {line}")
            }
        }
    }
}

impl std::error::Error for BenchmarkStatusError {}

impl TryFrom<&str> for BenchmarkDiagnosticStatus {
    type Error = BenchmarkStatusError;

    /// Parse one exact status record while ignoring unrelated surrounding output.
    fn try_from(output: &str) -> Result<Self, Self::Error> {
        let candidate_lines: Vec<&str> = output
            .lines()
            .filter(|line| line.starts_with(STATUS_PREFIX))
            .collect();

        let line = match candidate_lines.as_slice() {
            [] => return Err(BenchmarkStatusError::Missing),
            [line] => *line,
            lines => {
                return Err(BenchmarkStatusError::Duplicate { count: lines.len() });
            }
        };

        parse_status_line(line).ok_or_else(|| BenchmarkStatusError::Malformed {
            line: line.to_owned(),
        })
    }
}

fn parse_status_line(line: &str) -> Option<BenchmarkDiagnosticStatus> {
    let fields = line.strip_prefix(STATUS_FIELDS_PREFIX)?;
    let (error_count, warning_count) = fields.split_once(WARNING_FIELD_SEPARATOR)?;

    Some(BenchmarkDiagnosticStatus {
        error_count: parse_count(error_count)?,
        warning_count: parse_count(warning_count)?,
    })
}

fn parse_count(value: &str) -> Option<usize> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }

    value.parse().ok()
}

#[cfg(test)]
mod tests;
