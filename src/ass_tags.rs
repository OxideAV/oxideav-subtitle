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
//! `\u`, `\s` — the ones the IR `Segment` tree can model), the
//! colour / alpha family (`\c`, `\1c`–`\4c`, `\alpha`, `\1a`–`\4a`),
//! the two alignment tags (`\an` numpad, `\a` legacy), the karaoke
//! family (`\k`, `\K`, `\kf`, `\ko`), the three line-positioning
//! functions (`\pos`, `\move`, `\org`), and the font-metric / rotation
//! family (`\fn`, `\fs`, `\fscx` / `\fscy`, `\fsp`, `\fe`, and
//! `\frx` / `\fry` / `\frz` plus the bare `\fr`), the border / shadow
//! family (`\bord` / `\xbord` / `\ybord`, `\shad` / `\xshad` /
//! `\yshad`), the edge-blur family (`\be` / `\blur`), the clip family
//! (`\clip` / `\iclip`), the fade family (`\fad` / `\fade`), and the
//! `\t(...)` animated-transform tag (its *style modifiers* parsed
//! recursively into nested [`AssTag`] values across all four documented
//! arities). Every other tag is preserved verbatim in
//! [`AssTag::Other`], so [`emit`] reproduces the original text
//! byte-for-byte and no information is dropped.

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

/// Which highlight effect a `\k`-family karaoke tag selects.
///
/// Per the Aegisub reference, "The `\k` family of tags mark up
/// subtitles for karaoke effects by specifying the duration of each
/// syllable" — "The duration is given in centiseconds, ie. a duration
/// of 100 is equivalent to 1 second."
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssKaraokeKind {
    /// `\k` — "Before highlight, the syllable is filled with the
    /// secondary color and alpha. When the syllable starts, the fill
    /// is instantly changed to use primary color and alpha."
    Instant,
    /// `\K` — sweep, uppercase spelling. "`\K` and `\kf`: These two
    /// are identical. Note that `\K` is an uppercase K and is
    /// different from lowercase `\k`." Kept distinct from
    /// [`AssKaraokeKind::Sweep`] so [`emit`] stays byte-stable.
    SweepCap,
    /// `\kf` — "the fill changes from secondary to primary with a
    /// sweep from left to right, so the sweep ends when the syllable
    /// time is over."
    Sweep,
    /// `\ko` — "Similar to `\k`, except that before highlight, the
    /// border/outline of the syllable is removed, and appears
    /// instantly when the syllable starts."
    Outline,
}

/// Which axis a `\fr`-family rotation override turns around.
///
/// Per the SSA spec's Appendix A, `\fr[<x/y/z>]<degrees>` "sets the
/// rotation angle around the x/y/z axis", and bare "`\fr` defaults to
/// `\frz`". The three explicit spellings are kept distinct from the
/// bare default-Z spelling (carried by [`AssTag::Rotation::bare`]) so
/// [`emit`] stays byte-stable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssRotationAxis {
    /// `\frx` — rotation around the X axis.
    X,
    /// `\fry` — rotation around the Y axis.
    Y,
    /// `\frz` (and the bare `\fr`, which "defaults to `\frz`") —
    /// rotation around the Z axis.
    Z,
}

/// Which axis a `\bord` / `\shad` border or shadow override applies to.
///
/// Per the Aegisub override-tag reference, the combined `\bord` /
/// `\shad` set both axes at once, while `\xbord` / `\ybord` "set the
/// border size in X and Y direction separately" and `\xshad` / `\yshad`
/// "set the distance ... with X and Y position set separately". The
/// per-axis spellings are kept distinct from the combined form (carried
/// by [`AssBorderAxis::Both`]) so [`emit`] stays byte-stable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssBorderAxis {
    /// Combined `\bord` / `\shad` — both axes at once.
    Both,
    /// `\xbord` / `\xshad` — the X direction only.
    X,
    /// `\ybord` / `\yshad` — the Y direction only.
    Y,
}

/// Which blur algorithm a `\be` / `\blur` override drives.
///
/// Per the Aegisub override-tag reference both blur the edges of the
/// text, but `\be`'s strength "is the number of times to apply the
/// regular effect" and "must be an integer number", while `\blur` "uses
/// a more advanced algorithm that looks better at high strengths" and
/// "Unlike `\be`, the strength can be non-integer here". The two
/// spellings are kept distinct so [`emit`] stays byte-stable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssBlurKind {
    /// `\be` — the integer-count edge-softening effect.
    Edge,
    /// `\blur` — the gaussian edge blur with a non-integer strength.
    Gaussian,
}

/// The clipped region carried by a `\clip` / `\iclip` override.
///
/// Per the Aegisub override-tag reference, `\clip` / `\iclip` take one
/// of two argument shapes. The rectangular form
/// `\clip(<x1>,<y1>,<x2>,<y2>)` defines an axis-aligned box — "only the
/// part of the line that is inside the rectangle is visible" — whose
/// coordinates "are given in script resolution pixels and are relative
/// to the top-left corner of the video. The coordinates must be
/// integers, there is no possibility to use non-integer coordinates."
/// The vector-drawing form `\clip(<drawing commands>)` /
/// `\clip(<scale>,<drawing commands>)` clips against an arbitrary shape:
/// "The drawing commands are drawing commands as those used with the
/// `\p` tag". "If the scale is not specified it is assumed to be 1
/// (one), meaning that coordinates correspond directly to pixels."
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AssClipShape {
    /// `\clip(<x1>,<y1>,<x2>,<y2>)` — axis-aligned rectangle in
    /// script-resolution pixels. The coordinates "must be integers".
    Rectangle {
        /// Top-left X.
        x1: i32,
        /// Top-left Y.
        y1: i32,
        /// Bottom-right X.
        x2: i32,
        /// Bottom-right Y.
        y2: i32,
    },
    /// `\clip(<drawing commands>)` / `\clip(<scale>,<drawing commands>)`
    /// — a vector-drawing clip path. `scale` carries the optional
    /// integer scale ("the scale works the same way as the scale for
    /// `\p` drawings"); `None` is the omitted form ("assumed to be 1").
    /// `commands` is the verbatim drawing-command run, preserved exactly
    /// as written so [`emit`] stays byte-stable.
    Drawing {
        /// Optional integer scale, or `None` for the unscaled form.
        scale: Option<u32>,
        /// Verbatim `\p`-style drawing commands.
        commands: String,
    },
}

