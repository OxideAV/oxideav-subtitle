//! WebVTT parsing, writing, and round-trip.

use oxideav_core::Segment;
use oxideav_subtitle::webvtt;

const SAMPLE: &str = "WEBVTT Language: en

STYLE
::cue(.yellow) {
  color: yellow;
  font-weight: bold;
}

STYLE
::cue(.blue) {
  color: blue;
  font-style: italic;
}

REGION
id:speaker
width:40%

00:00:01.000 --> 00:00:03.500 position:25% line:90% align:center
<v Alice>Hello <c.yellow>world</c></v>

cue-2
00:00:04.000 --> 00:00:05.500
<b>bold</b> then <i>italic</i>
second line
";

#[test]
fn parses_header_and_style() {
    let t = webvtt::parse(SAMPLE.as_bytes()).unwrap();
    // Header trailing stored in metadata.
    assert!(t
        .metadata
        .iter()
        .any(|(k, v)| k == "header" && v == "Language: en"));
    // Two style classes + one region.
    let yellow = t.styles.iter().find(|s| s.name == "yellow").unwrap();
    assert!(yellow.bold);
    let blue = t.styles.iter().find(|s| s.name == "blue").unwrap();
    assert!(blue.italic);
    assert!(t.styles.iter().any(|s| s.name == "region:speaker"));
}

#[test]
fn parses_voice_and_class() {
    let t = webvtt::parse(SAMPLE.as_bytes()).unwrap();
    let c0 = &t.cues[0];
    assert_eq!(c0.start_us, 1_000_000);
    assert_eq!(c0.end_us, 3_500_000);
    // Positioning present.
    let pos = c0.positioning.as_ref().unwrap();
    assert_eq!(pos.x, Some(25.0));
    assert_eq!(pos.y, Some(90.0));
    // Voice + Class in the segment tree.
    let mut saw_voice = false;
    let mut saw_class = false;
    visit(&c0.segments, &mut |s| match s {
        Segment::Voice { name, .. } if name == "Alice" => saw_voice = true,
        Segment::Class { name, .. } if name == "yellow" => saw_class = true,
        _ => {}
    });
    assert!(saw_voice);
    assert!(saw_class);
}

#[test]
fn parses_b_i_multiline() {
    let t = webvtt::parse(SAMPLE.as_bytes()).unwrap();
    let c1 = &t.cues[1];
    let mut saw_bold = false;
    let mut saw_italic = false;
    visit(&c1.segments, &mut |s| match s {
        Segment::Bold(_) => saw_bold = true,
        Segment::Italic(_) => saw_italic = true,
        _ => {}
    });
    assert!(saw_bold);
    assert!(saw_italic);
}

#[test]
fn write_roundtrips_signatures() {
    let t = webvtt::parse(SAMPLE.as_bytes()).unwrap();
    let out = String::from_utf8(webvtt::write(&t)).unwrap();
    assert!(out.starts_with("WEBVTT"));
    assert!(out.contains("00:00:01.000 --> 00:00:03.500"));
    assert!(out.contains("<v Alice>"));

    let t2 = webvtt::parse(out.as_bytes()).unwrap();
    assert_eq!(t2.cues.len(), t.cues.len());
    for (a, b) in t.cues.iter().zip(t2.cues.iter()) {
        assert_eq!(a.start_us, b.start_us);
        assert_eq!(a.end_us, b.end_us);
    }
}

