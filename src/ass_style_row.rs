//! Parse an ASS / SSA `[V4+ Styles]` / `[V4 Styles]` `Style:` row into a
//! [`StyleBase`] for override-tag resolution.
//!
//! The SSA v4 script-format specification (mirrored at
//! `docs/subtitles/ass/ass-specs-tcax.html`) defines a *Format* line that
//! "must appear before any Styles - because it defines how SSA will
//! interpret the Style definition lines". Crucially: "The format line
//! allows new fields to be added to the script format in future, and yet
//! allow old versions of the software to read the fields it recognises -
//! even if the field order is changed." So the robust parse maps each
//! comma-separated `Style:` value onto a field *by the Format header
//! name*, not by fixed position — which makes the same parser handle the
//! SSA v4 layout (… `TertiaryColour, BackColour, … AlphaLevel, Encoding`)
//! and the ASS V4+ layout (… `OutlineColour, BackColour, … Outline,
//! Shadow, Alignment, … Encoding`) uniformly.
//!
//! When no Format line is available, [`DEFAULT_V4PLUS_FORMAT`] supplies
//! the canonical ASS V4+ field order as a fallback.
//!
//! The spec's field semantics this parser honours:
//!
//! * Colour fields are "A long integer BGR (blue-green-red) value. ie.
//!   the byte order in the hexadecimal equivalent of this number is
//!   BBGGRR". The ASS V4+ wire form prefixes `&H` and may carry an 8-bit
//!   alpha as the high byte (`&HaaBBGGRR`), where `00` is opaque and `FF`
//!   transparent — inverted here to straight alpha (`255` opaque).
//! * `Bold` / `Italic` / … are `-1` (true) / `0` (false) per the spec's
//!   "-1 is True, 0 is False"; `Bold` may also be an explicit weight.
//! * `Alignment` is the legacy SSA 1..=11 grid for a `[V4 Styles]` row
//!   ("1=Left, 2=Centered, 3=Right. Add 4 … 'Toptitle'. Add 8 …
//!   'Midtitle'") but the cleaner numpad 1..=9 for a `[V4+ Styles]` row;
//!   the two are distinguished by the section the Format line came from.

use crate::ass_resolve::{Rgba, StyleBase};
use crate::ass_tags::legacy_align_to_numpad;

/// The canonical ASS V4+ Style-field order, used when a `Style:` row has
/// no preceding `Format:` line to map against.
pub const DEFAULT_V4PLUS_FORMAT: &[&str] = &[
    "Name",
    "Fontname",
    "Fontsize",
    "PrimaryColour",
    "SecondaryColour",
    "OutlineColour",
    "BackColour",
    "Bold",
    "Italic",
    "Underline",
    "StrikeOut",
    "ScaleX",
    "ScaleY",
    "Spacing",
    "Angle",
    "BorderStyle",
    "Outline",
    "Shadow",
    "Alignment",
    "MarginL",
    "MarginR",
    "MarginV",
    "Encoding",
];

/// Parse a `Format:` header line into its ordered field names.
///
/// Accepts the line with or without the leading `Format:` token; field
/// names are trimmed of surrounding whitespace. Returns `None` if the
/// line carries no fields.
pub fn parse_format(line: &str) -> Option<Vec<String>> {
    let body = strip_keyword(line, "Format")?;
    let fields: Vec<String> = body.split(',').map(|f| f.trim().to_string()).collect();
    if fields.iter().all(|f| f.is_empty()) {
        return None;
    }
    Some(fields)
}

/// Parse a `Style:` row against an ordered field list (a [`parse_format`]
/// result or [`DEFAULT_V4PLUS_FORMAT`]) into a `(name, StyleBase)` pair.
///
/// `legacy_alignment` selects the alignment interpretation: `true` for a
/// `[V4 Styles]` row (SSA legacy 1..=11), `false` for `[V4+ Styles]`
/// (numpad 1..=9).
///
/// The `Style:` keyword may be present or omitted. Because the `Name`
/// field "Cannot include commas" and the layout is positional after the
/// header, the value list is split on commas up to the field count — the
/// final field absorbs any trailing commas so an over-long row doesn't
/// drop data. Unknown header names are skipped; missing values leave the
/// [`StyleBase::default`] value in place.
pub fn parse_style_row<S: AsRef<str>>(
    line: &str,
    format: &[S],
    legacy_alignment: bool,
) -> Option<(String, StyleBase)> {
    let body = strip_keyword(line, "Style")?;
    let n = format.len();
    if n == 0 {
        return None;
    }
    // Split into at most `n` fields; the last keeps any embedded commas.
    let values: Vec<&str> = body.splitn(n, ',').collect();

    let mut base = StyleBase::default();
    let mut name = String::new();

    for (i, field) in format.iter().enumerate() {
        let raw = match values.get(i) {
            Some(v) => v.trim(),
            None => continue,
        };
        match field.as_ref() {
            "Name" => name = raw.to_string(),
            "Fontname" => base.font_name = raw.to_string(),
            "Fontsize" => {
                if let Some(v) = parse_f64(raw) {
                    base.font_size = v;
                }
            }
            "PrimaryColour" | "PrimaryColor" => {
                if let Some(c) = parse_color(raw) {
                    base.primary = c;
                }
            }
            "SecondaryColour" | "SecondaryColor" => {
                if let Some(c) = parse_color(raw) {
                    base.secondary = c;
                }
            }
            // SSA v4 calls the outline colour TertiaryColour.
            "OutlineColour" | "OutlineColor" | "TertiaryColour" | "TertiaryColor" => {
                if let Some(c) = parse_color(raw) {
                    base.outline_color = c;
                }
            }
            "BackColour" | "BackColor" => {
                if let Some(c) = parse_color(raw) {
                    base.shadow_color = c;
                }
            }
            "Bold" => parse_bold(raw, &mut base),
            "Italic" => base.italic = parse_bool(raw),
            "Underline" => base.underline = parse_bool(raw),
            "StrikeOut" | "Strikeout" => base.strike = parse_bool(raw),
            "ScaleX" => {
                if let Some(v) = parse_f64(raw) {
                    base.scale_x = v;
                }
            }
            "ScaleY" => {
                if let Some(v) = parse_f64(raw) {
                    base.scale_y = v;
                }
            }
            "Spacing" => {
                if let Some(v) = parse_f64(raw) {
                    base.spacing = v;
                }
            }
            "Angle" => {
                if let Some(v) = parse_f64(raw) {
                    base.angle_z = v;
                }
            }
            "Outline" => {
                if let Some(v) = parse_f64(raw) {
                    base.border = v;
                }
            }
            "Shadow" => {
                if let Some(v) = parse_f64(raw) {
                    base.shadow = v;
                }
            }
            "Alignment" => {
                if let Some(a) = parse_alignment(raw, legacy_alignment) {
                    base.alignment = a;
                }
            }
            "Encoding" => {
                if let Ok(v) = raw.trim().parse::<i32>() {
                    base.encoding = v;
                }
            }
            // BorderStyle, MarginL/R/V, AlphaLevel, PlayResX… carry no
            // StyleBase field — silently skipped.
            _ => {}
        }
    }

    Some((name, base))
}

