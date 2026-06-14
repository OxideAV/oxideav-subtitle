//! End-to-end checks on the ASS Dialogue-text override-tag tokenizer
//! (`ass_tags`), chained with the `[Script Info]` accessor's
//! `WrapStyle` from the previous step.

use oxideav_subtitle::ass_tags::{
    decode_alpha_hex, decode_bgr_hex, decode_decimal, emit, legacy_align_to_numpad, plain_text,
    tokenize, AssRotationAxis,
};
use oxideav_subtitle::{
    AssBorderAxis, AssColorTarget, AssKaraokeKind, AssTag, AssToken, WrapStyle,
};

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
    // … the typeset block's position + numpad alignment (typed,
    // alongside the verbatim \t transform in the same brace set) …
    assert_eq!(
        tokens[0],
        AssToken::Override(vec![
            AssTag::Pos { x: 640, y: 360 },
            AssTag::AlignNumpad(Some(8)),
            AssTag::Color {
                target: AssColorTarget::Primary,
                short: false,
                hex: Some("D8F8F8".into()),
            },
            AssTag::Other("t(0,500,\\fscx120\\fscy120)".into()),
        ])
    );
    // … the karaoke syllable timings (centiseconds) …
    assert!(tokens.iter().any(|t| *t
        == AssToken::Override(vec![AssTag::Karaoke {
            kind: AssKaraokeKind::Instant,
            centisec: 62,
        }])));
    // … legacy-alignment conversion for renderers that only speak
    // numpad values …
    assert_eq!(legacy_align_to_numpad(6), Some(8));
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

#[test]
fn font_metric_and_rotation_family_round_trips_typed() {
    // A typesetting block exercising the whole \f* family: font name
    // (with a space), fractional size, per-axis scale above 100%,
    // negative spacing, charset, and rotation on each axis plus the
    // bare \fr (= \frz) spelling.
    let text = "{\\fnCourier New\\fs28.5\\fscx200\\fscy150\\fsp-2\\fe1\
                \\frx10\\fry-20\\frz-30.5\\fr45}text";
    let tokens = tokenize(text);
    assert_eq!(emit(&tokens), text, "font family block must round-trip");
    assert_eq!(
        tokens[0],
        AssToken::Override(vec![
            AssTag::FontName(Some("Courier New".into())),
            AssTag::FontSize(Some("28.5".into())),
            AssTag::FontScale {
                x_axis: true,
                percent: Some("200".into()),
            },
            AssTag::FontScale {
                x_axis: false,
                percent: Some("150".into()),
            },
            AssTag::FontSpacing(Some("-2".into())),
            AssTag::FontEncoding(Some("1".into())),
            AssTag::Rotation {
                axis: AssRotationAxis::X,
                bare: false,
                degrees: Some("10".into()),
            },
            AssTag::Rotation {
                axis: AssRotationAxis::Y,
                bare: false,
                degrees: Some("-20".into()),
            },
            AssTag::Rotation {
                axis: AssRotationAxis::Z,
                bare: false,
                degrees: Some("-30.5".into()),
            },
            // bare \fr is \frz with bare:true so emit stays byte-stable.
            AssTag::Rotation {
                axis: AssRotationAxis::Z,
                bare: true,
                degrees: Some("45".into()),
            },
        ])
    );
    // Decoders turn the verbatim runs into numbers.
    assert_eq!(decode_decimal("28.5"), Some(28.5));
    assert_eq!(decode_decimal("-30.5"), Some(-30.5));
    assert_eq!(decode_decimal("200"), Some(200.0));
}

#[test]
fn font_metric_reset_forms_are_typed_none() {
    // Parameterless \f* tags are the documented reset-to-style shape.
    let text = "{\\fn\\fs\\fscx\\fscy\\fsp\\fe\\frx\\fry\\frz\\fr}t";
    let tokens = tokenize(text);
    assert_eq!(emit(&tokens), text);
    assert_eq!(
        tokens[0],
        AssToken::Override(vec![
            AssTag::FontName(None),
            AssTag::FontSize(None),
            AssTag::FontScale {
                x_axis: true,
                percent: None,
            },
            AssTag::FontScale {
                x_axis: false,
                percent: None,
            },
            AssTag::FontSpacing(None),
            AssTag::FontEncoding(None),
            AssTag::Rotation {
                axis: AssRotationAxis::X,
                bare: false,
                degrees: None,
            },
            AssTag::Rotation {
                axis: AssRotationAxis::Y,
                bare: false,
                degrees: None,
            },
            AssTag::Rotation {
                axis: AssRotationAxis::Z,
                bare: false,
                degrees: None,
            },
            AssTag::Rotation {
                axis: AssRotationAxis::Z,
                bare: true,
                degrees: None,
            },
        ])
    );
}