/// The fade specification carried by a `\fad` / `\fade` override.
///
/// Per the Aegisub override-tag reference, the two fade tags are
/// mutually exclusive line-property tags. The simple form
/// `\fad(<fadein>,<fadeout>)` "produces a fade-in and fade-out effect.
/// The fadein and fadeout times are given in milliseconds"; either may
/// be 0 "to not have any fade effect on that end". The complex form
/// `\fade(<a1>,<a2>,<a3>,<t1>,<t2>,<t3>,<t4>)` performs "a five-part
/// fade using three alpha values … and four times": the alphas "are
/// given in decimal and are between 0 and 255, with 0 being fully
/// visible and 255 being invisible", and the times "are given in
/// milliseconds after the start of the line. All seven parameters are
/// required."
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssFadeSpec {
    /// `\fad(<fadein>,<fadeout>)` — a fade-in / fade-out pair, each a
    /// non-negative duration in milliseconds (0 disables that end).
    Simple {
        /// Fade-in duration in milliseconds.
        fadein: u32,
        /// Fade-out duration in milliseconds.
        fadeout: u32,
    },
    /// `\fade(<a1>,<a2>,<a3>,<t1>,<t2>,<t3>,<t4>)` — a five-part fade.
    /// "Before t1, the line has alpha a1. Between t1 and t2 the line
    /// fades from alpha a1 to alpha a2. Between t2 and t3 the line has
    /// alpha a2 constantly. Between t3 and t4 the line fades from alpha
    /// a2 to alpha a3. After t4 the line has alpha a3."
    Complex {
        /// Alpha before `t1` (0 fully visible … 255 invisible).
        a1: u8,
        /// Alpha held between `t2` and `t3`.
        a2: u8,
        /// Alpha after `t4`.
        a3: u8,
        /// First keyframe time in milliseconds after the line start.
        t1: u32,
        /// Second keyframe time in milliseconds.
        t2: u32,
        /// Third keyframe time in milliseconds.
        t3: u32,
        /// Fourth keyframe time in milliseconds.
        t4: u32,
    },
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
    /// `\an<1..=9>` — numpad alignment. Per the Aegisub reference,
    /// "The `\an` tag uses 'numpad' values for the pos, ie. the
    /// alignment values correspond to the positions of the digits on
    /// the numeric keypad" (1/2/3 bottom, 4/5/6 middle, 7/8/9 top,
    /// left-to-right within each row). "Only the first appearance
    /// counts." `None` is the parameterless reset-to-style form.
    AlignNumpad(Option<u8>),
    /// `\a<alignment>` — legacy SSA alignment. "Use 1 for
    /// left-alignment, 2 for center alignment and 3 for
    /// right-alignment"; "Adding 4 to the value specifies a
    /// 'Toptitle'. Adding 8 to the value specifies a 'Midtitle'."
    /// `Some(0)` is the explicit `\a0` spelling ("0 or nothing resets
    /// to the style default"), `None` the parameterless form; the
    /// other typed values are 1–3, 5–7, 9–11 (`\a4` / `\a8` are not
    /// documented shapes and stay verbatim). Convert with
    /// [`legacy_align_to_numpad`].
    AlignLegacy(Option<u8>),
    /// `\k` / `\K` / `\kf` / `\ko<duration>` — karaoke syllable
    /// timing; `duration` "is in hundredths of seconds" (the SSA
    /// spec) / "given in centiseconds" (the Aegisub reference). The
    /// undocumented `\kt` and the bare no-duration forms stay
    /// verbatim untyped.
    Karaoke {
        /// Which highlight effect the spelling selects.
        kind: AssKaraokeKind,
        /// Highlight duration in centiseconds.
        centisec: u32,
    },
    /// `\pos(<x>,<y>)` — line position. "The X and Y coordinates must
    /// be integers and are given in the script resolution coordinate
    /// system"; the line's alignment "is used as anchor point for the
    /// position". Per the SSA spec it "Defaults to
    /// `\move(<x>, <y>, <x>, <y>, 0, 0)`".
    Pos {
        /// Anchor X in script-resolution pixels.
        x: i32,
        /// Anchor Y in script-resolution pixels.
        y: i32,
    },
    /// `\move(<x1>,<y1>,<x2>,<y2>[,<t1>,<t2>])` — constant-speed line
    /// movement from `(x1, y1)` to `(x2, y2)` in script-resolution
    /// coordinates. `times` carries the optional `<t1>, <t2>`
    /// "Animation beginning, ending time offset [ms]" pair, relative
    /// to the line's start time; "Specifying both t1 and t2 as 0
    /// (zero) is the same as using the first version" but the two
    /// spellings are kept distinct so [`emit`] stays byte-stable.
    Move {
        /// Start X.
        x1: i32,
        /// Start Y.
        y1: i32,
        /// End X.
        x2: i32,
        /// End Y.
        y2: i32,
        /// Optional `(t1, t2)` animation window in milliseconds.
        times: Option<(u32, u32)>,
    },
    /// `\org(<x>,<y>)` — rotation origin. "Moves the default origin
    /// at (x,y)" (SSA spec); "Set the origin point used for rotation.
    /// This affects all rotations of the line. The X and Y
    /// coordinates are given in integer script resolution pixels"
    /// (Aegisub reference).
    Org {
        /// Origin X in script-resolution pixels.
        x: i32,
        /// Origin Y in script-resolution pixels.
        y: i32,
    },
    /// `\fn<font name>` — font family override. Per the SSA spec's
    /// Appendix A: "`<font name>` specifies a font which you have
    /// installed… This is case sensitive." The parameter is an
    /// arbitrary verbatim string (font names can carry spaces, e.g.
    /// `\fnCourier New`), kept exactly as written so [`emit`] is
    /// byte-stable. `None` is the parameterless `\fn` reset-to-style
    /// form ("Any style modifier followed by no recognizable parameter
    /// resets to the default").
    FontName(Option<String>),
    /// `\fs<font size>` — font point size. "`<font size>` is a number
    /// specifying a font point size." The verbatim digit/decimal run is
    /// preserved as a string (sizes are commonly fractional, e.g.
    /// `\fs28.5`) so emit stays byte-stable; decode it with
    /// [`decode_decimal`]. `None` is the parameterless reset-to-style
    /// form.
    FontSize(Option<String>),
    /// `\fscx<percent>` / `\fscy<percent>` — per-axis font scale.
    /// "`<x or y>` x scales horizontally, y scales vertically." The
    /// verbatim percent run is preserved (scale is commonly fractional
    /// or above 100, e.g. `\fscx200`); decode with [`decode_decimal`].
    /// `None` is the parameterless reset-to-style form.
    FontScale {
        /// `true` for `\fscx` (horizontal), `false` for `\fscy`
        /// (vertical).
        x_axis: bool,
        /// Verbatim percent run, or `None` for the reset form.
        percent: Option<String>,
    },
    /// `\fsp<pixels>` — letter spacing. "`<pixels>` changes the distance
    /// between letters. (default: 0)." The verbatim run is preserved
    /// (spacing may be fractional or negative, e.g. `\fsp-2`); decode
    /// with [`decode_decimal`]. `None` is the parameterless
    /// reset-to-style form.
    FontSpacing(Option<String>),
    /// `\fe<charset>` — font encoding / character set. "`<charset>` is a
    /// number specifying the character set (font encoding)." The
    /// verbatim digit run is preserved; `None` is the parameterless
    /// reset-to-style form.
    FontEncoding(Option<String>),
    /// `\fr[<x/y/z>]<degrees>` — rotation. "`<degrees>` sets the
    /// rotation angle around the x/y/z axis. `\fr` defaults to `\frz`."
    /// The verbatim degrees run is preserved (angles are commonly
    /// fractional or negative, e.g. `\frz-30.5`); decode with
    /// [`decode_decimal`]. `degrees: None` is the parameterless
    /// reset-to-style form. `bare` records the `\fr`-is-`\frz`
    /// abbreviation so emit stays byte-stable; it is only ever set with
    /// [`AssRotationAxis::Z`].
    Rotation {
        /// Which axis the rotation turns around.
        axis: AssRotationAxis,
        /// `true` when written bare `\fr` rather than `\frz`.
        bare: bool,
        /// Verbatim degrees run, or `None` for the reset form.
        degrees: Option<String>,
    },
    /// `\bord<size>` / `\xbord<size>` / `\ybord<size>` — border width.
    /// The Aegisub reference: "Change the width of the border around the
    /// text. Set the size to 0 (zero) to disable the border entirely",
    /// the value "doesn't have to be limited to whole integer pixels and
    /// can have decimal places", and "Border width cannot be negative".
    /// `\xbord` / `\ybord` "set the border size in X and Y direction
    /// separately". The verbatim run is preserved (widths are commonly
    /// fractional, e.g. `\bord3.7`); decode with [`decode_decimal`].
    /// Because the spec bars a negative border, a `-`-signed run is left
    /// untyped as [`AssTag::Other`]. `None` is the parameterless
    /// reset-to-style form.
    Border {
        /// Which axis the width applies to.
        axis: AssBorderAxis,
        /// Verbatim non-negative width run, or `None` for the reset form.
        size: Option<String>,
    },
    /// `\shad<depth>` / `\xshad<depth>` / `\yshad<depth>` — shadow
    /// distance. The Aegisub reference: "Set the distance from the text
    /// to position the shadow. Set the depth to 0 (zero) to disable
    /// shadow entirely." The combined `\shad` "distance can not be
    /// negative with this tag", but for the per-axis forms "unlike
    /// `\shad`, you can set the distance negative with these tags to
    /// position the shadow to the top or left of the text". The verbatim
    /// run is preserved (depths are commonly fractional); decode with
    /// [`decode_decimal`]. A `-`-signed run is therefore typed only for
    /// `\xshad` / `\yshad`; a negative `\shad` stays [`AssTag::Other`].
    /// `None` is the parameterless reset-to-style form.
    Shadow {
        /// Which axis the depth applies to.
        axis: AssBorderAxis,
        /// Verbatim depth run, or `None` for the reset form.
        depth: Option<String>,
    },
    /// `\be<strength>` / `\blur<strength>` — edge blur. The Aegisub
    /// reference: `\be` "Enable or disable a subtle softening-effect for
    /// the edges of the text … In the extended version, strength is the
    /// number of times to apply the regular effect … The strength must be
    /// an integer number." `\blur` "In general, this has the same
    /// function as the `\be` tag, but uses a more advanced algorithm …
    /// Unlike `\be`, the strength can be non-integer here. Set strength to
    /// 0 (zero) to disable the effect."
    ///
    /// Both blur the text edges; the distinct algorithms (`\be`'s
    /// integer-count softening vs `\blur`'s gaussian) are carried by
    /// [`AssBlurKind`] so [`emit`] stays byte-stable. The verbatim run is
    /// preserved — a `\be` strength is an integer count and a `\blur`
    /// strength a non-negative decimal; neither is meaningfully negative,
    /// so a `-`-signed value stays an untyped [`AssTag::Other`]. Decode a
    /// `\blur` strength with [`decode_decimal`]. `None` is the
    /// parameterless reset-to-style form.
    Blur {
        /// Which blur algorithm the strength drives.
        kind: AssBlurKind,
        /// Verbatim strength run, or `None` for the reset form.
        strength: Option<String>,
    },
    /// `\clip(...)` / `\iclip(...)` — clip the line to (or, for
    /// `\iclip`, away from) a region. Per the Aegisub override-tag
    /// reference `\clip` keeps "only the part of the line that is inside
    /// the rectangle", while "the `\iclip` tag has the opposite effect,
    /// it defines a rectangle where the line is not shown"; the same
    /// inverse relationship holds for the vector-drawing forms. The
    /// region shape is carried by [`AssClipShape`].
    ///
    /// `\clip` / `\iclip` are line-property tags ("Tags in the first
    /// category should appear at most once in a line"); the two are
    /// mutually exclusive. Only the documented argument shapes are typed
    /// — a rectangle of four canonical integers, a one-argument vector
    /// drawing, or a `<scale>,<drawing>` pair whose scale is a canonical
    /// integer. Any off-shape spelling (a non-integer rectangle
    /// coordinate, a two-coordinate argument list, trailing text after
    /// the closing parenthesis) stays an untyped [`AssTag::Other`] so
    /// [`emit`] is byte-stable.
    Clip {
        /// `true` for `\iclip` (hide inside the region), `false` for
        /// `\clip` (show only inside the region).
        inverse: bool,
        /// The clipped region.
        shape: AssClipShape,
    },
    /// `\fad(...)` / `\fade(...)` — a fade animation applied to the
    /// whole line. Per the Aegisub override-tag reference both are
    /// line-property tags ("Tags in the first category should appear at
    /// most once in a line") and the two are mutually exclusive. The
    /// simple `\fad(<fadein>,<fadeout>)` and complex
    /// `\fade(<a1>,<a2>,<a3>,<t1>,<t2>,<t3>,<t4>)` shapes are carried by
    /// [`AssFadeSpec`].
    ///
    /// Only the documented argument shapes are typed — two non-negative
    /// integer milliseconds for `\fad`, or three 0–255 alphas plus four
    /// non-negative integer milliseconds for `\fade`. Any off-shape
    /// spelling (wrong arity, a signed or non-integer value, an alpha
    /// above 255, trailing text after the closing parenthesis) stays an
    /// untyped [`AssTag::Other`] so [`emit`] is byte-stable.
    Fade(AssFadeSpec),
    /// `\t(...)` — a gradual, animated transformation from one style to
    /// another. Per the Aegisub override-tag reference the *style
    /// modifiers* "are other override tags as specified in this
    /// reference"; they are parsed recursively into [`AssTag`] values
    /// (`modifiers`) so a `\t(...)` carrying nested overrides round-trips
    /// byte-stably through the same per-tag emitter.
    ///
    /// The four documented arities map onto the optional fields:
    ///
    /// * `\t(<modifiers>)` — `t1`/`t2`/`accel` all `None`.
    /// * `\t(<accel>,<modifiers>)` — `accel` set, `t1`/`t2` `None`.
    /// * `\t(<t1>,<t2>,<modifiers>)` — `t1`/`t2` set, `accel` `None`.
    /// * `\t(<t1>,<t2>,<accel>,<modifiers>)` — all set.
    ///
    /// `t1`/`t2` are non-negative integer millisecond times "relative to
    /// the start time of the line" and are always present or absent
    /// together. `accel` is a non-negative decimal kept verbatim as a
    /// string (it "can be non-integer"; `1` is linear) — decode it with
    /// [`decode_decimal`]. Any off-shape spelling — a `\t()` with no
    /// modifiers, a wrong leading-argument arity, a signed / non-integer
    /// time, a negative or non-canonical accel, trailing text after the
    /// closing parenthesis — stays an untyped [`AssTag::Other`] so
    /// [`emit`] is byte-stable.
    Transform {
        /// First keyframe time in milliseconds, or `None` for the
        /// no-time arities (the transform runs over the whole line).
        t1: Option<u32>,
        /// Second keyframe time in milliseconds; present iff `t1` is.
        t2: Option<u32>,
        /// Optional acceleration exponent, verbatim (`1` is linear).
        accel: Option<String>,
        /// The animated *style modifiers*, parsed recursively.
        modifiers: Vec<AssTag>,
    },
    /// `\p<0/1/..>` — toggle drawing mode. Per the Aegisub reference,
    /// "Setting this tag to 1 or above enables drawing mode. Text after
    /// this override block will then be interpreted as drawing
    /// instructions, and not as actually visible text. Setting this to
    /// zero disables drawing mode". "When turning on, the value might be
    /// any integer larger than zero, and will be interpreted as the
    /// scale, in `2^(value-1)` mode" — so `\p2` halves the coordinate
    /// resolution and `\p4` scales by 8. `\p0` disables drawing.
    ///
    /// The field is the raw non-negative integer (`0` = off); convert it
    /// to the coordinate divisor with [`drawing_scale_divisor`]. The bare
    /// `\p` form carries no documented level and stays verbatim untyped
    /// ([`AssTag::Other`]) — only a canonical digit run types here so
    /// [`emit`] is byte-stable.
    Drawing(u32),
    /// `\pbo<y>` — baseline offset. Per the Aegisub reference, "Defines
    /// baseline offset for drawing. This is basically an Y offset to all
    /// coordinates" — `\pbo-50` draws 50 pixels above and `\pbo100` 100
    /// below. The field is a canonical signed integer. The bare `\pbo`
    /// form carries no documented value and stays verbatim untyped
    /// ([`AssTag::Other`]) so [`emit`] is byte-stable.
    BaselineOffset(i32),
    /// `\r` / `\r<style>` — style reset. Per the Aegisub reference,
    /// "Reset the style. This cancels all style overrides in effect,
    /// including animations, for all following text." "The first form
    /// that does not specify a style will reset to the style defined for
    /// the entire line, while the second form, that specifies the name of
    /// a style, will reset the style to that specific style."
    ///
    /// `None` is the bare `\r` (reset to the line's own style); `Some(s)`
    /// carries the verbatim named-style argument (which may contain
    /// spaces, e.g. `\rAlternate`). The name rides through verbatim so
    /// [`emit`] stays byte-stable.
    Reset(Option<String>),
    /// Any other tag, kept verbatim — the full body after the
    /// backslash, including parenthesised parameter lists (a `\fad` /
    /// `\fade` whose arguments fall outside the typed shape, a `\t(...)`
    /// whose argument shape isn't one of the four documented arities,
    /// …).
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
    if let Some(typed) = classify_position_karaoke(tag) {
        return typed;
    }
    if let Some(typed) = classify_font(tag) {
        return typed;
    }
    if let Some(typed) = classify_border_shadow(tag) {
        return typed;
    }
    if let Some(typed) = classify_blur(tag) {
        return typed;
    }
    if let Some(typed) = classify_clip(tag) {
        return typed;
    }
    if let Some(typed) = classify_fade(tag) {
        return typed;
    }
    if let Some(typed) = classify_transform(tag) {
        return typed;
    }
    if let Some(typed) = classify_drawing(tag) {
        return typed;
    }
    if let Some(typed) = classify_reset(tag) {
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

/// Try the alignment / karaoke / positioning tag family: `\an`, `\a`,
/// `\k` / `\K` / `\kf` / `\ko`, `\pos(...)`, `\move(...)`,
/// `\org(...)`. Only canonically-spelled parameters are typed (see
/// [`canon_i32`]); off-shape arities, signs, spacing, and the
/// undocumented cousins (`\kt`, `\a4` / `\a8`, `\an0`) return `None`
/// so the caller keeps them verbatim.
fn classify_position_karaoke(tag: &str) -> Option<AssTag> {
    if let Some(rest) = tag.strip_prefix("an") {
        if rest.is_empty() {
            return Some(AssTag::AlignNumpad(None));
        }
        let v = canon_u8(rest)?;
        return (1..=9).contains(&v).then_some(AssTag::AlignNumpad(Some(v)));
    }
    if let Some(args) = paren_args(tag, "pos") {
        let (x, y) = args.split_once(',')?;
        return Some(AssTag::Pos {
            x: canon_i32(x)?,
            y: canon_i32(y)?,
        });
    }
    if let Some(args) = paren_args(tag, "org") {
        let (x, y) = args.split_once(',')?;
        return Some(AssTag::Org {
            x: canon_i32(x)?,
            y: canon_i32(y)?,
        });
    }
    if let Some(args) = paren_args(tag, "move") {
        let p: Vec<&str> = args.split(',').collect();
        let times = match p.len() {
            4 => None,
            6 => Some((canon_u32(p[4])?, canon_u32(p[5])?)),
            _ => return None,
        };
        return Some(AssTag::Move {
            x1: canon_i32(p[0])?,
            y1: canon_i32(p[1])?,
            x2: canon_i32(p[2])?,
            y2: canon_i32(p[3])?,
            times,
        });
    }
    let (kind, arg) = if let Some(r) = tag.strip_prefix("kf") {
        (AssKaraokeKind::Sweep, r)
    } else if let Some(r) = tag.strip_prefix("ko") {
        (AssKaraokeKind::Outline, r)
    } else if let Some(r) = tag.strip_prefix('k') {
        (AssKaraokeKind::Instant, r)
    } else if let Some(r) = tag.strip_prefix('K') {
        (AssKaraokeKind::SweepCap, r)
    } else if let Some(rest) = tag.strip_prefix('a') {
        // Legacy \a. \alpha was consumed by the colour / alpha family
        // and \an by the arm above, so `rest` here is the bare or
        // numeric legacy form (or some unrelated a-prefixed tag).
        if rest.is_empty() {
            return Some(AssTag::AlignLegacy(None));
        }
        let v = canon_u8(rest)?;
        return matches!(v, 0..=3 | 5..=7 | 9..=11).then_some(AssTag::AlignLegacy(Some(v)));
    } else {
        return None;
    };
    // A karaoke tag with no duration is not a documented reset shape
    // (the duration has no style default to reset to), so the bare
    // forms fall through verbatim via canon_u32's None.
    Some(AssTag::Karaoke {
        kind,
        centisec: canon_u32(arg)?,
    })
}

/// Try the font-metric / rotation tag family: `\fn`, `\fs`, `\fscx`,
/// `\fscy`, `\fsp`, `\fe`, `\frx` / `\fry` / `\frz`, and bare `\fr`
/// (which "defaults to `\frz`").
///
/// `\fn`'s parameter is an arbitrary verbatim string (font names carry
/// spaces); the others take a canonical decimal run validated by
/// [`canon_decimal`] so a spelling that wouldn't re-emit identically
/// (embedded space, `+` sign, a `%` the spec doesn't use, a trailing
/// `.`) stays an untyped [`AssTag::Other`] and [`emit`] is byte-stable.
/// The empty parameter is the documented reset-to-style form in every
/// case.
///
/// Order matters: `\fsc*` and `\fsp` are checked before `\fs`, and the
/// axis-suffixed `\fr*` before bare `\fr`, because the shorter name is
/// a prefix of the longer.
fn classify_font(tag: &str) -> Option<AssTag> {
    if let Some(rest) = tag.strip_prefix("fn") {
        // Font name: arbitrary verbatim string, empty = reset.
        return Some(AssTag::FontName(
            (!rest.is_empty()).then(|| rest.to_string()),
        ));
    }
    if let Some(rest) = tag.strip_prefix("fscx") {
        return Some(AssTag::FontScale {
            x_axis: true,
            percent: canon_decimal_opt(rest)?,
        });
    }
    if let Some(rest) = tag.strip_prefix("fscy") {
        return Some(AssTag::FontScale {
            x_axis: false,
            percent: canon_decimal_opt(rest)?,
        });
    }
    if let Some(rest) = tag.strip_prefix("fsp") {
        return Some(AssTag::FontSpacing(canon_decimal_opt(rest)?));
    }
    if let Some(rest) = tag.strip_prefix("fs") {
        return Some(AssTag::FontSize(canon_decimal_opt(rest)?));
    }
    if let Some(rest) = tag.strip_prefix("fe") {
        return Some(AssTag::FontEncoding(canon_decimal_opt(rest)?));
    }
    if let Some(rest) = tag.strip_prefix("frx") {
        return Some(AssTag::Rotation {
            axis: AssRotationAxis::X,
            bare: false,
            degrees: canon_decimal_opt(rest)?,
        });
    }
    if let Some(rest) = tag.strip_prefix("fry") {
        return Some(AssTag::Rotation {
            axis: AssRotationAxis::Y,
            bare: false,
            degrees: canon_decimal_opt(rest)?,
        });
    }
    if let Some(rest) = tag.strip_prefix("frz") {
        return Some(AssTag::Rotation {
            axis: AssRotationAxis::Z,
            bare: false,
            degrees: canon_decimal_opt(rest)?,
        });
    }
    if let Some(rest) = tag.strip_prefix("fr") {
        return Some(AssTag::Rotation {
            axis: AssRotationAxis::Z,
            bare: true,
            degrees: canon_decimal_opt(rest)?,
        });
    }
    None
}

/// Try the border / shadow tag family: `\bord`, `\xbord`, `\ybord`,
/// `\shad`, `\xshad`, `\yshad`.
///
/// Per the Aegisub reference, border widths "cannot be negative" and
/// the combined `\shad` "distance can not be negative with this tag",
/// so those forms only accept a non-negative decimal run
/// ([`canon_decimal_nonneg_opt`]); a `-`-signed value stays an untyped
/// [`AssTag::Other`]. The per-axis shadow forms `\xshad` / `\yshad`
/// "can set the distance negative", so they accept the signed
/// [`canon_decimal_opt`] run. The empty parameter is the documented
/// reset-to-style form in every case.
///
/// Order matters: the axis-prefixed `\xbord` / `\ybord` /
/// `\xshad` / `\yshad` must be checked before the bare `\bord` /
/// `\shad`, because `b`/`s` is not a prefix of `xb`/`xs` but the
/// `\b`/`\s` style toggles handled by [`classify`] would otherwise be
/// shadowed — so this family runs first for any `b`/`x`/`y`/`s` lead and
/// returns `None` (deferring to the toggle path) when nothing matches.
fn classify_border_shadow(tag: &str) -> Option<AssTag> {
    if let Some(rest) = tag.strip_prefix("xbord") {
        return Some(AssTag::Border {
            axis: AssBorderAxis::X,
            size: canon_decimal_nonneg_opt(rest)?,
        });
    }
    if let Some(rest) = tag.strip_prefix("ybord") {
        return Some(AssTag::Border {
            axis: AssBorderAxis::Y,
            size: canon_decimal_nonneg_opt(rest)?,
        });
    }
    if let Some(rest) = tag.strip_prefix("bord") {
        return Some(AssTag::Border {
            axis: AssBorderAxis::Both,
            size: canon_decimal_nonneg_opt(rest)?,
        });
    }
    if let Some(rest) = tag.strip_prefix("xshad") {
        return Some(AssTag::Shadow {
            axis: AssBorderAxis::X,
            depth: canon_decimal_opt(rest)?,
        });
    }
    if let Some(rest) = tag.strip_prefix("yshad") {
        return Some(AssTag::Shadow {
            axis: AssBorderAxis::Y,
            depth: canon_decimal_opt(rest)?,
        });
    }
    if let Some(rest) = tag.strip_prefix("shad") {
        return Some(AssTag::Shadow {
            axis: AssBorderAxis::Both,
            depth: canon_decimal_nonneg_opt(rest)?,
        });
    }
    None
}

/// Try the edge-blur tag family: `\blur` and `\be`.
///
/// Per the Aegisub reference a `\be` strength "must be an integer
/// number" (a non-negative repeat count), so `\be` accepts only a
/// canonical unsigned-integer run ([`canon_uint_opt`]); a `\blur`
/// strength "can be non-integer" but is never meaningfully negative, so
/// `\blur` accepts a non-negative decimal ([`canon_decimal_nonneg_opt`]).
/// A spelling outside those shapes — a `-` sign, a `\be1.5`, a trailing
/// `.`, an embedded space — stays an untyped [`AssTag::Other`] and
/// [`emit`] is byte-stable. The empty parameter is the documented
/// reset-to-style form in both cases.
///
/// Order matters: `\blur` is checked before `\be` (the names share only
/// the leading `b`, so no collision, but the longer match is taken
/// first), and both run after the `\bord` family in [`classify`] so
/// `\bord` is never mistaken for a `\b`-toggle.
fn classify_blur(tag: &str) -> Option<AssTag> {
    if let Some(rest) = tag.strip_prefix("blur") {
        return Some(AssTag::Blur {
            kind: AssBlurKind::Gaussian,
            strength: canon_decimal_nonneg_opt(rest)?,
        });
    }
    if let Some(rest) = tag.strip_prefix("be") {
        return Some(AssTag::Blur {
            kind: AssBlurKind::Edge,
            strength: canon_uint_opt(rest)?,
        });
    }
    None
}

/// Try the clip tag family: `\clip(...)` and `\iclip(...)`.
///
/// Per the Aegisub override-tag reference these come in two argument
/// shapes — a rectangle `(<x1>,<y1>,<x2>,<y2>)` of four integers
/// ("the coordinates must be integers"), or a vector drawing
/// `(<drawing commands>)` / `(<scale>,<drawing commands>)` whose
/// optional leading scale "works the same way as the scale for `\p`
/// drawings". The argument list is disambiguated by its top-level comma
/// count: four comma-separated canonical integers is the rectangle;
/// a single argument is an unscaled drawing; a leading canonical
/// integer followed by one comma and the rest is a scaled drawing. The
/// drawing commands themselves are space-separated `\p` operations
/// (`m 50 0 b 100 0 …`) with no top-level comma, so the split is
/// unambiguous and the command run rides through verbatim.
///
/// Any spelling outside those shapes — a non-integer rectangle
/// coordinate, a two- or three-coordinate argument list, a non-integer
/// scale, trailing text after the closing parenthesis — returns `None`
/// so the caller keeps it as an untyped [`AssTag::Other`] and [`emit`]
/// stays byte-stable.
fn classify_clip(tag: &str) -> Option<AssTag> {
    let (inverse, args) = if let Some(a) = paren_args(tag, "iclip") {
        (true, a)
    } else if let Some(a) = paren_args(tag, "clip") {
        (false, a)
    } else {
        return None;
    };
    // A rectangle is exactly four canonical integers. The drawing-command
    // run never contains a top-level comma, so a four-comma split that
    // parses as four integers is unambiguously the rectangle form.
    let parts: Vec<&str> = args.split(',').collect();
    if parts.len() == 4 {
        return Some(AssTag::Clip {
            inverse,
            shape: AssClipShape::Rectangle {
                x1: canon_i32(parts[0])?,
                y1: canon_i32(parts[1])?,
                x2: canon_i32(parts[2])?,
                y2: canon_i32(parts[3])?,
            },
        });
    }
    // The scaled drawing form `<scale>,<drawing commands>`: a canonical
    // integer scale, one comma, then the verbatim command run. Split
    // only on the first comma. The command run must actually look like a
    // `\p` drawing (every drawing starts with a command letter such as
    // `m`/`l`/`b`); requiring an ASCII letter keeps a bare two-integer
    // coordinate pair like `(50,50)` out of the scaled-drawing arm, so
    // an undocumented two-coordinate list stays an untyped
    // [`AssTag::Other`].
    if parts.len() == 2 {
        if let Some((scale, commands)) = args.split_once(',') {
            if is_drawing_commands(commands) {
                let scale = canon_u32(scale)?;
                return Some(AssTag::Clip {
                    inverse,
                    shape: AssClipShape::Drawing {
                        scale: Some(scale),
                        commands: commands.to_string(),
                    },
                });
            }
        }
        return None;
    }
    // A single argument is an unscaled vector drawing, provided it looks
    // like a `\p` drawing. An empty argument list, or any other comma
    // arity (three coordinates, five-plus values), is not a documented
    // shape and stays verbatim.
    if parts.len() == 1 && is_drawing_commands(args) {
        return Some(AssTag::Clip {
            inverse,
            shape: AssClipShape::Drawing {
                scale: None,
                commands: args.to_string(),
            },
        });
    }
    None
}

/// Does `s` plausibly hold `\p`-style drawing commands? Every drawing
/// begins with a command letter (`m` move, `l` line, `b` bezier, `n`,
/// `s` spline, `p`, `c`), so a run that contains at least one ASCII
/// letter is treated as a drawing while a pure-numeric run (which would
/// be an undocumented bare coordinate list) is not.
fn is_drawing_commands(s: &str) -> bool {
    s.bytes().any(|b| b.is_ascii_alphabetic())
}

/// Try the fade tag family: `\fad(<fadein>,<fadeout>)` and the complex
/// `\fade(<a1>,<a2>,<a3>,<t1>,<t2>,<t3>,<t4>)`.
///
/// The simple form takes exactly two non-negative integer millisecond
/// values (per the Aegisub reference the times "are given in
/// milliseconds" and either may be 0). The complex form takes exactly
/// seven values: three alpha bytes ("between 0 and 255") followed by
/// four non-negative integer millisecond times. Both are canonically
/// spelled via [`canon_u32`] / [`canon_u8`], so any value that wouldn't
/// re-emit identically — a sign, a decimal point, a leading zero, an
/// alpha above 255 — returns `None` and the caller keeps the whole tag
/// as an untyped [`AssTag::Other`] so [`emit`] is byte-stable. The
/// `\fade` arm is tried first: a `\fade(...)` body has prefix `fade`,
/// which `paren_args(_, "fad")` rejects, but checking the longer name
/// first keeps the intent explicit.
fn classify_fade(tag: &str) -> Option<AssTag> {
    if let Some(args) = paren_args(tag, "fade") {
        let p: Vec<&str> = args.split(',').collect();
        if p.len() != 7 {
            return None;
        }
        return Some(AssTag::Fade(AssFadeSpec::Complex {
            a1: canon_u8(p[0])?,
            a2: canon_u8(p[1])?,
            a3: canon_u8(p[2])?,
            t1: canon_u32(p[3])?,
            t2: canon_u32(p[4])?,
            t3: canon_u32(p[5])?,
            t4: canon_u32(p[6])?,
        }));
    }
    if let Some(args) = paren_args(tag, "fad") {
        let (fadein, fadeout) = args.split_once(',')?;
        return Some(AssTag::Fade(AssFadeSpec::Simple {
            fadein: canon_u32(fadein)?,
            fadeout: canon_u32(fadeout)?,
        }));
    }
    None
}

/// Try the animated-transform tag `\t(...)`.
///
/// Per the Aegisub override-tag reference `\t` has four arities:
/// `\t(<modifiers>)`, `\t(<accel>,<modifiers>)`,
/// `\t(<t1>,<t2>,<modifiers>)`, and `\t(<t1>,<t2>,<accel>,<modifiers>)`.
/// The leading numeric arguments are separated from the *style
/// modifiers* by the first top-level backslash (the modifiers "are
/// other override tags"), so the argument list is split there: the
/// prefix before the first `\` is the comma-separated leading numbers,
/// and the remainder (which must start with `\`) is the modifiers run.
///
/// `t1`/`t2` are canonical non-negative integer milliseconds (via
/// [`canon_u32`]); `accel` is a canonical non-negative decimal (via
/// [`canon_decimal`], no leading `-`) kept verbatim. The modifiers are
/// parsed recursively with [`parse_block`]. Any off-shape spelling — no
/// modifiers at all, a leading-argument count other than 0/1/2/3, a
/// signed or non-integer time, a non-canonical or negative accel — makes
/// the whole tag stay an untyped [`AssTag::Other`] so [`emit`] is
/// byte-stable. The match is the exact `t(` prefix so the `\t` toggle
/// can't be confused with the typed `\fs` / `\fad` families above.
fn classify_transform(tag: &str) -> Option<AssTag> {
    let args = paren_args(tag, "t")?;
    // The style modifiers begin at the first backslash; everything
    // before it is the comma-separated leading numeric arguments.
    let split = args.find('\\')?;
    let lead = &args[..split];
    let mods_str = &args[split..];
    let modifiers = parse_block(mods_str);
    // A bare `\t(\fs20)` has an empty lead; otherwise the lead is a
    // trailing-comma-terminated list of 1..=3 numbers.
    let (t1, t2, accel) = if lead.is_empty() {
        (None, None, None)
    } else {
        let lead = lead.strip_suffix(',')?;
        let parts: Vec<&str> = lead.split(',').collect();
        match parts.as_slice() {
            // \t(<accel>,<modifiers>)
            [a] => (None, None, Some(canon_nonneg_decimal(a)?)),
            // \t(<t1>,<t2>,<modifiers>)
            [a, b] => (Some(canon_u32(a)?), Some(canon_u32(b)?), None),
            // \t(<t1>,<t2>,<accel>,<modifiers>)
            [a, b, c] => (
                Some(canon_u32(a)?),
                Some(canon_u32(b)?),
                Some(canon_nonneg_decimal(c)?),
            ),
            _ => return None,
        }
    };
    Some(AssTag::Transform {
        t1,
        t2,
        accel,
        modifiers,
    })
}

/// Try the drawing-mode tag family: `\p<0/1/..>` (toggle drawing mode)
/// and `\pbo<y>` (baseline offset).
///
/// `\pbo` is matched ahead of `\p` because the shorter name is a prefix
/// of the longer; `\pos` / `\pos(...)` were already consumed by the
/// positioning family above, so the `\p` arm here only sees a digit run.
/// `\p<level>` takes a canonical non-negative integer (the level "might
/// be any integer larger than zero" plus the `\p0` off form); `\pbo<y>`
/// takes a canonical signed integer (the Y offset "might be" negative,
/// e.g. `\pbo-50`). The bare `\p` / `\pbo` forms carry no documented
/// value, so they stay verbatim untyped via the `None` arm and [`emit`]
/// is byte-stable.
fn classify_drawing(tag: &str) -> Option<AssTag> {
    if let Some(rest) = tag.strip_prefix("pbo") {
        // Bare `\pbo` carries no documented value — stays verbatim.
        return Some(AssTag::BaselineOffset(canon_i32(rest)?));
    }
    if let Some(rest) = tag.strip_prefix('p') {
        // Bare `\p` carries no documented level — stays verbatim.
        return Some(AssTag::Drawing(canon_u32(rest)?));
    }
    None
}

/// Try the `\r` style-reset tag: bare `\r` (reset to the line's style) or
/// `\r<style>` (reset to a named style). The named-style argument is an
/// arbitrary verbatim string — style names carry spaces and arbitrary
/// characters — so the whole tail after `r` rides through unparsed and
/// [`emit`] stays byte-stable.
///
/// The `\fr*` rotation family was already consumed by [`classify_font`]
/// above, so a `\frz` never reaches here; only a leading `r` not followed
/// by the rotation spelling lands in this arm.
fn classify_reset(tag: &str) -> Option<AssTag> {
    let rest = tag.strip_prefix('r')?;
    if rest.is_empty() {
        Some(AssTag::Reset(None))
    } else {
        Some(AssTag::Reset(Some(rest.to_string())))
    }
}

/// A canonical non-negative decimal run, returned verbatim. Rejects a
/// leading `-` (the `\t` acceleration "value … between 0 and 1" / ">1"
/// is never negative) and any spelling [`decode_decimal`] couldn't
/// reproduce, so [`emit`] stays byte-stable.
fn canon_nonneg_decimal(s: &str) -> Option<String> {
    (!s.starts_with('-') && canon_decimal(s)).then(|| s.to_string())
}

/// [`canon_decimal_opt`] restricted to a non-negative run: the empty
/// reset form, or a [`canon_decimal`] run with no leading `-`. Used by
/// the `\bord` family and the combined `\shad`, which the spec forbids
/// from being negative — a `-`-signed value re-emits verbatim as an
/// untyped [`AssTag::Other`].
fn canon_decimal_nonneg_opt(rest: &str) -> Option<Option<String>> {
    if rest.is_empty() {
        return Some(None);
    }
    (!rest.starts_with('-') && canon_decimal(rest)).then(|| Some(rest.to_string()))
}

/// Match a font-metric tag's numeric parameter. `""` is the
/// parameterless reset form (`Some(None)`); a canonically-spelled
/// decimal run (per [`canon_decimal`]) yields the verbatim string
/// (`Some(Some(_))`); any other shape is `None` and the whole tag stays
/// an untyped [`AssTag::Other`].
fn canon_decimal_opt(rest: &str) -> Option<Option<String>> {
    if rest.is_empty() {
        return Some(None);
    }
    canon_decimal(rest).then(|| Some(rest.to_string()))
}

/// Match an integer-only tag parameter (the `\be` repeat count, which
/// the spec says "must be an integer number"). `""` is the
/// parameterless reset form (`Some(None)`); a canonically-spelled
/// non-negative integer run (per [`canon_u32`]) yields the verbatim
/// string (`Some(Some(_))`); any other shape — a sign, a decimal point,
/// a leading zero, embedded whitespace — is `None` and the whole tag
/// stays an untyped [`AssTag::Other`].
fn canon_uint_opt(rest: &str) -> Option<Option<String>> {
    if rest.is_empty() {
        return Some(None);
    }
    canon_u32(rest).map(|_| Some(rest.to_string()))
}

/// `<name>(<args>)` with nothing after the closing parenthesis →
/// the raw argument list.
fn paren_args<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    tag.strip_prefix(name)?.strip_prefix('(')?.strip_suffix(')')
}

/// Parse a canonically-spelled decimal integer: the typed layer only
/// accepts a spelling that re-emits identically
/// (`v.to_string() == s`), so leading zeroes, a `+` sign, `-0`, and
/// embedded whitespace all stay verbatim untyped and [`emit`] remains
/// byte-stable.
fn canon_i32(s: &str) -> Option<i32> {
    let v: i32 = s.parse().ok()?;
    (v.to_string() == s).then_some(v)
}

/// [`canon_i32`] for an unsigned millisecond / centisecond field.
fn canon_u32(s: &str) -> Option<u32> {
    let v: u32 = s.parse().ok()?;
    (v.to_string() == s).then_some(v)
}

/// [`canon_i32`] for a single-byte alignment field.
fn canon_u8(s: &str) -> Option<u8> {
    let v: u8 = s.parse().ok()?;
    (v.to_string() == s).then_some(v)
}

/// Is `s` a canonically-spelled decimal number — an optional leading
/// `-`, then ASCII digits with at most one `.` that has a digit on
/// either side?
///
/// Font-metric parameters (`\fs`, `\fsc*`, `\fsp`, `\fr*`) are commonly
/// fractional or negative, so unlike the integer-only positioning
/// tags they accept a decimal point. The typed layer still rejects any
/// spelling [`decode_decimal`] couldn't reproduce — a `+` sign, a bare
/// or trailing `.`, an embedded space, a `%` the spec doesn't use, or
/// a digit-grouping separator — so those stay verbatim untyped and
/// [`emit`] is byte-stable.
fn canon_decimal(s: &str) -> bool {
    let body = s.strip_prefix('-').unwrap_or(s);
    if body.is_empty() {
        return false;
    }
    let mut seen_dot = false;
    let mut prev_digit = false;
    let bytes = body.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'0'..=b'9' => prev_digit = true,
            b'.' => {
                // One dot only, with a digit before and after.
                if seen_dot || !prev_digit || i + 1 >= bytes.len() {
                    return false;
                }
                seen_dot = true;
                prev_digit = false;
            }
            _ => return false,
        }
    }
    // `-0`, `-0.0` etc. are accepted: they re-emit verbatim, which is
    // all the byte-stability contract requires.
    true
}

