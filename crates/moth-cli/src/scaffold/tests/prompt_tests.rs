//! Tests for the shared `ScriptedPrompt` test helper.
//!
//! `ScriptedPrompt` is shared by every `new_html_project` test module, so these
//! tests own only the behaviour all of those modules depend on: ordered
//! response consumption and exhaustion across `ask` and `confirm`, prompt
//! message capture, yes/y/no normalisation, and the two empty-response defaults.

use crate::scaffold::prompt::Prompt;

/// Test-only prompt that replays scripted responses.
#[cfg(test)]
pub struct ScriptedPrompt {
    pub responses: std::collections::VecDeque<String>,
    pub messages: Vec<String>,
}

#[cfg(test)]
impl ScriptedPrompt {
    pub fn new(responses: Vec<String>) -> Self {
        Self {
            responses: responses.into(),
            messages: Vec::new(),
        }
    }
}

#[cfg(test)]
impl Prompt for ScriptedPrompt {
    fn ask(&mut self, message: &str) -> Result<String, String> {
        self.messages.push(message.to_owned());
        self.responses
            .pop_front()
            .ok_or_else(|| "ScriptedPrompt ran out of ask responses".to_string())
    }

    fn confirm(&mut self, message: &str, default: bool) -> Result<bool, String> {
        self.messages.push(message.to_owned());
        let response = self
            .responses
            .pop_front()
            .ok_or_else(|| "ScriptedPrompt ran out of confirm responses".to_string())?;
        let trimmed = response.trim();
        if trimmed.is_empty() {
            return Ok(default);
        }
        let normalized = trimmed.to_ascii_lowercase();
        Ok(matches!(normalized.as_str(), "y" | "yes"))
    }
}

#[test]
fn scripted_prompt_consumes_responses_in_order_and_exhausts() {
    // The shared response queue is consumed in FIFO order across both ask and
    // confirm, then exhaustion surfaces a distinct error per method.
    let mut prompt = ScriptedPrompt::new(vec![
        String::from("first"),
        String::from("yes"),
        String::from("second"),
    ]);

    assert_eq!(prompt.ask("Q1?").unwrap(), "first");
    assert!(prompt.confirm("Ok?", false).unwrap());
    assert_eq!(prompt.ask("Q2?").unwrap(), "second");

    assert!(prompt.ask("Q3?").is_err());
    assert!(prompt.confirm("Ok?", false).is_err());
}

#[test]
fn scripted_prompt_records_each_prompt_message() {
    let mut prompt = ScriptedPrompt::new(vec![String::from("answer")]);

    let _ = prompt.ask("Hello?");

    assert_eq!(prompt.messages, vec!["Hello?"]);
}

#[test]
fn scripted_prompt_confirm_normalizes_yes_y_and_rejects_other() {
    // One labelled owner for the yes/y/no normalisation contract.
    for (input, expected) in [
        ("yes", true),
        ("Yes", true),
        ("Y", true),
        ("y", true),
        ("no", false),
        ("n", false),
        ("maybe", false),
    ] {
        let mut prompt = ScriptedPrompt::new(vec![String::from(input)]);

        assert_eq!(
            prompt.confirm("Ok?", false).unwrap(),
            expected,
            "input {input:?}"
        );
    }
}

#[test]
fn scripted_prompt_confirm_uses_default_on_empty_response() {
    // One labelled owner for both empty-response defaults.
    for default in [true, false] {
        let mut prompt = ScriptedPrompt::new(vec![String::from("")]);

        assert_eq!(
            prompt.confirm("Ok?", default).unwrap(),
            default,
            "default {default}"
        );
    }
}
