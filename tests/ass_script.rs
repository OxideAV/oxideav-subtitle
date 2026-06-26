//! Whole-document ASS / SSA parser tests.

use oxideav_core::Segment;
use oxideav_subtitle::ass_script::{parse, segments_to_ass_text, write};
use oxideav_subtitle::ass_script_info::script_info;
use oxideav_subtitle::ir::{plain_text, SourceFormat};

const SAMPLE: &str = "\
[Script Info]
; a leading comment line
Title: My Subs
ScriptType: v4.00+
PlayResX: 1280
PlayResY: 720
WrapStyle: 0

[V4+ Styles]
Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding
Style: Default,Arial,48,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,2,0,2,10,10,10,1
Style: Title,Verdana,72,&H0000FFFF,&H000000FF,&H00000000,&H00000000,-1,0,0,0,100,100,0,0,1,3,0,8,10,10,10,1

[Events]
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text
Dialogue: 0,0:00:01.00,0:00:03.50,Default,,0,0,0,,Hello {\\i1}world{\\i0}!
Comment: 0,0:00:04.00,0:00:05.00,Default,,0,0,0,,not shown
Dialogue: 0,0:00:06.00,0:00:08.00,Title,,0,0,0,,{\\b1}Big Title
";

#[test]
fn parses_source_format() {
    let track = parse(SAMPLE.as_bytes());
    assert_eq!(track.source, Some(SourceFormat::AssOrSsa));
}

#[test]
fn parses_script_info_metadata() {
    let track = parse(SAMPLE.as_bytes());
    let info = script_info(&track);
    assert_eq!(info.title.as_deref(), Some("My Subs"));
    assert_eq!(info.script_type.as_deref(), Some("v4.00+"));
    assert_eq!(info.play_res_x, Some(1280));
    assert_eq!(info.play_res_y, Some(720));
}

#[test]
fn parses_styles() {
    let track = parse(SAMPLE.as_bytes());
    assert_eq!(track.styles.len(), 2);
    let def = track.style("Default").unwrap();
    assert_eq!(def.font_family.as_deref(), Some("Arial"));
    assert_eq!(def.font_size, Some(48.0));
    assert!(!def.bold);
    let title = track.style("Title").unwrap();
    assert_eq!(title.font_family.as_deref(), Some("Verdana"));
    assert!(title.bold);
    // Title primary &H0000FFFF -> b=00 g=FF r=FF -> yellow
    assert_eq!(title.primary_color, Some((0xFF, 0xFF, 0, 255)));
}

#[test]
fn parses_dialogue_cues_skips_comment() {
    let track = parse(SAMPLE.as_bytes());
    // Two Dialogue lines; the Comment is skipped.
    assert_eq!(track.cues.len(), 2);
    let c0 = &track.cues[0];
    assert_eq!(c0.start_us, 1_000_000); // 1.00 s
    assert_eq!(c0.end_us, 3_500_000); // 3.50 s
    assert_eq!(c0.style_ref.as_deref(), Some("Default"));
    // plain text drops the override markup
    assert_eq!(plain_text(&c0.segments), "Hello world!");
}

#[test]
fn cue_segments_carry_italic() {
    let track = parse(SAMPLE.as_bytes());
    let c0 = &track.cues[0];
    // Somewhere in the segment tree there is an Italic node over "world".
    let mut found = false;
    fn walk(segs: &[Segment], found: &mut bool) {
        for s in segs {
            match s {
                Segment::Italic(children) => {
                    *found = true;
                    walk(children, found);
                }
                Segment::Bold(c) | Segment::Underline(c) | Segment::Strike(c) => walk(c, found),
                Segment::Color { children, .. } | Segment::Karaoke { children, .. } => {
                    walk(children, found)
                }
                _ => {}
            }
        }
    }
    walk(&c0.segments, &mut found);
    assert!(found, "expected an Italic segment over 'world'");
}

#[test]
fn cue_segments_carry_bold_from_title() {
    let track = parse(SAMPLE.as_bytes());
    let c1 = &track.cues[1];
    assert_eq!(c1.style_ref.as_deref(), Some("Title"));
    assert_eq!(plain_text(&c1.segments), "Big Title");
    // The Title style is bold AND the line opens \b1, so the run is bold.
    assert!(matches!(c1.segments.first(), Some(Segment::Bold(_))));
}

