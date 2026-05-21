//! Shared text-decoding helpers for the parsers in this crate.
//!
//! Every text subtitle format we support is, on paper, UTF-8. In practice
//! the wild emits four flavours:
//!
//! 1. **Plain UTF-8.** The common case.
//! 2. **UTF-8 with a leading byte-order mark** (`EF BB BF`). Common from
//!    Windows tooling (Notepad / Subtitle Workshop).
//! 3. **UTF-16 LE with BOM** (`FF FE`). Common from YouTube's subtitle
//!    export and some Windows DVD-authoring tooling, especially for SRT
//!    and SubViewer. RFC 8259 (JSON) banned non-UTF-8 in 2017, but the
//!    subtitle ecosystem has no equivalent standardisation pass.
//! 4. **UTF-16 BE with BOM** (`FE FF`). Rare but not unseen.
//!
//! Line endings have the same matrix:
//!
//! 1. **LF only** (`\n`). Unix / web.
//! 2. **CRLF** (`\r\n`). DOS / Windows.
//! 3. **CR only** (`\r`). Classic Mac OS (≤ Mac OS 9) and some early
//!    authoring tools that emitted CR-only files. Without normalisation
//!    the entire file becomes one line for a `split('\n')` parser.
//!
//! [`decode_text_lossy`] handles the encoding side: it sniffs the four
//! BOMs above and decodes accordingly, returning a `String`. Invalid
//! sequences are replaced with U+FFFD (the same `String::from_utf8_lossy`
//! behaviour the per-parser helpers gave; we never reject a file just
//! because one byte was bad).
//!
//! [`normalize_newlines`] handles the line-ending side: every standalone
//! `\r` (i.e. not paired into `\r\n`) is rewritten to `\n`, and `\r\n`
//! pairs are collapsed to `\n`. The result is a string whose only line
//! terminator is `\n`, so every parser's `split('\n')` / `.lines()` call
//! sees the structure the spec intended regardless of which line-ending
//! flavour the file used.
//!
//! [`decode_subtitle_text`] is the one-shot helper: BOM-sniff, decode,
//! normalise. Every parser routes through it.
//!
//! The WebVTT spec (§4) says line terminators are CRLF / LF / CR, so
//! this normalisation is exactly what WebVTT requires. The SRT, MicroDVD,
//! MPL2, … formats have no formal spec but the consensus behaviour
//! among interoperating decoders (mpv, VLC, subtitle-edit) is to accept
//! the same four BOMs and the same three line terminators.

/// One-shot helper: sniff the BOM, decode to UTF-8, normalise line
/// endings to LF. Every parser in this crate routes its raw byte slice
/// through this function to get a `String` it can then split on `\n`.
pub fn decode_subtitle_text(bytes: &[u8]) -> String {
    let decoded = decode_text_lossy(bytes);
    normalize_newlines(decoded)
}

/// Sniff the four BOMs (UTF-8, UTF-16 LE, UTF-16 BE — UTF-32 omitted
/// because no subtitle file has ever shipped UTF-32 in the wild) and
/// decode to a `String`. Invalid byte sequences become U+FFFD.
pub fn decode_text_lossy(bytes: &[u8]) -> String {
    // UTF-8 BOM: EF BB BF. Strip and pass through `from_utf8_lossy`.
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(&bytes[3..]).into_owned();
    }
    // UTF-16 LE BOM: FF FE. Decode pairs little-endian.
    if bytes.starts_with(&[0xFF, 0xFE]) {
        return decode_utf16(&bytes[2..], /*little_endian=*/ true);
    }
    // UTF-16 BE BOM: FE FF. Decode pairs big-endian.
    if bytes.starts_with(&[0xFE, 0xFF]) {
        return decode_utf16(&bytes[2..], /*little_endian=*/ false);
    }
    // No BOM — assume UTF-8 (the spec-required encoding for every text
    // subtitle format we host).
    String::from_utf8_lossy(bytes).into_owned()
}

/// Collapse `\r\n` and lone `\r` to `\n`. Allocates a new `String` only
/// when the input contains any `\r` (the LF-only common case is a free
/// `into()`).
///
/// The scan is byte-oriented but UTF-8 safe: `\r` is U+000D (ASCII),
/// which cannot appear as a continuation byte of any multibyte UTF-8
/// sequence (every continuation byte has its high bit set). Bytes other
/// than `\r` are copied verbatim, preserving multibyte sequences intact.
pub fn normalize_newlines<S: Into<String>>(input: S) -> String {
    let s = input.into();
    if !s.contains('\r') {
        return s;
    }
    let mut out = Vec::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'\r' {
            out.push(b'\n');
            // Eat the LF half of a CRLF pair so we don't emit a blank
            // line between every Windows-style cue.
            if i + 1 < bytes.len() && bytes[i + 1] == b'\n' {
                i += 2;
            } else {
                i += 1;
            }
        } else {
            out.push(b);
            i += 1;
        }
    }
    // `s` was a valid UTF-8 String; we only ever removed `\r` bytes
    // (ASCII, single-byte) or substituted them with `\n` (also ASCII,
    // single-byte). No multibyte boundary can have been split, so the
    // re-validation here is cheap and avoids `unsafe`.
    String::from_utf8(out).expect("normalize_newlines preserves UTF-8")
}

