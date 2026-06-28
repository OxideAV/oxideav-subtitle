//! Time evaluation of the ASS / SSA time-varying override tags.
//!
//! [`crate::ass_resolve`] folds an override-tag stream into a static
//! [`ResolvedLine`] — per-run [`ResolvedStyle`] plus a [`LineLayout`]
//! carrying the whole-line property tags. It deliberately stops short of
//! the *time-varying* tags: `\move` moves the line, `\fad` / `\fade` ramp
//! its opacity, `\t(...)` animates a subset of the style overrides, and
//! the `\k`-family karaoke beats sweep a per-syllable fill — none of which
//! a single static snapshot can represent.
//!
//! This module is the evaluator: given a [`ResolvedLine`] (and, for the
//! tags that need it, the line's total on-screen duration) plus a time
//! `t` in milliseconds *relative to the start of the line*, it produces
//! the effective position, fade opacity, animated style, and karaoke fill
//! at that instant. A renderer calls it once per output frame to drive
//! the animation.
//!
//! All four behaviours come straight from the Aegisub override-tag
//! reference (mirrored at `docs/subtitles/ass/aegisub-ass-tags.html`):
//!
//! * **`\move(x1,y1,x2,y2[,t1,t2])`** — "The subtitle starts out at point
//!   (x1, y1) and moves with constant speed so it ends up at (x2, y2)."
//!   "Before t1, the subtitle is stationary at point (x1, y1). Between t1
//!   and t2, the subtitle moves with constant speed … After t2 the
//!   subtitle is stationary at point (x2, y2)." "Specifying both t1 and t2
//!   as 0 … is the same as using the first version … the movement will
//!   occur from the start time of the line to the end time of the line."
//! * **`\fad(<fadein>,<fadeout>)`** — "Produce a fade-in and fade-out
//!   effect. The fadein and fadeout times are given in milliseconds …
//!   the start or end of the line's display time is used for the fade
//!   effect."
//! * **`\fade(<a1>,<a2>,<a3>,<t1>,<t2>,<t3>,<t4>)`** — "Before t1, the
//!   line has alpha a1. Between t1 and t2 the line fades from alpha a1 to
//!   alpha a2. Between t2 and t3 the line has alpha a2 constantly. Between
//!   t3 and t4 the line fades from alpha a2 to alpha a3. After t4 the line
//!   has alpha a3." Alphas are "between 0 and 255, with 0 being fully
//!   visible and 255 being invisible".
//! * **`\t([t1,t2,][accel,]<modifiers>)`** — "Before t1, the style is as
//!   all tags before the \t tag specify. After t2 the style is … further
//!   overridden by the given style overrides. Between t1 and t2 the style
//!   is gradually animated … following the acceleration function." The
//!   factor is "y = x with x ∈ `[0;1]` = (t - t1)/(t2 - t1)" raised to the
//!   accel exponent (accel 1 linear, 0..1 fast-then-slow, >1 slow-then-fast).

use crate::ass_resolve::{LineLayout, Move, ResolvedStyle, Rgba};
use crate::ass_tags::{
    decode_alpha_hex, decode_bgr_hex, AssBlurKind, AssBorderAxis, AssColorTarget, AssFadeSpec,
    AssRotationAxis, AssTag,
};

/// The fade opacity multiplier at a given instant, in the ASS alpha
/// convention: `0` fully visible (opaque), `255` fully invisible
/// (transparent). A renderer multiplies this into every component's
/// alpha. When a line carries no `\fad` / `\fade`, the multiplier is `0`
/// (the line is fully visible throughout).
///
/// The value is computed by [`fade_alpha_at`].
pub type FadeAlpha = u8;

/// Linear interpolation between two `u8` alpha endpoints by a fraction in
/// `[0, 1]`, rounded to nearest. Used by both fade forms.
fn lerp_alpha(a: u8, b: u8, frac: f64) -> u8 {
    let frac = frac.clamp(0.0, 1.0);
    let v = a as f64 + (b as f64 - a as f64) * frac;
    // Round to nearest; clamp guards float error at the endpoints.
    v.round().clamp(0.0, 255.0) as u8
}

/// Evaluate the fade opacity of a line at time `t` (milliseconds relative
/// to the line's start), given the line's total on-screen `duration_ms`.
///
/// Returns the ASS-convention alpha multiplier (`0` visible … `255`
/// invisible) the renderer applies on top of each component's own alpha.
///
/// * `\fad(fadein, fadeout)` ramps alpha `255 → 0` over the first
///   `fadein` ms, holds `0` in the middle, and ramps `0 → 255` over the
///   last `fadeout` ms (measured back from `duration_ms`). A `0` on
///   either end disables that ramp. When `fadein + fadeout` exceeds the
///   duration the two ramps are clamped so the line never goes more than
///   fully transparent (the fade-out endpoint wins at the overlap, since
///   it is evaluated against the later time).
/// * `\fade(a1, a2, a3, t1, t2, t3, t4)` follows the five-part schedule
///   verbatim.
///
/// With no fade on the layout the result is `0` (fully visible).
pub fn fade_alpha_at(layout: &LineLayout, t: i64, duration_ms: i64) -> FadeAlpha {
    match layout.fade {
        None => 0,
        Some(AssFadeSpec::Simple { fadein, fadeout }) => {
            simple_fade_alpha(fadein as i64, fadeout as i64, t, duration_ms)
        }
        Some(AssFadeSpec::Complex {
            a1,
            a2,
            a3,
            t1,
            t2,
            t3,
            t4,
        }) => complex_fade_alpha(a1, a2, a3, t1 as i64, t2 as i64, t3 as i64, t4 as i64, t),
    }
}

