//! Supported-shape `<script>` extraction for the rendered-output harness.
//!
//! WHAT: parses emitted HTML for the exact script shapes the Node harness can execute and rejects
//!       every other shape.
//! WHY: a permissive substring scanner silently decides what "runtime output" means. It executed
//!      data blocks such as `<script type="importmap">`, ignored external `src` scripts whose code
//!      never ran, matched tags case-sensitively and stopped quietly at a malformed closing tag —
//!      each of which lets a case claim runtime evidence the harness never produced. Unsupported
//!      shapes are harness failures here so that claim cannot be made silently.
//!
//! Supported shapes:
//! - `<script>` with no `type`, executed as classic script source
//! - `<script type="module">`, `type="text/javascript"` and `type="application/javascript"`
//!
//! Recognised but not executed (browsers do not run these as JavaScript either):
//! - `<script type="importmap">`, `type="application/json"`, `type="speculationrules"`
//!
//! Everything else — an external `src`, an unknown `type`, an execution-changing attribute, an
//! unterminated tag or an unterminated attribute value — is rejected.
//!
//! Scripts run in document order. `defer` is inert on an inline script, so it is ignored;
//! `async` and `nomodule` are not, so they are rejected rather than silently overridden.

use super::node_harness::RenderHarnessError;

/// `type` values the harness executes as JavaScript.
const EXECUTABLE_SCRIPT_TYPES: [&str; 3] = ["module", "text/javascript", "application/javascript"];

/// `type` values that mark a data block the harness deliberately skips.
const DATA_SCRIPT_TYPES: [&str; 3] = ["importmap", "application/json", "speculationrules"];

/// Attributes that change whether or in what order a browser runs an inline script.
///
/// A browser skips `nomodule` scripts wherever modules are supported, and `async` releases a
/// module script from document order. Running them anyway would execute code the real page does
/// not, or execute it in an order the real page does not use.
const EXECUTION_CHANGING_ATTRIBUTES: [&str; 2] = ["nomodule", "async"];

/// Returns the inline script sources the harness will execute, in document order.
///
/// Empty inline scripts are dropped because running them is a no-op; every other supported block
/// is returned verbatim so the harness executes exactly the emitted source.
pub(crate) fn extract_executable_scripts(html: &str) -> Result<Vec<String>, RenderHarnessError> {
    let mut scripts = Vec::new();
    let mut offset = 0usize;

    while let Some(tag_start) = find_script_tag_start(html, offset) {
        let open_tag = parse_open_tag(html, tag_start)?;
        let body_start = open_tag.body_start;

        let Some(close_start) = find_ascii_case_insensitive(html, "</script", body_start) else {
            return Err(RenderHarnessError::script_shape(format!(
                "rendered_output: the emitted HTML has a '<script>' tag at byte {tag_start} with \
                 no matching '</script>'. The harness cannot decide what source that tag contains."
            )));
        };

        // The emitter escapes `</script>` inside script bodies, so the terminator is always the
        // exact tag. Anything else — `</script foo>`, `</script` at EOF — is a shape whose browser
        // end-tag parsing the harness does not reproduce, and guessing would change which source
        // the case actually executes.
        let after_close_name = close_start + "</script".len();
        if html.as_bytes().get(after_close_name) != Some(&b'>') {
            return Err(RenderHarnessError::script_shape(format!(
                "rendered_output: the emitted HTML has a malformed '</script' closing tag at byte \
                 {close_start}. The harness terminates a script only at an exact '</script>'."
            )));
        }

        let body = &html[body_start..close_start];
        offset = after_close_name + 1;

        if open_tag.has_attribute("src") {
            return Err(RenderHarnessError::script_shape(format!(
                "rendered_output: the emitted HTML loads an external script 'src=\"{}\"'. \
                 The harness executes inline scripts only, so skipping it would let the case claim \
                 runtime coverage for code that never ran.",
                open_tag.attribute("src").unwrap_or_default()
            )));
        }

        for attribute in EXECUTION_CHANGING_ATTRIBUTES {
            if open_tag.has_attribute(attribute) {
                return Err(RenderHarnessError::script_shape(format!(
                    "rendered_output: the emitted HTML has a '<script {attribute}>' block. That \
                     attribute changes whether or when a browser runs the script, which the \
                     harness does not reproduce."
                )));
            }
        }

        match open_tag.attribute("type") {
            None => push_non_empty(&mut scripts, body),
            Some(script_type) => {
                let normalized = script_type.trim().to_ascii_lowercase();
                if EXECUTABLE_SCRIPT_TYPES.contains(&normalized.as_str()) {
                    push_non_empty(&mut scripts, body);
                } else if !DATA_SCRIPT_TYPES.contains(&normalized.as_str()) {
                    return Err(RenderHarnessError::script_shape(format!(
                        "rendered_output: the emitted HTML contains an unsupported script type \
                         '{script_type}'. Supported executable types are {EXECUTABLE_SCRIPT_TYPES:?} \
                         and recognised data types are {DATA_SCRIPT_TYPES:?}."
                    )));
                }
            }
        }
    }

    Ok(scripts)
}

fn push_non_empty(scripts: &mut Vec<String>, body: &str) {
    if !body.trim().is_empty() {
        scripts.push(body.to_owned());
    }
}