/// Decode a font-metric parameter run (`\fs`, `\fsc*`, `\fsp`, `\fr*`
/// degrees) into an `f64`. Returns `None` for the empty / reset run or
/// any spelling outside [`canon_decimal`].
pub fn decode_decimal(s: &str) -> Option<f64> {
    canon_decimal(s).then(|| s.parse().ok()).flatten()
}

/// Map a legacy `\a` alignment value to the equivalent `\an` numpad
/// value.
///
/// Per the references: legacy 1 / 2 / 3 are bottom
/// left / center / right ("Use 1 for left-alignment, 2 for center
/// alignment and 3 for right-alignment"); "Adding 4 to the value
/// specifies a 'Toptitle'" (5 / 6 / 7 → numpad top row 7 / 8 / 9);
/// "Adding 8 to the value specifies a 'Midtitle'" (9 / 10 / 11 →
/// numpad middle row 4 / 5 / 6). Returns `None` for 0 (reset — no
/// fixed numpad value), 4, 8, and anything above 11.
pub fn legacy_align_to_numpad(a: u8) -> Option<u8> {
    match a {
        1..=3 => Some(a),
        5..=7 => Some(a + 2),
        9..=11 => Some(a - 5),
        _ => None,
    }
}

/// Map an [`AssTag::Drawing`] level to the coordinate divisor it
/// implies.
///
/// Per the Aegisub reference, a level above 1 means the drawing
/// "resolution is doubled" per step: "the value … will be interpreted as
/// the scale, in `2^(value-1)` mode". So `\p1` divides by 1 (normal
/// coordinates), `\p2` by 2 ("drawing to 200,200 will actually draw to
/// 100,100"), and `\p4` by 8 ("resolution is 8x larger (`2^(4-1)`)").
/// `\p0` (drawing off) has no coordinate system; this returns `1` for
/// it as the neutral divisor.
pub fn drawing_scale_divisor(level: u32) -> f64 {
    match level {
        0 | 1 => 1.0,
        n => 2f64.powi((n - 1) as i32),
    }
}