/// `\fad`: fade-in over `[0, fadein)`, hold visible, fade-out over
/// `[duration - fadeout, duration]`. Alpha is `255` (invisible) at the
/// outer edges and `0` (visible) in the middle.
fn simple_fade_alpha(fadein: i64, fadeout: i64, t: i64, duration_ms: i64) -> u8 {
    // Fade-in contribution: 255 at t=0 → 0 at t=fadein.
    let in_alpha = if fadein > 0 && t < fadein {
        let frac = t.max(0) as f64 / fadein as f64;
        lerp_alpha(255, 0, frac)
    } else {
        0
    };
    // Fade-out contribution: 0 until duration-fadeout → 255 at duration.
    let out_start = duration_ms - fadeout;
    let out_alpha = if fadeout > 0 && t > out_start {
        let frac = (t - out_start) as f64 / fadeout as f64;
        lerp_alpha(0, 255, frac)
    } else {
        0
    };
    // The two ramps don't overlap on a well-formed line; on an
    // overlapping line take the more-transparent (larger) value so the
    // line never appears more opaque than either ramp allows.
    in_alpha.max(out_alpha)
}

/// `\fade`: the five-part schedule. `t1..=t4` are absolute line-relative
/// times; the two ramps interpolate `a1→a2` and `a2→a3`.
#[allow(clippy::too_many_arguments)]
fn complex_fade_alpha(a1: u8, a2: u8, a3: u8, t1: i64, t2: i64, t3: i64, t4: i64, t: i64) -> u8 {
    if t < t1 {
        a1
    } else if t < t2 {
        let span = (t2 - t1).max(1);
        lerp_alpha(a1, a2, (t - t1) as f64 / span as f64)
    } else if t < t3 {
        a2
    } else if t < t4 {
        let span = (t4 - t3).max(1);
        lerp_alpha(a2, a3, (t - t3) as f64 / span as f64)
    } else {
        a3
    }
}

/// Evaluate the line's screen position at time `t` (milliseconds relative
/// to the line's start), given the line's total on-screen `duration_ms`.
///
/// Returns `Some((x, y))` in script-resolution pixels when the line
/// carries a `\move` or `\pos`, or `None` when it carries neither (the
/// renderer then falls back to alignment-driven default placement).
///
/// `\move` wins over `\pos` when both are present, matching the
/// "stationary `\pos`" being the degenerate `\move` with equal endpoints.
/// The movement is constant-speed between `(x1, y1)` and `(x2, y2)` over
/// `[t1, t2]`; before `t1` the line sits at the start point and after
/// `t2` at the end point. A `\move` with no explicit times (or both
/// times `0`) animates over the whole `[0, duration_ms]` window.
pub fn position_at(layout: &LineLayout, t: i64, duration_ms: i64) -> Option<(f64, f64)> {
    if let Some(mv) = layout.mv {
        return Some(move_position(&mv, t, duration_ms));
    }
    layout.pos.map(|(x, y)| (x as f64, y as f64))
}

fn move_position(mv: &Move, t: i64, duration_ms: i64) -> (f64, f64) {
    let (t1, t2) = match mv.times {
        // Both-zero times degenerate to the whole-line window per spec.
        Some((0, 0)) | None => (0i64, duration_ms),
        Some((a, b)) => (a as i64, b as i64),
    };
    let frac = if t <= t1 {
        0.0
    } else if t >= t2 {
        1.0
    } else {
        let span = (t2 - t1).max(1);
        (t - t1) as f64 / span as f64
    };
    let x = mv.x1 as f64 + (mv.x2 - mv.x1) as f64 * frac;
    let y = mv.y1 as f64 + (mv.y2 - mv.y1) as f64 * frac;
    (x, y)
}

// --- \t(...) animated-transform style evaluation -----------------------

/// Compute the `\t(...)` interpolation factor at line-relative time `t`
/// (ms) for an animation window `[t1, t2]` with acceleration exponent
/// `accel`, given the line's total `duration_ms`.
///
/// Per the reference the raw progress is `x = (t - t1)/(t2 - t1)` clamped
/// to `[0, 1]`, and the eased factor is `x^accel` (accel `1` linear, in
/// `(0, 1)` fast-then-slow, `> 1` slow-then-fast). When the `\t` carried
/// no explicit times, `t1` defaults to `0` and `t2` to `duration_ms` (the
/// whole line); pass those resolved values in.
///
/// A zero-width window (`t2 <= t1`) snaps to `1.0` for `t >= t1` and `0.0`
/// before, so a degenerate `\t` behaves as an instant step at `t1`.
pub fn transform_factor(t1: i64, t2: i64, accel: f64, t: i64) -> f64 {
    if t2 <= t1 {
        return if t >= t1 { 1.0 } else { 0.0 };
    }
    let x = ((t - t1) as f64 / (t2 - t1) as f64).clamp(0.0, 1.0);
    let accel = if accel.is_finite() && accel > 0.0 {
        accel
    } else {
        1.0
    };
    if (accel - 1.0).abs() < f64::EPSILON {
        x
    } else {
        x.powf(accel)
    }
}

/// Resolve a `\t`'s `(t1, t2)` window against the line duration. `t1`
/// defaults to `0` and `t2` to `duration_ms` when the tag carried no
/// explicit keyframe times; a both-present pair is taken verbatim.
fn resolve_t_window(t1: Option<u32>, t2: Option<u32>, duration_ms: i64) -> (i64, i64) {
    match (t1, t2) {
        (Some(a), Some(b)) => (a as i64, b as i64),
        _ => (0, duration_ms),
    }
}