#[test]
fn roundtrips_cue_settings_the_ir_cannot_model() {
    // WebVTT §3.5 settings beyond what `CuePosition` carries: the vertical
    // writing direction, the `line`/`position` alignment suffixes, and a
    // region reference. These survive a parse → write → parse cycle via the
    // per-cue `vtt_cue_extra.<idx>` metadata channel.
    let src = "WEBVTT\n\n\
        00:00:01.000 --> 00:00:02.000 vertical:lr line:75%,center position:40%,line-right align:end\n\
        vertical cue\n\n\
        00:00:03.000 --> 00:00:04.000 line:-2\n\
        bottom line\n";
    let t = oxideav_subtitle::webvtt::parse(src.as_bytes()).unwrap();
    let out = String::from_utf8(oxideav_subtitle::webvtt::write(&t)).unwrap();
    assert!(out.contains("vertical:lr"), "{out}");
    assert!(out.contains("line:75%,center"), "{out}");
    assert!(out.contains("position:40%,line-right"), "{out}");
    assert!(out.contains("align:end"), "{out}");
    // Bare line number stays a line number (no spurious `%`).
    assert!(
        out.contains("line:-2") && !out.contains("line:-2%"),
        "{out}"
    );

    // Re-parse keeps the structured offsets intact.
    let t2 = oxideav_subtitle::webvtt::parse(out.as_bytes()).unwrap();
    let p0 = t2.cues[0].positioning.as_ref().unwrap();
    assert_eq!(p0.x, Some(40.0));
    assert_eq!(p0.y, Some(75.0));
    assert_eq!(t2.cues[1].positioning.as_ref().unwrap().y, Some(-2.0));
}

#[test]
fn full_region_block_round_trips_through_synthesised_write() {
    // WebVTT §4.3 REGION settings (`lines` / `regionanchor` / `viewportanchor`
    // / `scroll`) the IR `SubtitleStyle` can't model are captured per-region
    // and rebuilt by the synthesised (no-extradata) writer.
    let src = "WEBVTT\n\n\
        REGION\n\
        id:bottom\n\
        width:60%\n\
        lines:3\n\
        regionanchor:0%,100%\n\
        viewportanchor:50%,90%\n\
        scroll:up\n\n\
        00:00:01.000 --> 00:00:02.000 region:bottom\n\
        hi\n";
    let mut t = webvtt::parse(src.as_bytes()).unwrap();
    assert!(t.styles.iter().any(|s| s.name == "region:bottom"));
    assert!(t.metadata.iter().any(|(k, _)| k == "vtt_region.bottom"));

    // Force the synthesised path: drop the verbatim extradata.
    t.extradata.clear();
    let out = String::from_utf8(webvtt::write(&t)).unwrap();
    for needle in [
        "REGION\n",
        "id:bottom\n",
        "width:60%\n",
        "lines:3\n",
        "regionanchor:0%,100%\n",
        "viewportanchor:50%,90%\n",
        "scroll:up\n",
    ] {
        assert!(out.contains(needle), "missing {needle:?} in:\n{out}");
    }

    // The rebuilt block re-parses identically.
    let t2 = webvtt::parse(out.as_bytes()).unwrap();
    let s2 = t2
        .metadata
        .iter()
        .find(|(k, _)| k == "vtt_region.bottom")
        .map(|(_, v)| v.as_str())
        .unwrap();
    assert_eq!(
        s2,
        "width:60% lines:3 regionanchor:0%,100% viewportanchor:50%,90% scroll:up"
    );
}

#[test]
fn cue_payload_inline_markup_round_trips_end_to_end() {
    // WebVTT §3.5 cue components: bold/italic/underline, voice with
    // annotation, class chain, language span with a BCP 47 tag, ruby with
    // both explicit and implicit `</rt>`, an inline timestamp, and a
    // multi-byte UTF-8 codepoint in the surrounding text. The whole
    // bundle must survive a parse → write → parse cycle byte-for-byte.
    let src = "WEBVTT\n\n\
        00:00:01.000 --> 00:00:05.000\n\
        <v Alice>Sur les <i><lang en>playground</lang></i>, ici à Montpellier — \
<c.warn.big>look<00:00:02.000>here</c></v>\n\n\
        00:00:06.000 --> 00:00:09.000\n\
        <ruby>漢<rt>kan</rt>字<rt>ji</rt></ruby> + implicit: \
<ruby>明<rt>みん</ruby>\n";
    let t = webvtt::parse(src.as_bytes()).unwrap();

    // Re-emit and re-parse — the second emit must equal the first.
    let out1 = String::from_utf8(webvtt::write(&t)).unwrap();
    let t2 = webvtt::parse(out1.as_bytes()).unwrap();
    let out2 = String::from_utf8(webvtt::write(&t2)).unwrap();
    assert_eq!(
        out1, out2,
        "second-cycle drift:\n=== out1 ===\n{out1}\n=== out2 ===\n{out2}"
    );

    // Spot-check the key markup pieces survived to the rendered output.
    for needle in [
        "<v Alice>",
        "<i><lang en>playground</lang></i>",
        "ici à Montpellier",
        "<c.warn.big>",
        "<00:00:02.000>",
        "<ruby>",
        "<rt>kan</rt>",
        "<rt>ji</rt>",
        // The implicit `</rt>` is normalised to explicit on re-emit.
        "<rt>みん</rt></ruby>",
    ] {
        assert!(out1.contains(needle), "missing {needle:?} in:\n{out1}");
    }
}

