//! Style resolution for the ASS / SSA Dialogue `Text` field.
//!
//! [`crate::ass_tags`] tokenizes a Dialogue `Text` payload into text runs
//! and typed override tags but stops at the lexical layer — it does not
//! *apply* those overrides. This module folds an override-tag stream over
//! a base style (an ASS `Style:` row) into a sequence of
//! [`ResolvedSpan`]s, each pairing a stretch of visible text with the
//! fully [`ResolvedStyle`] in effect over it.
//!
//! The resolution rules come from the SSA v4 script-format specification's
//! "Appendix A: Style override codes" (mirrored at
//! `docs/subtitles/ass/ass-specs-tcax.html`) and the Aegisub override-tag
//! reference (mirrored at `docs/subtitles/ass/aegisub-ass-tags.html`):
//!
//! * "Several overrides can be used within one set of braces." Overrides
//!   apply from the point they occur and "carry on until the next time
//!   they are set, or the line ends".
//! * "Any style modifier followed by no recognizable parameter resets to
//!   the default" — the `None`-parameter shape on every typed tag resets
//!   the corresponding field to the base-style value (carried by
//!   [`StyleBase`]).
//! * The colour / alpha family targets four independent components
//!   (primary fill, secondary fill, border, shadow); `\c` is "an
//!   abbreviation of `\1c`" and `\alpha` "sets the alpha of all components
//!   at once".
//! * `\b`, `\i`, `\u`, `\s` toggle the four boolean style flags; a `\b`
//!   value above 1 is "an explicit font weight".
//!
//! Line-property tags that describe the *whole* line rather than a text
//! run — `\pos`, `\move`, `\org`, `\fad` / `\fade`, `\clip` / `\iclip`,
//! and the alignment tags `\an` / `\a` — are collected once into a
//! [`LineLayout`] instead of riding each span (per the Aegisub reference:
//! "Tags in the first category should appear at most once in a line. If
//! they appear more than once, only one of them will take effect").
//!
//! Animated-transform `\t(...)` modifiers and `\k`-family karaoke timing
//! are *not* folded into the resolved span state here: `\t` describes a
//! time-varying end state and `\k` a per-syllable highlight schedule,
//! neither of which a single static [`ResolvedStyle`] can represent.
//! Their presence is recorded structurally ([`ResolvedSpan::karaoke_cs`]
//! and the untouched base state) so a renderer can layer them on top.

use crate::ass_tags::{
    decode_alpha_hex, decode_bgr_hex, legacy_align_to_numpad, tokenize, AssClipShape,
    AssColorTarget, AssFadeSpec, AssTag, AssToken,
};

/// A 32-bit straight-alpha colour as resolved from an ASS `&Hbbggrr&`
/// colour plus a separate `&Haa&` alpha component.
///
/// ASS keeps colour and alpha on *separate* override channels (`\1c` vs
/// `\1a`), so the two are merged only at resolution time. `a` follows the
/// ASS convention inverted to the common straight-alpha sense: `255` is
/// fully opaque, `0` fully transparent (the ASS wire form is the
/// opposite — `00` opaque, `FF` transparent — and is inverted on the way
/// in).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    /// Straight alpha: `255` opaque, `0` transparent.
    pub a: u8,
}

impl Rgba {
    /// Opaque black, the neutral default when a base style supplies no
    /// colour for a component.
    pub const OPAQUE_BLACK: Rgba = Rgba {
        r: 0,
        g: 0,
        b: 0,
        a: 255,
    };

    /// Build from an `(r, g, b, a)` tuple in the [`oxideav_core`]
    /// `SubtitleStyle` convention (straight alpha, `255` opaque).
    pub fn from_core(c: (u8, u8, u8, u8)) -> Rgba {
        Rgba {
            r: c.0,
            g: c.1,
            b: c.2,
            a: c.3,
        }
    }
}

