//! What processes print: outcome lines for Statespace, and output to keep.

use regex::Regex;
use serde_json::{Map, Value};

/// The key that marks a printed JSON line as an outcome.
pub const OUTCOME_KEY: &str = "statespace";
/// Captured lines longer than this are cut.
const MAX_LINE_BYTES: usize = 16 * 1024;

/// An outcome a process printed, such as
/// `{"statespace": "purchase", "subject": "u_42", "amount": 42}`.
#[derive(Debug, PartialEq)]
pub struct Outcome {
    pub name: String,
    pub subject: String,
    pub data: Map<String, Value>,
}

/// What a line is.
#[derive(Debug, PartialEq)]
pub enum Line {
    Outcome(Outcome),
    /// A line marked as an outcome that cannot be recorded, and why.
    Invalid(&'static str),
    Plain,
}

pub fn classify(line: &str) -> Line {
    let trimmed = line.trim();
    if !trimmed.starts_with('{') || !trimmed.contains(OUTCOME_KEY) {
        return Line::Plain;
    }
    let Ok(Value::Object(mut fields)) = serde_json::from_str::<Value>(trimmed) else {
        return Line::Plain;
    };
    let Some(name) = fields.remove(OUTCOME_KEY) else {
        return Line::Plain;
    };
    let Value::String(name) = name else {
        return Line::Invalid("the outcome name must be a string");
    };
    if !valid_name(&name) {
        return Line::Invalid(
            "an outcome name must use 1 to 100 letters, numbers, dots, dashes, or underscores",
        );
    }
    let subject = match fields.remove("subject") {
        Some(Value::String(subject)) => subject,
        Some(Value::Number(number)) => number.to_string(),
        _ => return Line::Invalid("an outcome needs a subject"),
    };
    if subject.is_empty() || subject.len() > 128 {
        return Line::Invalid("a subject must contain 1 to 128 characters");
    }
    if trimmed.len() > 60 * 1024 {
        return Line::Invalid("an outcome must be smaller than 60 KiB");
    }
    Line::Outcome(Outcome {
        name,
        subject,
        data: fields,
    })
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 100
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

/// Replace every match of `patterns` and cut overly long lines, before a
/// line leaves the machine.
pub fn redact(line: &str, patterns: &[Regex]) -> String {
    let mut line = line.to_owned();
    for pattern in patterns {
        if pattern.is_match(&line) {
            line = pattern.replace_all(&line, "[redacted]").into_owned();
        }
    }
    if line.len() > MAX_LINE_BYTES {
        let mut end = MAX_LINE_BYTES;
        while !line.is_char_boundary(end) {
            end -= 1;
        }
        line.truncate(end);
    }
    line
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn recognizes_outcome_lines() {
        let line = r#"{"statespace": "purchase", "subject": "u_42", "amount": 42}"#;
        let Line::Outcome(outcome) = classify(line) else {
            panic!("not an outcome");
        };
        assert_eq!(
            (outcome.name.as_str(), outcome.subject.as_str()),
            ("purchase", "u_42")
        );
        assert_eq!(Value::Object(outcome.data), json!({"amount": 42}));
        assert!(matches!(
            classify(r#"{"statespace": "signup", "subject": 7}"#),
            Line::Outcome(_)
        ));
    }

    #[test]
    fn leaves_other_lines_alone() {
        for line in [
            "GET /checkout 200",
            r#"{"level": "info", "msg": "statespace is great"}"#,
            "{not json statespace",
        ] {
            assert_eq!(classify(line), Line::Plain, "{line}");
        }
        assert!(matches!(
            classify(r#"{"statespace": "purchase"}"#),
            Line::Invalid(_)
        ));
        assert!(matches!(
            classify(r#"{"statespace": "bad name!", "subject": "u"}"#),
            Line::Invalid(_)
        ));
    }

    #[test]
    fn redacts_before_lines_leave() {
        let patterns = [Regex::new(r"sk_live_\w+").unwrap()];
        assert_eq!(
            redact("charging with sk_live_abc123 now", &patterns),
            "charging with [redacted] now"
        );
        assert_eq!(
            redact(&"é".repeat(MAX_LINE_BYTES), &[]).len(),
            MAX_LINE_BYTES
        );
    }
}