/// A single command in a `\p` vector-drawing command stream (also used
/// by the vector-overload `\clip` / `\iclip` forms). The ops mirror the
/// Aegisub drawing-command reference; every coordinate is in
/// script-resolution pixels (before the `\p<level>` divisor and `\pbo`
/// baseline offset are applied).
#[derive(Debug, Clone, PartialEq)]
pub enum DrawCmd {
    /// `m <x> <y>` — move the cursor, auto-closing any open shape: "All
    /// drawing routines must start with this command."
    Move(f64, f64),
    /// `n <x> <y>` — move the cursor "without closing the current
    /// shape".
    MoveNoClose(f64, f64),
    /// `l <x> <y> ...` — one or more line segments to successive
    /// points. "Draws a line from the current cursor position to x,y,
    /// and moves the cursor there afterwards." Repeated coordinate
    /// pairs after a single `l` are kept as one command's point list.
    Line(Vec<(f64, f64)>),
    /// `b <x1> <y1> <x2> <y2> <x3> <y3> ...` — one or more cubic Bézier
    /// curves, three control points each (`(x1,y1)` and `(x2,y2)` are
    /// the control points, `(x3,y3)` the endpoint).
    Bezier(Vec<(f64, f64)>),
    /// `s <x1> <y1> .. <xN> <yN>` — a cubic uniform b-spline through the
    /// listed points. "This must contain at least 3 coordinates".
    Spline(Vec<(f64, f64)>),
    /// `p <x> <y>` — extend the b-spline to a further point ("the same
    /// as adding another pair of coordinates at the end of `s`").
    SplineExtend(f64, f64),
    /// `c` — close the b-spline.
    CloseSpline,
}