/// The base styling state a Dialogue line starts from — the resolved
/// values of the line's `Style:` row before any override tag applies.
///
/// Every override tag's reset (`None`-parameter) form restores the
/// matching field to this base. Construct one from a
/// [`oxideav_core::SubtitleStyle`] with [`StyleBase::from_style`], or
/// start from [`StyleBase::default`] (a libass-neutral 18pt opaque-white
/// Arial) and override individual fields.
#[derive(Clone, Debug, PartialEq)]
pub struct StyleBase {
    pub font_name: String,
    pub font_size: f64,
    pub bold: bool,
    /// Explicit weight when the base style carries one (`> 1`); `None`
    /// means a plain bold/non-bold toggle.
    pub weight: Option<u32>,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub primary: Rgba,
    pub secondary: Rgba,
    pub outline_color: Rgba,
    pub shadow_color: Rgba,
    pub border: f64,
    pub shadow: f64,
    pub scale_x: f64,
    pub scale_y: f64,
    pub spacing: f64,
    pub angle_z: f64,
    /// Numpad alignment (1..=9), the resolved base from the style row.
    pub alignment: u8,
    pub encoding: i32,
}

impl Default for StyleBase {
    fn default() -> Self {
        StyleBase {
            font_name: "Arial".to_string(),
            font_size: 18.0,
            bold: false,
            weight: None,
            italic: false,
            underline: false,
            strike: false,
            primary: Rgba {
                r: 255,
                g: 255,
                b: 255,
                a: 255,
            },
            secondary: Rgba {
                r: 255,
                g: 0,
                b: 0,
                a: 255,
            },
            outline_color: Rgba::OPAQUE_BLACK,
            shadow_color: Rgba::OPAQUE_BLACK,
            border: 2.0,
            shadow: 0.0,
            scale_x: 100.0,
            scale_y: 100.0,
            spacing: 0.0,
            angle_z: 0.0,
            alignment: 2,
            encoding: 1,
        }
    }
}

impl StyleBase {
    /// Derive a base from an [`oxideav_core::SubtitleStyle`] row, keeping
    /// the [`StyleBase::default`] value for any field the style leaves
    /// `None`.
    pub fn from_style(style: &oxideav_core::SubtitleStyle) -> StyleBase {
        let mut b = StyleBase::default();
        if let Some(f) = &style.font_family {
            b.font_name = f.clone();
        }
        if let Some(s) = style.font_size {
            b.font_size = s as f64;
        }
        b.bold = style.bold;
        b.italic = style.italic;
        b.underline = style.underline;
        b.strike = style.strike;
        if let Some(c) = style.primary_color {
            b.primary = Rgba::from_core(c);
        }
        if let Some(c) = style.outline_color {
            b.outline_color = Rgba::from_core(c);
        }
        if let Some(c) = style.back_color {
            b.shadow_color = Rgba::from_core(c);
        }
        if let Some(o) = style.outline {
            b.border = o as f64;
        }
        if let Some(s) = style.shadow {
            b.shadow = s as f64;
        }
        b.alignment = align_to_numpad(style.align);
        b
    }
}

/// Map a core [`oxideav_core::TextAlign`] onto an ASS numpad value.
///
/// ASS alignment is a 1..=9 numpad grid; the core IR only models the
/// horizontal axis, so the vertical band defaults to the bottom row
/// (1/2/3) the way an un-positioned subtitle sits. `Start`/`Left` →
/// bottom-left (1), `Center` → bottom-centre (2), `End`/`Right` →
/// bottom-right (3).
fn align_to_numpad(a: oxideav_core::TextAlign) -> u8 {
    use oxideav_core::TextAlign::*;
    match a {
        Start | Left => 1,
        Center => 2,
        End | Right => 3,
    }
}

/// The fully resolved styling state in effect over one [`ResolvedSpan`].
///
/// Built by folding the override stream over a [`StyleBase`]; every field
/// holds the effective value at the span, with reset (`None`-parameter)
/// tags restoring the base value.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedStyle {
    pub font_name: String,
    pub font_size: f64,
    pub bold: bool,
    pub weight: Option<u32>,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub primary: Rgba,
    pub secondary: Rgba,
    pub outline_color: Rgba,
    pub shadow_color: Rgba,
    pub border_x: f64,
    pub border_y: f64,
    pub shadow_x: f64,
    pub shadow_y: f64,
    pub scale_x: f64,
    pub scale_y: f64,
    pub spacing: f64,
    pub angle_x: f64,
    pub angle_y: f64,
    pub angle_z: f64,
    /// `\be` edge-softening pass count.
    pub blur_be: f64,
    /// `\blur` gaussian strength.
    pub blur_gauss: f64,
    pub encoding: i32,
}