#[test]
fn off_shape_font_params_stay_verbatim() {
    // Non-canonical numeric spellings the typed layer must not absorb,
    // each staying an untyped Other so emit is byte-stable: a `+`
    // sign, a trailing dot, a bare dot, a `%` the spec doesn't use, an
    // embedded space, and the prefix-cousins \be / \blur / \bord that
    // begin with letters \fs etc. must not swallow.
    for body in [
        "fs+12",
        "fs12.",
        "fsp.",
        "fscx1.2.3",
        "frz9 0",
        "fs1%",
        "fe0x10",
    ] {
        let text = format!("{{\\{body}}}x");
        let tokens = tokenize(&text);
        assert_eq!(emit(&tokens), text, "{body} must round-trip verbatim");
        assert_eq!(
            tokens[0],
            AssToken::Override(vec![AssTag::Other(body.into())]),
            "{body} must stay an untyped Other"
        );
    }
    // \fade / \fad are function tags, not the \fs* family — verbatim.
    let fade = "{\\fad(200,200)}x";
    assert_eq!(
        tokenize(fade)[0],
        AssToken::Override(vec![AssTag::Other("fad(200,200)".into())])
    );
}

#[test]
fn border_and_shadow_family_round_trips_typed() {
    // The full \bord / \shad family: combined width + depth, per-axis
    // split, fractional values, and the per-axis shadow's negative depth
    // ("unlike \shad, you can set the distance negative with these
    // tags").
    let text = "{\\bord3.7\\xbord2\\ybord0\\shad4\\xshad-3\\yshad1.5}text";
    let tokens = tokenize(text);
    assert_eq!(emit(&tokens), text, "border/shadow block must round-trip");
    assert_eq!(
        tokens[0],
        AssToken::Override(vec![
            AssTag::Border {
                axis: AssBorderAxis::Both,
                size: Some("3.7".into()),
            },
            AssTag::Border {
                axis: AssBorderAxis::X,
                size: Some("2".into()),
            },
            AssTag::Border {
                axis: AssBorderAxis::Y,
                size: Some("0".into()),
            },
            AssTag::Shadow {
                axis: AssBorderAxis::Both,
                depth: Some("4".into()),
            },
            AssTag::Shadow {
                axis: AssBorderAxis::X,
                depth: Some("-3".into()),
            },
            AssTag::Shadow {
                axis: AssBorderAxis::Y,
                depth: Some("1.5".into()),
            },
        ])
    );
    // Verbatim runs decode to numbers.
    assert_eq!(decode_decimal("3.7"), Some(3.7));
    assert_eq!(decode_decimal("-3"), Some(-3.0));
}

#[test]
fn border_shadow_reset_forms_are_typed_none() {
    // Parameterless forms are the documented reset-to-style shape.
    let text = "{\\bord\\xbord\\ybord\\shad\\xshad\\yshad}t";
    let tokens = tokenize(text);
    assert_eq!(emit(&tokens), text);
    assert_eq!(
        tokens[0],
        AssToken::Override(vec![
            AssTag::Border {
                axis: AssBorderAxis::Both,
                size: None,
            },
            AssTag::Border {
                axis: AssBorderAxis::X,
                size: None,
            },
            AssTag::Border {
                axis: AssBorderAxis::Y,
                size: None,
            },
            AssTag::Shadow {
                axis: AssBorderAxis::Both,
                depth: None,
            },
            AssTag::Shadow {
                axis: AssBorderAxis::X,
                depth: None,
            },
            AssTag::Shadow {
                axis: AssBorderAxis::Y,
                depth: None,
            },
        ])
    );
}

#[test]
fn negative_combined_border_shadow_stays_verbatim() {
    // The spec bars a negative width on \bord and a negative depth on
    // the combined \shad ("Border width cannot be negative"; the \shad
    // "distance can not be negative with this tag"), so a signed value
    // there stays an untyped Other and emit is byte-stable. The per-axis
    // \xshad / \yshad, by contrast, DO accept a negative (covered
    // above).
    for body in ["bord-1", "xbord-1", "ybord-2.5", "shad-1"] {
        let text = format!("{{\\{body}}}x");
        let tokens = tokenize(&text);
        assert_eq!(emit(&tokens), text, "{body} must round-trip verbatim");
        assert_eq!(
            tokens[0],
            AssToken::Override(vec![AssTag::Other(body.into())]),
            "{body} must stay an untyped Other"
        );
    }
    // The \b / \s style toggles and the \be blur-edges cousin must not
    // be swallowed by the border/shadow family.
    assert_eq!(
        tokenize("{\\b1}x")[0],
        AssToken::Override(vec![AssTag::Bold(Some(1))])
    );
    assert_eq!(
        tokenize("{\\s0}x")[0],
        AssToken::Override(vec![AssTag::Strikeout(Some(false))])
    );
    assert_eq!(
        tokenize("{\\be1}x")[0],
        AssToken::Override(vec![AssTag::Other("be1".into())])
    );
}