#[test]
fn cue_payload_language_span_with_bcp47_tag() {
    // BCP 47 tags often carry a subtag (e.g. `en-GB`, `zh-Hant`). The full
    // annotation including hyphens / digits must round-trip in the open tag.
    let src = "WEBVTT\n\n00:00:01.000 --> 00:00:02.000\n<lang zh-Hant-HK>漢字</lang>\n";
    let t = webvtt::parse(src.as_bytes()).unwrap();
    let out = String::from_utf8(webvtt::write(&t)).unwrap();
    assert!(out.contains("<lang zh-Hant-HK>"), "{out}");
    assert!(out.contains("</lang>"), "{out}");
    assert!(out.contains("漢字"), "{out}");
}

#[test]
fn style_block_full_property_round_trip_end_to_end() {
    // WebVTT §8.2.1 lists eleven properties that apply to the `::cue`
    // pseudo-element. Four land directly on `SubtitleStyle` (color,
    // font shorthand pieces, text-decoration, background); the other seven
    // ride the per-style metadata channel (`vtt_style.<name>.<prop>`) in
    // canonical spec order. Both halves must survive a parse → write →
    // parse cycle.
    let src = "\
WEBVTT

STYLE
::cue {
  color: white;
  background-color: black;
}

STYLE
::cue(#warn) {
  color: red;
  opacity: 0.9;
}

STYLE
::cue(b) {
  font-weight: bold;
}

STYLE
::cue(.fancy) {
  color: white;
  background-color: rgb(16, 32, 48);
  font-family: \"DejaVu Sans\";
  font-size: 24px;
  font-weight: bold;
  font-style: italic;
  text-decoration: underline line-through;
  opacity: 0.75;
  visibility: visible;
  text-shadow: 1px 1px 2px black;
  outline: 2px solid red;
  white-space: pre-wrap;
  text-combine-upright: all;
  ruby-position: over;
  line-height: 1.4;
}

warn
00:00:01.000 --> 00:00:02.000
<c.fancy>hi</c>
";
    let t1 = webvtt::parse(src.as_bytes()).unwrap();

    // Bare ::cue, ::cue(#id), ::cue(elem), and ::cue(.class) all surface as
    // distinct styles.
    assert!(t1.styles.iter().any(|s| s.name == "::cue"));
    assert!(t1.styles.iter().any(|s| s.name == "#warn"));
    assert!(t1.styles.iter().any(|s| s.name == "::cue(b)"));
    let fancy = t1.style("fancy").expect("fancy style");
    // IR fields populated.
    assert_eq!(fancy.primary_color.unwrap().0, 255);
    assert_eq!(fancy.back_color, Some((16, 32, 48, 255)));
    assert_eq!(fancy.font_family.as_deref(), Some("DejaVu Sans"));
    assert_eq!(fancy.font_size, Some(24.0));
    assert!(fancy.bold && fancy.italic && fancy.underline && fancy.strike);
    // All seven extras captured.
    for prop in [
        "opacity",
        "visibility",
        "text-shadow",
        "outline",
        "white-space",
        "text-combine-upright",
        "ruby-position",
        "line-height",
    ] {
        let key = format!("vtt_style.fancy.{prop}");
        assert!(
            t1.metadata.iter().any(|(k, _)| k == &key),
            "missing extra {key} (got: {:?})",
            t1.metadata
                .iter()
                .filter(|(k, _)| k.starts_with("vtt_style."))
                .collect::<Vec<_>>()
        );
    }

    // Force the synthesised write path and confirm the rebuilt block
    // re-parses byte-stable. Drift in the second cycle is the canonical
    // round-trip-fidelity signal.
    let mut t1_synth = t1.clone();
    t1_synth.extradata.clear();
    let out1 = String::from_utf8(webvtt::write(&t1_synth)).unwrap();
    let mut t2 = webvtt::parse(out1.as_bytes()).unwrap();
    t2.extradata.clear();
    let out2 = String::from_utf8(webvtt::write(&t2)).unwrap();
    assert_eq!(
        out1, out2,
        "second-cycle drift:\n=== out1 ===\n{out1}\n=== out2 ===\n{out2}"
    );
}

