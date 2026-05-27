//! TTML parsing, writing, and round-trip.
//!
//! Targets the public `oxideav_subtitle::ttml` module — available once
//! the caller adds `pub mod ttml;` to `lib.rs`.

use oxideav_core::Segment;
use oxideav_subtitle::ttml;

const SAMPLE: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<tt xmlns=\"http://www.w3.org/ns/ttml\" xmlns:tts=\"http://www.w3.org/ns/ttml#styling\" xml:lang=\"en\">\n\
  <head>\n\
    <styling>\n\
      <style xml:id=\"s1\" tts:color=\"yellow\" tts:fontWeight=\"bold\"/>\n\
    </styling>\n\
  </head>\n\
  <body>\n\
    <div>\n\
      <p begin=\"00:00:01.000\" end=\"00:00:03.000\" style=\"s1\">Hello <span tts:color=\"#FF0000\">world</span></p>\n\
      <p begin=\"00:00:04.500\" end=\"00:00:06.000\">Line one<br/>Line two</p>\n\
    </div>\n\
  </body>\n\
</tt>\n";

#[test]
fn parses_two_cues() {
    let t = ttml::parse(SAMPLE.as_bytes()).unwrap();
    assert_eq!(t.cues.len(), 2);
    assert_eq!(t.cues[0].start_us, 1_000_000);
    assert_eq!(t.cues[0].end_us, 3_000_000);
    assert_eq!(t.cues[0].style_ref.as_deref(), Some("s1"));
    assert_eq!(t.cues[1].start_us, 4_500_000);
    assert_eq!(t.cues[1].end_us, 6_000_000);
}

#[test]
fn parses_named_style() {
    let t = ttml::parse(SAMPLE.as_bytes()).unwrap();
    assert_eq!(t.styles.len(), 1);
    assert_eq!(t.styles[0].name, "s1");
    assert!(t.styles[0].bold);
}

#[test]
fn preserves_inline_color_span() {
    let t = ttml::parse(SAMPLE.as_bytes()).unwrap();
    let segs = &t.cues[0].segments;
    let mut saw_color = false;
    visit(segs, &mut |s| {
        if let Segment::Color { rgb, .. } = s {
            if *rgb == (255, 0, 0) {
                saw_color = true;
            }
        }
    });
    assert!(saw_color, "expected #FF0000 color span");
}

#[test]
fn preserves_linebreak_in_second_cue() {
    let t = ttml::parse(SAMPLE.as_bytes()).unwrap();
    let segs = &t.cues[1].segments;
    let mut saw_br = false;
    visit(segs, &mut |s| {
        if matches!(s, Segment::LineBreak) {
            saw_br = true;
        }
    });
    assert!(saw_br, "expected LineBreak from <br/>");
}

#[test]
fn write_roundtrips_basic_shape() {
    let t = ttml::parse(SAMPLE.as_bytes()).unwrap();
    let out = ttml::write(&t);
    let s = String::from_utf8(out).unwrap();
    assert!(s.contains("<tt"));
    assert!(s.contains("<body>"));
    assert!(s.contains("begin=\"00:00:01.000\""));
    assert!(s.contains("begin=\"00:00:04.500\""));

    // Reparse the output and confirm timing fidelity.
    let t2 = ttml::parse(s.as_bytes()).unwrap();
    assert_eq!(t2.cues.len(), 2);
    for (a, b) in t.cues.iter().zip(t2.cues.iter()) {
        assert_eq!(a.start_us, b.start_us);
        assert_eq!(a.end_us, b.end_us);
    }
}

