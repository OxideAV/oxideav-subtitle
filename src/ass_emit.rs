//! Serialize resolved ASS / SSA state back to the wire form.
//!
//! The inverse of [`crate::ass_resolve`] and [`crate::ass_style_row`]:
//!
//! * [`color_to_string`] emits the ASS `&HaaBBGGRR&` colour wire form
//!   (the inverse of [`crate::ass_style_row::parse_color`]).
//! * [`style_row_to_string`] emits a `Style:` row in a caller-chosen
//!   field order — round-trips a [`StyleBase`] through
//!   [`crate::ass_style_row::parse_style_row`].
//! * [`serialize_line`] emits a Dialogue `Text` field from a
//!   [`ResolvedLine`] as a *minimal* override stream: it diffs each
//!   span's [`ResolvedStyle`] against the running state and emits only
//!   the tags that changed, prefixed by the whole-line layout tags. The
//!   round-trip target is *semantic*: re-resolving the emitted text
//!   against the same [`StyleBase`] reproduces the same resolved spans
//!   and layout (not necessarily the original byte-for-byte authoring,
//!   which [`crate::ass_tags::emit`] preserves at the token layer).

use crate::ass_resolve::{ClipRegion, LineLayout, ResolvedLine, ResolvedStyle, Rgba, StyleBase};

/// Emit an [`Rgba`] as the ASS `&HaaBBGGRR&` colour wire form.
///
/// The alpha byte is inverted back to the ASS sense (`00` opaque). When
/// `with_alpha` is `false` the alpha byte is omitted (the `&HBBGGRR&`
/// six-digit form used by the `\c` / `\1c` colour overrides, which carry
/// no alpha).
pub fn color_to_string(c: Rgba, with_alpha: bool) -> String {
    let ass_alpha = 255 - c.a;
    if with_alpha {
        format!("&H{:02X}{:02X}{:02X}{:02X}&", ass_alpha, c.b, c.g, c.r)
    } else {
        format!("&H{:02X}{:02X}{:02X}&", c.b, c.g, c.r)
    }
}

/// Emit the bare `BBGGRR` hex digit run (no `&H` framing) that a `\c`
/// colour override carries between `&H` and the closing `&`.
fn color_hex_digits(c: Rgba) -> String {
    format!("{:02X}{:02X}{:02X}", c.b, c.g, c.r)
}

/// Emit the bare `aa` alpha hex digits a `\1a`-family override carries.
fn alpha_hex_digits(c: Rgba) -> String {
    format!("{:02X}", 255 - c.a)
}

/// Serialize a `(name, StyleBase)` pair into a `Style:` row for the given
/// field order.
///
/// Each field name is emitted in `format` order so the result parses back
/// through [`crate::ass_style_row::parse_style_row`] to an equal
/// [`StyleBase`]. Fields the [`StyleBase`] cannot supply (`BorderStyle`,
/// `MarginL/R/V`, `AlphaLevel`, …) are emitted with a neutral default
/// (`0` numeric, empty string otherwise) so the column count stays
/// correct. `legacy_alignment` is currently informational — alignment is
/// emitted as numpad in both layouts (the parser accepts numpad for V4+
/// and remaps legacy on the way in); pass the same flag used at parse
/// time for symmetry.
pub fn style_row_to_string<S: AsRef<str>>(
    name: &str,
    base: &StyleBase,
    format: &[S],
    _legacy_alignment: bool,
) -> String {
    let mut out = String::from("Style: ");
    for (i, field) in format.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let cell: String = match field.as_ref() {
            "Name" => name.to_string(),
            "Fontname" => base.font_name.clone(),
            "Fontsize" => fmt_num(base.font_size),
            "PrimaryColour" | "PrimaryColor" => color_to_string(base.primary, true),
            "SecondaryColour" | "SecondaryColor" => color_to_string(base.secondary, true),
            "OutlineColour" | "OutlineColor" | "TertiaryColour" | "TertiaryColor" => {
                color_to_string(base.outline_color, true)
            }
            "BackColour" | "BackColor" => color_to_string(base.shadow_color, true),
            "Bold" => fmt_flag(base.bold, base.weight),
            "Italic" => fmt_bool(base.italic),
            "Underline" => fmt_bool(base.underline),
            "StrikeOut" | "Strikeout" => fmt_bool(base.strike),
            "ScaleX" => fmt_num(base.scale_x),
            "ScaleY" => fmt_num(base.scale_y),
            "Spacing" => fmt_num(base.spacing),
            "Angle" => fmt_num(base.angle_z),
            "Outline" => fmt_num(base.border),
            "Shadow" => fmt_num(base.shadow),
            "Alignment" => base.alignment.to_string(),
            "Encoding" => base.encoding.to_string(),
            // Numeric placeholder fields the StyleBase doesn't model.
            "BorderStyle" => "1".to_string(),
            "MarginL" | "MarginR" | "MarginV" | "AlphaLevel" => "0".to_string(),
            _ => String::new(),
        };
        out.push_str(&cell);
    }
    out
}

