//! Tokenizer for the ASS / SSA Dialogue `Text` field: override blocks,
//! escape codes, and a byte-stable re-emit.
//!
//! Parsing of `.ass` / `.ssa` *files* lives in the sibling
//! `oxideav-ass` crate; this module operates on the per-event `Text`
//! payload only, continuing the IR-side helper chain started by
//! [`crate::ass_script_info`].
//!
//! The grammar comes from the SSA v4 script-format specification's
//! "Appendix A: Style override codes" (mirrored at
//! `docs/subtitles/ass/ass-specs-tcax.html`):
//!
//! * "All Override codes appear within braces `{ }` except the newline
//!   `\n` and `\N` codes."
//! * "All override codes are always preceded by a backslash `\`."
//! * "Several overrides can be used within one set of braces."
//! * "Any style modifier followed by no recognizable parameter resets
//!   to the default."
//!
//! and from the Aegisub override-tag reference (mirrored at
//! `docs/subtitles/ass/aegisub-ass-tags.html`), which adds:
//!
//! * the `\h` non-breaking hard-space escape, written "in the middle
//!   of the text, and not inside override blocks" like `\n` / `\N`;
//! * "Any unrecognized text within override blocks is silently
//!   ignored, so they are also commonly used for inline comments";
//! * the `\b <weight>` form: "Font weights are multiples of 100, such
//!   that 100 is the lowest, 400 is 'normal', 700 is 'bold' and 900 is
//!   the heaviest";
//! * complex tags taking parenthesised comma-separated parameter
//!   lists (`\t(...)`, `\move(...)`, `\pos(...)`, `\fad(...)`, …).
//!
//! This first step types only the four boolean style flags (`\b`,
//! `\i`, `\u`, `\s` — the ones the IR `Segment` tree can model). Every
//! other tag is preserved verbatim in [`AssTag::Other`], so
//! [`emit`] reproduces the original text byte-for-byte and no
//! information is dropped. Typed coverage of the wider tag set
//! (colours, positioning, karaoke, …) is follow-up material.

use crate::ass_script_info::WrapStyle;

/// One lexical unit of a Dialogue `Text` field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AssToken {
    /// Plain visible text (verbatim, including any backslash sequence
    /// that is not one of the three recognised escapes).
    Text(String),
    /// One `{...}` override block holding zero or more tags.
    Override(Vec<AssTag>),
    /// `\n` — soft line break. Per the spec's Appendix A it is
    /// "ignored by SSA if smart-wrapping is enabled"; per the Aegisub
    /// reference it only breaks in wrapping mode 2 and "is replaced by
    /// a regular space" in all other modes.
    SoftBreak,
    /// `\N` — hard line break "regardless of wrapping mode".
    HardBreak,
    /// `\h` — non-breaking hard space (Aegisub reference: "The line
    /// will never break automatically right before or after a hard
    /// space").
    HardSpace,
}

/// One tag inside an override block.
///
/// The four typed variants carry the parsed parameter; `None` is the
/// parameterless form, which per the spec "resets to the default"
/// (the line's style value). A parameter outside the recognised shape
/// (e.g. `\i2`) falls through to [`AssTag::Other`] verbatim rather
/// than guessing at semantics, so re-emit stays byte-stable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AssTag {
    /// `\b` — bold. `Some(0)` off, `Some(1)` on; values greater than 1
    /// are an explicit font weight ("400 = Normal, 700 = Bold" per the
    /// spec; multiples of 100 per the Aegisub reference). `None` is
    /// the parameterless reset-to-style form.
    Bold(Option<u32>),
    /// `\i` — italic on (`Some(true)`) / off (`Some(false)`) / reset
    /// (`None`).
    Italic(Option<bool>),
    /// `\u` — underline, same shapes as `\i`.
    Underline(Option<bool>),
    /// `\s` — strikeout, same shapes as `\i`.
    Strikeout(Option<bool>),
    /// Any other tag, kept verbatim — the full body after the
    /// backslash, including parenthesised parameter lists
    /// (`t(0,1000,\fscx200)`, `pos(320,240)`, `1c&HFF&`, …).
    Other(String),
    /// Non-tag text inside the block, kept verbatim. The Aegisub
    /// reference: "Any unrecognized text within override blocks is
    /// silently ignored, so they are also commonly used for inline
    /// comments."
    Comment(String),
}

