//! EBU STL binary parser + writer round-trip.
//!
//! Targets the public `oxideav_subtitle::ebu_stl` module — available
//! once the caller adds `pub mod ebu_stl;` to `lib.rs`.

use oxideav_core::Segment;
use oxideav_subtitle::ebu_stl::{self, GSI_SIZE, TTI_SIZE};

/// Build a synthetic STL buffer with a GSI block and two TTI records.
fn synthetic() -> Vec<u8> {
    let mut out = vec![0x20u8; GSI_SIZE];
    // CPN = "850", DFC = "STL25.01", DSC = '1', CCT/LC = "00"/"00".
    out[0..3].copy_from_slice(b"850");
    out[3..11].copy_from_slice(b"STL25.01");
    out[11] = b'1';
    out[12..14].copy_from_slice(b"00");
    out[14..16].copy_from_slice(b"00");
    // TNB = 2, TNS = 2, MNC = 40, MNR = 23, TCS = '1'.
    out[238..243].copy_from_slice(b"00002");
    out[243..248].copy_from_slice(b"00002");
    out[248..250].copy_from_slice(b"40");
    out[250..252].copy_from_slice(b"23");
    out[252] = b'1';

    for (idx, (tci, tco, text)) in [
        ((0u8, 0u8, 1u8, 0u8), (0u8, 0u8, 3u8, 0u8), "Hello world"),
        ((0u8, 0u8, 5u8, 0u8), (0u8, 0u8, 7u8, 0u8), "Second line"),
    ]
    .iter()
    .enumerate()
    {
        let mut tti = [0u8; TTI_SIZE];
        let sn = (idx as u16).to_le_bytes();
        tti[1] = sn[0];
        tti[2] = sn[1];
        tti[3] = 0xFF; // EBN = last
        tti[5] = tci.0;
        tti[6] = tci.1;
        tti[7] = tci.2;
        tti[8] = tci.3;
        tti[9] = tco.0;
        tti[10] = tco.1;
        tti[11] = tco.2;
        tti[12] = tco.3;
        tti[13] = 22;
        tti[14] = 0x02;
        let text_bytes = text.as_bytes();
        tti[16..16 + text_bytes.len()].copy_from_slice(text_bytes);
        for b in &mut tti[16 + text_bytes.len()..128] {
            *b = 0x8F;
        }
        out.extend_from_slice(&tti);
    }
    out
}

#[test]
fn parses_two_cues_from_gsi_plus_tti() {
    let buf = synthetic();
    let t = ebu_stl::parse(&buf).unwrap();
    assert_eq!(t.cues.len(), 2);
    assert_eq!(t.cues[0].start_us, 1_000_000);
    assert_eq!(t.cues[0].end_us, 3_000_000);
    assert_eq!(t.cues[1].start_us, 5_000_000);
    assert_eq!(t.cues[1].end_us, 7_000_000);
}

#[test]
fn keeps_text_fields_intact() {
    let t = ebu_stl::parse(&synthetic()).unwrap();
    match &t.cues[0].segments[0] {
        Segment::Text(s) => assert_eq!(s, "Hello world"),
        other => panic!("expected text, got {other:?}"),
    }
    match &t.cues[1].segments[0] {
        Segment::Text(s) => assert_eq!(s, "Second line"),
        other => panic!("expected text, got {other:?}"),
    }
}

#[test]
fn gsi_metadata_preserved() {
    let t = ebu_stl::parse(&synthetic()).unwrap();
    let get = |k: &str| {
        t.metadata
            .iter()
            .find(|(kk, _)| kk == k)
            .map(|(_, v)| v.clone())
    };
    assert_eq!(get("dfc").as_deref(), Some("STL25.01"));
    assert_eq!(get("cpn").as_deref(), Some("850"));
    assert_eq!(get("mnc").as_deref(), Some("40"));
    assert_eq!(get("mnr").as_deref(), Some("23"));
}

