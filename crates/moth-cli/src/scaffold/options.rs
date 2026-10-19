//! CLI options for `moth new html`.

/// Parsed command-line options for the HTML project scaffold command.
#[derive(Debug, PartialEq, Eq)]
pub struct NewHtmlProjectOptions {
    pub raw_path: Option<String>,
    pub force: bool,
}