/// Tokenize a Dialogue `Text` field into text runs, override blocks,
/// and the three mid-text escapes.
///
/// The tokenizer never fails: an unterminated `{` (no closing `}`
/// before end of input) is kept as literal text, and a backslash
/// followed by anything other than `n` / `N` / `h` stays literal, so
/// `emit(&tokenize(s)) == s` for every input.
pub fn tokenize(text: &str) -> Vec<AssToken> {
    let mut out = Vec::new();
    let mut buf = String::new();
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        match c {
            '{' => {
                if let Some(end) = rest.find('}') {
                    flush(&mut buf, &mut out);
                    out.push(AssToken::Override(parse_block(&rest[1..end])));
                    rest = &rest[end + 1..];
                } else {
                    // Unterminated block: literal text.
                    buf.push_str(rest);
                    rest = "";
                }
            }
            '\\' => {
                let mut it = rest.chars();
                it.next();
                match it.next() {
                    Some('n') => {
                        flush(&mut buf, &mut out);
                        out.push(AssToken::SoftBreak);
                        rest = &rest[2..];
                    }
                    Some('N') => {
                        flush(&mut buf, &mut out);
                        out.push(AssToken::HardBreak);
                        rest = &rest[2..];
                    }
                    Some('h') => {
                        flush(&mut buf, &mut out);
                        out.push(AssToken::HardSpace);
                        rest = &rest[2..];
                    }
                    Some(other) => {
                        buf.push('\\');
                        buf.push(other);
                        rest = &rest[1 + other.len_utf8()..];
                    }
                    None => {
                        buf.push('\\');
                        rest = "";
                    }
                }
            }
            _ => {
                buf.push(c);
                rest = &rest[c.len_utf8()..];
            }
        }
    }
    flush(&mut buf, &mut out);
    out
}

fn flush(buf: &mut String, out: &mut Vec<AssToken>) {
    if !buf.is_empty() {
        out.push(AssToken::Text(std::mem::take(buf)));
    }
}

/// Split an override block's interior into tags. Each tag body runs
/// from its `\` to the next `\` at parenthesis depth zero (a complex
/// tag's parameter list may itself contain backslash modifiers — the
/// spec's `\t(<t1>, <t2>, <accel>, <style modifiers>)`).
fn parse_block(body: &str) -> Vec<AssTag> {
    let mut tags = Vec::new();
    let mut i = 0;
    let bytes = body.as_bytes();
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            // Find the end of this tag body.
            let start = i + 1;
            let mut depth = 0usize;
            let mut j = start;
            while j < bytes.len() {
                match bytes[j] {
                    b'(' => depth += 1,
                    b')' => depth = depth.saturating_sub(1),
                    b'\\' if depth == 0 => break,
                    _ => {}
                }
                j += 1;
            }
            tags.push(classify(&body[start..j]));
            i = j;
        } else {
            // Comment run until the next backslash.
            let start = i;
            while i < bytes.len() && bytes[i] != b'\\' {
                i += 1;
            }
            tags.push(AssTag::Comment(body[start..i].to_string()));
        }
    }
    tags
}

/// Map one tag body (text after the backslash) to a typed variant.
/// Only an exactly-recognised parameter shape is typed; everything
/// else is preserved verbatim. The name match is exact-prefix +
/// digits-only-remainder, so `\bord`, `\be`, `\blur`, `\shad`, and
/// `\iclip` cannot be mistaken for `\b` / `\s` / `\i` forms.
fn classify(tag: &str) -> AssTag {
    let (head, arg) = match tag.chars().next() {
        Some(c @ ('b' | 'i' | 'u' | 's')) => (c, &tag[1..]),
        _ => return AssTag::Other(tag.to_string()),
    };
    if !arg.is_empty() && !arg.bytes().all(|b| b.is_ascii_digit()) {
        return AssTag::Other(tag.to_string());
    }
    match head {
        'b' => match arg {
            "" => AssTag::Bold(None),
            _ => match arg.parse::<u32>() {
                Ok(w) => AssTag::Bold(Some(w)),
                Err(_) => AssTag::Other(tag.to_string()),
            },
        },
        _ => {
            let flag = match arg {
                "" => None,
                "0" => Some(false),
                "1" => Some(true),
                // \i2 etc.: not a documented shape; keep verbatim.
                _ => return AssTag::Other(tag.to_string()),
            };
            match head {
                'i' => AssTag::Italic(flag),
                'u' => AssTag::Underline(flag),
                _ => AssTag::Strikeout(flag),
            }
        }
    }
}