/// Parse a `\p` drawing-command stream into structured [`DrawCmd`]s.
///
/// The stream is the verbatim run that appears between `\p<level>` and
/// `\p0` (or inside the vector-overload `\clip(<drawing>)` form). Tokens
/// are whitespace-separated: a command letter (`m` / `n` / `l` / `b` /
/// `s` / `p` / `c`) followed by its coordinate arguments, with
/// coordinates parsed as decimals (they are commonly fractional under a
/// `\p2`+ subpixel scale, and may be negative). Per the reference `l`
/// and `b` accept repeated coordinate groups after a single command
/// letter, and `s` accepts an arbitrary-length point list.
///
/// Returns `None` on any malformed stream — a leading token that isn't a
/// command letter, a coordinate that isn't a decimal, a `b` whose
/// coordinate count isn't a positive multiple of three, an `l` / `s`
/// with too few points, or a stray token after `c`. A well-formed empty
/// stream (only whitespace) parses to an empty command list.
pub fn parse_drawing(stream: &str) -> Option<Vec<DrawCmd>> {
    let mut toks = stream.split_ascii_whitespace().peekable();
    let mut out = Vec::new();
    while let Some(letter) = toks.next() {
        match letter {
            "m" | "n" | "p" => {
                let x = parse_draw_coord(toks.next()?)?;
                let y = parse_draw_coord(toks.next()?)?;
                out.push(match letter {
                    "m" => DrawCmd::Move(x, y),
                    "n" => DrawCmd::MoveNoClose(x, y),
                    _ => DrawCmd::SplineExtend(x, y),
                });
            }
            "c" => out.push(DrawCmd::CloseSpline),
            "l" | "b" | "s" => {
                // Greedily collect the trailing coordinate pairs that
                // follow this command letter (up to the next letter).
                let mut pts = Vec::new();
                while let Some(t) = toks.peek() {
                    if !t.bytes().next()?.is_ascii_digit()
                        && !t.starts_with('-')
                        && !t.starts_with('.')
                    {
                        break;
                    }
                    let x = parse_draw_coord(toks.next()?)?;
                    let y = parse_draw_coord(toks.next()?)?;
                    pts.push((x, y));
                }
                match letter {
                    // `l` needs at least one segment.
                    "l" if !pts.is_empty() => out.push(DrawCmd::Line(pts)),
                    // `b` is "cubic", three control points per curve.
                    "b" if !pts.is_empty() && pts.len() % 3 == 0 => out.push(DrawCmd::Bezier(pts)),
                    // `s` "must contain at least 3 coordinates".
                    "s" if pts.len() >= 3 => out.push(DrawCmd::Spline(pts)),
                    _ => return None,
                }
            }
            _ => return None,
        }
    }
    Some(out)
}

/// Parse one drawing-command coordinate: a canonical decimal per
/// [`canon_decimal`] (optional `-`, digits, at most one interior `.`).
fn parse_draw_coord(tok: &str) -> Option<f64> {
    decode_decimal(tok)
}

/// Serialise structured [`DrawCmd`]s back into a `\p` drawing-command
/// stream. The inverse of [`parse_drawing`] for any stream whose
/// coordinates were already canonically spelled and singly
/// space-separated; the round-trip normalises whitespace to single
/// spaces and re-emits coordinates via their shortest `f64` spelling, so
/// it is value-stable rather than byte-stable on arbitrary input.
pub fn emit_drawing(cmds: &[DrawCmd]) -> String {
    let mut parts: Vec<String> = Vec::new();
    let push_pts = |parts: &mut Vec<String>, letter: &str, pts: &[(f64, f64)]| {
        parts.push(letter.to_string());
        for (x, y) in pts {
            parts.push(fmt_coord(*x));
            parts.push(fmt_coord(*y));
        }
    };
    for cmd in cmds {
        match cmd {
            DrawCmd::Move(x, y) => {
                parts.push("m".into());
                parts.push(fmt_coord(*x));
                parts.push(fmt_coord(*y));
            }
            DrawCmd::MoveNoClose(x, y) => {
                parts.push("n".into());
                parts.push(fmt_coord(*x));
                parts.push(fmt_coord(*y));
            }
            DrawCmd::SplineExtend(x, y) => {
                parts.push("p".into());
                parts.push(fmt_coord(*x));
                parts.push(fmt_coord(*y));
            }
            DrawCmd::CloseSpline => parts.push("c".into()),
            DrawCmd::Line(pts) => push_pts(&mut parts, "l", pts),
            DrawCmd::Bezier(pts) => push_pts(&mut parts, "b", pts),
            DrawCmd::Spline(pts) => push_pts(&mut parts, "s", pts),
        }
    }
    parts.join(" ")
}

/// Format a drawing coordinate using its shortest round-trippable
/// spelling — an integral value emits without a trailing `.0`.
fn fmt_coord(v: f64) -> String {
    if v.fract() == 0.0 && v.is_finite() {
        format!("{}", v as i64)
    } else {
        format!("{v}")
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
                    emit_tag(&mut out, tag);
                }
                out.push('}');
            }
        }
    }
    out
}