impl ResolvedStyle {
    fn from_base(b: &StyleBase) -> ResolvedStyle {
        ResolvedStyle {
            font_name: b.font_name.clone(),
            font_size: b.font_size,
            bold: b.bold,
            weight: b.weight,
            italic: b.italic,
            underline: b.underline,
            strike: b.strike,
            primary: b.primary,
            secondary: b.secondary,
            outline_color: b.outline_color,
            shadow_color: b.shadow_color,
            border_x: b.border,
            border_y: b.border,
            shadow_x: b.shadow,
            shadow_y: b.shadow,
            scale_x: b.scale_x,
            scale_y: b.scale_y,
            spacing: b.spacing,
            angle_x: 0.0,
            angle_y: 0.0,
            angle_z: b.angle_z,
            blur_be: 0.0,
            blur_gauss: 0.0,
            encoding: b.encoding,
        }
    }
}

/// One contiguous run of visible text plus the styling that applies to it.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedSpan {
    /// The visible text of this run, with `\N`/`\n`/`\h` already turned
    /// into `\n` / space / NBSP per the line's wrap mode (no wrap context
    /// here — `\n` becomes a space, matching `WrapStyle` default 0/1).
    pub text: String,
    /// The resolved styling state in effect over `text`.
    pub style: ResolvedStyle,
    /// `Some(cs)` when a `\k`-family karaoke beat opened immediately
    /// before this run, carrying its centisecond duration; `None`
    /// otherwise.
    pub karaoke_cs: Option<u32>,
}

/// A clip region resolved from a `\clip` / `\iclip` override.
#[derive(Clone, Debug, PartialEq)]
pub enum ClipRegion {
    Rectangle {
        inverse: bool,
        x1: i32,
        y1: i32,
        x2: i32,
        y2: i32,
    },
    Drawing {
        inverse: bool,
        scale: Option<u32>,
        commands: String,
    },
}

/// A resolved `\move(x1, y1, x2, y2[, t1, t2])` line movement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Move {
    pub x1: i32,
    pub y1: i32,
    pub x2: i32,
    pub y2: i32,
    /// Optional `(t1, t2)` animation window in milliseconds.
    pub times: Option<(u32, u32)>,
}

/// Whole-line layout state collected from the line-property override tags
/// (`\pos`, `\move`, `\org`, `\fad` / `\fade`, `\clip` / `\iclip`,
/// `\an` / `\a`). Per the Aegisub reference these "should appear at most
/// once in a line"; when one appears more than once the *last* occurrence
/// wins (libass applies the override in source order).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LineLayout {
    /// `\pos(x, y)`.
    pub pos: Option<(i32, i32)>,
    /// `\move(x1, y1, x2, y2[, t1, t2])`.
    pub mv: Option<Move>,
    /// `\org(x, y)`.
    pub org: Option<(i32, i32)>,
    /// The resolved alignment override (numpad 1..=9), or `None` if the
    /// line never overrode the style alignment.
    pub alignment: Option<u8>,
    /// `\fad` / `\fade` fade spec.
    pub fade: Option<AssFadeSpec>,
    /// `\clip` / `\iclip` region.
    pub clip: Option<ClipRegion>,
}

/// The full resolution of a Dialogue `Text` field: per-run resolved
/// styling plus the whole-line layout state.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedLine {
    pub spans: Vec<ResolvedSpan>,
    pub layout: LineLayout,
}

/// Resolve a Dialogue `Text` field against a base style.
///
/// Tokenizes `text` with [`tokenize`] and folds the override stream over
/// `base`, emitting a [`ResolvedSpan`] for each maximal run of visible
/// text under a constant resolved style. Whole-line property tags are
/// collected into the returned [`LineLayout`].
pub fn resolve_line(text: &str, base: &StyleBase) -> ResolvedLine {
    resolve_tokens(&tokenize(text), base)
}