/// One parsed `<script ...>` opening tag.
struct ScriptOpenTag {
    /// Byte offset immediately after the opening tag's `>`.
    body_start: usize,
    /// Attribute names lowercased, paired with their raw values.
    attributes: Vec<(String, Option<String>)>,
}

impl ScriptOpenTag {
    /// The attribute's value, or `None` when it is absent or valueless.
    fn attribute(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|(attribute_name, _)| attribute_name == name)
            .and_then(|(_, value)| value.as_deref())
    }

    /// Whether the attribute is present at all, with or without a value.
    ///
    /// Boolean attributes such as `nomodule` carry no value, and a valueless `src` still makes a
    /// script external, so presence and value are separate questions.
    fn has_attribute(&self, name: &str) -> bool {
        self.attributes
            .iter()
            .any(|(attribute_name, _)| attribute_name == name)
    }
}

/// Finds the next `<script` tag start, ignoring ASCII case and names such as `<scriptish`.
fn find_script_tag_start(html: &str, from: usize) -> Option<usize> {
    let mut search_from = from;

    while let Some(candidate) = find_ascii_case_insensitive(html, "<script", search_from) {
        let after_name = candidate + "<script".len();
        match html.as_bytes().get(after_name).copied() {
            // A `<script` that ends the document is still a script tag; `parse_open_tag` reports
            // it as unterminated rather than letting it vanish from the harness input.
            None => return Some(candidate),
            Some(byte) if byte.is_ascii_whitespace() || byte == b'>' || byte == b'/' => {
                return Some(candidate);
            }
            Some(_) => search_from = after_name,
        }
    }

    None
}

/// Parses the opening tag's attributes, honouring quoted values that contain `>`.
fn parse_open_tag(html: &str, tag_start: usize) -> Result<ScriptOpenTag, RenderHarnessError> {
    let bytes = html.as_bytes();
    let mut cursor = tag_start + "<script".len();
    let mut attributes = Vec::new();

    loop {
        while matches!(bytes.get(cursor), Some(byte) if byte.is_ascii_whitespace()) {
            cursor += 1;
        }

        match bytes.get(cursor) {
            None => {
                return Err(RenderHarnessError::script_shape(format!(
                    "rendered_output: the emitted HTML has an unterminated '<script' tag at byte \
                     {tag_start}."
                )));
            }
            Some(b'>') => {
                return Ok(ScriptOpenTag {
                    body_start: cursor + 1,
                    attributes,
                });
            }
            Some(b'/') if bytes.get(cursor + 1) == Some(&b'>') => {
                return Ok(ScriptOpenTag {
                    body_start: cursor + 2,
                    attributes,
                });
            }
            Some(_) => {}
        }

        let name_start = cursor;
        while matches!(
            bytes.get(cursor),
            Some(byte)
                if !byte.is_ascii_whitespace()
                    && *byte != b'='
                    && *byte != b'>'
                    && *byte != b'/'
        ) {
            cursor += 1;
        }

        // A lone `/` or `=` with no name would otherwise leave the cursor parked forever.
        if cursor == name_start {
            cursor += 1;
            continue;
        }

        let name = html[name_start..cursor].to_ascii_lowercase();

        while matches!(bytes.get(cursor), Some(byte) if byte.is_ascii_whitespace()) {
            cursor += 1;
        }

        if bytes.get(cursor) != Some(&b'=') {
            attributes.push((name, None));
            continue;
        }
        cursor += 1;

        while matches!(bytes.get(cursor), Some(byte) if byte.is_ascii_whitespace()) {
            cursor += 1;
        }

        let value = match bytes.get(cursor) {
            Some(quote @ (b'"' | b'\'')) => {
                let quote = *quote;
                let value_start = cursor + 1;
                let Some(relative_end) =
                    bytes[value_start..].iter().position(|byte| *byte == quote)
                else {
                    return Err(RenderHarnessError::script_shape(format!(
                        "rendered_output: the emitted HTML has an unterminated attribute value in \
                         the '<script' tag at byte {tag_start}."
                    )));
                };
                let value_end = value_start + relative_end;
                cursor = value_end + 1;
                html[value_start..value_end].to_owned()
            }
            Some(_) => {
                let value_start = cursor;
                while matches!(
                    bytes.get(cursor),
                    Some(byte) if !byte.is_ascii_whitespace() && *byte != b'>'
                ) {
                    cursor += 1;
                }
                html[value_start..cursor].to_owned()
            }
            None => {
                return Err(RenderHarnessError::script_shape(format!(
                    "rendered_output: the emitted HTML has an unterminated '<script' tag at byte \
                     {tag_start}."
                )));
            }
        };

        attributes.push((name, Some(value)));
    }
}

/// ASCII-case-insensitive substring search returning a byte offset.
///
/// The needle is ASCII, so every returned offset lands on a character boundary.
fn find_ascii_case_insensitive(haystack: &str, needle: &str, from: usize) -> Option<usize> {
    let haystack = haystack.as_bytes();
    let needle = needle.as_bytes();

    if from >= haystack.len() || needle.len() > haystack.len() - from {
        return None;
    }

    (from..=haystack.len() - needle.len())
        .find(|start| haystack[*start..*start + needle.len()].eq_ignore_ascii_case(needle))
}
