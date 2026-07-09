//! Hostile-input hardening: every text-subtitle parser must degrade to a
//! typed `Err` (or an empty/partial track) on malformed, truncated, and
//! multi-byte-boundary input — it must NEVER panic.
//!
//! This is a bounded, deterministic no-panic sweep (a fast in-tree fuzzer):
//! a fixed corpus of adversarial byte strings — token soup, truncations at
//! every length, and multi-byte UTF-8 / invalid-byte injections at every
//! offset — is fed through each `parse` entry point under `catch_unwind`.
//!
//! Regression anchor: two char-boundary slice panics (`ass_script::keyword`
//! and `microdvd::parse_bgr`) were caught by this style of sweep; the
//! curated seeds below include their triggering shapes so the guard stays
//! meaningful even at the reduced (CI-fast) corpus size.

use std::panic::{catch_unwind, AssertUnwindSafe};

use oxideav_subtitle::{
    aqtitle, ass_script, ebu_stl, jacosub, microdvd, mpl2, mpsub, pjs, realtext, sami, srt,
    subviewer1, subviewer2, ttml, vplayer, webvtt,
};

type ParseFn = fn(&[u8]);

fn parsers() -> Vec<(&'static str, ParseFn)> {
    vec![
        ("srt", |b| {
            let _ = srt::parse(b);
        }),
        ("webvtt", |b| {
            let _ = webvtt::parse(b);
        }),
        ("microdvd", |b| {
            let _ = microdvd::parse(b);
        }),
        ("mpl2", |b| {
            let _ = mpl2::parse(b);
        }),
        ("mpsub", |b| {
            let _ = mpsub::parse(b);
        }),
        ("vplayer", |b| {
            let _ = vplayer::parse(b);
        }),
        ("pjs", |b| {
            let _ = pjs::parse(b);
        }),
        ("aqtitle", |b| {
            let _ = aqtitle::parse(b);
        }),
        ("jacosub", |b| {
            let _ = jacosub::parse(b);
        }),
        ("realtext", |b| {
            let _ = realtext::parse(b);
        }),
        ("subviewer1", |b| {
            let _ = subviewer1::parse(b);
        }),
        ("subviewer2", |b| {
            let _ = subviewer2::parse(b);
        }),
        ("ttml", |b| {
            let _ = ttml::parse(b);
        }),
        ("sami", |b| {
            let _ = sami::parse(b);
        }),
        ("ebu_stl", |b| {
            let _ = ebu_stl::parse(b);
        }),
        ("ass", |b| {
            let _ = ass_script::parse(b);
        }),
    ]
}

/// Fragments that steer mutations onto the interesting branches of each
/// format (timing lines, tag openers, colour runs, section headers).
const SEED_STRINGS: &[&str] = &[
    "",
    "\u{FEFF}",
    "WEBVTT\n\n00:00:01.000 --> 00:00:02.000\n<v Bob>hi <c.y>x</c></v>\n",
    "WEBVTT\n\nSTYLE\n::cue(.y){color:red}\n\nREGION\nid=r width=40%\n",
    "1\n00:00:01,000 --> 00:00:02,000\n<i>hi</i>\n",
    "{1}{2}25\n{25}{75}{y:i}{c:$aébcd}{f:Arial}hi\n",
    "[0][10]Hello\n[10][20]World\n",
    "0:00:01.00,0:00:02.00\nText here\n",
    "[Script Info]\né lead\nTitle: T\n\n[V4+ Styles]\nFormat: Name, Fontname\nStyle: D,Arial\n\n[Events]\nDialogue: 0,0:00:01.00,0:00:02.00,D,,0,0,0,,{\\b1}hi{\\r}\n",
    "<sami><body><sync start=100><p class=x>hi</p></sync></body></sami>",
    "<tt><body><div><p begin=\"1s\" end=\"2s\">hi<br/><span tts:color=\"red\">x</span></p></div></body></tt>",
    "<window><time begin=\"1\"/>hi</window>",
    "#J 1.0 3.0 D hi\n",
    "0000 0001 hi\n",
    "position:50% line:90%,end align:center vertical:rl",
    "\\N\\h{\\pos(0,0)}{\\t(0,1,\\frz9)}{\\clip(m 0 0 l 1 1)}{\\fad(1,1)}",
    "é😀€\u{FEFF}\u{0}\u{7}",
];

fn build_corpus() -> Vec<Vec<u8>> {
    let mut corpus: Vec<Vec<u8>> = Vec::new();
    let inject: &[&[u8]] = &[
        "é".as_bytes(),
        "😀".as_bytes(),
        &[0xFF],
        &[0x00],
        &[0xC0],
        &[0x80, 0x80],
    ];
    for seed in SEED_STRINGS {
        let b = seed.as_bytes();
        // truncation at every length
        for len in 0..=b.len() {
            corpus.push(b[..len].to_vec());
        }
        // multi-byte / invalid-byte injection at every offset
        for i in 0..=b.len() {
            for mb in inject {
                let mut m = b[..i].to_vec();
                m.extend_from_slice(mb);
                m.extend_from_slice(&b[i..]);
                corpus.push(m);
            }
        }
    }
    // solid single-byte runs (binary formats + degenerate delimiters)
    for byte in 0u16..=255 {
        corpus.push(vec![byte as u8; 32]);
    }
    // pathological large inputs (unbounded-loop / OOM guards)
    corpus.push(b"A".repeat(100_000));
    corpus.push(b"00:00:01.000 --> 00:00:02.000\n".repeat(2000).to_vec());
    corpus.push(b"{".repeat(50_000));
    corpus
}

#[test]
fn no_parser_panics_on_hostile_input() {
    let corpus = build_corpus();
    let parsers = parsers();
    let mut panicked: Vec<String> = Vec::new();
    // Silence the default panic hook so caught panics don't spam the log.
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    for (name, f) in &parsers {
        for input in &corpus {
            let inp = input.as_slice();
            if catch_unwind(AssertUnwindSafe(|| f(inp))).is_err() {
                let hex: String = input.iter().take(48).map(|b| format!("{b:02x}")).collect();
                panicked.push(format!("{name}: {} bytes [{hex}]", input.len()));
                break; // one report per parser is enough
            }
        }
    }
    std::panic::set_hook(prev);
    assert!(
        panicked.is_empty(),
        "parser(s) panicked on hostile input:\n{}",
        panicked.join("\n")
    );
}
