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
//! The typed layer covers the four boolean style flags (`\b`, `\i`,
//! `\u`, `\s` — the ones the IR `Segment` tree can model) and the
//! colour / alpha family (`\c`, `\1c`–`\4c`, `\alpha`, `\1a`–`\4a`).
//! Every other tag is preserved verbatim in [`AssTag::Other`], so
//! [`emit`] reproduces the original text byte-for-byte and no
//! information is dropped. Typed coverage of the remaining tag set
//! (positioning, karaoke, …) is follow-up material.

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

/// Which of the four colour / alpha components a `\<n>c` / `\<n>a`
/// override targets.
///
/// Per the Aegisub reference: `\1c` "sets the primary fill color",
/// `\2c` "sets the secondary fill color. This is only used for
/// pre-highlight in standard karaoke", `\3c` "sets the border color",
/// `\4c` "sets the shadow color" — and the `\1a`–`\4a` alpha tags
/// address the same four components.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssColorTarget {
    /// `\1c` / `\1a` — primary fill (also the `\c` abbreviation's
    /// target: "The `\c` tag is an abbreviation of `\1c`").
    Primary,
    /// `\2c` / `\2a` — secondary fill (standard-karaoke pre-highlight).
    Secondary,
    /// `\3c` / `\3a` — border.
    Border,
    /// `\4c` / `\4a` — shadow.
    Shadow,
}

/// One tag inside an override block.
///
/// The typed variants carry the parsed parameter; `None` is the
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
    /// `\c&H<bbggrr>&` / `\1c`–`\4c&H<bbggrr>&` — colour override.
    ///
    /// `hex` is the verbatim digit run between `&H` and the closing
    /// `&` (the spec: "Leading zeroes are not required", so `FF` is
    /// pure red) — decode it with [`decode_bgr_hex`]. `hex: None` is
    /// the parameterless reset-to-style form. `short` records whether
    /// the tag was written as the `\c` abbreviation of `\1c` so emit
    /// stays byte-stable; it is only ever set with
    /// [`AssColorTarget::Primary`].
    Color {
        /// Which fill component the override targets.
        target: AssColorTarget,
        /// `true` when written `\c` rather than `\1c`.
        short: bool,
        /// Verbatim `<bbggrr>` hex digits, or `None` for the reset form.
        hex: Option<String>,
    },
    /// `\alpha&H<aa>&` / `\1a`–`\4a&H<aa>&` — alpha override.
    ///
    /// `target: None` is the `\alpha` form, which "sets the alpha of
    /// all components at once" (per the SSA spec it "defaults to
    /// `\1a`"). `hex` is the verbatim digit run — decode it with
    /// [`decode_alpha_hex`]; "An alpha of 00 (zero) means
    /// opaque/fully visible, and an alpha of FF (ie. 255 in decimal)
    /// is fully transparent/invisible". `hex: None` is the
    /// parameterless reset-to-style form.
    Alpha {
        /// The component, or `None` for the all-components `\alpha`.
        target: Option<AssColorTarget>,
        /// Verbatim `<aa>` hex digits, or `None` for the reset form.
        hex: Option<String>,
    },
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
    if let Some(typed) = classify_color_alpha(tag) {
        return typed;
    }
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

/// Try the colour / alpha tag family: `\c`, `\1c`–`\4c`, `\alpha`,
/// `\1a`–`\4a`. Only the canonical `&H<hex>&` parameter shape (per the
/// Aegisub reference, "Color codes must always start with `&H` and end
/// with `&`") and the bare reset form are typed; anything else —
/// `\clip(...)`, a missing closing `&`, an over-long digit run —
/// returns `None` so the caller keeps it verbatim.
fn classify_color_alpha(tag: &str) -> Option<AssTag> {
    if let Some(rest) = tag.strip_prefix("alpha") {
        let hex = amp_hex_param(rest, 2)?;
        return Some(AssTag::Alpha { target: None, hex });
    }
    if let Some(rest) = tag.strip_prefix('c') {
        let hex = amp_hex_param(rest, 6)?;
        return Some(AssTag::Color {
            target: AssColorTarget::Primary,
            short: true,
            hex,
        });
    }
    let b = tag.as_bytes();
    if b.len() < 2 {
        return None;
    }
    let target = match b[0] {
        b'1' => AssColorTarget::Primary,
        b'2' => AssColorTarget::Secondary,
        b'3' => AssColorTarget::Border,
        b'4' => AssColorTarget::Shadow,
        _ => return None,
    };
    match b[1] {
        b'c' => {
            let hex = amp_hex_param(&tag[2..], 6)?;
            Some(AssTag::Color {
                target,
                short: false,
                hex,
            })
        }
        b'a' => {
            let hex = amp_hex_param(&tag[2..], 2)?;
            Some(AssTag::Alpha {
                target: Some(target),
                hex,
            })
        }
        _ => None,
    }
}

