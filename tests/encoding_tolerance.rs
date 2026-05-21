//! End-to-end checks for the shared encoding-tolerance layer wired
//! through every text-subtitle parser in this crate.
//!
//! Three flavours of "non-canonical" input we expect to round-trip
//! cleanly:
//!
//! 1. **UTF-16 LE BOM** (`FF FE …`). Common from YouTube SRT export
//!    and from Windows DVD-authoring tooling.
//! 2. **UTF-16 BE BOM** (`FE FF …`). Less common but seen.
//! 3. **Mac classic line endings** (CR only, no LF). Some older
//!    authoring chains still emit CR-only files; without
//!    normalisation a `split('\n')` parser sees the whole file as
//!    one line.
//!
//! For each format we encode the same canonical UTF-8 + LF reference
//! into the format under test, then verify the parser produces the
//! same cue list it would for the canonical encoding.

use oxideav_subtitle::{microdvd, mpl2, srt, webvtt};

fn utf16_le_with_bom(s: &str) -> Vec<u8> {
    let mut out = vec![0xFF, 0xFE];
    for u in s.encode_utf16() {
        out.extend_from_slice(&u.to_le_bytes());
    }
    out
}

fn utf16_be_with_bom(s: &str) -> Vec<u8> {
    let mut out = vec![0xFE, 0xFF];
    for u in s.encode_utf16() {
        out.extend_from_slice(&u.to_be_bytes());
    }
    out
}

fn mac_cr_only(s: &str) -> Vec<u8> {
    s.replace('\n', "\r").into_bytes()
}

fn dos_crlf(s: &str) -> Vec<u8> {
    s.replace('\n', "\r\n").into_bytes()
}

// ---------------------------------------------------------------------------
// SRT
// ---------------------------------------------------------------------------

const SRT_REF: &str = "\
1
00:00:01,000 --> 00:00:03,500
Hello world
second line

2
00:00:04,000 --> 00:00:06,000
final cue
";

#[test]
fn srt_utf16_le_bom_decodes() {
    let bytes = utf16_le_with_bom(SRT_REF);
    let t = srt::parse(&bytes).expect("UTF-16 LE BOM should parse");
    assert_eq!(t.cues.len(), 2);
    assert_eq!(t.cues[0].start_us, 1_000_000);
    assert_eq!(t.cues[0].end_us, 3_500_000);
    assert_eq!(t.cues[1].start_us, 4_000_000);
}

#[test]
fn srt_utf16_be_bom_decodes() {
    let bytes = utf16_be_with_bom(SRT_REF);
    let t = srt::parse(&bytes).expect("UTF-16 BE BOM should parse");
    assert_eq!(t.cues.len(), 2);
    assert_eq!(t.cues[1].end_us, 6_000_000);
}

#[test]
fn srt_mac_cr_only_line_endings_decode() {
    let bytes = mac_cr_only(SRT_REF);
    let t = srt::parse(&bytes).expect("CR-only line endings should parse");
    assert_eq!(t.cues.len(), 2);
    assert_eq!(t.cues[0].start_us, 1_000_000);
    assert_eq!(t.cues[1].start_us, 4_000_000);
}

#[test]
fn srt_dos_crlf_line_endings_decode() {
    let bytes = dos_crlf(SRT_REF);
    let t = srt::parse(&bytes).expect("CRLF line endings should parse");
    assert_eq!(t.cues.len(), 2);
}

#[test]
fn srt_utf16_le_with_mac_cr_endings_decodes() {
    // The line-ending normaliser must run on the post-decode string,
    // so a UTF-16 file with CR-only endings should still split into
    // cues correctly.
    let bytes = utf16_le_with_bom(&SRT_REF.replace('\n', "\r"));
    let t = srt::parse(&bytes).expect("UTF-16 LE + CR-only should parse");
    assert_eq!(t.cues.len(), 2);
    assert_eq!(t.cues[0].end_us, 3_500_000);
}

// ---------------------------------------------------------------------------
// WebVTT
// ---------------------------------------------------------------------------

const WEBVTT_REF: &str = "\
WEBVTT

00:00:01.000 --> 00:00:03.500
Hello world

00:00:04.000 --> 00:00:06.000
final
";

#[test]
fn webvtt_utf16_le_bom_decodes() {
    let bytes = utf16_le_with_bom(WEBVTT_REF);
    let t = webvtt::parse(&bytes).expect("WebVTT UTF-16 LE BOM should parse");
    assert_eq!(t.cues.len(), 2);
    assert_eq!(t.cues[0].start_us, 1_000_000);
}

#[test]
fn webvtt_mac_cr_only_line_endings_decode() {
    // The WebVTT spec (§4) explicitly lists CR alone as a valid line
    // terminator, so this is conformance, not tolerance.
    let bytes = mac_cr_only(WEBVTT_REF);
    let t = webvtt::parse(&bytes).expect("WebVTT CR-only should parse");
    assert_eq!(t.cues.len(), 2);
    assert_eq!(t.cues[1].end_us, 6_000_000);
}

// ---------------------------------------------------------------------------
// MicroDVD
// ---------------------------------------------------------------------------

const MICRODVD_REF: &str = "\
{1}{50}Hello world
{100}{150}second cue
";

#[test]
fn microdvd_utf16_le_bom_decodes() {
    let bytes = utf16_le_with_bom(MICRODVD_REF);
    let t = microdvd::parse(&bytes).expect("MicroDVD UTF-16 LE BOM should parse");
    assert_eq!(t.cues.len(), 2);
}

#[test]
fn microdvd_mac_cr_only_line_endings_decode() {
    let bytes = mac_cr_only(MICRODVD_REF);
    let t = microdvd::parse(&bytes).expect("MicroDVD CR-only should parse");
    assert_eq!(t.cues.len(), 2);
}

// ---------------------------------------------------------------------------
// MPL2
// ---------------------------------------------------------------------------

const MPL2_REF: &str = "\
[10][35]Hello world
[40][60]second cue
";

#[test]
fn mpl2_utf16_le_bom_decodes() {
    let bytes = utf16_le_with_bom(MPL2_REF);
    let t = mpl2::parse(&bytes).expect("MPL2 UTF-16 LE BOM should parse");
    assert_eq!(t.cues.len(), 2);
}

#[test]
fn mpl2_mac_cr_only_line_endings_decode() {
    let bytes = mac_cr_only(MPL2_REF);
    let t = mpl2::parse(&bytes).expect("MPL2 CR-only should parse");
    assert_eq!(t.cues.len(), 2);
}
