//! ASS / SSA `[Events]` Dialogue / Comment line parsing tests.

use oxideav_subtitle::ass_event::{
    event_to_string, fmt_time, parse_event, parse_time, AssEvent, DEFAULT_EVENT_FORMAT,
};
use oxideav_subtitle::ass_style_row::parse_format;

#[test]
fn time_parse_dotted() {
    // 0:00:01.50 = 1 s + 50 cs = 150 cs.
    assert_eq!(parse_time("0:00:01.50"), Some(150));
    // 1:02:03.04 = 3600+120+3 s + 4 cs
    assert_eq!(parse_time("1:02:03.04"), Some((3600 + 120 + 3) * 100 + 4));
}

#[test]
fn time_parse_colon_hundredths() {
    // spec example writes the hundredths after a colon.
    assert_eq!(parse_time("0:00:01:50"), Some(150));
}

#[test]
fn time_parse_one_digit_hundredths() {
    // a single hundredths digit means tenths: .5 -> 50 cs.
    assert_eq!(parse_time("0:00:00.5"), Some(50));
}

#[test]
fn time_parse_rejects_garbage() {
    assert_eq!(parse_time("abc"), None);
    assert_eq!(parse_time("0:99:00.00"), None); // minutes out of range
    assert_eq!(parse_time("0:00:99.00"), None); // seconds out of range
}

#[test]
fn time_fmt_canonical() {
    assert_eq!(fmt_time(150), "0:00:01.50");
    assert_eq!(fmt_time((3600 + 120 + 3) * 100 + 4), "1:02:03.04");
    assert_eq!(fmt_time(0), "0:00:00.00");
}

#[test]
fn time_roundtrip() {
    for cs in [0, 1, 99, 100, 150, 360000, 12345678] {
        assert_eq!(parse_time(&fmt_time(cs)), Some(cs), "cs={cs}");
    }
}

#[test]
fn parse_dialogue_default_format() {
    let line = "Dialogue: 0,0:00:01.00,0:00:03.50,Default,Bob,10,20,30,karaoke,{\\b1}Hello, world";
    let ev = parse_event(line, DEFAULT_EVENT_FORMAT).unwrap();
    assert!(!ev.comment);
    assert_eq!(ev.layer, 0);
    assert_eq!(ev.start_cs, 100);
    assert_eq!(ev.end_cs, 350);
    assert_eq!(ev.style, "Default");
    assert_eq!(ev.name, "Bob");
    assert_eq!(ev.margin_l, 10);
    assert_eq!(ev.margin_r, 20);
    assert_eq!(ev.margin_v, 30);
    assert_eq!(ev.effect, "karaoke");
    // Text keeps the embedded comma.
    assert_eq!(ev.text, "{\\b1}Hello, world");
}

#[test]
fn parse_comment_line() {
    let line = "Comment: 0,0:00:00.00,0:00:00.00,Default,,0,0,0,,not shown";
    let ev = parse_event(line, DEFAULT_EVENT_FORMAT).unwrap();
    assert!(ev.comment);
    assert_eq!(ev.text, "not shown");
}

#[test]
fn parse_text_with_many_commas() {
    let line = "Dialogue: 0,0:00:01.00,0:00:02.00,D,,0,0,0,,a,b,c,d";
    let ev = parse_event(line, DEFAULT_EVENT_FORMAT).unwrap();
    assert_eq!(ev.text, "a,b,c,d");
}

#[test]
fn ssa_marked_field() {
    // SSA classic: first field is Marked=N; the integer after = is kept.
    let fmt = parse_format(
        "Format: Marked, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text",
    )
    .unwrap();
    let line = "Dialogue: Marked=1,0:00:01.00,0:00:02.00,Def,,0,0,0,,hi";
    let ev = parse_event(line, &fmt).unwrap();
    assert_eq!(ev.layer, 1);
    assert_eq!(ev.text, "hi");
}

#[test]
fn reordered_format_remaps_by_name() {
    let fmt = parse_format("Format: Start, End, Text, Style, Layer").unwrap();
    // Note: Text isn't last here, so it cannot contain commas — but the
    // mapping must still honour the header order.
    let line = "Dialogue: 0:00:05.00,0:00:06.00,hello,Title,3";
    let ev = parse_event(line, &fmt).unwrap();
    assert_eq!(ev.start_cs, 500);
    assert_eq!(ev.end_cs, 600);
    assert_eq!(ev.style, "Title");
    assert_eq!(ev.layer, 3);
    // Text is field index 2 (not last), so it does NOT absorb the later
    // comma-separated columns — only the final positional field does.
    assert_eq!(ev.text, "hello");
}

#[test]
fn no_timing_field_returns_none() {
    let line = "Dialogue: not,a,real,event";
    assert_eq!(parse_event(line, DEFAULT_EVENT_FORMAT), None);
}

#[test]
fn event_roundtrips_through_serialize() {
    let line =
        "Dialogue: 2,0:01:02.30,0:01:05.00,Main,Alice,5,5,15,fx,{\\i1}Some text, with commas";
    let ev = parse_event(line, DEFAULT_EVENT_FORMAT).unwrap();
    let emitted = event_to_string(&ev, DEFAULT_EVENT_FORMAT);
    let ev2 = parse_event(&emitted, DEFAULT_EVENT_FORMAT).unwrap();
    assert_eq!(ev, ev2);
    // The serialized form is canonical and re-emits identically.
    assert_eq!(event_to_string(&ev2, DEFAULT_EVENT_FORMAT), emitted);
}

#[test]
fn event_to_string_shape() {
    let ev = AssEvent {
        comment: false,
        layer: 0,
        start_cs: 100,
        end_cs: 350,
        style: "Default".into(),
        name: String::new(),
        margin_l: 0,
        margin_r: 0,
        margin_v: 0,
        effect: String::new(),
        text: "Hi".into(),
    };
    assert_eq!(
        event_to_string(&ev, DEFAULT_EVENT_FORMAT),
        "Dialogue: 0,0:00:01.00,0:00:03.50,Default,,0,0,0,,Hi"
    );
}
