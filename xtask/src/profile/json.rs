//! Narrow JSON string escaping for profile-owned manual artifact writers.

pub(super) fn escape(value: &str) -> String {
    let encoded = serde_json::to_string(value).expect("serializing a string to JSON cannot fail");
    encoded[1..encoded.len() - 1].to_owned()
}

#[cfg(test)]
#[path = "json_tests.rs"]
mod tests;
