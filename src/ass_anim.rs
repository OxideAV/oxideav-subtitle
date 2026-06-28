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
//!   factor is "y = x with x ∈ [0;1] = (t - t1)/(t2 - t1)" raised to the
//!   accel exponent (accel 1 linear, 0..1 fast-then-slow, >1 slow-then-fast).

use crate::ass_resolve::{LineLayout, Move};
use crate::ass_tags::AssFadeSpec;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ass_resolve::{resolve_line, StyleBase};

    fn layout(text: &str) -> LineLayout {
        resolve_line(text, &StyleBase::default()).layout
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
}
