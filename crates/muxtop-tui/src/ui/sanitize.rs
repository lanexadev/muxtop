//! Defensive sanitizer for strings that originate outside the muxtop process
//! (container names/images/status, process names/commands/users, network
//! interface names) and that we render verbatim into a TUI cell.
//!
//! Without this, a hostile container `LABEL` or process `comm` field
//! containing `\x1b]0;evil\x07` would be interpreted by the terminal as an
//! OSC sequence and could rewrite the window title, change colours, or
//! otherwise escape the muxtop UI sandbox (MED-S5).
//!
//! The sanitizer replaces every ASCII control byte with `?` while preserving
//! UTF-8 multi-byte characters and the tab character (`\t`, useful in some
//! command-line displays).

use std::borrow::Cow;

/// Return `s` with every ASCII control byte replaced by `?`, preserving
/// UTF-8 sequences and the tab character.
///
/// Returns [`Cow::Borrowed`] when the input is already clean — the hot path
/// (almost every render frame) allocates nothing in that case.
///
/// Stripped bytes:
/// - `0x00..=0x08`
/// - `0x0a..=0x1f` (which includes ESC `0x1b` — the OSC/CSI introducer)
/// - `0x7f` (DEL)
///
/// Preserved bytes:
/// - `0x09` (TAB)
/// - any byte `>= 0x80` (continuation/leading bytes of UTF-8 multi-byte chars)
pub fn scrub_ctrl(s: &str) -> Cow<'_, str> {
    let needs_scrubbing = s.bytes().any(is_offending);
    if !needs_scrubbing {
        return Cow::Borrowed(s);
    }

    // Allocate at most `s.len()` bytes — replacement is one ASCII byte for
    // one ASCII byte, and multi-byte UTF-8 bytes are passed through as-is.
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        let code = ch as u32;
        if code < 0x80 && is_offending(code as u8) {
            out.push('?');
        } else {
            out.push(ch);
        }
    }
    Cow::Owned(out)
}

#[inline]
fn is_offending(b: u8) -> bool {
    // Control range minus TAB, plus DEL.
    (b < 0x20 && b != b'\t') || b == 0x7f
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_ascii_is_borrowed() {
        let s = "nginx-prod-01";
        let out = scrub_ctrl(s);
        assert!(matches!(out, Cow::Borrowed(_)));
        assert_eq!(out, "nginx-prod-01");
    }

    #[test]
    fn empty_string_is_borrowed() {
        let out = scrub_ctrl("");
        assert!(matches!(out, Cow::Borrowed(_)));
    }

    #[test]
    fn tab_is_preserved() {
        let s = "col1\tcol2";
        let out = scrub_ctrl(s);
        assert!(matches!(out, Cow::Borrowed(_)));
        assert_eq!(out, "col1\tcol2");
    }

    #[test]
    fn osc_sequence_is_replaced() {
        // OSC 0; "evil" BEL — would rewrite the window title if rendered raw.
        let s = "\x1b]0;evil\x07";
        let out = scrub_ctrl(s);
        assert!(matches!(out, Cow::Owned(_)));
        // ESC, BEL → '?'; semicolon and 0/evil are printable → preserved.
        assert_eq!(out, "?]0;evil?");
    }

    #[test]
    fn csi_color_sequence_is_replaced() {
        // \x1b[31m red, \x1b[0m reset.
        let s = "\x1b[31mRED\x1b[0m";
        let out = scrub_ctrl(s);
        assert!(matches!(out, Cow::Owned(_)));
        assert_eq!(out, "?[31mRED?[0m");
    }

    #[test]
    fn newline_and_carriage_return_are_replaced() {
        let s = "line1\nline2\rline3";
        let out = scrub_ctrl(s);
        assert!(matches!(out, Cow::Owned(_)));
        assert_eq!(out, "line1?line2?line3");
    }

    #[test]
    fn del_byte_is_replaced() {
        let s = "abc\x7fdef";
        let out = scrub_ctrl(s);
        assert!(matches!(out, Cow::Owned(_)));
        assert_eq!(out, "abc?def");
    }

    #[test]
    fn null_byte_is_replaced() {
        let s = "abc\x00def";
        let out = scrub_ctrl(s);
        assert!(matches!(out, Cow::Owned(_)));
        assert_eq!(out, "abc?def");
    }

    #[test]
    fn multibyte_utf8_is_preserved() {
        // Latin-1 supplement and CJK — both must round-trip unchanged.
        let s = "café 中文 日本語";
        let out = scrub_ctrl(s);
        assert!(matches!(out, Cow::Borrowed(_)));
        assert_eq!(&*out, s);
        // Cheap String round-trip to confirm bytes are intact.
        let bytes: Vec<u8> = out.as_bytes().to_vec();
        assert_eq!(String::from_utf8(bytes).unwrap(), s);
    }

    #[test]
    fn multibyte_with_control_strips_only_control() {
        let s = "café\x1bbar";
        let out = scrub_ctrl(s);
        assert!(matches!(out, Cow::Owned(_)));
        assert_eq!(&*out, "café?bar");
    }

    #[test]
    fn single_escape_is_replaced() {
        let s = "\x1b";
        let out = scrub_ctrl(s);
        assert_eq!(&*out, "?");
    }
}