/// Serialize a [`ResolvedLine`] into a Dialogue `Text` field as a minimal
/// override stream, relative to `base`.
///
/// The whole-line layout tags are emitted first (in a leading override
/// block), then each span's visible text is prefixed by an override block
/// carrying only the styling that *differs* from the running state
/// (which starts at `base`). A re-resolve of the result against the same
/// `base` reproduces the input's spans and layout.
pub fn serialize_line(line: &ResolvedLine, base: &StyleBase) -> String {
    let mut out = String::new();

    // Leading layout block.
    let layout_tags = emit_layout(&line.layout);
    if !layout_tags.is_empty() {
        out.push('{');
        out.push_str(&layout_tags);
        out.push('}');
    }

    let mut cur = ResolvedStyle::from_base(base);
    for span in &line.spans {
        let mut block = String::new();
        if let Some(cs) = span.karaoke_cs {
            block.push_str(&format!("\\k{cs}"));
        }
        block.push_str(&diff_tags(&cur, &span.style));
        if !block.is_empty() {
            out.push('{');
            out.push_str(&block);
            out.push('}');
        }
        out.push_str(&escape_text(&span.text));
        cur = span.style.clone();
    }
    out
}

/// Emit the whole-line layout override tags (no surrounding braces).
fn emit_layout(l: &LineLayout) -> String {
    let mut s = String::new();
    if let Some(a) = l.alignment {
        s.push_str(&format!("\\an{a}"));
    }
    if let Some((x, y)) = l.pos {
        s.push_str(&format!("\\pos({x},{y})"));
    }
    if let Some(m) = l.mv {
        match m.times {
            Some((t1, t2)) => s.push_str(&format!(
                "\\move({},{},{},{},{},{})",
                m.x1, m.y1, m.x2, m.y2, t1, t2
            )),
            None => s.push_str(&format!("\\move({},{},{},{})", m.x1, m.y1, m.x2, m.y2)),
        }
    }
    if let Some((x, y)) = l.org {
        s.push_str(&format!("\\org({x},{y})"));
    }
    if let Some(spec) = l.fade {
        use crate::ass_tags::AssFadeSpec::*;
        match spec {
            Simple { fadein, fadeout } => s.push_str(&format!("\\fad({fadein},{fadeout})")),
            Complex {
                a1,
                a2,
                a3,
                t1,
                t2,
                t3,
                t4,
            } => s.push_str(&format!("\\fade({a1},{a2},{a3},{t1},{t2},{t3},{t4})")),
        }
    }
    if let Some(clip) = &l.clip {
        match clip {
            ClipRegion::Rectangle {
                inverse,
                x1,
                y1,
                x2,
                y2,
            } => s.push_str(&format!(
                "\\{}clip({x1},{y1},{x2},{y2})",
                if *inverse { "i" } else { "" }
            )),
            ClipRegion::Drawing {
                inverse,
                scale,
                commands,
            } => {
                let pre = if *inverse { "i" } else { "" };
                match scale {
                    Some(sc) => s.push_str(&format!("\\{pre}clip({sc},{commands})")),
                    None => s.push_str(&format!("\\{pre}clip({commands})")),
                }
            }
        }
    }
    s
}

