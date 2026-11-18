//! Text normalization shared by every extractor.
//!
//! Normalized text is what gets stored in [`Block::text`] (the model has a
//! single text field): differences in insignificant whitespace, Unicode
//! composition form or non-breaking spaces must not create spurious diff
//! blocks in later phases.

use unicode_normalization::UnicodeNormalization;

/// Normalize a text fragment:
///
/// 1. Unicode NFC composition (Vietnamese diacritics arrive in mixed forms
///    depending on the producing application).
/// 2. Collapse every run of whitespace characters to a single ASCII space —
///    this includes non-breaking spaces (U+00A0), tabs, CRLF and exotic
///    separators (`char::is_whitespace` implements the Unicode `White_Space`
///    property, which covers all of them).
/// 3. Trim leading/trailing whitespace.
///
/// Returns an empty string when the input contains no printable content;
/// callers drop blocks whose normalized text is empty.
pub fn normalize(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut pending_space = false;
    for ch in input.nfc() {
        if ch.is_whitespace() {
            pending_space = true;
        } else {
            if pending_space && !out.is_empty() {
                out.push(' ');
            }
            pending_space = false;
            out.push(ch);
        }
    }
    out
}

/// True when the text carries no printable content after normalization.
pub fn is_blank(input: &str) -> bool {
    normalize(input).is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collapses_whitespace_runs_including_nbsp() {
        assert_eq!(normalize("a \t b\u{00A0}\u{00A0}c\r\n\rd"), "a b c d");
    }

    #[test]
    fn trims_ends() {
        assert_eq!(normalize("  hello\u{00A0}"), "hello");
        assert_eq!(normalize("   "), "");
    }

    #[test]
    fn composes_nfc() {
        // "ế" as precomposed U+1EBF vs decomposed "e" + U+0302 + U+0301.
        let nfc = "\u{1ebf}";
        let nfd = "e\u{0302}\u{0301}";
        assert_eq!(normalize(nfd), normalize(nfc));
    }

    #[test]
    fn blank_detection() {
        assert!(is_blank(" \u{00A0}\n"));
        assert!(!is_blank("x"));
    }
}
