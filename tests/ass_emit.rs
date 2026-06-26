//! ASS / SSA serialization + round-trip fidelity tests.

use oxideav_subtitle::ass_emit::{color_to_string, serialize_line, style_row_to_string};
use oxideav_subtitle::ass_resolve::{resolve_line, Rgba, StyleBase};
use oxideav_subtitle::ass_style_row::{parse_style_row, DEFAULT_V4PLUS_FORMAT};

fn base() -> StyleBase {
    StyleBase::default()
}

/// parse(text) -> serialize -> re-resolve must reproduce the same spans
/// and layout.
fn assert_roundtrip(text: &str, base: &StyleBase) {
    let first = resolve_line(text, base);
    let serialized = serialize_line(&first, base);
    let second = resolve_line(&serialized, base);
    assert_eq!(
        first, second,
        "round-trip mismatch\n  input:      {text}\n  serialized: {serialized}"
    );
}

#[test]
fn color_to_string_alpha_inverts() {
    // straight alpha 255 (opaque) -> ASS 00.
    let c = Rgba {
        r: 0x12,
        g: 0x34,
        b: 0x56,
        a: 255,
    };
    assert_eq!(color_to_string(c, true), "&H00563412&");
    assert_eq!(color_to_string(c, false), "&H563412&");
    // straight alpha 0 (transparent) -> ASS FF.
    let t = Rgba {
        r: 0xFF,
        g: 0,
        b: 0,
        a: 0,
    };
    assert_eq!(color_to_string(t, true), "&HFF0000FF&");
}

#[test]
fn roundtrip_plain_text() {
    assert_roundtrip("Hello world", &base());
}

#[test]
fn roundtrip_bold_italic() {
    assert_roundtrip("a{\\b1}b{\\i1}c{\\b0}d", &base());
}

#[test]
fn roundtrip_colors() {
    assert_roundtrip("{\\c&H0000FF&}red{\\3c&H00FF00&}grn", &base());
}

#[test]
fn roundtrip_alpha() {
    assert_roundtrip("{\\alpha&H80&}half{\\1a&H00&}full", &base());
}

#[test]
fn roundtrip_font_metrics() {
    assert_roundtrip("{\\fnTimes\\fs36\\fscx200\\fscy50\\fsp3}x", &base());
}

#[test]
fn roundtrip_rotation() {
    assert_roundtrip("{\\frx10\\fry20\\frz30}x", &base());
}

#[test]
fn roundtrip_border_shadow() {
    assert_roundtrip("{\\bord3\\shad2}x{\\xbord1\\ybord4}y", &base());
}

#[test]
fn roundtrip_blur() {
    assert_roundtrip("{\\be2\\blur3.5}x", &base());
}

#[test]
fn roundtrip_layout_tags() {
    assert_roundtrip("{\\an7\\pos(100,200)\\org(10,20)}x", &base());
    assert_roundtrip("{\\move(1,2,3,4,10,20)}x", &base());
    assert_roundtrip("{\\fad(200,300)}x", &base());
    assert_roundtrip("{\\clip(0,0,100,50)}x", &base());
    assert_roundtrip("{\\iclip(m 0 0 l 10 0 10 10)}x", &base());
}

#[test]
fn roundtrip_karaoke() {
    assert_roundtrip("{\\k50}ka{\\k30}ra{\\k20}o", &base());
}

#[test]
fn roundtrip_linebreaks() {
    assert_roundtrip("line one\\Nline two", &base());
    assert_roundtrip("hard\\hspace", &base());
}

#[test]
fn roundtrip_against_nondefault_base() {
    let mut b = base();
    b.bold = true;
    b.primary = Rgba {
        r: 0,
        g: 0xFF,
        b: 0,
        a: 255,
    };
    b.font_size = 40.0;
    // \b0 then bare \b restores base bold; \c reset restores base green.
    assert_roundtrip("{\\b0}x{\\b}y{\\c&H0000FF&}z{\\c}w", &b);
}

#[test]
fn roundtrip_combined_realistic_line() {
    let line = "{\\an8\\pos(640,50)}{\\c&H00FFFF&\\b1}Title{\\b0\\c&HFFFFFF&}: subtitle text\\Nsecond line";
    assert_roundtrip(line, &base());
}

#[test]
fn style_row_roundtrips_through_parse() {
    let row = "Style: Default,Arial,28,&H00FFFFFF,&H000000FF,&H00FF0000,&H80000000,-1,0,0,0,100,100,0,0,1,2,3,5,10,10,10,1";
    let (name, parsed) = parse_style_row(row, DEFAULT_V4PLUS_FORMAT, false).unwrap();
    let emitted = style_row_to_string(&name, &parsed, DEFAULT_V4PLUS_FORMAT, false);
    let (name2, reparsed) = parse_style_row(&emitted, DEFAULT_V4PLUS_FORMAT, false).unwrap();
    assert_eq!(name, name2);
    assert_eq!(parsed, reparsed);
}

#[test]
fn style_row_emit_then_resolve_matches() {
    // Emit a StyleBase to a row, reparse, and confirm resolution sees the
    // same base values.
    let mut b = base();
    b.font_name = "Verdana".into();
    b.font_size = 24.0;
    b.bold = true;
    b.primary = Rgba {
        r: 1,
        g: 2,
        b: 3,
        a: 200,
    };
    b.alignment = 5;
    let row = style_row_to_string("Mine", &b, DEFAULT_V4PLUS_FORMAT, false);
    let (_, reparsed) = parse_style_row(&row, DEFAULT_V4PLUS_FORMAT, false).unwrap();
    assert_eq!(b, reparsed);
}
