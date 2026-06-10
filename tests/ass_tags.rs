//! End-to-end checks on the ASS Dialogue-text override-tag tokenizer
//! (`ass_tags`), chained with the `[Script Info]` accessor's
//! `WrapStyle` from the previous step.

use oxideav_subtitle::ass_tags::{emit, plain_text, tokenize};
use oxideav_subtitle::{AssTag, AssToken, WrapStyle};

#[test]
fn realistic_dialogue_text_round_trips_byte_stable() {
    // A typesetting-heavy event line: positioning, transform animation
    // with nested modifiers, colours, karaoke, flags, escapes, and an
    // inline comment block.
    let text = "{\\pos(640,360)\\an8\\1c&HD8F8F8&\\t(0,500,\\fscx120\\fscy120)}\
                {\\b1}Stage{\\b0} direction\\N\
                {\\k28}so{\\k31}re{\\k62}wa\\h…\
                {timing checked 2024-01}";
    let tokens = tokenize(text);
    assert_eq!(emit(&tokens), text);

    // The typed layer surfaces the flags the IR can model …
    assert!(tokens
        .iter()
        .any(|t| *t == AssToken::Override(vec![AssTag::Bold(Some(1))])));
    assert!(tokens
        .iter()
        .any(|t| *t == AssToken::Override(vec![AssTag::Bold(Some(0))])));
    // … and the escapes.
    assert!(tokens.contains(&AssToken::HardBreak));
    assert!(tokens.contains(&AssToken::HardSpace));
    // The comment block is preserved as a Comment tag, not text.
    assert!(tokens.iter().any(|t| matches!(
        t,
        AssToken::Override(tags)
            if tags == &vec![AssTag::Comment("timing checked 2024-01".into())]
    )));
}

#[test]
fn plain_text_extraction_honours_script_wrap_style() {
    let text = "Line one\\nline two\\Nline three";
    let tokens = tokenize(text);

    // WrapStyle 0..1, 3 (and absent): \n is a regular space.
    for wrap in [
        None,
        Some(WrapStyle::SmartEven),
        Some(WrapStyle::EndOfLine),
        Some(WrapStyle::SmartLower),
    ] {
        assert_eq!(
            plain_text(&tokens, wrap),
            "Line one line two\nline three",
            "for {wrap:?}"
        );
    }
    // WrapStyle 2: both \n and \N force line breaks.
    assert_eq!(
        plain_text(&tokens, Some(WrapStyle::None)),
        "Line one\nline two\nline three"
    );
}