/// The subset of `\t` modifiers this evaluator animates, mapped onto the
/// matching [`ResolvedStyle`] field. The reference lists exactly these as
/// the animatable tags (font metrics + geometry + colour/alpha/blur);
/// every other modifier inside a `\t(...)` is ignored here.
fn apply_animatable_tag(tag: &AssTag, cur: &mut ResolvedStyle) {
    match tag {
        AssTag::FontSize(v) => {
            if let Some(x) = decode_opt(v) {
                cur.font_size = x;
            }
        }
        AssTag::FontScale { x_axis, percent } => {
            if let Some(x) = decode_opt(percent) {
                if *x_axis {
                    cur.scale_x = x;
                } else {
                    cur.scale_y = x;
                }
            }
        }
        AssTag::FontSpacing(v) => {
            if let Some(x) = decode_opt(v) {
                cur.spacing = x;
            }
        }
        AssTag::Rotation { axis, degrees, .. } => {
            if let Some(x) = decode_opt(degrees) {
                match axis {
                    AssRotationAxis::X => cur.angle_x = x,
                    AssRotationAxis::Y => cur.angle_y = x,
                    AssRotationAxis::Z => cur.angle_z = x,
                }
            }
        }
        AssTag::Border { axis, size } => {
            if let Some(x) = decode_opt(size) {
                match axis {
                    AssBorderAxis::Both => {
                        cur.border_x = x;
                        cur.border_y = x;
                    }
                    AssBorderAxis::X => cur.border_x = x,
                    AssBorderAxis::Y => cur.border_y = x,
                }
            }
        }
        AssTag::Shadow { axis, depth } => {
            if let Some(x) = decode_opt(depth) {
                match axis {
                    AssBorderAxis::Both => {
                        cur.shadow_x = x;
                        cur.shadow_y = x;
                    }
                    AssBorderAxis::X => cur.shadow_x = x,
                    AssBorderAxis::Y => cur.shadow_y = x,
                }
            }
        }
        AssTag::Blur { kind, strength } => {
            if let Some(x) = decode_opt(strength) {
                match kind {
                    AssBlurKind::Edge => cur.blur_be = x,
                    AssBlurKind::Gaussian => cur.blur_gauss = x,
                }
            }
        }
        AssTag::Color { target, hex, .. } => {
            if let Some(rgb) = hex.as_deref().and_then(decode_bgr_hex) {
                let slot = color_slot(cur, *target);
                slot.r = rgb.0;
                slot.g = rgb.1;
                slot.b = rgb.2;
            }
        }
        AssTag::Alpha { target, hex } => {
            if let Some(av) = hex.as_deref().and_then(decode_alpha_hex) {
                // ASS wire alpha (00 opaque, FF transparent) inverts to
                // straight alpha to match ResolvedStyle's convention.
                let straight = 255 - av;
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
                    color_slot(cur, *t).a = straight;
                }
            }
        }
        _ => {}
    }
}

fn color_slot(cur: &mut ResolvedStyle, target: AssColorTarget) -> &mut Rgba {
    match target {
        AssColorTarget::Primary => &mut cur.primary,
        AssColorTarget::Secondary => &mut cur.secondary,
        AssColorTarget::Border => &mut cur.outline_color,
        AssColorTarget::Shadow => &mut cur.shadow_color,
    }
}

fn decode_opt(v: &Option<String>) -> Option<f64> {
    v.as_deref().and_then(crate::ass_tags::decode_decimal)
}

fn lerp_f64(a: f64, b: f64, frac: f64) -> f64 {
    a + (b - a) * frac
}

fn lerp_rgba(a: Rgba, b: Rgba, frac: f64) -> Rgba {
    Rgba {
        r: lerp_alpha(a.r, b.r, frac),
        g: lerp_alpha(a.g, b.g, frac),
        b: lerp_alpha(a.b, b.b, frac),
        a: lerp_alpha(a.a, b.a, frac),
    }
}

/// Interpolate every animatable [`ResolvedStyle`] field between `from`
/// (the pre-`\t` state) and `to` (the state after all the `\t` modifiers
/// apply) by `frac` in `[0, 1]`. Non-animatable fields (bold / italic /
/// underline / strike / weight / font name / encoding) snap to `to` once
/// `frac` reaches `1.0` and otherwise stay at `from`, matching the
/// "before t1 … after t2 … overridden by the given overrides" rule for
/// the tags the reference does not list as animatable.
fn interpolate_style(from: &ResolvedStyle, to: &ResolvedStyle, frac: f64) -> ResolvedStyle {
    let frac = frac.clamp(0.0, 1.0);
    let snapped = if frac >= 1.0 { to } else { from };
    ResolvedStyle {
        font_name: snapped.font_name.clone(),
        bold: snapped.bold,
        weight: snapped.weight,
        italic: snapped.italic,
        underline: snapped.underline,
        strike: snapped.strike,
        encoding: snapped.encoding,
        font_size: lerp_f64(from.font_size, to.font_size, frac),
        scale_x: lerp_f64(from.scale_x, to.scale_x, frac),
        scale_y: lerp_f64(from.scale_y, to.scale_y, frac),
        spacing: lerp_f64(from.spacing, to.spacing, frac),
        angle_x: lerp_f64(from.angle_x, to.angle_x, frac),
        angle_y: lerp_f64(from.angle_y, to.angle_y, frac),
        angle_z: lerp_f64(from.angle_z, to.angle_z, frac),
        border_x: lerp_f64(from.border_x, to.border_x, frac),
        border_y: lerp_f64(from.border_y, to.border_y, frac),
        shadow_x: lerp_f64(from.shadow_x, to.shadow_x, frac),
        shadow_y: lerp_f64(from.shadow_y, to.shadow_y, frac),
        blur_be: lerp_f64(from.blur_be, to.blur_be, frac),
        blur_gauss: lerp_f64(from.blur_gauss, to.blur_gauss, frac),
        primary: lerp_rgba(from.primary, to.primary, frac),
        secondary: lerp_rgba(from.secondary, to.secondary, frac),
        outline_color: lerp_rgba(from.outline_color, to.outline_color, frac),
        shadow_color: lerp_rgba(from.shadow_color, to.shadow_color, frac),
    }
}