/// Resolve an already-tokenized Dialogue `Text` field. The token-level
/// entry point behind [`resolve_line`].
pub fn resolve_tokens(tokens: &[AssToken], base: &StyleBase) -> ResolvedLine {
    let mut cur = ResolvedStyle::from_base(base);
    let mut layout = LineLayout::default();
    let mut spans: Vec<ResolvedSpan> = Vec::new();
    let mut pending = String::new();
    // Karaoke beat opened by the most recent `\k` since the last flush.
    let mut pending_k: Option<u32> = None;
    // Style snapshot the current `pending` buffer is accumulating under.
    let mut buf_style = cur.clone();
    let mut buf_k: Option<u32> = None;

    // Flush the pending text under buf_style into a span.
    macro_rules! flush {
        () => {
            if !pending.is_empty() {
                spans.push(ResolvedSpan {
                    text: std::mem::take(&mut pending),
                    style: buf_style.clone(),
                    karaoke_cs: buf_k.take(),
                });
            }
        };
    }

    for tok in tokens {
        match tok {
            AssToken::Text(s) => {
                if pending.is_empty() {
                    // Start a fresh buffer under the current state.
                    buf_style = cur.clone();
                    buf_k = pending_k.take();
                }
                pending.push_str(s);
            }
            AssToken::SoftBreak => {
                if pending.is_empty() {
                    buf_style = cur.clone();
                    buf_k = pending_k.take();
                }
                pending.push(' ');
            }
            AssToken::HardBreak => {
                if pending.is_empty() {
                    buf_style = cur.clone();
                    buf_k = pending_k.take();
                }
                pending.push('\n');
            }
            AssToken::HardSpace => {
                if pending.is_empty() {
                    buf_style = cur.clone();
                    buf_k = pending_k.take();
                }
                pending.push('\u{00A0}');
            }
            AssToken::Override(tags) => {
                // An override block that changes visible styling closes
                // the current run.
                flush!();
                for tag in tags {
                    apply_tag(tag, base, &mut cur, &mut layout, &mut pending_k);
                }
            }
        }
    }
    flush!();

    ResolvedLine { spans, layout }
}