/// Decode a UTF-16 stream (already past its BOM) into a `String`. Lone
/// surrogates and odd-byte tails become U+FFFD so the result is always
/// a valid `String`.
fn decode_utf16(bytes: &[u8], little_endian: bool) -> String {
    let mut units = Vec::with_capacity(bytes.len() / 2);
    let mut i = 0;
    while i + 1 < bytes.len() {
        let unit = if little_endian {
            u16::from_le_bytes([bytes[i], bytes[i + 1]])
        } else {
            u16::from_be_bytes([bytes[i], bytes[i + 1]])
        };
        units.push(unit);
        i += 2;
    }
    // A trailing single byte at odd length is malformed; the existing
    // `from_utf8_lossy` policy is to replace, so we drop it (there's no
    // U+FFFD code unit equivalent for "half a code unit").
    String::from_utf16_lossy(&units)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_utf8_passes_through() {
        let s = decode_subtitle_text(b"Hello, world.\n");
        assert_eq!(s, "Hello, world.\n");
    }

    #[test]
    fn utf8_bom_is_stripped() {
        let mut buf = vec![0xEF, 0xBB, 0xBF];
        buf.extend_from_slice(b"Hello.\n");
        let s = decode_subtitle_text(&buf);
        assert_eq!(s, "Hello.\n");
    }

    #[test]
    fn utf16_le_bom_decodes() {
        // BOM + "Hi\n" in UTF-16 LE.
        let buf = [
            0xFF, 0xFE, // BOM
            b'H', 0x00, b'i', 0x00, b'\n', 0x00,
        ];
        let s = decode_subtitle_text(&buf);
        assert_eq!(s, "Hi\n");
    }

    #[test]
    fn utf16_be_bom_decodes() {
        // BOM + "Hi\n" in UTF-16 BE.
        let buf = [
            0xFE, 0xFF, // BOM
            0x00, b'H', 0x00, b'i', 0x00, b'\n',
        ];
        let s = decode_subtitle_text(&buf);
        assert_eq!(s, "Hi\n");
    }

    #[test]
    fn utf16_le_decodes_non_ascii_bmp() {
        // BOM + "é" (U+00E9 → 0xE9 0x00 LE).
        let buf = [0xFF, 0xFE, 0xE9, 0x00];
        let s = decode_subtitle_text(&buf);
        assert_eq!(s, "é");
    }

    #[test]
    fn utf16_le_decodes_surrogate_pair() {
        // 😀 (U+1F600) → high D83D, low DE00.
        let buf = [
            0xFF, 0xFE, // BOM
            0x3D, 0xD8, 0x00, 0xDE,
        ];
        let s = decode_subtitle_text(&buf);
        assert_eq!(s, "😀");
    }

    #[test]
    fn mac_cr_only_becomes_lf() {
        let s = decode_subtitle_text(b"one\rtwo\rthree\r");
        assert_eq!(s, "one\ntwo\nthree\n");
    }

    #[test]
    fn dos_crlf_collapses_to_lf() {
        let s = decode_subtitle_text(b"one\r\ntwo\r\n");
        assert_eq!(s, "one\ntwo\n");
    }

    #[test]
    fn mixed_endings_normalise() {
        // CRLF + lone CR + lone LF should all flatten to LF.
        let s = decode_subtitle_text(b"a\r\nb\rc\nd");
        assert_eq!(s, "a\nb\nc\nd");
    }

    #[test]
    fn cr_inside_text_is_normalised() {
        // A standalone CR mid-string becomes LF (we don't try to
        // preserve a CR that isn't part of a CRLF pair).
        let s = decode_subtitle_text(b"abc\rdef");
        assert_eq!(s, "abc\ndef");
    }

    #[test]
    fn lf_only_is_zero_allocation_path() {
        // Sanity: an LF-only string isn't mangled and equals what we
        // sent in.
        let s = decode_subtitle_text(b"a\nb\nc\n");
        assert_eq!(s, "a\nb\nc\n");
    }

    #[test]
    fn empty_input_yields_empty_string() {
        assert_eq!(decode_subtitle_text(&[]), "");
    }

    #[test]
    fn bare_bom_yields_empty_string() {
        assert_eq!(decode_subtitle_text(&[0xEF, 0xBB, 0xBF]), "");
        assert_eq!(decode_subtitle_text(&[0xFF, 0xFE]), "");
        assert_eq!(decode_subtitle_text(&[0xFE, 0xFF]), "");
    }

    #[test]
    fn utf16_le_normalises_cr() {
        // BOM + "a\rb" in UTF-16 LE. CR should still become LF after
        // the UTF-16 decode pass.
        let buf = [
            0xFF, 0xFE, // BOM
            b'a', 0x00, b'\r', 0x00, b'b', 0x00,
        ];
        let s = decode_subtitle_text(&buf);
        assert_eq!(s, "a\nb");
    }

    #[test]
    fn utf16_le_normalises_crlf() {
        // BOM + "a\r\nb" in UTF-16 LE.
        let buf = [
            0xFF, 0xFE, // BOM
            b'a', 0x00, b'\r', 0x00, b'\n', 0x00, b'b', 0x00,
        ];
        let s = decode_subtitle_text(&buf);
        assert_eq!(s, "a\nb");
    }

    #[test]
    fn utf16_le_odd_tail_byte_is_dropped() {
        // BOM + "a" + dangling 0x00 — odd-byte tail should not panic;
        // String stays valid.
        let buf = [
            0xFF, 0xFE, // BOM
            b'a', 0x00, 0x42,
        ];
        let s = decode_subtitle_text(&buf);
        assert_eq!(s, "a");
    }

    #[test]
    fn invalid_utf8_replaces_with_replacement_char() {
        // 0xFF 0x40 is not a legal UTF-8 sequence (and 0xFF is not a
        // BOM byte either on its own).
        let s = decode_subtitle_text(&[0xFF, 0x40, b'a']);
        assert!(s.contains('\u{FFFD}'));
        assert!(s.ends_with("@a"));
    }
}
