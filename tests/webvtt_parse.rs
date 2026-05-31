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