/// Evaluate the animated style of a span at line-relative time `t` (ms),
/// given the line's total `duration_ms`.
///
/// `base` is the static span style produced by [`crate::ass_resolve`]
/// (the state *before* any `\t` modifiers apply — the resolver leaves
/// `\t` unfolded for exactly this reason). `transforms` is the list of
/// `\t(...)` tags in effect over the span, in source order. Each `\t`
/// builds its own target state by applying its animatable modifiers to a
/// running copy and interpolating per the per-tag acceleration factor; a
/// later `\t` animates on top of the earlier ones' result, so stacked
/// `\t(...)` blocks compose.
///
/// When `transforms` is empty (or none of them carry animatable
/// modifiers) the returned style equals `base`.
pub fn animate_style_at(
    base: &ResolvedStyle,
    transforms: &[&AssTag],
    t: i64,
    duration_ms: i64,
) -> ResolvedStyle {
    let mut cur = base.clone();
    for tag in transforms {
        if let AssTag::Transform {
            t1,
            t2,
            accel,
            modifiers,
        } = tag
        {
            let (w1, w2) = resolve_t_window(*t1, *t2, duration_ms);
            let accel_val = accel
                .as_deref()
                .and_then(crate::ass_tags::decode_decimal)
                .unwrap_or(1.0);
            let factor = transform_factor(w1, w2, accel_val, t);
            // Target = current state with all the \t's animatable
            // modifiers applied; interpolate from current toward it.
            let mut target = cur.clone();
            for m in modifiers {
                apply_animatable_tag(m, &mut target);
            }
            cur = interpolate_style(&cur, &target, factor);
        }
    }
    cur
}

/// Collect every `\t(...)` tag from a resolved Dialogue `Text` token
/// stream, in source order, as borrowed references suitable for
/// [`animate_style_at`]. A renderer that already holds the
/// [`crate::ass_tags::AssToken`] stream can pull the transforms out
/// without re-tokenizing.
pub fn collect_transforms(tokens: &[crate::ass_tags::AssToken]) -> Vec<&AssTag> {
    let mut out = Vec::new();
    for tok in tokens {
        if let crate::ass_tags::AssToken::Override(tags) = tok {
            for tag in tags {
                if matches!(tag, AssTag::Transform { .. }) {
                    out.push(tag);
                }
            }
        }
    }
    out
}

// --- \k-family karaoke fill evaluation ---------------------------------

use crate::ass_tags::{AssKaraokeKind, AssToken};

/// One karaoke syllable: the visible text run a `\k`-family beat opened,
/// its highlight kind, its cumulative timeline window, and the fill state
/// at the evaluated instant.
///
/// Produced by [`karaoke_fills`].
#[derive(Clone, Debug, PartialEq)]
pub struct KaraokeSyllable {
    /// The visible text of the syllable (between this beat and the next).
    pub text: String,
    /// Which highlight effect the beat selected.
    pub kind: AssKaraokeKind,
    /// Cumulative start time of the syllable in milliseconds, measured
    /// from the line start (the sum of all earlier beat durations).
    pub start_ms: u32,
    /// The syllable's own beat duration in milliseconds.
    pub dur_ms: u32,
    /// Fill fraction at the evaluated time: `0.0` un-highlighted
    /// (secondary fill / removed outline), `1.0` fully highlighted
    /// (primary fill / outline present). For an instant `\k` / `\ko`
    /// this is `0.0` before the syllable starts and `1.0` from its
    /// start onward; for a sweeping `\K` / `\kf` it ramps linearly from
    /// `0.0` to `1.0` across the syllable's own window.
    pub fill: f64,
}

/// Evaluate the per-syllable karaoke fill of a Dialogue `Text` token
/// stream at time `t` (milliseconds relative to the line start).
///
/// Each `\k` / `\K` / `\kf` / `\ko` beat opens a syllable whose start is
/// the running sum of all earlier beat durations (centiseconds → ms) and
/// whose duration is the beat's own value. The visible text up to the
/// next beat (or the end of the line) is the syllable body; `\N` / `\n` /
/// `\h` and intervening non-karaoke override blocks pass through into the
/// current syllable's text.
///
/// Text that precedes the first `\k` beat carries no karaoke and is not
/// returned as a syllable (a renderer draws it in the resolved primary
/// fill). When the line has no `\k`-family tag the result is empty.
pub fn karaoke_fills(tokens: &[AssToken], t: i64) -> Vec<KaraokeSyllable> {
    let mut out: Vec<KaraokeSyllable> = Vec::new();
    // Cumulative start of the syllable currently being accumulated, in ms.
    let mut cursor_ms: u32 = 0;
    // Index in `out` of the syllable currently accumulating text, if any.
    let mut open: Option<usize> = None;

    for tok in tokens {
        match tok {
            AssToken::Text(s) => push_syllable_text(&mut out, open, s),
            AssToken::SoftBreak => push_syllable_text(&mut out, open, " "),
            AssToken::HardBreak => push_syllable_text(&mut out, open, "\n"),
            AssToken::HardSpace => push_syllable_text(&mut out, open, "\u{00A0}"),
            AssToken::Override(tags) => {
                for tag in tags {
                    if let AssTag::Karaoke { kind, centisec } = tag {
                        let dur_ms = centisec.saturating_mul(10);
                        out.push(KaraokeSyllable {
                            text: String::new(),
                            kind: *kind,
                            start_ms: cursor_ms,
                            dur_ms,
                            fill: 0.0,
                        });
                        open = Some(out.len() - 1);
                        cursor_ms = cursor_ms.saturating_add(dur_ms);
                    }
                }
            }
        }
    }

    // Resolve every syllable's fill at time t now that windows are known.
    for syl in &mut out {
        syl.fill = syllable_fill(syl.kind, syl.start_ms as i64, syl.dur_ms as i64, t);
    }
    out
}