/// Apply one override tag to the running resolved-style / layout state.
fn apply_tag(
    tag: &AssTag,
    base: &StyleBase,
    cur: &mut ResolvedStyle,
    layout: &mut LineLayout,
    pending_k: &mut Option<u32>,
) {
    use crate::ass_tags::{AssBlurKind, AssBorderAxis, AssRotationAxis};
    match tag {
        AssTag::Bold(v) => match v {
            None => {
                cur.bold = base.bold;
                cur.weight = base.weight;
            }
            Some(0) => {
                cur.bold = false;
                cur.weight = None;
            }
            Some(1) => {
                cur.bold = true;
                cur.weight = None;
            }
            Some(w) => {
                cur.bold = *w >= 700;
                cur.weight = Some(*w);
            }
        },
        AssTag::Italic(v) => cur.italic = v.unwrap_or(base.italic),
        AssTag::Underline(v) => cur.underline = v.unwrap_or(base.underline),
        AssTag::Strikeout(v) => cur.strike = v.unwrap_or(base.strike),
        AssTag::Color { target, hex, .. } => {
            let (slot, base_c) = color_slot(cur, base, *target);
            match hex {
                None => set_rgb(slot, (base_c.r, base_c.g, base_c.b)),
                Some(h) => {
                    if let Some(rgb) = decode_bgr_hex(h) {
                        set_rgb(slot, rgb);
                    }
                }
            }
        }
        AssTag::Alpha { target, hex } => {
            // `\alpha` (target None) sets all four components at once.
            let targets: &[AssColorTarget] = match target {
                Some(t) => std::slice::from_ref(t),
                None => &[
                    AssColorTarget::Primary,
                    AssColorTarget::Secondary,
                    AssColorTarget::Border,
                    AssColorTarget::Shadow,
                ],
            };
            for t in targets {
                let (slot, base_c) = color_slot(cur, base, *t);
                match hex {
                    None => slot.a = base_c.a,
                    Some(h) => {
                        if let Some(av) = decode_alpha_hex(h) {
                            // ASS alpha: 00 opaque, FF transparent → invert.
                            slot.a = 255 - av;
                        }
                    }
                }
            }
        }
        AssTag::FontName(v) => {
            cur.font_name = v.clone().unwrap_or_else(|| base.font_name.clone());
        }
        AssTag::FontSize(v) => {
            cur.font_size = decode_opt(v).unwrap_or(base.font_size);
        }
        AssTag::FontScale { x_axis, percent } => {
            let val = decode_opt(percent);
            if *x_axis {
                cur.scale_x = val.unwrap_or(base.scale_x);
            } else {
                cur.scale_y = val.unwrap_or(base.scale_y);
            }
        }
        AssTag::FontSpacing(v) => {
            cur.spacing = decode_opt(v).unwrap_or(base.spacing);
        }
        AssTag::FontEncoding(v) => {
            cur.encoding = v
                .as_deref()
                .and_then(|s| s.trim().parse::<i32>().ok())
                .unwrap_or(base.encoding);
        }
        AssTag::Rotation { axis, degrees, .. } => {
            let val = decode_opt(degrees);
            match axis {
                AssRotationAxis::X => cur.angle_x = val.unwrap_or(0.0),
                AssRotationAxis::Y => cur.angle_y = val.unwrap_or(0.0),
                AssRotationAxis::Z => cur.angle_z = val.unwrap_or(base.angle_z),
            }
        }
        AssTag::Border { axis, size } => {
            let val = decode_opt(size);
            match axis {
                AssBorderAxis::Both => {
                    cur.border_x = val.unwrap_or(base.border);
                    cur.border_y = val.unwrap_or(base.border);
                }
                AssBorderAxis::X => cur.border_x = val.unwrap_or(base.border),
                AssBorderAxis::Y => cur.border_y = val.unwrap_or(base.border),
            }
        }
        AssTag::Shadow { axis, depth } => {
            let val = decode_opt(depth);
            match axis {
                AssBorderAxis::Both => {
                    cur.shadow_x = val.unwrap_or(base.shadow);
                    cur.shadow_y = val.unwrap_or(base.shadow);
                }
                AssBorderAxis::X => cur.shadow_x = val.unwrap_or(base.shadow),
                AssBorderAxis::Y => cur.shadow_y = val.unwrap_or(base.shadow),
            }
        }
        AssTag::Blur { kind, strength } => {
            let val = decode_opt(strength).unwrap_or(0.0);
            match kind {
                AssBlurKind::Edge => cur.blur_be = val,
                AssBlurKind::Gaussian => cur.blur_gauss = val,
            }
        }
        AssTag::Karaoke { centisec, .. } => {
            *pending_k = Some(*centisec);
        }
        // --- whole-line property tags ---
        AssTag::AlignNumpad(v) => {
            layout.alignment = match v {
                Some(n) if (1..=9).contains(n) => Some(*n),
                _ => Some(base.alignment),
            };
        }
        AssTag::AlignLegacy(v) => {
            layout.alignment = match v {
                Some(a) => legacy_align_to_numpad(*a).or(Some(base.alignment)),
                None => Some(base.alignment),
            };
        }
        AssTag::Pos { x, y } => layout.pos = Some((*x, *y)),
        AssTag::Move {
            x1,
            y1,
            x2,
            y2,
            times,
        } => {
            layout.mv = Some(Move {
                x1: *x1,
                y1: *y1,
                x2: *x2,
                y2: *y2,
                times: *times,
            })
        }
        AssTag::Org { x, y } => layout.org = Some((*x, *y)),
        AssTag::Fade(spec) => layout.fade = Some(*spec),
        AssTag::Clip { inverse, shape } => {
            layout.clip = Some(match shape {
                AssClipShape::Rectangle { x1, y1, x2, y2 } => ClipRegion::Rectangle {
                    inverse: *inverse,
                    x1: *x1,
                    y1: *y1,
                    x2: *x2,
                    y2: *y2,
                },
                AssClipShape::Drawing { scale, commands } => ClipRegion::Drawing {
                    inverse: *inverse,
                    scale: *scale,
                    commands: commands.clone(),
                },
            });
        }
        // `\t(...)` describes a time-varying end state; a single static
        // resolved style can't represent it, so we leave the running
        // state untouched (a renderer animates separately). Likewise the
        // drawing-mode / baseline / comment tags carry no styling we fold.
        AssTag::Transform { .. }
        | AssTag::Drawing(_)
        | AssTag::BaselineOffset(_)
        | AssTag::Other(_)
        | AssTag::Comment(_) => {}
    }
}

/// Borrow the mutable colour slot for a target and return its base value.
fn color_slot<'a>(
    cur: &'a mut ResolvedStyle,
    base: &StyleBase,
    target: AssColorTarget,
) -> (&'a mut Rgba, Rgba) {
    match target {
        AssColorTarget::Primary => (&mut cur.primary, base.primary),
        AssColorTarget::Secondary => (&mut cur.secondary, base.secondary),
        AssColorTarget::Border => (&mut cur.outline_color, base.outline_color),
        AssColorTarget::Shadow => (&mut cur.shadow_color, base.shadow_color),
    }
}

fn set_rgb(slot: &mut Rgba, rgb: (u8, u8, u8)) {
    slot.r = rgb.0;
    slot.g = rgb.1;
    slot.b = rgb.2;
}

fn decode_opt(v: &Option<String>) -> Option<f64> {
    v.as_deref().and_then(crate::ass_tags::decode_decimal)
}