/// Decode an ASS / SSA colour value into [`Rgba`].
///
/// Accepts the ASS `&H[aa]bbggrr&` wire form (the trailing `&` and the
/// `&H` prefix are both optional and case-insensitive) and the bare SSA
/// "long integer" decimal form. The hexadecimal byte order is BGR with an
/// optional high alpha byte; alpha is inverted from the ASS sense
/// (`00` opaque) to straight alpha (`255` opaque). Returns `None` on a
/// malformed value.
pub fn parse_color(s: &str) -> Option<Rgba> {
    let t = s.trim();
    let v = if let Some(hex) = strip_ass_hex(t) {
        if hex.is_empty() || hex.len() > 8 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        u32::from_str_radix(hex, 16).ok()?
    } else {
        // Bare decimal long-integer form (SSA classic).
        t.parse::<i64>().ok().and_then(|n| u32::try_from(n).ok())?
    };
    let r = (v & 0xFF) as u8;
    let g = ((v >> 8) & 0xFF) as u8;
    let b = ((v >> 16) & 0xFF) as u8;
    let aa = ((v >> 24) & 0xFF) as u8;
    Some(Rgba {
        r,
        g,
        b,
        a: 255 - aa,
    })
}

/// Strip a leading `&H` / `&h` prefix and an optional trailing `&` from
/// an ASS colour token, returning the inner hex digits. Returns `None`
/// when the token is not in `&H…` form.
fn strip_ass_hex(t: &str) -> Option<&str> {
    let rest = t
        .strip_prefix("&H")
        .or_else(|| t.strip_prefix("&h"))
        .or_else(|| t.strip_prefix("&"))?;
    Some(rest.strip_suffix('&').unwrap_or(rest))
}

/// Strip a leading `Keyword:` token (case-insensitive) if present,
/// returning the remaining body trimmed of leading whitespace. When the
/// keyword is absent the whole line is returned (so a bare value list
/// still parses). Returns `None` only for an empty line.
fn strip_keyword<'a>(line: &'a str, keyword: &str) -> Option<&'a str> {
    let t = line.trim_start();
    if t.is_empty() {
        return None;
    }
    if let Some(rest) = t.strip_prefix(keyword).or_else(|| {
        // Case-insensitive prefix check.
        if t.len() >= keyword.len() && t[..keyword.len()].eq_ignore_ascii_case(keyword) {
            Some(&t[keyword.len()..])
        } else {
            None
        }
    }) {
        if let Some(after) = rest.trim_start().strip_prefix(':') {
            return Some(after.trim_start());
        }
    }
    Some(t)
}

fn parse_f64(s: &str) -> Option<f64> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    t.parse().ok()
}

/// SSA booleans are `-1` (True) / `0` (False); tolerate `1` as True too.
fn parse_bool(s: &str) -> bool {
    matches!(s.trim(), "-1" | "1")
}

/// `Bold` is a boolean in the common case but the field can also carry an
/// explicit font weight (a value above 1).
fn parse_bold(s: &str, base: &mut StyleBase) {
    match s.trim().parse::<i64>() {
        Ok(-1) | Ok(1) => {
            base.bold = true;
            base.weight = None;
        }
        Ok(0) => {
            base.bold = false;
            base.weight = None;
        }
        Ok(w) if w > 1 => {
            base.bold = w >= 700;
            base.weight = Some(w as u32);
        }
        _ => {}
    }
}

/// Resolve an alignment value to numpad form. Legacy `[V4 Styles]` rows
/// use the SSA 1..=11 grid (mapped via [`legacy_align_to_numpad`]); V4+
/// rows already use numpad 1..=9.
fn parse_alignment(s: &str, legacy: bool) -> Option<u8> {
    let n: u8 = s.trim().parse().ok()?;
    if legacy {
        legacy_align_to_numpad(n)
    } else if (1..=9).contains(&n) {
        Some(n)
    } else {
        None
    }
}