/// Match a colour / alpha tag's parameter. `""` is the parameterless
/// reset form (`Some(None)`); `&H<1..=max hex digits>&` yields the
/// verbatim digit run (`Some(Some(_))`); any other shape is `None` and
/// the whole tag stays an untyped [`AssTag::Other`].
fn amp_hex_param(rest: &str, max: usize) -> Option<Option<String>> {
    if rest.is_empty() {
        return Some(None);
    }
    let digits = rest.strip_prefix("&H")?.strip_suffix('&')?;
    if digits.is_empty() || digits.len() > max || !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    Some(Some(digits.to_string()))
}

/// Decode an [`AssTag::Color`] digit run into `(r, g, b)`.
///
/// "The color codes are given in hexadecimal in Blue Green Red order.
/// Note that this is the opposite order of HTML color codes." and
/// "Leading zeroes are not required" — so `"FF"` is pure red and
/// `"FF0000"` is pure blue. Returns `None` unless the run is 1..=6
/// ASCII hex digits.
pub fn decode_bgr_hex(hex: &str) -> Option<(u8, u8, u8)> {
    if hex.is_empty() || hex.len() > 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let v = u32::from_str_radix(hex, 16).ok()?;
    Some((
        (v & 0xFF) as u8,
        ((v >> 8) & 0xFF) as u8,
        ((v >> 16) & 0xFF) as u8,
    ))
}