/// Emit the override tags needed to move the running state `from` to the
/// target state `to` (only the differing fields).
fn diff_tags(from: &ResolvedStyle, to: &ResolvedStyle) -> String {
    let mut s = String::new();

    if from.bold != to.bold || from.weight != to.weight {
        match to.weight {
            Some(w) => s.push_str(&format!("\\b{w}")),
            None => s.push_str(if to.bold { "\\b1" } else { "\\b0" }),
        }
    }
    if from.italic != to.italic {
        s.push_str(if to.italic { "\\i1" } else { "\\i0" });
    }
    if from.underline != to.underline {
        s.push_str(if to.underline { "\\u1" } else { "\\u0" });
    }
    if from.strike != to.strike {
        s.push_str(if to.strike { "\\s1" } else { "\\s0" });
    }
    // Colours: RGB on \Nc, alpha on \Na, emitted only when changed.
    emit_color_diff(&mut s, "1", from.primary, to.primary);
    emit_color_diff(&mut s, "2", from.secondary, to.secondary);
    emit_color_diff(&mut s, "3", from.outline_color, to.outline_color);
    emit_color_diff(&mut s, "4", from.shadow_color, to.shadow_color);

    if from.font_name != to.font_name {
        s.push_str(&format!("\\fn{}", to.font_name));
    }
    if from.font_size != to.font_size {
        s.push_str(&format!("\\fs{}", fmt_num(to.font_size)));
    }
    if from.scale_x != to.scale_x {
        s.push_str(&format!("\\fscx{}", fmt_num(to.scale_x)));
    }
    if from.scale_y != to.scale_y {
        s.push_str(&format!("\\fscy{}", fmt_num(to.scale_y)));
    }
    if from.spacing != to.spacing {
        s.push_str(&format!("\\fsp{}", fmt_num(to.spacing)));
    }
    if from.encoding != to.encoding {
        s.push_str(&format!("\\fe{}", to.encoding));
    }
    if from.angle_x != to.angle_x {
        s.push_str(&format!("\\frx{}", fmt_num(to.angle_x)));
    }
    if from.angle_y != to.angle_y {
        s.push_str(&format!("\\fry{}", fmt_num(to.angle_y)));
    }
    if from.angle_z != to.angle_z {
        s.push_str(&format!("\\frz{}", fmt_num(to.angle_z)));
    }
    // Border / shadow: combined tag when both axes share a value, else
    // per-axis.
    if from.border_x != to.border_x || from.border_y != to.border_y {
        if to.border_x == to.border_y {
            s.push_str(&format!("\\bord{}", fmt_num(to.border_x)));
        } else {
            if from.border_x != to.border_x {
                s.push_str(&format!("\\xbord{}", fmt_num(to.border_x)));
            }
            if from.border_y != to.border_y {
                s.push_str(&format!("\\ybord{}", fmt_num(to.border_y)));
            }
        }
    }
    if from.shadow_x != to.shadow_x || from.shadow_y != to.shadow_y {
        if to.shadow_x == to.shadow_y {
            s.push_str(&format!("\\shad{}", fmt_num(to.shadow_x)));
        } else {
            if from.shadow_x != to.shadow_x {
                s.push_str(&format!("\\xshad{}", fmt_num(to.shadow_x)));
            }
            if from.shadow_y != to.shadow_y {
                s.push_str(&format!("\\yshad{}", fmt_num(to.shadow_y)));
            }
        }
    }
    if from.blur_be != to.blur_be {
        s.push_str(&format!("\\be{}", fmt_num(to.blur_be)));
    }
    if from.blur_gauss != to.blur_gauss {
        s.push_str(&format!("\\blur{}", fmt_num(to.blur_gauss)));
    }
    s
}

/// Emit the colour + alpha override for one component (`target` = the
/// `1`/`2`/`3`/`4` index), only for the channels that changed.
fn emit_color_diff(out: &mut String, target: &str, from: Rgba, to: Rgba) {
    if (from.r, from.g, from.b) != (to.r, to.g, to.b) {
        out.push_str(&format!("\\{target}c&H{}&", color_hex_digits(to)));
    }
    if from.a != to.a {
        out.push_str(&format!("\\{target}a&H{}&", alpha_hex_digits(to)));
    }
}

/// Escape Dialogue text so the override-block / line-break tokens survive
/// a round-trip. A literal newline becomes `\N` (hard break); a NBSP
/// becomes `\h`. Braces in visible text would otherwise open a spurious
/// override block, so a literal `{` / `}` is left as-is only when the
/// text carries no brace (the resolved model never holds raw braces, so
/// this is a defensive pass).
fn escape_text(text: &str) -> String {
    let mut s = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\n' => s.push_str("\\N"),
            '\u{00A0}' => s.push_str("\\h"),
            _ => s.push(ch),
        }
    }
    s
}

/// Format an `f64` without a trailing `.0` so an integral value emits as
/// a bare integer (the common authoring form) yet a fractional value
/// keeps its decimals.
fn fmt_num(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

fn fmt_bool(b: bool) -> String {
    if b { "-1" } else { "0" }.to_string()
}

fn fmt_flag(bold: bool, weight: Option<u32>) -> String {
    match weight {
        Some(w) => w.to_string(),
        None => fmt_bool(bold),
    }
}