#[test]
fn write_roundtrips_to_same_cue_count() {
    let buf = synthetic();
    let t = ebu_stl::parse(&buf).unwrap();
    let out = ebu_stl::write(&t).unwrap();
    assert_eq!(out.len(), GSI_SIZE + 2 * TTI_SIZE);
    let t2 = ebu_stl::parse(&out).unwrap();
    assert_eq!(t2.cues.len(), 2);
    assert_eq!(t2.cues[0].start_us, 1_000_000);
    assert_eq!(t2.cues[0].end_us, 3_000_000);
    assert_eq!(t2.cues[1].start_us, 5_000_000);
}

#[test]
fn italic_wraps_in_tf_control_bytes() {
    // Craft a cue containing italic text and confirm 0x80/0x81 appear.
    let mut buf = vec![0x20u8; GSI_SIZE];
    buf[0..3].copy_from_slice(b"850");
    buf[3..11].copy_from_slice(b"STL25.01");
    buf[11] = b'1';
    buf[12..16].copy_from_slice(b"0000");
    buf[238..243].copy_from_slice(b"00001");
    buf[243..248].copy_from_slice(b"00001");
    buf[248..250].copy_from_slice(b"40");
    buf[250..252].copy_from_slice(b"23");
    buf[252] = b'1';
    let mut tti = [0u8; TTI_SIZE];
    tti[3] = 0xFF;
    tti[5..9].copy_from_slice(&[0, 0, 1, 0]);
    tti[9..13].copy_from_slice(&[0, 0, 2, 0]);
    tti[14] = 0x02;
    // 0x80 Italic on, "Hi", 0x81 italic off, pad.
    tti[16] = 0x80;
    tti[17] = b'H';
    tti[18] = b'i';
    tti[19] = 0x81;
    for b in &mut tti[20..128] {
        *b = 0x8F;
    }
    buf.extend_from_slice(&tti);

    let t = ebu_stl::parse(&buf).unwrap();
    assert_eq!(t.cues.len(), 1);
    let mut saw_italic = false;
    visit(&t.cues[0].segments, &mut |s| {
        if matches!(s, Segment::Italic(_)) {
            saw_italic = true;
        }
    });
    assert!(
        saw_italic,
        "italic control bytes should produce Segment::Italic"
    );
}

