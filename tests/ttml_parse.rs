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

// ---------------------------------------------------------------------------
// TTML2 §8.1.5 — inline `tts:*` styling attributes on `<p>` content
// elements. Modelled attrs wrap the cue's content with the equivalent
// IR segment; IR-unmodelled attrs ride the `ttml_p_extra.<idx>`
// track-metadata channel and round-trip on the `<p>`.

#[test]
fn inline_p_styling_modelled_attrs_wrap_segments() {
    // tts:fontWeight + tts:color directly on <p>, no <span>.
    let src = "<?xml version=\"1.0\"?>\n\
<tt xmlns=\"http://www.w3.org/ns/ttml\" xmlns:tts=\"http://www.w3.org/ns/ttml#styling\">\n\
  <body><div>\n\
    <p begin=\"0s\" end=\"1s\" tts:fontWeight=\"bold\" tts:color=\"#00FF00\">hello</p>\n\
  </div></body>\n\
</tt>";
    let t = ttml::parse(src.as_bytes()).unwrap();
    assert_eq!(t.cues.len(), 1);
    // The cue's segments should be wrapped Bold > Color > "hello"
    // (or Color > Bold > "hello"; we only verify both shapes are seen).
    let mut saw_bold = false;
    let mut saw_color = false;
    visit(&t.cues[0].segments, &mut |s| match s {
        Segment::Bold(_) => saw_bold = true,
        Segment::Color { rgb, .. } if *rgb == (0, 255, 0) => saw_color = true,
        _ => {}
    });
    assert!(saw_bold, "inline tts:fontWeight=\"bold\" must wrap content");
    assert!(saw_color, "inline tts:color must wrap content");
}

#[test]
fn inline_p_styling_unmodelled_attrs_ride_extras() {
    // tts:textAlign + tts:lineHeight + tts:opacity on <p>. Of these
    // `textAlign` has an IR home on SubtitleStyle but not on a per-cue
    // basis — at the cue level it has no Segment mapping, so it (and the
    // other two IR-unmodelled attrs) ride the per-cue extras channel.
    let src = "<?xml version=\"1.0\"?>\n\
<tt xmlns=\"http://www.w3.org/ns/ttml\" xmlns:tts=\"http://www.w3.org/ns/ttml#styling\">\n\
  <body><div>\n\
    <p begin=\"0s\" end=\"1s\" tts:textAlign=\"center\" tts:lineHeight=\"125%\" tts:opacity=\"0.8\">x</p>\n\
  </div></body>\n\
</tt>";
    let t = ttml::parse(src.as_bytes()).unwrap();
    let extras = t
        .metadata
        .iter()
        .find(|(k, _)| k == "ttml_p_extra.0")
        .map(|(_, v)| v.as_str())
        .expect("inline IR-unmodelled tts:* attrs on <p> ride ttml_p_extra.0");
    // Canonical order: textAlign before lineHeight before opacity.
    let i_ta = extras.find("tts:textAlign").unwrap();
    let i_lh = extras.find("tts:lineHeight").unwrap();
    let i_op = extras.find("tts:opacity").unwrap();
    assert!(i_ta < i_lh && i_lh < i_op, "canonical order: {}", extras);
    assert!(extras.contains("tts:textAlign=\"center\""));
    assert!(extras.contains("tts:lineHeight=\"125%\""));
    assert!(extras.contains("tts:opacity=\"0.8\""));
}

#[test]
fn inline_p_styling_round_trips() {
    // Mix of modelled + unmodelled inline tts:* on <p>. After
    // parse → write → parse, the cue still has the same Bold wrapper
    // and the same extras list.
    let src = "<?xml version=\"1.0\"?>\n\
<tt xmlns=\"http://www.w3.org/ns/ttml\" xmlns:tts=\"http://www.w3.org/ns/ttml#styling\">\n\
  <body><div>\n\
    <p begin=\"0s\" end=\"1s\" tts:fontStyle=\"italic\" tts:displayAlign=\"after\">round</p>\n\
  </div></body>\n\
</tt>";
    let t = ttml::parse(src.as_bytes()).unwrap();
    // Parse-side: Italic wrapper + ttml_p_extra.0 with displayAlign.
    let mut saw_italic = false;
    visit(&t.cues[0].segments, &mut |s| {
        if matches!(s, Segment::Italic(_)) {
            saw_italic = true;
        }
    });
    assert!(saw_italic, "inline tts:fontStyle=\"italic\" wraps content");
    assert!(t
        .metadata
        .iter()
        .any(|(k, v)| k == "ttml_p_extra.0" && v.contains("tts:displayAlign=\"after\"")));

    // Write: the <p> regrows both the inline displayAlign extra and
    // the writer emits `<span tts:fontStyle="italic">` from the
    // Segment::Italic wrapper.
    let written = ttml::write(&t);
    let s = String::from_utf8(written).unwrap();
    assert!(s.contains("tts:displayAlign=\"after\""), "{}", s);
    assert!(s.contains("<span tts:fontStyle=\"italic\""), "{}", s);

    // Re-parse the output and verify the same shape survives.
    let t2 = ttml::parse(s.as_bytes()).unwrap();
    assert_eq!(t2.cues.len(), 1);
    let mut saw_italic2 = false;
    visit(&t2.cues[0].segments, &mut |s| {
        if matches!(s, Segment::Italic(_)) {
            saw_italic2 = true;
        }
    });
    assert!(saw_italic2, "italic survives round-trip");
    assert!(t2
        .metadata
        .iter()
        .any(|(k, v)| k == "ttml_p_extra.0" && v.contains("tts:displayAlign=\"after\"")));
}

#[test]
fn inline_p_styling_textalign_justify_carried_too() {
    // `justify` is the textAlign value with no IR home; it must ride
    // the extras the same way the named-style path treats it.
    let src = "<?xml version=\"1.0\"?>\n\
<tt xmlns=\"http://www.w3.org/ns/ttml\" xmlns:tts=\"http://www.w3.org/ns/ttml#styling\">\n\
  <body><div>\n\
    <p begin=\"0s\" end=\"1s\" tts:textAlign=\"justify\">x</p>\n\
  </div></body>\n\
</tt>";
    let t = ttml::parse(src.as_bytes()).unwrap();
    assert!(t
        .metadata
        .iter()
        .any(|(k, v)| k == "ttml_p_extra.0" && v.contains("tts:textAlign=\"justify\"")));
}

#[test]
fn p_without_inline_styling_does_not_emit_extras_key() {
    // Negative case — a plain <p> must NOT push a stray `ttml_p_extra.<idx>`.
    let t = ttml::parse(SAMPLE.as_bytes()).unwrap();
    assert!(!t
        .metadata
        .iter()
        .any(|(k, _)| k.starts_with("ttml_p_extra.")));
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
