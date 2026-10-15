//! What arms print. Every line is recorded; patterns are redacted first.

use regex::Regex;

/// Recorded lines longer than this are cut.
const MAX_LINE_BYTES: usize = 16 * 1024;

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
    use super::*;

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
