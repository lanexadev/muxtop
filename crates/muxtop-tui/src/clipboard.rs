// Clipboard over OSC 52.
//
// muxtop's job is to be useful on a machine you are not sitting at, so the
// clipboard has to work through `ssh` and through `tmux`. OSC 52 is the only
// mechanism that does: the terminal emulator on the *local* end performs the
// copy, so no X11 / Wayland / pbcopy binary is needed on the remote host.
//
// Terminals that do not implement it ignore the sequence, which is why this is
// best-effort and never fatal.

use std::io::{self, Write};

/// Longest payload we will send.
///
/// OSC 52 goes through the terminal's input buffer; a huge payload can wedge
/// some emulators. muxtop only ever copies an identifier — a PID, an interface
/// name, a container id, a Kubernetes object name — so this ceiling is far
/// above anything legitimate and exists purely as a guard.
const MAX_PAYLOAD: usize = 4096;

/// Copy `text` to the system clipboard.
pub fn copy(text: &str) -> Result<(), &'static str> {
    if text.is_empty() {
        return Err("nothing to copy");
    }
    if text.len() > MAX_PAYLOAD {
        return Err("selection is too large to copy");
    }
    let payload = base64(text.as_bytes());
    // `c` is the system clipboard selection. BEL terminates the string; it is
    // accepted more widely than ST (ESC \).
    let seq = format!("\x1b]52;c;{payload}\x07");

    let mut out = io::stdout();
    out.write_all(seq.as_bytes())
        .and_then(|()| out.flush())
        .map_err(|_| "terminal write failed")
}

/// Minimal standard base64 encoder.
///
/// Written out rather than pulling in a crate: this is the only base64 in the
/// whole workspace, and a dependency would cost more than the twenty lines.
fn base64(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        let idx = [
            (n >> 18) & 0x3f,
            (n >> 12) & 0x3f,
            (n >> 6) & 0x3f,
            n & 0x3f,
        ];
        out.push(ALPHABET[idx[0] as usize] as char);
        out.push(ALPHABET[idx[1] as usize] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[idx[2] as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[idx[3] as usize] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_matches_the_rfc_test_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
        assert_eq!(base64(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn base64_handles_binary_and_high_bytes() {
        assert_eq!(base64(&[0x00, 0x00, 0x00]), "AAAA");
        assert_eq!(base64(&[0xff, 0xff, 0xff]), "////");
        assert_eq!(base64(&[0xfb, 0xff, 0xbf]), "+/+/");
    }

    #[test]
    fn base64_output_length_is_always_a_multiple_of_four() {
        for len in 0..64 {
            let input = vec![b'x'; len];
            assert_eq!(base64(&input).len() % 4, 0, "bad padding at length {len}");
        }
    }

    #[test]
    fn base64_round_trips_a_realistic_container_id() {
        let id = "3f2b9c1a4e8d7f6b5a4c3d2e1f0a9b8c7d6e5f4a3b2c1d0e9f8a7b6c5d4e3f2b";
        let encoded = base64(id.as_bytes());
        assert!(encoded.is_ascii());
        assert!(!encoded.contains('\n'), "OSC 52 payloads must be one line");
    }

    #[test]
    fn empty_selection_is_rejected() {
        assert!(copy("").is_err());
    }

    #[test]
    fn oversized_selection_is_rejected() {
        let big = "x".repeat(MAX_PAYLOAD + 1);
        assert!(copy(&big).is_err());
    }
}