/// Decode an [`AssTag::Alpha`] digit run: `00` is opaque, `FF` fully
/// transparent. Returns `None` unless the run is 1..=2 ASCII hex
/// digits.
pub fn decode_alpha_hex(hex: &str) -> Option<u8> {
    if hex.is_empty() || hex.len() > 2 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    u8::from_str_radix(hex, 16).ok()
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
                        AssTag::Color { target, short, hex } => {
                            out.push('\\');
                            if *short && *target == AssColorTarget::Primary {
                                out.push('c');
                            } else {
                                out.push(target_digit(*target));
                                out.push('c');
                            }
                            push_amp_hex(&mut out, hex);
                        }
                        AssTag::Alpha { target, hex } => {
                            out.push('\\');
                            match target {
                                None => out.push_str("alpha"),
                                Some(t) => {
                                    out.push(target_digit(*t));
                                    out.push('a');
                                }
                            }
                            push_amp_hex(&mut out, hex);
                        }
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

fn target_digit(target: AssColorTarget) -> char {
    match target {
        AssColorTarget::Primary => '1',
        AssColorTarget::Secondary => '2',
        AssColorTarget::Border => '3',
        AssColorTarget::Shadow => '4',
    }
}

fn push_amp_hex(out: &mut String, hex: &Option<String>) {
    if let Some(h) = hex {
        out.push_str("&H");
        out.push_str(h);
        out.push('&');
    }
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
    fn positioning_and_font_tags_stay_verbatim() {
        let s = "{\\pos(320,240)\\fnCourier New\\fs28}x";
        assert_eq!(
            tokenize(s)[0],
            AssToken::Override(vec![
                AssTag::Other("pos(320,240)".into()),
                AssTag::Other("fnCourier New".into()),
                AssTag::Other("fs28".into()),
            ])
        );
        roundtrip(s);
    }

    #[test]
    fn spec_colour_examples_type_and_decode() {
        // Appendix A examples: "{\c&HFF&}This is pure, full intensity
        // red" … "{\c&HA0A0A&}This is dark grey" — leading zeroes are
        // not required.
        for (s, hex, rgb) in [
            ("{\\c&HFF&}", "FF", (0xFF, 0, 0)),
            ("{\\c&HFF00&}", "FF00", (0, 0xFF, 0)),
            ("{\\c&HFF0000&}", "FF0000", (0, 0, 0xFF)),
            ("{\\c&HFFFFFF&}", "FFFFFF", (0xFF, 0xFF, 0xFF)),
            ("{\\c&HA0A0A&}", "A0A0A", (0x0A, 0x0A, 0x0A)),
        ] {
            assert_eq!(
                tokenize(s)[0],
                AssToken::Override(vec![AssTag::Color {
                    target: AssColorTarget::Primary,
                    short: true,
                    hex: Some(hex.into()),
                }]),
                "for {s:?}"
            );
            assert_eq!(decode_bgr_hex(hex), Some(rgb), "for {hex:?}");
            roundtrip(s);
        }
    }

    #[test]
    fn numbered_colour_tags_carry_their_target() {
        let toks = tokenize("{\\1c&H11&\\2c&H22&\\3c&H33&\\4c&H44&}x");
        assert_eq!(
            toks[0],
            AssToken::Override(vec![
                AssTag::Color {
                    target: AssColorTarget::Primary,
                    short: false,
                    hex: Some("11".into()),
                },
                AssTag::Color {
                    target: AssColorTarget::Secondary,
                    short: false,
                    hex: Some("22".into()),
                },
                AssTag::Color {
                    target: AssColorTarget::Border,
                    short: false,
                    hex: Some("33".into()),
                },
                AssTag::Color {
                    target: AssColorTarget::Shadow,
                    short: false,
                    hex: Some("44".into()),
                },
            ])
        );
        // \c abbreviates \1c, but the two spellings emit differently.
        roundtrip("{\\1c&H11&\\2c&H22&\\3c&H33&\\4c&H44&}x");
        roundtrip("{\\c&H11&}{\\1c&H11&}");
    }

    #[test]
    fn alpha_tags_type_and_decode() {
        // Aegisub reference examples: \alpha&H80& (50% transparent),
        // \1a&HFF& (invisible primary fill).
        let toks = tokenize("{\\alpha&H80&}a{\\1a&HFF&\\2a&H0&\\3a&H40&\\4a&HC0&}b");
        assert_eq!(
            toks[0],
            AssToken::Override(vec![AssTag::Alpha {
                target: None,
                hex: Some("80".into()),
            }])
        );
        assert_eq!(
            toks[2],
            AssToken::Override(vec![
                AssTag::Alpha {
                    target: Some(AssColorTarget::Primary),
                    hex: Some("FF".into()),
                },
                AssTag::Alpha {
                    target: Some(AssColorTarget::Secondary),
                    hex: Some("0".into()),
                },
                AssTag::Alpha {
                    target: Some(AssColorTarget::Border),
                    hex: Some("40".into()),
                },
                AssTag::Alpha {
                    target: Some(AssColorTarget::Shadow),
                    hex: Some("C0".into()),
                },
            ])
        );
        assert_eq!(decode_alpha_hex("80"), Some(0x80));
        assert_eq!(decode_alpha_hex("FF"), Some(0xFF));
        assert_eq!(decode_alpha_hex("0"), Some(0));
        roundtrip("{\\alpha&H80&}a{\\1a&HFF&\\2a&H0&\\3a&H40&\\4a&HC0&}b");
    }

    #[test]
    fn parameterless_colour_and_alpha_are_reset_forms() {
        // "Any style modifier followed by no recognizable parameter
        // resets to the default."
        let toks = tokenize("{\\c\\1c\\alpha\\2a}x");
        assert_eq!(
            toks[0],
            AssToken::Override(vec![
                AssTag::Color {
                    target: AssColorTarget::Primary,
                    short: true,
                    hex: None,
                },
                AssTag::Color {
                    target: AssColorTarget::Primary,
                    short: false,
                    hex: None,
                },
                AssTag::Alpha {
                    target: None,
                    hex: None,
                },
                AssTag::Alpha {
                    target: Some(AssColorTarget::Secondary),
                    hex: None,
                },
            ])
        );
        roundtrip("{\\c\\1c\\alpha\\2a}x");
    }

    #[test]
    fn off_shape_colour_parameters_stay_verbatim_other() {
        // Codes "must always start with &H and end with &" — anything
        // off-shape is preserved byte-for-byte, untyped. \clip shares
        // the \c prefix and must not be mistaken for a colour.
        for (s, body) in [
            ("{\\c&HFF}", "c&HFF"),                     // no closing &
            ("{\\cHFF&}", "cHFF&"),                     // no &H opener
            ("{\\c&H&}", "c&H&"),                       // empty digit run
            ("{\\c&HGG&}", "c&HGG&"),                   // non-hex digits
            ("{\\c&HFFFFFFF&}", "c&HFFFFFFF&"),         // 7 digits
            ("{\\alpha&H123&}", "alpha&H123&"),         // 3 digits
            ("{\\5c&HFF&}", "5c&HFF&"),                 // no 5th component
            ("{\\clip(0,0,10,10)}", "clip(0,0,10,10)"), // prefix cousin
        ] {
            assert_eq!(
                tokenize(s),
                vec![AssToken::Override(vec![AssTag::Other(body.into())])],
                "for {s:?}"
            );
            roundtrip(s);
        }
        assert_eq!(decode_bgr_hex("FFFFFFF"), None);
        assert_eq!(decode_bgr_hex(""), None);
        assert_eq!(decode_alpha_hex("123"), None);
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