/// Re-emit tokens to the Dialogue `Text` wire form. Inverse of
/// [`tokenize`]: byte-stable for every input that round-trips through
/// it.
pub fn emit(tokens: &[AssToken]) -> String {
    let mut out = String::new();
    for tok in tokens {
        match tok {
            AssToken::Text(s) => out.push_str(s),
            AssToken::SoftBreak => out.push_str("\\n"),
            AssToken::HardBreak => out.push_str("\\N"),
            AssToken::HardSpace => out.push_str("\\h"),
            AssToken::Override(tags) => {
                out.push('{');
                for tag in tags {
                    match tag {
                        AssTag::Bold(b) => {
                            out.push_str("\\b");
                            if let Some(w) = b {
                                out.push_str(&w.to_string());
                            }
                        }
                        AssTag::Italic(f) => push_flag(&mut out, 'i', *f),
                        AssTag::Underline(f) => push_flag(&mut out, 'u', *f),
                        AssTag::Strikeout(f) => push_flag(&mut out, 's', *f),
                        AssTag::Other(body) => {
                            out.push('\\');
                            out.push_str(body);
                        }
                        AssTag::Comment(s) => out.push_str(s),
                    }
                }
                out.push('}');
            }
        }
    }
    out
}

fn push_flag(out: &mut String, name: char, flag: Option<bool>) {
    out.push('\\');
    out.push(name);
    match flag {
        Some(true) => out.push('1'),
        Some(false) => out.push('0'),
        None => {}
    }
}