#[test]
fn note_comment_blocks_captured_and_round_trip() {
    // WebVTT §4.1 comment blocks (`NOTE …`) are formally ignored by the
    // parser but a faithful round-trip must preserve them — they carry
    // author notes that should survive a parse → write cycle. We capture
    // each block verbatim into `vtt_note.<idx>` metadata and remember
    // which cue it preceded via `vtt_note_pos.<idx>`. The §1.5 example
    // exercises the three placement positions: before any cue, between
    // cues, and trailing after the last cue. A multi-line NOTE body must
    // also survive intact.
    let src = "WEBVTT\n\n\
        NOTE\n\
        This file was written by Jill. I hope\n\
        you enjoy reading it.\n\n\
        00:00:01.000 --> 00:00:04.000\n\
        Never drink liquid nitrogen.\n\n\
        NOTE check next cue\n\n\
        00:00:05.000 --> 00:00:09.000\n\
        — It will perforate your stomach.\n\
        — You could die.\n\n\
        NOTE end of file\n";
    let t = webvtt::parse(src.as_bytes()).unwrap();
    assert_eq!(t.cues.len(), 2);

    // Three NOTE blocks captured with their positions.
    let note_body = |i: usize| {
        t.metadata
            .iter()
            .find(|(k, _)| k == &format!("vtt_note.{i}"))
            .map(|(_, v)| v.as_str())
            .unwrap_or_else(|| panic!("missing vtt_note.{i}"))
    };
    let note_pos = |i: usize| {
        t.metadata
            .iter()
            .find(|(k, _)| k == &format!("vtt_note_pos.{i}"))
            .map(|(_, v)| v.as_str())
            .unwrap_or_else(|| panic!("missing vtt_note_pos.{i}"))
    };
    // First note: multi-line body, precedes cue 0.
    assert_eq!(
        note_body(0),
        "NOTE\nThis file was written by Jill. I hope\nyou enjoy reading it."
    );
    assert_eq!(note_pos(0), "0");
    // Second note: single-line body, precedes cue 1.
    assert_eq!(note_body(1), "NOTE check next cue");
    assert_eq!(note_pos(1), "1");
    // Third note: trails the final cue.
    assert_eq!(note_body(2), "NOTE end of file");
    assert_eq!(note_pos(2), "2");

    // The extradata path preserves NOTE bodies verbatim.
    let out_extra = String::from_utf8(webvtt::write(&t)).unwrap();
    assert!(out_extra.contains("NOTE\nThis file was written"));
    assert!(out_extra.contains("NOTE check next cue"));
    assert!(out_extra.contains("NOTE end of file"));

    // The synth path rebuilds the same NOTE interleaving from metadata.
    let mut t_synth = t.clone();
    t_synth.extradata.clear();
    let out_synth = String::from_utf8(webvtt::write(&t_synth)).unwrap();
    assert!(out_synth.contains("NOTE\nThis file was written"));
    assert!(out_synth.contains("NOTE check next cue"));
    assert!(out_synth.contains("NOTE end of file"));
    // Position-relative ordering preserved: the "check next cue" NOTE
    // sits between the two cue timing lines, not after both.
    let idx_first_cue = out_synth.find("00:00:01.000").unwrap();
    let idx_mid_note = out_synth.find("NOTE check next cue").unwrap();
    let idx_second_cue = out_synth.find("00:00:05.000").unwrap();
    let idx_end_note = out_synth.find("NOTE end of file").unwrap();
    assert!(idx_first_cue < idx_mid_note);
    assert!(idx_mid_note < idx_second_cue);
    assert!(idx_second_cue < idx_end_note);

    // The synth output re-parses to the same NOTE metadata.
    let t2 = webvtt::parse(out_synth.as_bytes()).unwrap();
    assert_eq!(
        t2.metadata
            .iter()
            .filter(|(k, _)| k.starts_with("vtt_note."))
            .count(),
        3
    );
}