#[test]
fn empty_input_yields_empty_track() {
    let track = parse(b"");
    assert!(track.cues.is_empty());
    assert!(track.styles.is_empty());
}

#[test]
fn ssa_v4_legacy_section() {
    let ssa = "\
[Script Info]
Title: Legacy

[V4 Styles]
Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, TertiaryColour, BackColour, Bold, Italic, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, AlphaLevel, Encoding
Style: Def,Arial,24,16777215,255,0,0,0,0,1,2,0,2,10,10,10,0,0

[Events]
Format: Marked, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text
Dialogue: Marked=0,0:00:00.50,0:00:02.00,Def,,0,0,0,,classic line
";
    let track = parse(ssa.as_bytes());
    assert_eq!(track.styles.len(), 1);
    // 16777215 = 0xFFFFFF -> white primary
    assert_eq!(
        track.style("Def").unwrap().primary_color,
        Some((0xFF, 0xFF, 0xFF, 255))
    );
    assert_eq!(track.cues.len(), 1);
    assert_eq!(track.cues[0].start_us, 500_000);
    assert_eq!(plain_text(&track.cues[0].segments), "classic line");
}

#[test]
fn header_less_events_use_default_order() {
    // No Format: line in [Events] -> the canonical V4+ order is assumed.
    let doc = "\
[Events]
Dialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,no header
";
    let track = parse(doc.as_bytes());
    assert_eq!(track.cues.len(), 1);
    assert_eq!(plain_text(&track.cues[0].segments), "no header");
}

#[test]
fn segments_to_ass_text_emits_overrides() {
    let segs = vec![
        Segment::Text("Hello ".into()),
        Segment::Bold(vec![Segment::Italic(vec![Segment::Text("world".into())])]),
        Segment::Text("!".into()),
    ];
    let text = segments_to_ass_text(&segs);
    // bold wraps italic wraps "world".
    assert_eq!(text, "Hello {\\b1}{\\i1}world{\\i0}{\\b0}!");
}

#[test]
fn segments_color_and_linebreak() {
    let segs = vec![
        Segment::Color {
            rgb: (0xFF, 0, 0),
            children: vec![Segment::Text("red".into())],
        },
        Segment::LineBreak,
        Segment::Text("next".into()),
    ];
    // \c emits BGR: r=FF -> &H0000FF&
    assert_eq!(segments_to_ass_text(&segs), "{\\c&H0000FF&}red{\\c}\\Nnext");
}

/// Whole-file round-trip: parse -> write -> re-parse preserves metadata,
/// styles, and cue timing + plain text.
#[test]
fn whole_file_roundtrips() {
    let first = parse(SAMPLE.as_bytes());
    let bytes = write(&first);
    let second = parse(&bytes);

    // Metadata keys survive (compare as sets of (key, value)).
    let mut m1 = first.metadata.clone();
    let mut m2 = second.metadata.clone();
    m1.sort();
    m2.sort();
    assert_eq!(
        m1,
        m2,
        "metadata round-trip\n{}",
        String::from_utf8_lossy(&bytes)
    );

    // Style count + names + key fields survive.
    assert_eq!(first.styles.len(), second.styles.len());
    for (a, b) in first.styles.iter().zip(&second.styles) {
        assert_eq!(a.name, b.name);
        assert_eq!(a.font_family, b.font_family);
        assert_eq!(a.font_size, b.font_size);
        assert_eq!(a.bold, b.bold);
        assert_eq!(a.primary_color, b.primary_color);
    }

    // Cue count + timing + plain text + style ref survive.
    assert_eq!(first.cues.len(), second.cues.len());
    for (a, b) in first.cues.iter().zip(&second.cues) {
        assert_eq!(a.start_us, b.start_us);
        assert_eq!(a.end_us, b.end_us);
        assert_eq!(a.style_ref, b.style_ref);
        assert_eq!(plain_text(&a.segments), plain_text(&b.segments));
    }
}

#[test]
fn write_produces_parseable_sections() {
    let track = parse(SAMPLE.as_bytes());
    let text = String::from_utf8(write(&track)).unwrap();
    assert!(text.contains("[Script Info]"));
    assert!(text.contains("[V4+ Styles]"));
    assert!(text.contains("[Events]"));
    assert!(text.contains("Format: "));
    assert!(text.contains("Dialogue: "));
}