/// Strip a token stream down to the user-visible text.
///
/// Override blocks (tags and inline comments alike) are dropped. The
/// escapes map per the wrap-style rules:
///
/// * [`AssToken::HardBreak`] (`\N`) is a newline "regardless of
///   wrapping mode".
/// * [`AssToken::SoftBreak`] (`\n`) is a newline only in wrapping
///   mode 2 ([`WrapStyle::None`] — "Both `\n` and `\N` force line
///   breaks"); in every other mode it "is replaced by a regular
///   space". Pass `None` when the script carries no `WrapStyle:`
///   header — the field's default (`0`, smart wrapping) treats `\n`
///   as a space.
/// * [`AssToken::HardSpace`] (`\h`) maps to U+00A0 NO-BREAK SPACE,
///   the plain-text carrier of the reference's "non-breaking 'hard'
///   space" behaviour.
pub fn plain_text(tokens: &[AssToken], wrap: Option<WrapStyle>) -> String {
    let soft_breaks = wrap == Some(WrapStyle::None);
    let mut out = String::new();
    for tok in tokens {
        match tok {
            AssToken::Text(s) => out.push_str(s),
            AssToken::Override(_) => {}
            AssToken::SoftBreak => out.push(if soft_breaks { '\n' } else { ' ' }),
            AssToken::HardBreak => out.push('\n'),
            AssToken::HardSpace => out.push('\u{00A0}'),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(s: &str) {
        assert_eq!(emit(&tokenize(s)), s, "byte-stable round-trip for {s:?}");
    }

    #[test]
    fn spec_bold_example_tokenizes() {
        // Appendix A: "There is a {\b1}bold {\b0}word here"
        let toks = tokenize("There is a {\\b1}bold {\\b0}word here");
        assert_eq!(
            toks,
            vec![
                AssToken::Text("There is a ".into()),
                AssToken::Override(vec![AssTag::Bold(Some(1))]),
                AssToken::Text("bold ".into()),
                AssToken::Override(vec![AssTag::Bold(Some(0))]),
                AssToken::Text("word here".into()),
            ]
        );
        roundtrip("There is a {\\b1}bold {\\b0}word here");
    }

    #[test]
    fn spec_italic_example_tokenizes() {
        // Appendix A: "There is an {\i1}italicised {\i0}word here"
        let toks = tokenize("There is an {\\i1}italicised {\\i0}word here");
        assert_eq!(
            toks[1],
            AssToken::Override(vec![AssTag::Italic(Some(true))])
        );
        assert_eq!(
            toks[3],
            AssToken::Override(vec![AssTag::Italic(Some(false))])
        );
        roundtrip("There is an {\\i1}italicised {\\i0}word here");
    }

    #[test]
    fn underline_and_strikeout_flags() {
        let toks = tokenize("{\\u1}u{\\u0}{\\s1}s{\\s0}");
        assert_eq!(
            toks[0],
            AssToken::Override(vec![AssTag::Underline(Some(true))])
        );
        assert_eq!(
            toks[2],
            AssToken::Override(vec![AssTag::Underline(Some(false))])
        );
        assert_eq!(
            toks[3],
            AssToken::Override(vec![AssTag::Strikeout(Some(true))])
        );
        roundtrip("{\\u1}u{\\u0}{\\s1}s{\\s0}");
    }

    #[test]
    fn bold_weight_forms_parse_to_weight() {
        // Aegisub reference example: {\b100}How {\b300}bold {\b500}can
        // {\b700}you {\b900}get?
        let s = "{\\b100}How {\\b300}bold {\\b500}can {\\b700}you {\\b900}get?";
        let toks = tokenize(s);
        assert_eq!(toks[0], AssToken::Override(vec![AssTag::Bold(Some(100))]));
        assert_eq!(toks[6], AssToken::Override(vec![AssTag::Bold(Some(700))]));
        assert_eq!(toks[8], AssToken::Override(vec![AssTag::Bold(Some(900))]));
        roundtrip(s);
    }

    #[test]
    fn parameterless_flags_are_reset_forms() {
        // "Any style modifier followed by no recognizable parameter
        // resets to the default."
        let toks = tokenize("{\\b\\i\\u\\s}x");
        assert_eq!(
            toks[0],
            AssToken::Override(vec![
                AssTag::Bold(None),
                AssTag::Italic(None),
                AssTag::Underline(None),
                AssTag::Strikeout(None),
            ])
        );
        roundtrip("{\\b\\i\\u\\s}x");
    }

    #[test]
    fn longer_tags_sharing_a_flag_prefix_stay_other() {
        // \bord, \be, \blur, \shad, \iclip must not be mistaken for
        // \b / \s / \i numeric forms.
        for (s, body) in [
            ("{\\bord3.7}", "bord3.7"),
            ("{\\be1}", "be1"),
            ("{\\blur2}", "blur2"),
            ("{\\shad2}", "shad2"),
            ("{\\iclip(0,0,100,100)}", "iclip(0,0,100,100)"),
        ] {
            assert_eq!(
                tokenize(s),
                vec![AssToken::Override(vec![AssTag::Other(body.into())])],
                "for {s:?}"
            );
            roundtrip(s);
        }
    }

    #[test]
    fn unrecognised_flag_parameter_stays_verbatim_other() {
        // \i2 is not a documented shape; preserved byte-for-byte.
        assert_eq!(
            tokenize("{\\i2}"),
            vec![AssToken::Override(vec![AssTag::Other("i2".into())])]
        );
        // \b1 followed by junk likewise.
        assert_eq!(
            tokenize("{\\b1junk}"),
            vec![AssToken::Override(vec![AssTag::Other("b1junk".into())])]
        );
        roundtrip("{\\i2}{\\b1junk}");
    }

    #[test]
    fn several_overrides_in_one_brace_set() {
        // "Several overrides can be used within one set of braces."
        let toks = tokenize("{\\b1\\i1}both{\\b0\\i0}");
        assert_eq!(
            toks[0],
            AssToken::Override(vec![AssTag::Bold(Some(1)), AssTag::Italic(Some(true))])
        );
        roundtrip("{\\b1\\i1}both{\\b0\\i0}");
    }

    #[test]
    fn complex_tag_with_nested_modifiers_is_one_other() {
        // \t's parameter list contains backslash modifiers; the whole
        // parenthesised body is one tag.
        let s = "{\\t(0,1000,\\fscx200\\fscy200)}grow";
        assert_eq!(
            tokenize(s)[0],
            AssToken::Override(vec![AssTag::Other("t(0,1000,\\fscx200\\fscy200)".into())])
        );
        roundtrip(s);
    }

    #[test]
    fn positioning_and_colour_tags_stay_verbatim() {
        let s = "{\\pos(320,240)\\1c&HFF&\\alpha&H80&\\fnCourier New\\fs28}x";
        assert_eq!(
            tokenize(s)[0],
            AssToken::Override(vec![
                AssTag::Other("pos(320,240)".into()),
                AssTag::Other("1c&HFF&".into()),
                AssTag::Other("alpha&H80&".into()),
                AssTag::Other("fnCourier New".into()),
                AssTag::Other("fs28".into()),
            ])
        );
        roundtrip(s);
    }

    #[test]
    fn karaoke_line_round_trips() {
        // Appendix A example: {\k94}This {\k48}is {\k24}a {\k150}karaoke
        roundtrip("{\\k94}This {\\k48}is {\\k24}a {\\k150}karaoke {\\k94}line");
    }

    #[test]
    fn comment_only_block_is_comment() {
        assert_eq!(
            tokenize("{just a comment}x"),
            vec![
                AssToken::Override(vec![AssTag::Comment("just a comment".into())]),
                AssToken::Text("x".into()),
            ]
        );
        roundtrip("{just a comment}x");
    }

    #[test]
    fn comment_mixed_with_tags_keeps_order() {
        let toks = tokenize("{note\\b1tail}");
        assert_eq!(
            toks[0],
            AssToken::Override(vec![
                AssTag::Comment("note".into()),
                AssTag::Other("b1tail".into()),
            ])
        );
        roundtrip("{note\\b1tail}");
    }

    #[test]
    fn empty_block_round_trips() {
        assert_eq!(tokenize("a{}b")[1], AssToken::Override(vec![]));
        roundtrip("a{}b");
    }

    #[test]
    fn escapes_outside_braces() {
        // Appendix A: "All Override codes appear within braces { }
        // except the newline \n and \N codes." \h per the Aegisub
        // reference is likewise mid-text.
        let toks = tokenize("first\\nsecond\\Nthird\\hfourth");
        assert_eq!(
            toks,
            vec![
                AssToken::Text("first".into()),
                AssToken::SoftBreak,
                AssToken::Text("second".into()),
                AssToken::HardBreak,
                AssToken::Text("third".into()),
                AssToken::HardSpace,
                AssToken::Text("fourth".into()),
            ]
        );
        roundtrip("first\\nsecond\\Nthird\\hfourth");
    }

    #[test]
    fn unrecognised_backslash_stays_literal_text() {
        let toks = tokenize("a\\zb\\");
        assert_eq!(toks, vec![AssToken::Text("a\\zb\\".into())]);
        roundtrip("a\\zb\\");
    }

    #[test]
    fn unterminated_block_is_literal_text() {
        let toks = tokenize("oops {\\b1 no close");
        assert_eq!(toks, vec![AssToken::Text("oops {\\b1 no close".into())]);
        roundtrip("oops {\\b1 no close");
    }

    #[test]
    fn multibyte_text_survives_around_blocks_and_escapes() {
        let s = "漢字{\\i1}みんな{\\i0}\\Né";
        let toks = tokenize(s);
        assert_eq!(toks[0], AssToken::Text("漢字".into()));
        assert_eq!(toks[2], AssToken::Text("みんな".into()));
        assert_eq!(toks[5], AssToken::Text("é".into()));
        roundtrip(s);
    }

    #[test]
    fn plain_text_drops_blocks_and_maps_breaks() {
        let toks = tokenize("{\\b1}Hello\\Nworld\\nagain\\hend{comment}");
        // Default / smart wrapping: \n is a space.
        assert_eq!(plain_text(&toks, None), "Hello\nworld again\u{00A0}end");
        assert_eq!(
            plain_text(&toks, Some(WrapStyle::SmartEven)),
            "Hello\nworld again\u{00A0}end"
        );
        // Wrapping mode 2: "Both \n and \N force line breaks."
        assert_eq!(
            plain_text(&toks, Some(WrapStyle::None)),
            "Hello\nworld\nagain\u{00A0}end"
        );
    }

    #[test]
    fn drawing_mode_block_round_trips() {
        // Appendix A drawing example shape kept verbatim through Other.
        let s = "{\\p1}m 0 0 l 100 0 100 100 0 100{\\p0}";
        let toks = tokenize(s);
        assert_eq!(
            toks[0],
            AssToken::Override(vec![AssTag::Other("p1".into())])
        );
        assert_eq!(
            toks[1],
            AssToken::Text("m 0 0 l 100 0 100 100 0 100".into())
        );
        roundtrip(s);
    }
}
