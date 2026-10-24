//! Container probe scoring.

use std::io::Cursor;

use oxideav_core::ContainerRegistry;

fn make_registry() -> ContainerRegistry {
    let mut reg = ContainerRegistry::new();
    oxideav_subtitle::register_containers(&mut reg);
    reg
}

#[test]
fn webvtt_probes_top() {
    let reg = make_registry();
    let mut c = Cursor::new(b"WEBVTT\n\n00:00:01.000 --> 00:00:02.000\nhi\n".to_vec());
    let name = reg.probe_input(&mut c, Some("vtt")).unwrap();
    assert_eq!(name, "webvtt");
}

#[test]
fn srt_probes_srt() {
    let reg = make_registry();
    let mut c = Cursor::new(b"1\n00:00:01,000 --> 00:00:02,000\nhi\n\n".to_vec());
    let name = reg.probe_input(&mut c, Some("srt")).unwrap();
    assert_eq!(name, "srt");
}

// ASS probe test moved to the `oxideav-ass` crate.

#[test]
fn demuxer_yields_one_packet_per_cue() {
    let reg = make_registry();
    let src =
        b"1\n00:00:01,000 --> 00:00:02,000\nfirst\n\n2\n00:00:03,000 --> 00:00:04,500\nsecond\n"
            .to_vec();
    let mut c = Cursor::new(src);
    let name = reg.probe_input(&mut c, Some("srt")).unwrap();
    let mut dmx = reg
        .open_demuxer(&name, Box::new(c), &oxideav_core::NullCodecResolver)
        .unwrap();
    let p1 = dmx.next_packet().unwrap();
    assert_eq!(p1.pts, Some(1_000_000));
    let p2 = dmx.next_packet().unwrap();
    assert_eq!(p2.pts, Some(3_000_000));
    assert!(dmx.next_packet().is_err());
}

#[test]
fn mux_srt_reemits_cues() {
    use oxideav_core::CodecRegistry;
    use oxideav_core::{CodecId, CodecParameters, Frame};

    let mut codecs = CodecRegistry::new();
    oxideav_subtitle::register_codecs(&mut codecs);

    let reg = make_registry();
    // Demux a source first.
    let src =
        b"1\n00:00:01,000 --> 00:00:02,000\nfirst\n\n2\n00:00:03,000 --> 00:00:04,500\nsecond\n"
            .to_vec();
    let c = Cursor::new(src);
    let mut dmx = reg
        .open_demuxer("srt", Box::new(c), &oxideav_core::NullCodecResolver)
        .unwrap();
    let stream = dmx.streams()[0].clone();
    let mut packets = Vec::new();
    while let Ok(p) = dmx.next_packet() {
        packets.push(p);
    }
    assert_eq!(packets.len(), 2);

    // Re-mux.
    let buf = Cursor::new(Vec::<u8>::new());
    let mut mux = reg
        .open_muxer("srt", Box::new(buf), std::slice::from_ref(&stream))
        .unwrap();
    mux.write_header().unwrap();
    for p in &packets {
        mux.write_packet(p).unwrap();
    }
    mux.write_trailer().unwrap();
    drop(mux);

    // Decode packets through the codec for good measure.
    let dec_params = CodecParameters::subtitle(CodecId::new("subrip"));
    let mut dec = codecs.first_decoder(&dec_params).unwrap();
    dec.send_packet(&packets[0]).unwrap();
    let f = dec.receive_frame().unwrap();
    match f {
        Frame::Subtitle(cue) => {
            assert_eq!(cue.start_us, 1_000_000);
        }
        _ => panic!("expected subtitle frame"),
    }
}

// ---------------------------------------------------------------------------
// Negative probing: no text-subtitle probe may claim binary or unrelated
// content (umbrella playback corpus: raw `.ac3` / `.yuv` / `.mp4` used to
// open as EBU STL because byte 11 happened to be printable).

/// Every extension a subtitle container here registers, plus "none".
const EXT_HINTS: &[Option<&str>] = &[
    None,
    Some("srt"),
    Some("vtt"),
    Some("sub"),
    Some("txt"),
    Some("mpl"),
    Some("vpl"),
    Some("pjs"),
    Some("aqt"),
    Some("jss"),
    Some("js"),
    Some("rt"),
    Some("ttml"),
    Some("dfxp"),
    Some("xml"),
    Some("smi"),
    Some("sami"),
    Some("stl"),
];

fn claims(reg: &ContainerRegistry, buf: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    for ext in EXT_HINTS {
        for c in reg.probe_candidates(&oxideav_core::ProbeData { buf, ext: *ext }) {
            out.push(format!("{}={} (ext {:?})", c.name, c.score, ext));
        }
    }
    out
}