#[test]
fn probe_positive_and_negative() {
    let buf = synthetic();
    assert!(ebu_stl::probe(&buf) > 60);
    assert_eq!(ebu_stl::probe(b"WEBVTT\n"), 0);
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

#[test]
fn per_cue_tti_fields_round_trip_end_to_end() {
    // First TTI: SGN=3, SN=42, CS=1, VP=11, JC=0x01 (left-justified).
    // Second TTI: SGN=7, SN=43, CS=0, VP=12, JC=0x03 (right-justified).
    let mut buf = vec![0x20u8; GSI_SIZE];
    buf[0..3].copy_from_slice(b"850");
    buf[3..11].copy_from_slice(b"STL25.01");
    buf[11] = b'1';
    buf[12..14].copy_from_slice(b"00");
    buf[14..16].copy_from_slice(b"00");
    buf[238..243].copy_from_slice(b"00002");
    buf[243..248].copy_from_slice(b"00002");
    buf[248..250].copy_from_slice(b"40");
    buf[250..252].copy_from_slice(b"23");
    buf[252] = b'1';
    for (sgn, sn, cs, vp, jc, body) in [
        (3u8, 42u16, 1u8, 11u8, 0x01u8, "Hello"),
        (7u8, 43u16, 0u8, 12u8, 0x03u8, "World"),
    ] {
        // Header bytes default to 0 in real STL; TF area pads with 0x8F.
        let mut tti = [0x8Fu8; TTI_SIZE];
        tti[..16].fill(0);
        tti[0] = sgn;
        let sn_le = sn.to_le_bytes();
        tti[1] = sn_le[0];
        tti[2] = sn_le[1];
        tti[3] = 0xFF;
        tti[4] = cs;
        tti[5..9].copy_from_slice(&[0, 0, 1, 0]);
        tti[9..13].copy_from_slice(&[0, 0, 2, 0]);
        tti[13] = vp;
        tti[14] = jc;
        tti[15] = 0;
        let body_bytes = body.as_bytes();
        tti[16..16 + body_bytes.len()].copy_from_slice(body_bytes);
        buf.extend_from_slice(&tti);
    }

    // Parse → write → parse-and-inspect each cue's TTI fields.
    let t1 = ebu_stl::parse(&buf).unwrap();
    let out = ebu_stl::write(&t1).unwrap();
    let t2 = ebu_stl::parse(&out).unwrap();
    let get2 = |k: &str| {
        t2.metadata
            .iter()
            .find(|(kk, _)| kk == k)
            .map(|(_, v)| v.clone())
    };
    assert_eq!(get2("ebu_tti.0.sgn").as_deref(), Some("3"));
    assert_eq!(get2("ebu_tti.0.sn").as_deref(), Some("42"));
    assert_eq!(get2("ebu_tti.0.cs").as_deref(), Some("1"));
    assert_eq!(get2("ebu_tti.0.vp").as_deref(), Some("11"));
    assert_eq!(get2("ebu_tti.0.jc").as_deref(), Some("1"));
    assert_eq!(get2("ebu_tti.1.sgn").as_deref(), Some("7"));
    assert_eq!(get2("ebu_tti.1.sn").as_deref(), Some("43"));
    assert_eq!(get2("ebu_tti.1.vp").as_deref(), Some("12"));
    assert_eq!(get2("ebu_tti.1.jc").as_deref(), Some("3"));
}

#[test]
fn comment_flag_row_survives_parse_write_parse() {
    let mut buf = vec![0x20u8; GSI_SIZE];
    buf[0..3].copy_from_slice(b"850");
    buf[3..11].copy_from_slice(b"STL25.01");
    buf[11] = b'1';
    buf[12..16].copy_from_slice(b"0000");
    buf[238..243].copy_from_slice(b"00002");
    buf[243..248].copy_from_slice(b"00001");
    buf[248..250].copy_from_slice(b"40");
    buf[250..252].copy_from_slice(b"23");
    buf[252] = b'1';

    let mut play = [0x8Fu8; TTI_SIZE];
    play[..16].fill(0);
    play[3] = 0xFF;
    play[5..9].copy_from_slice(&[0, 0, 1, 0]);
    play[9..13].copy_from_slice(&[0, 0, 2, 0]);
    play[13] = 22;
    play[14] = 0x02;
    play[16] = b'A';
    buf.extend_from_slice(&play);

    let mut comment = [0x8Fu8; TTI_SIZE];
    comment[..16].fill(0);
    comment[3] = 0xFF;
    comment[15] = 1; // CF = comment.
    comment[16] = b'N';
    comment[17] = b'O';
    comment[18] = b'T';
    comment[19] = b'E';
    buf.extend_from_slice(&comment);

    let t1 = ebu_stl::parse(&buf).unwrap();
    assert_eq!(t1.cues.len(), 1, "comment row not a playable cue");
    let out = ebu_stl::write(&t1).unwrap();
    // Two TTI rows on the way out (one playable + one comment).
    assert_eq!(out.len(), GSI_SIZE + 2 * TTI_SIZE);
    // The second emitted row should carry CF=1 (comment).
    assert_eq!(out[GSI_SIZE + TTI_SIZE + 15], 1, "comment flag re-emitted");
    // Reparse — comment flag is again excluded from cues.
    let t2 = ebu_stl::parse(&out).unwrap();
    assert_eq!(
        t2.cues.len(),
        1,
        "comment row still not a cue after round-trip"
    );
    let has_comment = t2
        .metadata
        .iter()
        .any(|(k, _)| k.starts_with("ebu_tti.comment.0."));
    assert!(has_comment, "comment metadata preserved on second parse");
}