#[test]
fn note_block_with_arrow_in_body_does_not_swallow_following_cue() {
    // §4.1 forbids the substring `-->` inside a NOTE body. We don't
    // enforce that on input (we capture whatever the author wrote), but
    // we must not let a leading `NOTE` swallow the following cue's
    // timing line: each block was already split on blank lines at parse
    // time, so a NOTE-prefixed block ends at the same blank line as
    // anything else.
    let src = "WEBVTT\n\n\
        NOTE just a heads-up\n\n\
        00:00:01.000 --> 00:00:02.000\n\
        hello\n";
    let t = webvtt::parse(src.as_bytes()).unwrap();
    assert_eq!(t.cues.len(), 1);
    assert_eq!(t.cues[0].start_us, 1_000_000);
    assert!(t.metadata.iter().any(|(k, _)| k == "vtt_note.0"));
}

#[test]
fn lowercased_note_prefix_is_not_a_comment_block() {
    // §4.1 NOTE token is case-sensitive. A block whose first line begins
    // with lowercase `note` (or e.g. `Notebook`) is not a comment block;
    // it falls through to the cue-block code path where the absence of a
    // timing line skips it harmlessly. Crucially, it must not be
    // captured as `vtt_note.<idx>` metadata.
    let src = "WEBVTT\n\n\
        Notebook\n00:00:01.000 --> 00:00:02.000\nhi\n";
    let t = webvtt::parse(src.as_bytes()).unwrap();
    assert!(!t.metadata.iter().any(|(k, _)| k.starts_with("vtt_note.")));
    assert_eq!(t.cues.len(), 1);
}

#[test]
fn strict_signature_and_timestamp_validation_end_to_end() {
    // §4.1 file signature: missing SPACE/TAB separator before header
    // trailing text means the parser must reject — the previous lenient
    // implementation silently accepted `WEBVTTHEADER` and dropped the
    // "HEADER" suffix into trailing-text metadata.
    let no_sep = b"WEBVTTHEADER\n\n00:00:01.000 --> 00:00:02.000\nhi\n";
    assert!(webvtt::parse(no_sep).is_err());

    // The valid SPACE / TAB / empty separator cases all parse cleanly.
    for sep in [" Lang: en", "\tLang: en", ""] {
        let mut src = String::from("WEBVTT");
        src.push_str(sep);
        src.push_str("\n\n00:00:01.000 --> 00:00:02.000\nhi\n");
        let t = webvtt::parse(src.as_bytes())
            .unwrap_or_else(|e| panic!("separator {sep:?} should parse: {e:?}"));
        assert_eq!(t.cues.len(), 1, "separator {sep:?}");
    }

    // §3.3 timestamp: only the canonical MM:SS.fff or HH:MM:SS.fff shape
    // is accepted. Non-canonical timings make the cue block fail to match
    // a timing line and the parser drops the cue rather than silently
    // mis-interpreting the offset.
    for bad in [
        "0:00:01.000 --> 00:00:02.000",  // 1-digit hours
        "00:0:01.000 --> 00:00:02.000",  // 1-digit minutes
        "00:00:1.000 --> 00:00:02.000",  // 1-digit seconds
        "00:00:01 --> 00:00:02",         // missing fraction
        "00:00:01.00 --> 00:00:02.00",   // 2-digit fraction
        "00:60:01.000 --> 00:60:02.000", // minutes > 59
        "00:00:60.000 --> 00:00:61.000", // seconds > 59
    ] {
        let src = format!("WEBVTT\n\n{bad}\nhi\n");
        let t = webvtt::parse(src.as_bytes()).unwrap();
        assert_eq!(t.cues.len(), 0, "should reject {bad:?}");
    }

    // The two shapes the spec accepts both parse, and the offsets are
    // computed correctly.
    let src = "WEBVTT\n\n01:30.500 --> 02:00.000\nshort\n\n00:00:03.250 --> 00:00:04.750\nlong\n";
    let t = webvtt::parse(src.as_bytes()).unwrap();
    assert_eq!(t.cues.len(), 2);
    assert_eq!(t.cues[0].start_us, 90_500_000);
    assert_eq!(t.cues[0].end_us, 120_000_000);
    assert_eq!(t.cues[1].start_us, 3_250_000);
    assert_eq!(t.cues[1].end_us, 4_750_000);
}

