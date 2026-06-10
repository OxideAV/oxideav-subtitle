//! End-to-end checks on the ASS Dialogue-text override-tag tokenizer
//! (`ass_tags`), chained with the `[Script Info]` accessor's
//! `WrapStyle` from the previous step.

use oxideav_subtitle::ass_tags::{decode_alpha_hex, decode_bgr_hex, emit, plain_text, tokenize};
use oxideav_subtitle::{AssColorTarget, AssTag, AssToken, WrapStyle};

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
    // … the primary-fill colour from the typeset block (numbered \1c
    // spelling, target + verbatim digits preserved, BGR decode) …
    let colour = AssTag::Color {
        target: AssColorTarget::Primary,
        short: false,
        hex: Some("D8F8F8".into()),
    };
    assert!(tokens.iter().any(|t| matches!(
        t,
        AssToken::Override(tags) if tags.contains(&colour)
    )));
    assert_eq!(decode_bgr_hex("D8F8F8"), Some((0xF8, 0xF8, 0xD8)));
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
fn karaoke_highlight_colours_round_trip_typed() {
    // A standard-karaoke shape: secondary fill is the pre-highlight,
    // border + shadow recoloured, all-component alpha fade-in level.
    let text = "{\\2c&H00FFFF&\\3c&H40&\\4a&H80&\\alpha&HFF&}ka{\\alpha}ra";
    let tokens = tokenize(text);
    assert_eq!(emit(&tokens), text);
    assert_eq!(
        tokens[0],
        AssToken::Override(vec![
            AssTag::Color {
                target: AssColorTarget::Secondary,
                short: false,
                hex: Some("00FFFF".into()),
            },
            AssTag::Color {
                target: AssColorTarget::Border,
                short: false,
                hex: Some("40".into()),
            },
            AssTag::Alpha {
                target: Some(AssColorTarget::Shadow),
                hex: Some("80".into()),
            },
            AssTag::Alpha {
                target: None,
                hex: Some("FF".into()),
            },
        ])
    );
    // {\alpha} bare = reset all alphas to style.
    assert_eq!(
        tokens[2],
        AssToken::Override(vec![AssTag::Alpha {
            target: None,
            hex: None,
        }])
    );
    // \2c&H00FFFF& = yellow (BGR order); decode helpers agree.
    assert_eq!(decode_bgr_hex("00FFFF"), Some((0xFF, 0xFF, 0x00)));
    assert_eq!(decode_alpha_hex("FF"), Some(255));
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