/// Re-emit one typed tag (the leading `\` plus its body) into `out`.
/// Factored out of [`emit`] so the `\t(...)` animated-transform tag can
/// re-emit its nested *style modifiers* through the same per-tag logic,
/// keeping the whole transform byte-stable.
fn emit_tag(out: &mut String, tag: &AssTag) {
    match tag {
        AssTag::Bold(b) => {
            out.push_str("\\b");
            if let Some(w) = b {
                out.push_str(&w.to_string());
            }
        }
        AssTag::Italic(f) => push_flag(out, 'i', *f),
        AssTag::Underline(f) => push_flag(out, 'u', *f),
        AssTag::Strikeout(f) => push_flag(out, 's', *f),
        AssTag::Color { target, short, hex } => {
            out.push('\\');
            if *short && *target == AssColorTarget::Primary {
                out.push('c');
            } else {
                out.push(target_digit(*target));
                out.push('c');
            }
            push_amp_hex(out, hex);
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
            push_amp_hex(out, hex);
        }
        AssTag::AlignNumpad(v) => {
            out.push_str("\\an");
            if let Some(v) = v {
                out.push_str(&v.to_string());
            }
        }
        AssTag::AlignLegacy(v) => {
            out.push_str("\\a");
            if let Some(v) = v {
                out.push_str(&v.to_string());
            }
        }
        AssTag::Karaoke { kind, centisec } => {
            out.push_str(match kind {
                AssKaraokeKind::Instant => "\\k",
                AssKaraokeKind::SweepCap => "\\K",
                AssKaraokeKind::Sweep => "\\kf",
                AssKaraokeKind::Outline => "\\ko",
            });
            out.push_str(&centisec.to_string());
        }
        AssTag::Pos { x, y } => {
            out.push_str(&format!("\\pos({x},{y})"));
        }
        AssTag::Org { x, y } => {
            out.push_str(&format!("\\org({x},{y})"));
        }
        AssTag::FontName(name) => {
            out.push_str("\\fn");
            if let Some(n) = name {
                out.push_str(n);
            }
        }
        AssTag::FontSize(v) => {
            out.push_str("\\fs");
            if let Some(v) = v {
                out.push_str(v);
            }
        }
        AssTag::FontScale { x_axis, percent } => {
            out.push_str(if *x_axis { "\\fscx" } else { "\\fscy" });
            if let Some(p) = percent {
                out.push_str(p);
            }
        }
        AssTag::FontSpacing(v) => {
            out.push_str("\\fsp");
            if let Some(v) = v {
                out.push_str(v);
            }
        }
        AssTag::FontEncoding(v) => {
            out.push_str("\\fe");
            if let Some(v) = v {
                out.push_str(v);
            }
        }
        AssTag::Rotation {
            axis,
            bare,
            degrees,
        } => {
            out.push_str("\\fr");
            if !*bare {
                out.push(match axis {
                    AssRotationAxis::X => 'x',
                    AssRotationAxis::Y => 'y',
                    AssRotationAxis::Z => 'z',
                });
            }
            if let Some(d) = degrees {
                out.push_str(d);
            }
        }
        AssTag::Border { axis, size } => {
            out.push('\\');
            match axis {
                AssBorderAxis::Both => {}
                AssBorderAxis::X => out.push('x'),
                AssBorderAxis::Y => out.push('y'),
            }
            out.push_str("bord");
            if let Some(s) = size {
                out.push_str(s);
            }
        }
        AssTag::Shadow { axis, depth } => {
            out.push('\\');
            match axis {
                AssBorderAxis::Both => {}
                AssBorderAxis::X => out.push('x'),
                AssBorderAxis::Y => out.push('y'),
            }
            out.push_str("shad");
            if let Some(d) = depth {
                out.push_str(d);
            }
        }
        AssTag::Blur { kind, strength } => {
            out.push_str(match kind {
                AssBlurKind::Edge => "\\be",
                AssBlurKind::Gaussian => "\\blur",
            });
            if let Some(s) = strength {
                out.push_str(s);
            }
        }
        AssTag::Move {
            x1,
            y1,
            x2,
            y2,
            times,
        } => {
            out.push_str(&format!("\\move({x1},{y1},{x2},{y2}"));
            if let Some((t1, t2)) = times {
                out.push_str(&format!(",{t1},{t2}"));
            }
            out.push(')');
        }
        AssTag::Clip { inverse, shape } => {
            out.push('\\');
            out.push_str(if *inverse { "iclip(" } else { "clip(" });
            match shape {
                AssClipShape::Rectangle { x1, y1, x2, y2 } => {
                    out.push_str(&format!("{x1},{y1},{x2},{y2}"));
                }
                AssClipShape::Drawing { scale, commands } => {
                    if let Some(s) = scale {
                        out.push_str(&format!("{s},"));
                    }
                    out.push_str(commands);
                }
            }
            out.push(')');
        }
        AssTag::Fade(spec) => match spec {
            AssFadeSpec::Simple { fadein, fadeout } => {
                out.push_str(&format!("\\fad({fadein},{fadeout})"));
            }
            AssFadeSpec::Complex {
                a1,
                a2,
                a3,
                t1,
                t2,
                t3,
                t4,
            } => {
                out.push_str(&format!("\\fade({a1},{a2},{a3},{t1},{t2},{t3},{t4})"));
            }
        },
        AssTag::Transform {
            t1,
            t2,
            accel,
            modifiers,
        } => {
            out.push_str("\\t(");
            if let (Some(t1), Some(t2)) = (t1, t2) {
                out.push_str(&format!("{t1},{t2},"));
            }
            if let Some(a) = accel {
                out.push_str(a);
                out.push(',');
            }
            for m in modifiers {
                emit_tag(out, m);
            }
            out.push(')');
        }
        AssTag::Drawing(level) => {
            out.push_str("\\p");
            out.push_str(&level.to_string());
        }
        AssTag::BaselineOffset(y) => {
            out.push_str("\\pbo");
            out.push_str(&y.to_string());
        }
        AssTag::Reset(style) => {
            out.push_str("\\r");
            if let Some(s) = style {
                out.push_str(s);
            }
        }
        AssTag::Other(body) => {
            out.push('\\');
            out.push_str(body);
        }
        AssTag::Comment(s) => out.push_str(s),
    }
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
        // \be / \blur share the \b prefix but are the typed blur family,
        // not the \b flag toggle. \iclip shares the \i prefix but is the
        // typed clip family, not the \i toggle — the exact-prefix paren
        // match keeps the two distinct.
        assert_eq!(
            tokenize("{\\be1}"),
            vec![AssToken::Override(vec![AssTag::Blur {
                kind: AssBlurKind::Edge,
                strength: Some("1".into()),
            }])]
        );
        assert_eq!(
            tokenize("{\\blur2}"),
            vec![AssToken::Override(vec![AssTag::Blur {
                kind: AssBlurKind::Gaussian,
                strength: Some("2".into()),
            }])]
        );
        assert_eq!(
            tokenize("{\\iclip(0,0,100,100)}"),
            vec![AssToken::Override(vec![AssTag::Clip {
                inverse: true,
                shape: AssClipShape::Rectangle {
                    x1: 0,
                    y1: 0,
                    x2: 100,
                    y2: 100,
                },
            }])]
        );
        roundtrip("{\\be1}");
        roundtrip("{\\blur2}");
        roundtrip("{\\iclip(0,0,100,100)}");
        // \bord / \shad share the \b / \s prefix but are the typed
        // border / shadow family, not the flag toggles — they must NOT
        // resolve to Bold / Strikeout.
        assert_eq!(
            tokenize("{\\bord3.7}"),
            vec![AssToken::Override(vec![AssTag::Border {
                axis: AssBorderAxis::Both,
                size: Some("3.7".into()),
            }])]
        );
        assert_eq!(
            tokenize("{\\shad2}"),
            vec![AssToken::Override(vec![AssTag::Shadow {
                axis: AssBorderAxis::Both,
                depth: Some("2".into()),
            }])]
        );
        roundtrip("{\\bord3.7}{\\shad2}");
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
    fn transform_t1_t2_modifiers_types() {
        // \t(<t1>,<t2>,<modifiers>): the parenthesised body's leading
        // numbers are the time window, and the backslash run is the
        // animated style modifiers parsed recursively.
        let s = "{\\t(0,1000,\\fscx200\\fscy200)}grow";
        assert_eq!(
            tokenize(s)[0],
            AssToken::Override(vec![AssTag::Transform {
                t1: Some(0),
                t2: Some(1000),
                accel: None,
                modifiers: vec![
                    AssTag::FontScale {
                        x_axis: true,
                        percent: Some("200".into()),
                    },
                    AssTag::FontScale {
                        x_axis: false,
                        percent: Some("200".into()),
                    },
                ],
            }])
        );
        roundtrip(s);
    }

    #[test]
    fn transform_all_four_arities_type_and_round_trip() {
        // \t(<modifiers>) — Aegisub reference example: the text starts
        // blue and fades to red over the whole line.
        let modifiers_only = "{\\1c&HFF0000&\\t(\\1c&H0000FF&)}Hello!";
        match &tokenize(modifiers_only)[0] {
            AssToken::Override(tags) => match &tags[1] {
                AssTag::Transform {
                    t1,
                    t2,
                    accel,
                    modifiers,
                } => {
                    assert_eq!((*t1, *t2, accel.clone()), (None, None, None));
                    assert_eq!(modifiers.len(), 1);
                    assert!(matches!(modifiers[0], AssTag::Color { .. }));
                }
                other => panic!("expected Transform, got {other:?}"),
            },
            other => panic!("expected Override, got {other:?}"),
        }
        roundtrip(modifiers_only);

        // \t(<accel>,<modifiers>) — a single leading decimal is the
        // acceleration exponent, kept verbatim.
        let accel = "{\\t(0.5,\\frz360)}spin";
        assert_eq!(
            tokenize(accel)[0],
            AssToken::Override(vec![AssTag::Transform {
                t1: None,
                t2: None,
                accel: Some("0.5".into()),
                modifiers: vec![AssTag::Rotation {
                    axis: AssRotationAxis::Z,
                    bare: false,
                    degrees: Some("360".into()),
                }],
            }])
        );
        roundtrip(accel);

        // \t(<t1>,<t2>,<accel>,<modifiers>) — full four-argument form.
        let full = "{\\t(100,900,2,\\fs40)}grow";
        assert_eq!(
            tokenize(full)[0],
            AssToken::Override(vec![AssTag::Transform {
                t1: Some(100),
                t2: Some(900),
                accel: Some("2".into()),
                modifiers: vec![AssTag::FontSize(Some("40".into()))],
            }])
        );
        roundtrip(full);
    }

    #[test]
    fn transform_off_shape_stays_verbatim() {
        // Each off-shape spelling stays an untyped Other and re-emits
        // byte-for-byte.
        for body in [
            "t(0,1000)",            // no modifiers at all
            "t()",                  // empty argument list
            "t(0,1000,2,3,\\fs40)", // too many leading numbers
            "t(-0.5,\\frz360)",     // negative accel
            "t(+1,\\fs40)",         // signed time
            "t(0,1000,\\fs40)x",    // trailing text after close paren
        ] {
            let s = format!("{{\\{body}}}");
            assert_eq!(
                tokenize(&s)[0],
                AssToken::Override(vec![AssTag::Other(body.into())]),
                "expected {body} verbatim",
            );
            roundtrip(&s);
        }
    }

    #[test]
    fn transform_nested_clip_rectangle_round_trips() {
        // The Aegisub note: only the rectangle \clip can be animated.
        // The nested \clip rides through as a recursively-parsed Clip
        // modifier and re-emits byte-stably.
        let s = "{\\t(\\clip(0,0,640,360))}reveal";
        match &tokenize(s)[0] {
            AssToken::Override(tags) => match &tags[0] {
                AssTag::Transform { modifiers, .. } => {
                    assert_eq!(modifiers.len(), 1);
                    assert!(matches!(modifiers[0], AssTag::Clip { inverse: false, .. }));
                }
                other => panic!("expected Transform, got {other:?}"),
            },
            other => panic!("expected Override, got {other:?}"),
        }
        roundtrip(s);
    }

    #[test]
    fn simple_fade_spec_example_types() {
        // Aegisub reference example: "\fad(1200,250)" — "Fade in the
        // line in the first 1.2 seconds it is to be displayed, and fade
        // it out for the last one quarter second it is displayed."
        let s = "{\\fad(1200,250)}hi";
        assert_eq!(
            tokenize(s)[0],
            AssToken::Override(vec![AssTag::Fade(AssFadeSpec::Simple {
                fadein: 1200,
                fadeout: 250,
            })])
        );
        roundtrip(s);
        // Either end may be 0 "to not have any fade effect on that end".
        assert_eq!(
            tokenize("{\\fad(0,500)}"),
            vec![AssToken::Override(vec![AssTag::Fade(
                AssFadeSpec::Simple {
                    fadein: 0,
                    fadeout: 500,
                }
            )])]
        );
        roundtrip("{\\fad(0,500)}");
        roundtrip("{\\fad(500,0)}");
    }

    #[test]
    fn complex_fade_spec_example_types() {
        // Aegisub reference example:
        // "\fade(255,32,224,0,500,2000,2200)" — "Starts invisible,
        // fades to almost totally opaque, then fades to almost totally
        // invisible."
        let s = "{\\fade(255,32,224,0,500,2000,2200)}hi";
        assert_eq!(
            tokenize(s)[0],
            AssToken::Override(vec![AssTag::Fade(AssFadeSpec::Complex {
                a1: 255,
                a2: 32,
                a3: 224,
                t1: 0,
                t2: 500,
                t3: 2000,
                t4: 2200,
            })])
        );
        roundtrip(s);
    }

    #[test]
    fn off_shape_fades_stay_verbatim_other() {
        // \fade is tried ahead of \fad and the longer name wins, but a
        // mis-arity, signed, non-integer, or out-of-range value falls
        // through to a verbatim Other so emit stays byte-stable.
        for s in [
            "{\\fad(200)}",                    // simple needs two values
            "{\\fad(200,300,400)}",            // too many
            "{\\fad(-200,300)}",               // negative not canonical u32
            "{\\fad(2.5,300)}",                // non-integer
            "{\\fad(200, 300)}",               // embedded space
            "{\\fade(255,32,224,0,500,2000)}", // complex needs seven
            "{\\fade(256,0,0,0,1,2,3)}",       // alpha above 255
            "{\\fade(0,0,0,-1,1,2,3)}",        // negative time
        ] {
            let body = &s[2..s.len() - 1];
            assert_eq!(
                tokenize(s),
                vec![AssToken::Override(vec![AssTag::Other(body.into())])],
                "for {s:?}"
            );
            roundtrip(s);
        }
    }

    #[test]
    fn fade_distinct_from_font_and_flag_prefixes() {
        // \fad / \fade share the \f prefix with the font-metric family
        // (\fs, \fr, \fn, …) and the \fade body starts with \fad; the
        // exact-prefix paren match keeps all three distinct.
        let s = "{\\fs28\\fad(100,100)\\fade(0,128,255,0,1,2,3)}x";
        assert_eq!(
            tokenize(s)[0],
            AssToken::Override(vec![
                AssTag::FontSize(Some("28".into())),
                AssTag::Fade(AssFadeSpec::Simple {
                    fadein: 100,
                    fadeout: 100,
                }),
                AssTag::Fade(AssFadeSpec::Complex {
                    a1: 0,
                    a2: 128,
                    a3: 255,
                    t1: 0,
                    t2: 1,
                    t3: 2,
                    t4: 3,
                }),
            ])
        );
        roundtrip(s);
    }

    #[test]
    fn font_tags_type_next_to_typed_position() {
        let s = "{\\pos(320,240)\\fnCourier New\\fs28}x";
        assert_eq!(
            tokenize(s)[0],
            AssToken::Override(vec![
                AssTag::Pos { x: 320, y: 240 },
                AssTag::FontName(Some("Courier New".into())),
                AssTag::FontSize(Some("28".into())),
            ])
        );
        roundtrip(s);
    }

    #[test]
    fn font_scale_and_spacing_axes_type() {
        assert_eq!(
            tokenize("{\\fscx120}")[0],
            AssToken::Override(vec![AssTag::FontScale {
                x_axis: true,
                percent: Some("120".into()),
            }])
        );
        assert_eq!(
            tokenize("{\\fscy80}")[0],
            AssToken::Override(vec![AssTag::FontScale {
                x_axis: false,
                percent: Some("80".into()),
            }])
        );
        // \fsp must not be mistaken for \fs (the shorter prefix).
        assert_eq!(
            tokenize("{\\fsp3}")[0],
            AssToken::Override(vec![AssTag::FontSpacing(Some("3".into()))])
        );
        roundtrip("{\\fscx120\\fscy80\\fsp3}");
    }

    #[test]
    fn rotation_axis_spellings_distinct_from_bare() {
        // \frz and bare \fr are both Z but kept distinct so emit is
        // byte-stable.
        assert_eq!(
            tokenize("{\\frz30}")[0],
            AssToken::Override(vec![AssTag::Rotation {
                axis: AssRotationAxis::Z,
                bare: false,
                degrees: Some("30".into()),
            }])
        );
        assert_eq!(
            tokenize("{\\fr30}")[0],
            AssToken::Override(vec![AssTag::Rotation {
                axis: AssRotationAxis::Z,
                bare: true,
                degrees: Some("30".into()),
            }])
        );
        roundtrip("{\\frx10\\fry20\\frz30\\fr40}");
    }

    #[test]
    fn font_prefix_cousins_not_swallowed() {
        // \fad / \fade share the \f* prefix but are the typed fade
        // family (see the fade tests), not a font-metric tag — the
        // font-metric family must not absorb them.
        // (\bord / \shad are the typed border / shadow family — see the
        // border_shadow tests; \be / \blur are the typed blur family —
        // see the blur tests.)
        assert_eq!(
            tokenize("{\\fad(1,2)}")[0],
            AssToken::Override(vec![AssTag::Fade(AssFadeSpec::Simple {
                fadein: 1,
                fadeout: 2,
            })])
        );
        assert_eq!(
            tokenize("{\\fade(1,2,3,4,5,6,7)}")[0],
            AssToken::Override(vec![AssTag::Fade(AssFadeSpec::Complex {
                a1: 1,
                a2: 2,
                a3: 3,
                t1: 4,
                t2: 5,
                t3: 6,
                t4: 7,
            })])
        );
        roundtrip("{\\fad(1,2)}{\\fade(1,2,3,4,5,6,7)}");
    }

    #[test]
    fn blur_family_types_and_distinguishes_kind() {
        // \be is an integer-count edge softening; \blur is the gaussian
        // variant whose strength may be fractional. Both keep their
        // spelling so emit is byte-stable.
        assert_eq!(
            tokenize("{\\be2\\blur1.5}")[0],
            AssToken::Override(vec![
                AssTag::Blur {
                    kind: AssBlurKind::Edge,
                    strength: Some("2".into()),
                },
                AssTag::Blur {
                    kind: AssBlurKind::Gaussian,
                    strength: Some("1.5".into()),
                },
            ])
        );
        // Parameterless forms reset to the line style.
        assert_eq!(
            tokenize("{\\be\\blur}")[0],
            AssToken::Override(vec![
                AssTag::Blur {
                    kind: AssBlurKind::Edge,
                    strength: None,
                },
                AssTag::Blur {
                    kind: AssBlurKind::Gaussian,
                    strength: None,
                },
            ])
        );
        // A decimal \be (count must be integer) and any signed strength
        // fall through to verbatim Other.
        for body in ["be1.5", "be-1", "blur-1"] {
            let s = format!("{{\\{body}}}");
            assert_eq!(
                tokenize(&s)[0],
                AssToken::Override(vec![AssTag::Other(body.into())]),
                "{body} must stay verbatim"
            );
        }
        roundtrip("{\\be0}{\\be4}{\\blur0}{\\blur3.2}x");
    }

    #[test]
    fn numpad_alignment_types_and_validates() {
        assert_eq!(
            tokenize("{\\an8}")[0],
            AssToken::Override(vec![AssTag::AlignNumpad(Some(8))])
        );
        // Parameterless = reset to the line style's alignment.
        assert_eq!(
            tokenize("{\\an}")[0],
            AssToken::Override(vec![AssTag::AlignNumpad(None)])
        );
        // Outside the numpad 1..=9 (or non-canonical): verbatim.
        for (s, body) in [
            ("{\\an0}", "an0"),
            ("{\\an10}", "an10"),
            ("{\\an08}", "an08"),
            ("{\\an8x}", "an8x"),
        ] {
            assert_eq!(
                tokenize(s),
                vec![AssToken::Override(vec![AssTag::Other(body.into())])],
                "for {s:?}"
            );
            roundtrip(s);
        }
        roundtrip("{\\an8}{\\an}");
    }

    #[test]
    fn legacy_alignment_spec_examples_type() {
        // Appendix A examples: {\a1} left-justified subtitle, {\a5}
        // left-justified toptitle, {\a11} right-justified midtitle.
        for (s, v) in [
            ("{\\a1}", 1u8),
            ("{\\a2}", 2),
            ("{\\a3}", 3),
            ("{\\a5}", 5),
            ("{\\a11}", 11),
        ] {
            assert_eq!(
                tokenize(s)[0],
                AssToken::Override(vec![AssTag::AlignLegacy(Some(v))]),
                "for {s:?}"
            );
            roundtrip(s);
        }
        // "0 or nothing resets to the style default" — both spellings
        // typed, kept distinct for emit.
        assert_eq!(
            tokenize("{\\a0}")[0],
            AssToken::Override(vec![AssTag::AlignLegacy(Some(0))])
        );
        assert_eq!(
            tokenize("{\\a}")[0],
            AssToken::Override(vec![AssTag::AlignLegacy(None)])
        );
        roundtrip("{\\a0}{\\a}");
        // 4 / 8 / 12 / non-canonical spellings are not documented
        // legacy values: verbatim.
        for (s, body) in [
            ("{\\a4}", "a4"),
            ("{\\a8}", "a8"),
            ("{\\a12}", "a12"),
            ("{\\a02}", "a02"),
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
    fn legacy_to_numpad_mapping() {
        // Bottom row maps to itself; "Adding 4" = toptitle = numpad
        // top row; "Adding 8" = midtitle = numpad middle row.
        for (legacy, numpad) in [
            (1, 1),
            (2, 2),
            (3, 3),
            (5, 7),
            (6, 8),
            (7, 9),
            (9, 4),
            (10, 5),
            (11, 6),
        ] {
            assert_eq!(legacy_align_to_numpad(legacy), Some(numpad));
        }
        for bad in [0, 4, 8, 12, 255] {
            assert_eq!(legacy_align_to_numpad(bad), None);
        }
    }

    #[test]
    fn karaoke_spec_example_types() {
        // Appendix A: {\k94}This {\k48}is {\k24}a {\k150}karaoke line
        // — "The durations are in hundredths of seconds."
        let s = "{\\k94}This {\\k48}is {\\k24}a {\\k150}karaoke {\\k94}line";
        let toks = tokenize(s);
        assert_eq!(
            toks[0],
            AssToken::Override(vec![AssTag::Karaoke {
                kind: AssKaraokeKind::Instant,
                centisec: 94,
            }])
        );
        assert_eq!(
            toks[6],
            AssToken::Override(vec![AssTag::Karaoke {
                kind: AssKaraokeKind::Instant,
                centisec: 150,
            }])
        );
        roundtrip(s);
    }

    #[test]
    fn karaoke_kinds_keep_their_spelling() {
        // \K and \kf are "identical" effects but distinct bytes.
        let toks = tokenize("{\\K94}{\\kf94}{\\ko30}{\\k0}");
        assert_eq!(
            toks,
            vec![
                AssToken::Override(vec![AssTag::Karaoke {
                    kind: AssKaraokeKind::SweepCap,
                    centisec: 94,
                }]),
                AssToken::Override(vec![AssTag::Karaoke {
                    kind: AssKaraokeKind::Sweep,
                    centisec: 94,
                }]),
                AssToken::Override(vec![AssTag::Karaoke {
                    kind: AssKaraokeKind::Outline,
                    centisec: 30,
                }]),
                AssToken::Override(vec![AssTag::Karaoke {
                    kind: AssKaraokeKind::Instant,
                    centisec: 0,
                }]),
            ]
        );
        roundtrip("{\\K94}{\\kf94}{\\ko30}{\\k0}");
    }

    #[test]
    fn off_shape_karaoke_stays_verbatim() {
        // Bare forms (duration has no style default), the undocumented
        // \kt, and non-canonical digits are preserved untyped.
        for (s, body) in [
            ("{\\k}", "k"),
            ("{\\K}", "K"),
            ("{\\kf}", "kf"),
            ("{\\ko}", "ko"),
            ("{\\kt94}", "kt94"),
            ("{\\k094}", "k094"),
            ("{\\k-5}", "k-5"),
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
    fn pos_and_org_type_with_integer_coordinates() {
        assert_eq!(
            tokenize("{\\pos(640,360)}")[0],
            AssToken::Override(vec![AssTag::Pos { x: 640, y: 360 }])
        );
        // "The X and Y coordinates must be integers" — negatives are
        // legal integers (off-screen anchor).
        assert_eq!(
            tokenize("{\\pos(-10,5)}")[0],
            AssToken::Override(vec![AssTag::Pos { x: -10, y: 5 }])
        );
        assert_eq!(
            tokenize("{\\org(320,240)}")[0],
            AssToken::Override(vec![AssTag::Org { x: 320, y: 240 }])
        );
        roundtrip("{\\pos(640,360)}{\\pos(-10,5)}{\\org(320,240)}");
    }

    #[test]
    fn move_types_both_arities() {
        // Aegisub example: \move(100,150,300,350) — full-duration move.
        assert_eq!(
            tokenize("{\\move(100,150,300,350)}")[0],
            AssToken::Override(vec![AssTag::Move {
                x1: 100,
                y1: 150,
                x2: 300,
                y2: 350,
                times: None,
            }])
        );
        // Six-parameter form with the [ms] animation window; the
        // both-zero window means the same as the four-parameter form
        // but keeps its own spelling.
        assert_eq!(
            tokenize("{\\move(100,150,300,350,500,1500)}")[0],
            AssToken::Override(vec![AssTag::Move {
                x1: 100,
                y1: 150,
                x2: 300,
                y2: 350,
                times: Some((500, 1500)),
            }])
        );
        roundtrip("{\\move(100,150,300,350)}{\\move(100,150,300,350,0,0)}");
    }

    #[test]
    fn off_shape_position_functions_stay_verbatim() {
        for (s, body) in [
            ("{\\pos(320, 240)}", "pos(320, 240)"),     // embedded space
            ("{\\pos(007,2)}", "pos(007,2)"),           // leading zeroes
            ("{\\pos(+3,2)}", "pos(+3,2)"),             // plus sign
            ("{\\pos(-0,2)}", "pos(-0,2)"),             // negative zero
            ("{\\pos(1.5,2)}", "pos(1.5,2)"),           // not an integer
            ("{\\pos(320)}", "pos(320)"),               // missing y
            ("{\\pos(1,2,3)}", "pos(1,2,3)"),           // extra coordinate
            ("{\\pos(1,2)x}", "pos(1,2)x"),             // trailing junk
            ("{\\pos}", "pos"),                         // no parameter list
            ("{\\move(1,2,3,4,5)}", "move(1,2,3,4,5)"), // 5-arg arity
            ("{\\move(1,2,3,4,-5,6)}", "move(1,2,3,4,-5,6)"), // negative ms
            // `\p` / `\pbo` drawing-mode cousins: only canonical integer
            // levels / offsets type; off-shape spellings stay verbatim.
            ("{\\pbo+4}", "pbo+4"),   // plus sign on baseline offset
            ("{\\pbo1.5}", "pbo1.5"), // non-integer baseline offset
            ("{\\p-1}", "p-1"),       // negative drawing level
            ("{\\p2.0}", "p2.0"),     // non-integer drawing level
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
        // off-shape is preserved byte-for-byte, untyped.
        for (s, body) in [
            ("{\\c&HFF}", "c&HFF"),             // no closing &
            ("{\\cHFF&}", "cHFF&"),             // no &H opener
            ("{\\c&H&}", "c&H&"),               // empty digit run
            ("{\\c&HGG&}", "c&HGG&"),           // non-hex digits
            ("{\\c&HFFFFFFF&}", "c&HFFFFFFF&"), // 7 digits
            ("{\\alpha&H123&}", "alpha&H123&"), // 3 digits
            ("{\\5c&HFF&}", "5c&HFF&"),         // no 5th component
        ] {
            assert_eq!(
                tokenize(s),
                vec![AssToken::Override(vec![AssTag::Other(body.into())])],
                "for {s:?}"
            );
            roundtrip(s);
        }
        // \clip shares the \c prefix but is the typed clip family, not a
        // colour — the exact-prefix paren match keeps the two distinct.
        assert_eq!(
            tokenize("{\\clip(0,0,10,10)}"),
            vec![AssToken::Override(vec![AssTag::Clip {
                inverse: false,
                shape: AssClipShape::Rectangle {
                    x1: 0,
                    y1: 0,
                    x2: 10,
                    y2: 10,
                },
            }])]
        );
        roundtrip("{\\clip(0,0,10,10)}");
        assert_eq!(decode_bgr_hex("FFFFFFF"), None);
        assert_eq!(decode_bgr_hex(""), None);
        assert_eq!(decode_alpha_hex("123"), None);
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
        // Appendix A drawing example: `\p1` now types to the drawing-mode
        // toggle, `\p0` disables it, and the command run between them is
        // ordinary (drawing-instruction) text kept verbatim.
        let s = "{\\p1}m 0 0 l 100 0 100 100 0 100{\\p0}";
        let toks = tokenize(s);
        assert_eq!(toks[0], AssToken::Override(vec![AssTag::Drawing(1)]));
        assert_eq!(
            toks[1],
            AssToken::Text("m 0 0 l 100 0 100 100 0 100".into())
        );
        assert_eq!(toks[2], AssToken::Override(vec![AssTag::Drawing(0)]));
        roundtrip(s);
    }

    #[test]
    fn drawing_toggle_levels_type_and_round_trip() {
        // "Setting this tag to 1 or above enables drawing mode … the
        // value might be any integer larger than zero, … `2^(value-1)`".
        assert_eq!(
            tokenize("{\\p1}"),
            vec![AssToken::Override(vec![AssTag::Drawing(1)])]
        );
        assert_eq!(
            tokenize("{\\p2}"),
            vec![AssToken::Override(vec![AssTag::Drawing(2)])]
        );
        assert_eq!(
            tokenize("{\\p4}"),
            vec![AssToken::Override(vec![AssTag::Drawing(4)])]
        );
        assert_eq!(
            tokenize("{\\p0}"),
            vec![AssToken::Override(vec![AssTag::Drawing(0)])]
        );
        // Bare `\p` has no documented level — stays verbatim untyped.
        assert_eq!(
            tokenize("{\\p}"),
            vec![AssToken::Override(vec![AssTag::Other("p".into())])]
        );
        for s in ["{\\p1}", "{\\p2}", "{\\p4}", "{\\p0}", "{\\p}"] {
            roundtrip(s);
        }
        // The scale divisor: `\p2` → 2, `\p4` → 8, `\p1`/`\p0` → 1.
        assert_eq!(drawing_scale_divisor(1), 1.0);
        assert_eq!(drawing_scale_divisor(2), 2.0);
        assert_eq!(drawing_scale_divisor(4), 8.0);
        assert_eq!(drawing_scale_divisor(0), 1.0);
    }

    #[test]
    fn p_prefix_does_not_swallow_pos_or_pbo() {
        // `\pos(...)` is the positioning family, not a `\p` drawing tag.
        assert_eq!(
            tokenize("{\\pos(320,240)}"),
            vec![AssToken::Override(vec![AssTag::Pos { x: 320, y: 240 }])]
        );
        // `\pbo` is the baseline-offset tag, checked ahead of `\p`.
        assert_eq!(
            tokenize("{\\pbo-50}"),
            vec![AssToken::Override(vec![AssTag::BaselineOffset(-50)])]
        );
        assert_eq!(
            tokenize("{\\pbo100}"),
            vec![AssToken::Override(vec![AssTag::BaselineOffset(100)])]
        );
        // Bare `\pbo` carries no documented value — verbatim untyped.
        assert_eq!(
            tokenize("{\\pbo}"),
            vec![AssToken::Override(vec![AssTag::Other("pbo".into())])]
        );
        for s in ["{\\pos(320,240)}", "{\\pbo-50}", "{\\pbo100}", "{\\pbo}"] {
            roundtrip(s);
        }
        // Off-shape level / offset spellings stay verbatim untyped.
        assert_eq!(
            tokenize("{\\p-1}"),
            vec![AssToken::Override(vec![AssTag::Other("p-1".into())])]
        );
        assert_eq!(
            tokenize("{\\pbo+5}"),
            vec![AssToken::Override(vec![AssTag::Other("pbo+5".into())])]
        );
        roundtrip("{\\p-1}");
        roundtrip("{\\pbo+5}");
    }

    #[test]
    fn parse_drawing_spec_examples() {
        // "Square": m 0 0 l 100 0 100 100 0 100
        assert_eq!(
            parse_drawing("m 0 0 l 100 0 100 100 0 100"),
            Some(vec![
                DrawCmd::Move(0.0, 0.0),
                DrawCmd::Line(vec![(100.0, 0.0), (100.0, 100.0), (0.0, 100.0)]),
            ])
        );
        // "Rounded square": m 0 0 s 100 0 100 100 0 100 c
        assert_eq!(
            parse_drawing("m 0 0 s 100 0 100 100 0 100 c"),
            Some(vec![
                DrawCmd::Move(0.0, 0.0),
                DrawCmd::Spline(vec![(100.0, 0.0), (100.0, 100.0), (0.0, 100.0)]),
                DrawCmd::CloseSpline,
            ])
        );
        // "Circle (almost)": m 50 0 b 100 0 100 100 50 100 b 0 100 0 0 50 0
        assert_eq!(
            parse_drawing("m 50 0 b 100 0 100 100 50 100 b 0 100 0 0 50 0"),
            Some(vec![
                DrawCmd::Move(50.0, 0.0),
                DrawCmd::Bezier(vec![(100.0, 0.0), (100.0, 100.0), (50.0, 100.0)]),
                DrawCmd::Bezier(vec![(0.0, 100.0), (0.0, 0.0), (50.0, 0.0)]),
            ])
        );
    }

    #[test]
    fn parse_drawing_extras_and_round_trip() {
        // `n` move-no-close, `p` spline-extend, fractional + negative
        // coordinates (common under a `\p2`+ subpixel scale).
        let stream = "n 1.5 -2 p 3 4 c";
        assert_eq!(
            parse_drawing(stream),
            Some(vec![
                DrawCmd::MoveNoClose(1.5, -2.0),
                DrawCmd::SplineExtend(3.0, 4.0),
                DrawCmd::CloseSpline,
            ])
        );
        // emit_drawing is the value-stable inverse for canonical streams.
        let cmds = parse_drawing("m 0 0 l 100 0 100 100 0 100").unwrap();
        assert_eq!(emit_drawing(&cmds), "m 0 0 l 100 0 100 100 0 100");
        let cmds2 = parse_drawing("n 1.5 -2 p 3 4 c").unwrap();
        assert_eq!(parse_drawing(&emit_drawing(&cmds2)), Some(cmds2));
        // An empty / whitespace-only stream is a valid empty drawing.
        assert_eq!(parse_drawing("   "), Some(vec![]));
    }

    #[test]
    fn parse_drawing_rejects_malformed() {
        // Leading token must be a command letter.
        assert_eq!(parse_drawing("0 0 l 1 1"), None);
        // Non-decimal coordinate.
        assert_eq!(parse_drawing("m a b"), None);
        // A `b` Bézier needs a positive multiple of three control points.
        assert_eq!(parse_drawing("m 0 0 b 1 1 2 2"), None);
        // `l` with no points, `s` with fewer than three.
        assert_eq!(parse_drawing("l"), None);
        assert_eq!(parse_drawing("m 0 0 s 1 1 2 2"), None);
        // An odd trailing coordinate (missing its Y) fails.
        assert_eq!(parse_drawing("m 0 0 l 1 1 2"), None);
        // An unknown command letter fails.
        assert_eq!(parse_drawing("m 0 0 z 1 1"), None);
    }
}