fn visit<F: FnMut(&Segment)>(segs: &[Segment], f: &mut F) {
    for s in segs {
        f(s);
        match s {
            Segment::Bold(c) | Segment::Italic(c) | Segment::Underline(c) | Segment::Strike(c) => {
                visit(c, f)
            }
            Segment::Color { children, .. }
            | Segment::Font { children, .. }
            | Segment::Voice { children, .. }
            | Segment::Class { children, .. }
            | Segment::Karaoke { children, .. } => visit(children, f),
            _ => {}
        }
    }
}

/// WebVTT §5 worked example: `<c.yellow.bg_blue.magenta.bg_black>` renders
/// as "magenta text on a black background" because the spec's cascade rule
/// (§5.2 closing paragraph: "the order of appearance determines the
/// cascade") picks the last matching class within each presentational
/// target. The resolver consumes the dot-chain the parser already places on
/// `Segment::Class::name`, so the hop from "parsed cue body" to "effective
/// presentational hint" is one function call.
#[test]
fn default_cue_component_classes_5_resolve_through_class_segment_name() {
    let src = "WEBVTT

00:00:01.000 --> 00:00:02.000
<c.yellow.bg_blue.magenta.bg_black>cascade winner</c>

00:00:03.000 --> 00:00:04.000
<c.warning.red.bg_lime>mixed author + default</c>

00:00:05.000 --> 00:00:06.000
<c.foo.bar>author-only chain</c>
";
    let t = webvtt::parse(src.as_bytes()).unwrap();
    assert_eq!(t.cues.len(), 3);

    // Cue 1 — §5 worked example: magenta over bg_black wins the cascade.
    let chain1 = match &t.cues[0].segments[0] {
        Segment::Class { name, .. } => name.clone(),
        other => panic!("expected Class, got {other:?}"),
    };
    assert_eq!(
        webvtt::resolve_default_class_colors(&chain1),
        (Some((255, 0, 255, 0xff)), Some((0, 0, 0, 0xff))),
    );

    // Cue 2 — author class `warning` doesn't shadow §5 `red` / `bg_lime`.
    let chain2 = match &t.cues[1].segments[0] {
        Segment::Class { name, .. } => name.clone(),
        other => panic!("expected Class, got {other:?}"),
    };
    assert_eq!(
        webvtt::resolve_default_class_colors(&chain2),
        (Some((255, 0, 0, 0xff)), Some((0, 255, 0, 0xff))),
    );

    // Cue 3 — chain has no §5 classes, both slots empty. Caller defers to
    // author-supplied `::cue(.foo)` / `::cue(.bar)` STYLE rules.
    let chain3 = match &t.cues[2].segments[0] {
        Segment::Class { name, .. } => name.clone(),
        other => panic!("expected Class, got {other:?}"),
    };
    assert_eq!(webvtt::resolve_default_class_colors(&chain3), (None, None),);

    // The per-name single-class resolver agrees with the spec table for
    // both presentational targets and is case-sensitive.
    let (kind, rgba) = webvtt::default_class_color("cyan").unwrap();
    assert_eq!(kind, webvtt::DefaultClassKind::Foreground);
    assert_eq!(rgba, (0, 255, 255, 0xff));
    let (kind, rgba) = webvtt::default_class_color("bg_yellow").unwrap();
    assert_eq!(kind, webvtt::DefaultClassKind::Background);
    assert_eq!(rgba, (255, 255, 0, 0xff));
    assert!(webvtt::default_class_color("Cyan").is_none());
    assert!(webvtt::default_class_color("BG_YELLOW").is_none());
}