struct XorShift(u64);
impl XorShift {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

#[test]
fn random_4k_buffers_probe_zero_everywhere() {
    let reg = make_registry();
    let mut rng = XorShift(0x5EED_0F5B_71E5_u64);
    for i in 0..500 {
        let mut buf: Vec<u8> = (0..4096).map(|_| rng.next() as u8).collect();
        // Every few buffers, plant the strings the probes key on so the
        // structural checks (not only the text gate) are exercised.
        match i % 5 {
            1 => buf[3..11].copy_from_slice(b"STL25.01"),
            2 => buf[..6].copy_from_slice(b"WEBVTT"),
            3 => buf[..12].copy_from_slice(b"<?xml ?><tt "),
            4 => buf[..8].copy_from_slice(b"{1}{25}x"),
            _ => {}
        }
        let c = claims(&reg, &buf);
        assert!(c.is_empty(), "random buffer #{i} claimed by {c:?}");
    }
}

#[test]
fn random_printable_text_probes_zero() {
    // 4 KiB of random printable ASCII + newlines: text, but no
    // subtitle structure.
    let reg = make_registry();
    let mut rng = XorShift(0x0BAD_C0DE_1234_5678);
    for i in 0..200 {
        let buf: Vec<u8> = (0..4096)
            .map(|_| match rng.next() % 40 {
                0 => b'\n',
                _ => b' ' + (rng.next() % 95) as u8,
            })
            .collect();
        let c = claims(&reg, &buf);
        assert!(c.is_empty(), "random text #{i} claimed by {c:?}");
    }
}

#[test]
fn binary_media_shapes_probe_zero() {
    let reg = make_registry();
    let mut rng = XorShift(42);
    // AC-3 sync frame shape: 0B 77, CRC, then payload.
    let mut ac3: Vec<u8> = (0..4096).map(|_| rng.next() as u8).collect();
    ac3[..2].copy_from_slice(&[0x0B, 0x77]);
    ac3[11] = b'A';
    // Raw 8-bit planar YUV test pattern (values 16..=235) with an ASCII
    // byte at offset 11.
    let mut yuv: Vec<u8> = (0..65536u32).map(|i| 16 + (i % 220) as u8).collect();
    yuv[11] = b'Z';
    // ISO-BMFF `ftyp` head.
    let mut mp4 = vec![0x00, 0x00, 0x00, 0x20];
    mp4.extend_from_slice(b"ftypisom\x00\x00\x02\x00isomiso2avc1mp41");
    mp4.resize(2048, 0);
    // MPEG program stream pack header.
    let mut ps = vec![
        0x00, 0x00, 0x01, 0xBA, 0x44, 0x00, 0x04, 0x00, 0x04, 0x01, 0x01, 0x89,
    ];
    ps.resize(2048, 0xFF);
    for (name, buf) in [("ac3", &ac3), ("yuv", &yuv), ("mp4", &mp4), ("mpg", &ps)] {
        let c = claims(&reg, buf);
        assert!(c.is_empty(), "{name} claimed by {c:?}");
    }
}

#[test]
fn ebu_stl_probe_needs_dfc_and_cpn() {
    let mut gsi = vec![b' '; 1024];
    gsi[0..3].copy_from_slice(b"850");
    gsi[3..11].copy_from_slice(b"STL25.01");
    gsi[11] = b'1';
    assert_eq!(oxideav_subtitle::ebu_stl::probe(&gsi), 95);
    let mut g = gsi.clone();
    g[3..11].copy_from_slice(b"STL30.01");
    assert_eq!(oxideav_subtitle::ebu_stl::probe(&g), 95);
    // Non-digit CPN.
    let mut g = gsi.clone();
    g[0..3].copy_from_slice(b"ABC");
    assert_eq!(oxideav_subtitle::ebu_stl::probe(&g), 0);
    // DFC not of the STLxx.yy shape.
    for bad in [b"STLXX.01", b"STL25-01", b"XTL25.01", b"STL25.0A"] {
        let mut g = gsi.clone();
        g[3..11].copy_from_slice(bad);
        assert_eq!(oxideav_subtitle::ebu_stl::probe(&g), 0, "{bad:?}");
    }
    // A text log starting with three digits and a printable byte 11.
    let mut log = b"123 started encoder pass 1 of 2\n".repeat(64);
    log.truncate(1024);
    assert_eq!(oxideav_subtitle::ebu_stl::probe(&log), 0);
    // Truncated GSI.
    assert_eq!(oxideav_subtitle::ebu_stl::probe(&gsi[..1023]), 0);
}

#[test]
fn ttml_probe_needs_a_tt_root_element() {
    use oxideav_subtitle::ttml::probe;
    let ns =
        br#"<?xml version="1.0"?><!-- c --><tt xmlns="http://www.w3.org/ns/ttml"><body/></tt>"#;
    assert_eq!(probe(ns), 95);
    assert_eq!(
        probe(br#"<ttml:tt xmlns:ttml="http://www.w3.org/ns/ttml"/>"#),
        95
    );
    assert_eq!(probe(b"<tt><body/></tt>"), 60);
    // XHTML with the teletype element, SVG, a bare XML declaration.
    assert_eq!(
        probe(b"<!DOCTYPE html><html><body><tt>code</tt></body></html>"),
        0
    );
    assert_eq!(
        probe(br#"<?xml version="1.0"?><svg xmlns="http://www.w3.org/2000/svg"/>"#),
        0
    );
    assert_eq!(probe(br#"<?xml version="1.0"?>"#), 0);
    // A document merely mentioning the namespace.
    assert_eq!(probe(b"see http://www.w3.org/ns/ttml for tts: attrs"), 0);
}

#[test]
fn sami_probe_needs_the_sami_element() {
    use oxideav_subtitle::sami::probe;
    assert_eq!(probe(b"<html><body><sync>x</sync></body></html>"), 0);
    assert_eq!(
        probe(b"<SAMI><BODY><SYNC Start=0><P>hi</P></BODY></SAMI>"),
        90
    );
}