fn push_syllable_text(out: &mut [KaraokeSyllable], open: Option<usize>, s: &str) {
    if let Some(i) = open {
        out[i].text.push_str(s);
    }
}

/// Fill fraction of one syllable at time `t` (ms). Instant kinds (`\k`,
/// `\ko`) step from `0.0` to `1.0` at the syllable start; sweeping kinds
/// (`\K`, `\kf`) ramp linearly across the syllable's own duration.
fn syllable_fill(kind: AssKaraokeKind, start_ms: i64, dur_ms: i64, t: i64) -> f64 {
    match kind {
        AssKaraokeKind::Instant | AssKaraokeKind::Outline => {
            if t >= start_ms {
                1.0
            } else {
                0.0
            }
        }
        AssKaraokeKind::Sweep | AssKaraokeKind::SweepCap => {
            if t <= start_ms {
                0.0
            } else if dur_ms <= 0 || t >= start_ms + dur_ms {
                1.0
            } else {
                (t - start_ms) as f64 / dur_ms as f64
            }
        }
    }
}

// --- combined per-frame line evaluation --------------------------------

use crate::ass_resolve::ResolvedLine;

/// One visible run of an [`EvaluatedLine`]: the run's text plus the
/// fully animated [`ResolvedStyle`] in effect over it at the evaluated
/// instant and its karaoke fill fraction (`1.0` when the run carries no
/// `\k` beat).
#[derive(Clone, Debug, PartialEq)]
pub struct EvaluatedSpan {
    /// The visible text of the run.
    pub text: String,
    /// The animated style at the evaluated time (the resolved span style
    /// with every active `\t(...)` applied for the run's `\t` scope).
    pub style: ResolvedStyle,
    /// Karaoke fill fraction: `0.0` un-highlighted, `1.0` highlighted;
    /// `1.0` for a run with no `\k`-family beat.
    pub karaoke_fill: f64,
}

/// The full evaluation of a Dialogue line at one instant: the effective
/// screen position, fade opacity, and per-run animated style + karaoke
/// fill. A renderer calls [`evaluate_line_at`] once per output frame and
/// draws each [`EvaluatedSpan`] at `position` (or alignment-driven
/// default when `None`) with `fade_alpha` multiplied into every run's
/// component alpha.
#[derive(Clone, Debug, PartialEq)]
pub struct EvaluatedLine {
    /// Effective `(x, y)` from `\move` / `\pos`, or `None` (default
    /// placement) when the line carries neither.
    pub position: Option<(f64, f64)>,
    /// Whole-line fade opacity multiplier (`0` visible … `255` invisible).
    pub fade_alpha: FadeAlpha,
    /// The animated visible runs in source order.
    pub spans: Vec<EvaluatedSpan>,
    /// The line layout the evaluation was taken against (alignment, clip,
    /// origin, wrap), surfaced so a renderer needn't re-resolve.
    pub layout: LineLayout,
}