#[test]
fn probe_positive_and_negative() {
    assert!(ttml::probe(SAMPLE.as_bytes()) > 60);
    assert_eq!(ttml::probe(b"WEBVTT\n"), 0);
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

// ---------------------------------------------------------------------------
// IMSC1 §6 + §7 end-to-end integration.

const IMSC1_SAMPLE: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<tt xmlns=\"http://www.w3.org/ns/ttml\"\n\
    xmlns:tts=\"http://www.w3.org/ns/ttml#styling\"\n\
    xmlns:ttp=\"http://www.w3.org/ns/ttml#parameter\"\n\
    xmlns:ittp=\"http://www.w3.org/ns/ttml/profile/imsc1#parameter\"\n\
    xmlns:itts=\"http://www.w3.org/ns/ttml/profile/imsc1#styling\"\n\
    ttp:frameRate=\"25\" ttp:tickRate=\"10000000\" ttp:timeBase=\"media\"\n\
    ittp:aspectRatio=\"16 9\" xml:lang=\"en\">\n\
  <head>\n\
    <styling>\n\
      <style xml:id=\"sCenter\" tts:color=\"white\" tts:textAlign=\"center\" tts:displayAlign=\"after\" tts:lineHeight=\"120%\"/>\n\
    </styling>\n\
    <layout>\n\
      <region xml:id=\"bottom\" tts:origin=\"10% 80%\" tts:extent=\"80% 10%\" tts:displayAlign=\"after\" tts:textAlign=\"center\"/>\n\
      <region xml:id=\"top\" tts:origin=\"10% 10%\" tts:extent=\"80% 10%\" tts:displayAlign=\"before\"/>\n\
    </layout>\n\
  </head>\n\
  <body>\n\
    <div>\n\
      <p begin=\"00:00:01:05\" end=\"00:00:02:00\" style=\"sCenter\" region=\"bottom\">First line</p>\n\
      <p begin=\"50f\" end=\"75f\" region=\"top\">Second line</p>\n\
    </div>\n\
  </body>\n\
</tt>\n";

#[test]
fn full_imsc1_document_parses_and_round_trips() {
    let t = ttml::parse(IMSC1_SAMPLE.as_bytes()).unwrap();
    // Two cues — both timed in HH:MM:SS:FF / Nf forms against
    // ttp:frameRate="25" (so 00:00:01:05 = 1.2 s; 50f = 2.0 s).
    assert_eq!(t.cues.len(), 2);
    assert_eq!(t.cues[0].start_us, 1_200_000);
    assert_eq!(t.cues[0].end_us, 2_000_000);
    assert_eq!(t.cues[1].start_us, 2_000_000);
    assert_eq!(t.cues[1].end_us, 3_000_000);

    // Style: textAlign mapped to align.
    assert_eq!(t.styles[0].align, oxideav_core::TextAlign::Center);
    let extras = t
        .metadata
        .iter()
        .find(|(k, _)| k == "ttml_style_extra.sCenter")
        .map(|(_, v)| v.as_str())
        .expect("style extras carry displayAlign + lineHeight");
    assert!(extras.contains("tts:displayAlign=\"after\""));
    assert!(extras.contains("tts:lineHeight=\"120%\""));

    // Region table.
    assert!(t.metadata.iter().any(|(k, _)| k == "ttml_region.bottom"));
    assert!(t.metadata.iter().any(|(k, _)| k == "ttml_region.top"));

    // Per-cue region references.
    assert!(t
        .metadata
        .iter()
        .any(|(k, v)| k == "ttml_cue_region.0" && v == "bottom"));
    assert!(t
        .metadata
        .iter()
        .any(|(k, v)| k == "ttml_cue_region.1" && v == "top"));

    // Params captured.
    assert_eq!(
        t.metadata
            .iter()
            .find(|(k, _)| k == "ttml_param.frameRate")
            .map(|(_, v)| v.as_str()),
        Some("25")
    );

    // Round-trip: write → parse, all observables identical.
    let written = ttml::write(&t);
    let s = String::from_utf8(written).unwrap();
    assert!(s.contains("ttp:frameRate=\"25\""));
    assert!(s.contains("ittp:aspectRatio=\"16 9\""));
    assert!(s.contains("<layout>"));
    assert!(s.contains("<region xml:id=\"bottom\""));
    assert!(s.contains("<region xml:id=\"top\""));
    assert!(s.contains("region=\"bottom\""));
    assert!(s.contains("region=\"top\""));
    assert!(s.contains("tts:displayAlign=\"after\""));

    let t2 = ttml::parse(s.as_bytes()).unwrap();
    assert_eq!(t2.cues.len(), 2);
    for (a, b) in t.cues.iter().zip(t2.cues.iter()) {
        assert_eq!(a.start_us, b.start_us);
        assert_eq!(a.end_us, b.end_us);
    }
    assert!(t2
        .metadata
        .iter()
        .any(|(k, v)| k == "ttml_region.bottom" && v.contains("tts:origin=\"10% 80%\"")));
    assert_eq!(
        t2.metadata
            .iter()
            .find(|(k, _)| k == "ttml_param.frameRate")
            .map(|(_, v)| v.as_str()),
        Some("25")
    );
}

#[test]
fn imsc1_region_without_cue_ref_still_round_trips() {
    // Region defined but no <p region="..."> — should still write the
    // <layout> back out so authoring intent survives.
    let src = "<?xml version=\"1.0\"?>\n\
<tt xmlns=\"http://www.w3.org/ns/ttml\" xmlns:tts=\"http://www.w3.org/ns/ttml#styling\">\n\
  <head><layout>\n\
    <region xml:id=\"r1\" tts:origin=\"0% 0%\" tts:extent=\"100% 100%\"/>\n\
  </layout></head>\n\
  <body><div><p begin=\"0s\" end=\"1s\">x</p></div></body>\n\
</tt>";
    let t = ttml::parse(src.as_bytes()).unwrap();
    let s = String::from_utf8(ttml::write(&t)).unwrap();
    assert!(s.contains("<region xml:id=\"r1\""), "{}", s);
    assert!(s.contains("tts:origin=\"0% 0%\""));
}