/// Evaluate a whole Dialogue line at time `t` (milliseconds relative to
/// the line start), given its total on-screen `duration_ms`.
///
/// `line` is the static [`ResolvedLine`] from [`crate::ass_resolve`] and
/// `tokens` its source token stream (needed for the `\t(...)` transform
/// scope and the `\k`-family karaoke timeline). The two must come from
/// the same Dialogue `Text`. Each span is animated by the `\t(...)` tags
/// that were *in scope* when it began — a `\t` only affects the text that
/// follows it, so a `{\t(...)}` at line start animates every span while a
/// mid-line `{\t(...)}` animates only the spans after it.
///
/// Position and fade are whole-line properties evaluated once via
/// [`position_at`] / [`fade_alpha_at`]; karaoke fill is taken from the
/// per-syllable [`karaoke_fills`] timeline, matched to spans by source
/// order.
pub fn evaluate_line_at(
    line: &ResolvedLine,
    tokens: &[AssToken],
    t: i64,
    duration_ms: i64,
) -> EvaluatedLine {
    // Walk the tokens to learn, for each visible run, the set of \t tags
    // in scope at the point the run began. ass_resolve emits one span per
    // maximal constant-style run; we mirror its run boundaries by
    // flushing whenever an override block intervenes between text.
    let mut active: Vec<&AssTag> = Vec::new();
    // Per-run scoped-transform snapshots in source order.
    let mut run_scopes: Vec<Vec<&AssTag>> = Vec::new();
    let mut in_run = false;

    for tok in tokens {
        match tok {
            AssToken::Text(_) | AssToken::SoftBreak | AssToken::HardBreak | AssToken::HardSpace => {
                if !in_run {
                    run_scopes.push(active.clone());
                    in_run = true;
                }
            }
            AssToken::Override(tags) => {
                // An override block closes the current run (matching the
                // resolver's flush) and may add \t tags to the scope.
                in_run = false;
                for tag in tags {
                    if matches!(tag, AssTag::Transform { .. }) {
                        active.push(tag);
                    }
                }
            }
        }
    }

    // Karaoke fills are produced per-syllable in source order; the
    // resolver emits one span per syllable too, so zip by index.
    let fills = karaoke_fills(tokens, t);

    let mut spans = Vec::with_capacity(line.spans.len());
    for (i, span) in line.spans.iter().enumerate() {
        let scope = run_scopes.get(i).map(Vec::as_slice).unwrap_or(&[]);
        let style = animate_style_at(&span.style, scope, t, duration_ms);
        // A span carries a karaoke beat iff the resolver tagged it; fall
        // back to the matching syllable fill, else fully highlighted.
        let karaoke_fill = if span.karaoke_cs.is_some() {
            fills.get(i).map(|s| s.fill).unwrap_or(1.0)
        } else {
            1.0
        };
        spans.push(EvaluatedSpan {
            text: span.text.clone(),
            style,
            karaoke_fill,
        });
    }

    EvaluatedLine {
        position: position_at(&line.layout, t, duration_ms),
        fade_alpha: fade_alpha_at(&line.layout, t, duration_ms),
        spans,
        layout: line.layout.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ass_resolve::{resolve_line, ResolvedLine, StyleBase};
    use crate::ass_tags::tokenize;

    fn layout(text: &str) -> LineLayout {
        resolve_line(text, &StyleBase::default()).layout
    }

    fn resolved(text: &str) -> ResolvedLine {
        resolve_line(text, &StyleBase::default())
    }

    #[test]
    fn no_fade_is_fully_visible() {
        let l = layout("plain text");
        assert_eq!(fade_alpha_at(&l, 0, 5000), 0);
        assert_eq!(fade_alpha_at(&l, 2500, 5000), 0);
    }

    #[test]
    fn simple_fade_in_ramps_255_to_0() {
        // \fad(1000,0): invisible at t=0, visible by t=1000.
        let l = layout("{\\fad(1000,0)}hi");
        assert_eq!(fade_alpha_at(&l, 0, 5000), 255);
        assert_eq!(fade_alpha_at(&l, 500, 5000), 128);
        assert_eq!(fade_alpha_at(&l, 1000, 5000), 0);
        assert_eq!(fade_alpha_at(&l, 3000, 5000), 0);
    }

    #[test]
    fn simple_fade_out_ramps_0_to_255() {
        // \fad(0,1000) on a 5s line: visible until t=4000, gone at t=5000.
        let l = layout("{\\fad(0,1000)}bye");
        assert_eq!(fade_alpha_at(&l, 0, 5000), 0);
        assert_eq!(fade_alpha_at(&l, 4000, 5000), 0);
        assert_eq!(fade_alpha_at(&l, 4500, 5000), 128);
        assert_eq!(fade_alpha_at(&l, 5000, 5000), 255);
    }

    #[test]
    fn simple_fade_both_ends() {
        // \fad(1200,250): the README example.
        let l = layout("{\\fad(1200,250)}x");
        assert_eq!(fade_alpha_at(&l, 0, 4000), 255);
        assert_eq!(fade_alpha_at(&l, 1200, 4000), 0);
        assert_eq!(fade_alpha_at(&l, 2000, 4000), 0);
        assert_eq!(fade_alpha_at(&l, 4000, 4000), 255);
    }

    #[test]
    fn complex_fade_five_part_schedule() {
        // \fade(255,32,224,0,500,2000,2200): the spec example.
        let l = layout("{\\fade(255,32,224,0,500,2000,2200)}x");
        // Before t1=0 → a1; t<t1 never happens (t1=0), so at t=0 we are in
        // the first ramp at frac 0 = a1.
        assert_eq!(fade_alpha_at(&l, 0, 5000), 255);
        // Midway through first ramp (t=250 of [0,500]): 255→32.
        assert_eq!(fade_alpha_at(&l, 250, 5000), 144);
        // Hold region [500,2000] = a2.
        assert_eq!(fade_alpha_at(&l, 1000, 5000), 32);
        // Second ramp midpoint (t=2100 of [2000,2200]): 32→224.
        assert_eq!(fade_alpha_at(&l, 2100, 5000), 128);
        // After t4 = a3.
        assert_eq!(fade_alpha_at(&l, 3000, 5000), 224);
    }

    #[test]
    fn pos_is_static() {
        let l = layout("{\\pos(100,200)}x");
        assert_eq!(position_at(&l, 0, 5000), Some((100.0, 200.0)));
        assert_eq!(position_at(&l, 5000, 5000), Some((100.0, 200.0)));
    }

    #[test]
    fn no_position_returns_none() {
        let l = layout("plain");
        assert_eq!(position_at(&l, 0, 5000), None);
    }

    #[test]
    fn move_whole_line_constant_speed() {
        // \move(100,150,300,350): spans the whole duration.
        let l = layout("{\\move(100,150,300,350)}x");
        assert_eq!(position_at(&l, 0, 4000), Some((100.0, 150.0)));
        assert_eq!(position_at(&l, 2000, 4000), Some((200.0, 250.0)));
        assert_eq!(position_at(&l, 4000, 4000), Some((300.0, 350.0)));
    }

    #[test]
    fn move_with_times_clamps_before_and_after() {
        // \move(100,150,300,350,500,1500): stationary, move, stationary.
        let l = layout("{\\move(100,150,300,350,500,1500)}x");
        assert_eq!(position_at(&l, 0, 4000), Some((100.0, 150.0)));
        assert_eq!(position_at(&l, 500, 4000), Some((100.0, 150.0)));
        assert_eq!(position_at(&l, 1000, 4000), Some((200.0, 250.0)));
        assert_eq!(position_at(&l, 1500, 4000), Some((300.0, 350.0)));
        assert_eq!(position_at(&l, 3000, 4000), Some((300.0, 350.0)));
    }

    #[test]
    fn move_zero_times_is_whole_line() {
        let l = layout("{\\move(0,0,100,100,0,0)}x");
        assert_eq!(position_at(&l, 0, 1000), Some((0.0, 0.0)));
        assert_eq!(position_at(&l, 500, 1000), Some((50.0, 50.0)));
        assert_eq!(position_at(&l, 1000, 1000), Some((100.0, 100.0)));
    }

    #[test]
    fn factor_linear_accel_1() {
        // accel 1 is linear: factor == raw progress.
        assert_eq!(transform_factor(0, 1000, 1.0, 0), 0.0);
        assert_eq!(transform_factor(0, 1000, 1.0, 500), 0.5);
        assert_eq!(transform_factor(0, 1000, 1.0, 1000), 1.0);
        // Clamped outside the window.
        assert_eq!(transform_factor(0, 1000, 1.0, -100), 0.0);
        assert_eq!(transform_factor(0, 1000, 1.0, 2000), 1.0);
    }

    #[test]
    fn factor_accel_curve_shapes() {
        // accel 0.5 (fast-then-slow) > linear at the midpoint.
        let mid_fast = transform_factor(0, 1000, 0.5, 500);
        assert!(mid_fast > 0.5, "accel<1 should be ahead at midpoint");
        // accel 2 (slow-then-fast) < linear at the midpoint.
        let mid_slow = transform_factor(0, 1000, 2.0, 500);
        assert!(mid_slow < 0.5, "accel>1 should lag at midpoint");
        assert!((mid_slow - 0.25).abs() < 1e-9, "0.5^2 == 0.25");
    }

    #[test]
    fn factor_zero_width_window_is_step() {
        assert_eq!(transform_factor(500, 500, 1.0, 499), 0.0);
        assert_eq!(transform_factor(500, 500, 1.0, 500), 1.0);
        assert_eq!(transform_factor(500, 500, 1.0, 600), 1.0);
    }

    #[test]
    fn animate_no_transform_equals_base() {
        let r = resolved("plain");
        let base = &r.spans[0].style;
        let got = animate_style_at(base, &[], 500, 1000);
        assert_eq!(&got, base);
    }

    #[test]
    fn animate_rotation_over_whole_line() {
        // {\an5\t(0,5000,\frz3600)}Wheee — the spec example: 10 turns in 5s.
        let r = resolved("{\\an5\\t(0,5000,\\frz3600)}Wheee");
        let toks = tokenize("{\\an5\\t(0,5000,\\frz3600)}Wheee");
        let xforms = collect_transforms(&toks);
        let base = &r.spans[0].style;
        assert_eq!(base.angle_z, 0.0);
        // Halfway: 1800 degrees.
        let mid = animate_style_at(base, &xforms, 2500, 5000);
        assert!((mid.angle_z - 1800.0).abs() < 1e-6);
        // End: full 3600.
        let end = animate_style_at(base, &xforms, 5000, 5000);
        assert!((end.angle_z - 3600.0).abs() < 1e-6);
    }

    #[test]
    fn animate_default_window_spans_line() {
        // \t(\frz360) with no times animates over the whole line duration.
        let r = resolved("{\\t(\\frz360)}x");
        let toks = tokenize("{\\t(\\frz360)}x");
        let xforms = collect_transforms(&toks);
        let base = &r.spans[0].style;
        let mid = animate_style_at(base, &xforms, 1000, 2000);
        assert!((mid.angle_z - 180.0).abs() < 1e-6);
    }

    #[test]
    fn animate_color_interpolates() {
        // {\1c&HFF0000&\t(\1c&H0000FF&)}: blue -> red over the line.
        // &HFF0000& is BGR = (r=0,g=0,b=255) blue; &H0000FF& = red.
        let text = "{\\1c&HFF0000&\\t(\\1c&H0000FF&)}Hi";
        let r = resolved(text);
        let toks = tokenize(text);
        let xforms = collect_transforms(&toks);
        let base = &r.spans[0].style;
        assert_eq!(
            base.primary,
            Rgba {
                r: 0,
                g: 0,
                b: 255,
                a: 255
            }
        );
        // Midpoint blends halfway between blue and red.
        let mid = animate_style_at(base, &xforms, 2000, 4000);
        assert_eq!(mid.primary.r, 128);
        assert_eq!(mid.primary.b, 128);
        // End is fully red.
        let end = animate_style_at(base, &xforms, 4000, 4000);
        assert_eq!(
            end.primary,
            Rgba {
                r: 255,
                g: 0,
                b: 0,
                a: 255
            }
        );
    }

    #[test]
    fn animate_accel_eases_scale() {
        // {\fscx0\fscy0\t(0,500,\fscx100\fscy100)}Boo! — grow from 0 to 100%.
        let text = "{\\fscx0\\fscy0\\t(0,500,\\fscx100\\fscy100)}Boo!";
        let r = resolved(text);
        let toks = tokenize(text);
        let xforms = collect_transforms(&toks);
        let base = &r.spans[0].style;
        assert_eq!(base.scale_x, 0.0);
        let mid = animate_style_at(base, &xforms, 250, 5000);
        assert!((mid.scale_x - 50.0).abs() < 1e-6);
        assert!((mid.scale_y - 50.0).abs() < 1e-6);
        let end = animate_style_at(base, &xforms, 500, 5000);
        assert!((end.scale_x - 100.0).abs() < 1e-6);
    }

    #[test]
    fn animate_stacked_transforms_compose() {
        // Two \t blocks: one animates scale_x, the other angle_z.
        let text = "{\\t(0,1000,\\fscx200)\\t(0,1000,\\frz90)}x";
        let r = resolved(text);
        let toks = tokenize(text);
        let xforms = collect_transforms(&toks);
        assert_eq!(xforms.len(), 2);
        let base = &r.spans[0].style;
        let end = animate_style_at(base, &xforms, 1000, 1000);
        assert!((end.scale_x - 200.0).abs() < 1e-6);
        assert!((end.angle_z - 90.0).abs() < 1e-6);
    }

    #[test]
    fn karaoke_no_tag_is_empty() {
        let toks = tokenize("plain line");
        assert!(karaoke_fills(&toks, 100).is_empty());
    }

    #[test]
    fn karaoke_instant_steps_at_boundary() {
        // {\k50}Ka{\k50}ra: two 50cs (500ms) instant syllables.
        let toks = tokenize("{\\k50}Ka{\\k50}ra");
        // At t=0: neither has started (start at 0 -> first is filled).
        let f0 = karaoke_fills(&toks, 0);
        assert_eq!(f0.len(), 2);
        assert_eq!(f0[0].text, "Ka");
        assert_eq!(f0[0].start_ms, 0);
        assert_eq!(f0[0].dur_ms, 500);
        assert_eq!(f0[0].fill, 1.0); // started at t>=0
        assert_eq!(f0[1].start_ms, 500);
        assert_eq!(f0[1].fill, 0.0);
        // At t=600ms: both syllables have started.
        let f600 = karaoke_fills(&toks, 600);
        assert_eq!(f600[0].fill, 1.0);
        assert_eq!(f600[1].fill, 1.0);
    }

    #[test]
    fn karaoke_sweep_ramps_across_window() {
        // {\kf100}Sweep: one 100cs (1000ms) sweep syllable.
        let toks = tokenize("{\\kf100}Sweep");
        assert_eq!(karaoke_fills(&toks, 0)[0].fill, 0.0);
        assert_eq!(karaoke_fills(&toks, 500)[0].fill, 0.5);
        assert_eq!(karaoke_fills(&toks, 1000)[0].fill, 1.0);
        assert_eq!(karaoke_fills(&toks, 2000)[0].fill, 1.0);
    }

    #[test]
    fn karaoke_sweep_cap_matches_kf() {
        // \K is identical to \kf per spec.
        let toks = tokenize("{\\K100}Sweep");
        assert_eq!(karaoke_fills(&toks, 500)[0].kind, AssKaraokeKind::SweepCap);
        assert_eq!(karaoke_fills(&toks, 500)[0].fill, 0.5);
    }

    #[test]
    fn karaoke_outline_is_instant() {
        let toks = tokenize("{\\ko30}Edge");
        let f = karaoke_fills(&toks, 100);
        assert_eq!(f[0].kind, AssKaraokeKind::Outline);
        assert_eq!(f[0].fill, 1.0); // instant, started
        assert_eq!(f[0].dur_ms, 300);
    }

    #[test]
    fn karaoke_text_before_first_beat_not_a_syllable() {
        // "Intro " precedes the first \k and is not karaoke-marked.
        let toks = tokenize("Intro {\\k50}beat");
        let f = karaoke_fills(&toks, 0);
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].text, "beat");
    }

    #[test]
    fn karaoke_timeline_accumulates() {
        // Three beats of 20/30/40 cs: starts at 0/200/500 ms.
        let toks = tokenize("{\\k20}a{\\k30}b{\\k40}c");
        let f = karaoke_fills(&toks, 0);
        assert_eq!(f[0].start_ms, 0);
        assert_eq!(f[1].start_ms, 200);
        assert_eq!(f[2].start_ms, 500);
    }

    #[test]
    fn evaluate_combines_position_fade_and_style() {
        // Position via \move, fade via \fad, animated scale via \t.
        let text = "{\\move(0,0,100,100)\\fad(500,0)\\t(0,4000,\\fscx200)}Go";
        let line = resolved(text);
        let toks = tokenize(text);
        let ev = evaluate_line_at(&line, &toks, 2000, 4000);
        // \move halfway.
        assert_eq!(ev.position, Some((50.0, 50.0)));
        // \fad(500,0): fully visible by t=500, so alpha 0 at t=2000.
        assert_eq!(ev.fade_alpha, 0);
        // \t scale halfway from 100 to 200.
        assert_eq!(ev.spans.len(), 1);
        assert!((ev.spans[0].style.scale_x - 150.0).abs() < 1e-6);
        assert_eq!(ev.spans[0].text, "Go");
        assert_eq!(ev.spans[0].karaoke_fill, 1.0);
    }

    #[test]
    fn evaluate_transform_scope_is_per_span() {
        // A mid-line \t only animates the text after it. The first span
        // ("A") has no \t in scope; the second ("B") does.
        let text = "A{\\t(0,1000,\\frz90)}B";
        let line = resolved(text);
        let toks = tokenize(text);
        let ev = evaluate_line_at(&line, &toks, 1000, 1000);
        assert_eq!(ev.spans.len(), 2);
        assert_eq!(ev.spans[0].text, "A");
        assert_eq!(ev.spans[0].style.angle_z, 0.0, "A not animated");
        assert_eq!(ev.spans[1].text, "B");
        assert!(
            (ev.spans[1].style.angle_z - 90.0).abs() < 1e-6,
            "B animated"
        );
    }

    #[test]
    fn evaluate_karaoke_fill_per_span() {
        // Two karaoke syllables; at t=600ms the first is fully filled and
        // the second is mid-sweep.
        let text = "{\\kf50}Ka{\\kf50}ra";
        let line = resolved(text);
        let toks = tokenize(text);
        let ev = evaluate_line_at(&line, &toks, 600, 2000);
        assert_eq!(ev.spans.len(), 2);
        assert_eq!(ev.spans[0].karaoke_fill, 1.0); // [0,500] done
                                                   // second sweeps [500,1000]: at 600 -> 0.2.
        assert!((ev.spans[1].karaoke_fill - 0.2).abs() < 1e-9);
    }

    #[test]
    fn evaluate_plain_line_is_static() {
        let line = resolved("just text");
        let toks = tokenize("just text");
        let ev = evaluate_line_at(&line, &toks, 1234, 5000);
        assert_eq!(ev.position, None);
        assert_eq!(ev.fade_alpha, 0);
        assert_eq!(ev.spans.len(), 1);
        assert_eq!(&ev.spans[0].style, &line.spans[0].style);
        assert_eq!(ev.spans[0].karaoke_fill, 1.0);
    }
}
