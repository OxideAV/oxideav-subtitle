//! Whole-document ASS / SSA parser: read a `.ass` / `.ssa` byte stream
//! into a [`SubtitleTrack`].
//!
//! Ties the four ASS helper layers together — [`crate::ass_script_info`]
//! (the `[Script Info]` key contract), [`crate::ass_style_row`] (the
//! `[V4+ Styles]` / `[V4 Styles]` `Style:` rows), [`crate::ass_event`]
//! (the `[Events]` `Dialogue:` / `Comment:` rows), and
//! [`crate::ass_resolve`] (folding each event's override stream into
//! styled spans).
//!
//! The file is a line-based INI-style document (per the SSA v4 spec
//! mirrored at `docs/subtitles/ass/ass-specs-tcax.html`):
//!
//! * `[Script Info]` — metadata; lines are `Key: Value` and `; …`
//!   comments. Keys are normalised to the lowercase-snake-case IR
//!   convention documented on [`crate::ass_script_info`] so
//!   [`crate::ass_script_info::script_info`] reads them back typed.
//! * `[V4+ Styles]` (ASS) / `[V4 Styles]` (SSA) — a `Format:` header
//!   then `Style:` rows. Each becomes an IR [`SubtitleStyle`].
//! * `[Events]` — a `Format:` header then `Dialogue:` / `Comment:` rows.
//!   Each `Dialogue:` becomes a [`SubtitleCue`]; `Comment:` rows are
//!   skipped (they are not rendered).
//!
//! Cue bodies are resolved into IR [`Segment`]s: each event's `Text` is
//! folded against its style row (via [`crate::ass_resolve`]) into styled
//! spans, then each span's run is wrapped in the IR style segments it
//! carries (bold / italic / underline / strike / primary-colour /
//! karaoke). Visible `\N` line breaks become [`Segment::LineBreak`].

use oxideav_core::{Segment, SubtitleCue, SubtitleStyle, TextAlign};

use crate::ass_event::{parse_event, AssEvent};
use crate::ass_resolve::{resolve_line, ResolvedSpan, StyleBase};
use crate::ass_style_row::{parse_style_row, DEFAULT_V4PLUS_FORMAT};
use crate::ir::{SourceFormat, SubtitleTrack};

/// Centiseconds → microseconds (the IR cue timing unit).
fn cs_to_us(cs: i64) -> i64 {
    cs * 10_000
}

/// Microseconds → centiseconds (rounding to the nearest hundredth).
fn us_to_cs(us: i64) -> i64 {
    (us + 5_000) / 10_000
}

/// Parse a UTF-8 (or BOM-prefixed / UTF-16-BOM) `.ass` / `.ssa` payload
/// into a [`SubtitleTrack`].
///
/// Never fails: malformed style / event rows are skipped, and an empty
/// or section-less file yields an empty track.
pub fn parse(bytes: &[u8]) -> SubtitleTrack {
    let text = crate::encoding::decode_subtitle_text(bytes);
    let mut track = SubtitleTrack::new().with_source(SourceFormat::AssOrSsa);

    // Resolved style bases, keyed by style name, for event resolution.
    let mut style_bases: Vec<(String, StyleBase)> = Vec::new();

    let mut section = Section::None;
    // The active Format header for the current Styles / Events section.
    let mut style_format: Vec<String> = Vec::new();
    let mut event_format: Vec<String> = Vec::new();

    for raw_line in text.split('\n') {
        let line = raw_line.trim_end_matches('\r');
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Section header.
        if let Some(name) = trimmed.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            section = Section::classify(name);
            continue;
        }

        match section {
            Section::ScriptInfo => {
                if trimmed.starts_with(';') {
                    continue; // comment line
                }
                if let Some((k, v)) = split_kv(trimmed) {
                    track
                        .metadata
                        .push((normalise_key(k), v.trim().to_string()));
                }
            }
            Section::Styles { legacy } => {
                if let Some(rest) = keyword(trimmed, "Format") {
                    style_format = rest.split(',').map(|f| f.trim().to_string()).collect();
                } else if keyword(trimmed, "Style").is_some() {
                    let fmt: &[String] = if style_format.is_empty() {
                        // No Format header yet — fall back to the canonical
                        // V4+ order.
                        &fallback_v4plus()
                    } else {
                        &style_format
                    };
                    if let Some((name, base)) = parse_style_row(trimmed, fmt, legacy) {
                        track.styles.push(style_base_to_ir(&name, &base));
                        style_bases.push((name, base));
                    }
                }
            }
            Section::Events => {
                if let Some(rest) = keyword(trimmed, "Format") {
                    event_format = rest.split(',').map(|f| f.trim().to_string()).collect();
                } else if keyword(trimmed, "Dialogue").is_some()
                    || keyword(trimmed, "Comment").is_some()
                {
                    let fmt: Vec<String> = if event_format.is_empty() {
                        DEFAULT_V4PLUS_EVENT.iter().map(|s| s.to_string()).collect()
                    } else {
                        event_format.clone()
                    };
                    if let Some(ev) = parse_event(trimmed, &fmt) {
                        if ev.comment {
                            continue; // comments are not rendered cues
                        }
                        let base = base_for_style(&style_bases, &ev.style);
                        let cue = event_to_cue(&ev, &base);
                        track.cues.push(cue);
                    }
                }
            }
            Section::None | Section::Other => {}
        }
    }

    track
}

/// Which file section the parser is currently inside.
#[derive(Clone, Copy)]
enum Section {
    None,
    ScriptInfo,
    Styles { legacy: bool },
    Events,
    Other,
}

impl Section {
    fn classify(name: &str) -> Section {
        let n = name.trim();
        if n.eq_ignore_ascii_case("Script Info") {
            Section::ScriptInfo
        } else if n.eq_ignore_ascii_case("V4+ Styles") || n.eq_ignore_ascii_case("V4 Styles+") {
            Section::Styles { legacy: false }
        } else if n.eq_ignore_ascii_case("V4 Styles") || n.eq_ignore_ascii_case("v4 Styles") {
            Section::Styles { legacy: true }
        } else if n.eq_ignore_ascii_case("Events") {
            Section::Events
        } else {
            Section::Other
        }
    }
}

/// The canonical V4+ event field order for header-less `[Events]`
/// sections.
const DEFAULT_V4PLUS_EVENT: &[&str] = &[
    "Layer", "Start", "End", "Style", "Name", "MarginL", "MarginR", "MarginV", "Effect", "Text",
];

fn fallback_v4plus() -> Vec<String> {
    DEFAULT_V4PLUS_FORMAT
        .iter()
        .map(|s| s.to_string())
        .collect()
}

/// Look up a parsed style base by name, falling back to a default base
/// (the SSA-neutral 18pt Arial) when the event references an unknown
/// style.
fn base_for_style(bases: &[(String, StyleBase)], name: &str) -> StyleBase {
    bases
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, b)| b.clone())
        .unwrap_or_default()
}

/// Build an IR [`SubtitleCue`] from a parsed event + its style base.
fn event_to_cue(ev: &AssEvent, base: &StyleBase) -> SubtitleCue {
    let resolved = resolve_line(&ev.text, base);
    let mut segments = Vec::new();
    for span in &resolved.spans {
        push_span_segments(span, &mut segments);
    }
    SubtitleCue {
        start_us: cs_to_us(ev.start_cs),
        end_us: cs_to_us(ev.end_cs),
        style_ref: if ev.style.is_empty() {
            None
        } else {
            Some(ev.style.clone())
        },
        positioning: None,
        segments,
    }
}

/// Convert one resolved span into IR segments, wrapping the visible run
/// in the style segments it carries.
fn push_span_segments(span: &ResolvedSpan, out: &mut Vec<Segment>) {
    // Split the run on line breaks into Text / LineBreak segments.
    let mut inner: Vec<Segment> = Vec::new();
    let mut first = true;
    for part in span.text.split('\n') {
        if !first {
            inner.push(Segment::LineBreak);
        }
        first = false;
        if !part.is_empty() {
            inner.push(Segment::Text(part.to_string()));
        }
    }
    if inner.is_empty() {
        return;
    }

    let st = &span.style;
    // Wrap with primary colour when it differs from the IR's implicit
    // white default (only when not opaque-white, to avoid noise).
    if (st.primary.r, st.primary.g, st.primary.b) != (255, 255, 255) {
        inner = vec![Segment::Color {
            rgb: (st.primary.r, st.primary.g, st.primary.b),
            children: inner,
        }];
    }
    if st.strike {
        inner = vec![Segment::Strike(inner)];
    }
    if st.underline {
        inner = vec![Segment::Underline(inner)];
    }
    if st.italic {
        inner = vec![Segment::Italic(inner)];
    }
    if st.bold {
        inner = vec![Segment::Bold(inner)];
    }
    if let Some(cs) = span.karaoke_cs {
        inner = vec![Segment::Karaoke {
            cs,
            children: inner,
        }];
    }
    out.extend(inner);
}

/// Project a [`StyleBase`] onto the IR [`SubtitleStyle`] for the track's
/// style table.
fn style_base_to_ir(name: &str, base: &StyleBase) -> SubtitleStyle {
    SubtitleStyle {
        name: name.to_string(),
        font_family: Some(base.font_name.clone()),
        font_size: Some(base.font_size as f32),
        primary_color: Some((
            base.primary.r,
            base.primary.g,
            base.primary.b,
            base.primary.a,
        )),
        outline_color: Some((
            base.outline_color.r,
            base.outline_color.g,
            base.outline_color.b,
            base.outline_color.a,
        )),
        back_color: Some((
            base.shadow_color.r,
            base.shadow_color.g,
            base.shadow_color.b,
            base.shadow_color.a,
        )),
        bold: base.bold,
        italic: base.italic,
        underline: base.underline,
        strike: base.strike,
        align: numpad_to_align(base.alignment),
        margin_l: None,
        margin_r: None,
        margin_v: None,
        outline: Some(base.border as f32),
        shadow: Some(base.shadow as f32),
    }
}

/// Map an ASS numpad alignment (1..=9) onto the IR horizontal
/// [`TextAlign`]. The numpad columns are left (1/4/7), centre (2/5/8),
/// right (3/6/9).
fn numpad_to_align(a: u8) -> TextAlign {
    match a % 3 {
        1 => TextAlign::Left,
        0 => TextAlign::Right,
        _ => TextAlign::Center,
    }
}

/// Normalise a `[Script Info]` key to the lowercase-snake-case IR
/// convention (documented on [`crate::ass_script_info`]).
///
/// Most keys map by the generic rule (lowercased, ASCII spaces →
/// underscores, trimmed), but the camelCase keys without a space
/// separator have an explicit IR spelling the generic rule cannot
/// reproduce — `ScriptType` → `script_type`, `PlayResX` → `play_res_x`,
/// etc. Those go through an exact-match table first.
fn normalise_key(k: &str) -> String {
    let trimmed = k.trim();
    for (raw, ir) in CAMEL_KEY_MAP {
        if trimmed.eq_ignore_ascii_case(raw) {
            return ir.to_string();
        }
    }
    trimmed
        .chars()
        .map(|c| {
            if c == ' ' {
                '_'
            } else {
                c.to_ascii_lowercase()
            }
        })
        .collect()
}

/// `[Script Info]` camelCase keys whose IR spelling inserts underscores
/// the generic space-rule can't, per the key contract on
/// [`crate::ass_script_info`].
const CAMEL_KEY_MAP: &[(&str, &str)] = &[
    ("ScriptType", "script_type"),
    ("PlayResX", "play_res_x"),
    ("PlayResY", "play_res_y"),
    ("PlayDepth", "play_depth"),
    ("WrapStyle", "wrap_style"),
    ("ScaledBorderAndShadow", "scaled_border_and_shadow"),
];

/// Split a `Key: Value` line at the first colon.
fn split_kv(line: &str) -> Option<(&str, &str)> {
    let idx = line.find(':')?;
    Some((&line[..idx], &line[idx + 1..]))
}

/// Return the body after a leading `Keyword:` token (case-insensitive),
/// or `None` if the keyword is absent.
fn keyword<'a>(line: &'a str, kw: &str) -> Option<&'a str> {
    if line.len() >= kw.len() && line[..kw.len()].eq_ignore_ascii_case(kw) {
        let rest = line[kw.len()..].trim_start();
        return rest.strip_prefix(':').map(|r| r.trim_start());
    }
    None
}

// ---------------------------------------------------------------------
// Serialization (the inverse of `parse`).
// ---------------------------------------------------------------------

use crate::ass_emit::style_row_to_string;
use crate::ass_event::event_to_string;
use crate::ass_resolve::Rgba;

/// Serialize a [`SubtitleTrack`] back into a `.ass` (V4+) byte stream.
///
/// Emits the three canonical sections in order — `[Script Info]`,
/// `[V4+ Styles]`, `[Events]` — each with the canonical `Format:` header.
/// `parse(write(track))` reproduces the track's metadata, styles, and
/// cues (a semantic round-trip; the cue `Text` is rebuilt from the IR
/// segments as a minimal override stream).
pub fn write(track: &SubtitleTrack) -> Vec<u8> {
    let mut out = String::new();

    // [Script Info]
    out.push_str("[Script Info]\n");
    for (k, v) in &track.metadata {
        out.push_str(&denormalise_key(k));
        out.push_str(": ");
        out.push_str(v);
        out.push('\n');
    }
    out.push('\n');

    // [V4+ Styles]
    out.push_str("[V4+ Styles]\n");
    out.push_str("Format: ");
    out.push_str(&DEFAULT_V4PLUS_FORMAT.join(", "));
    out.push('\n');
    for style in &track.styles {
        let base = ir_to_style_base(style);
        out.push_str(&style_row_to_string(
            &style.name,
            &base,
            DEFAULT_V4PLUS_FORMAT,
            false,
        ));
        out.push('\n');
    }
    out.push('\n');

    // [Events]
    out.push_str("[Events]\n");
    out.push_str("Format: ");
    out.push_str(&DEFAULT_V4PLUS_EVENT.join(", "));
    out.push('\n');
    for cue in &track.cues {
        let ev = AssEvent {
            comment: false,
            layer: 0,
            start_cs: us_to_cs(cue.start_us),
            end_cs: us_to_cs(cue.end_us),
            style: cue.style_ref.clone().unwrap_or_default(),
            name: String::new(),
            margin_l: 0,
            margin_r: 0,
            margin_v: 0,
            effect: String::new(),
            text: segments_to_ass_text(&cue.segments),
        };
        out.push_str(&event_to_string(&ev, DEFAULT_V4PLUS_EVENT));
        out.push('\n');
    }

    out.into_bytes()
}

/// Reverse [`normalise_key`] for the documented camelCase keys; other
/// keys are reconstructed by Title-Casing each underscore-separated word
/// (so `original_script` → `Original Script`). This recovers the
/// documented `[Script Info]` spellings exactly and gives a stable,
/// re-parseable spelling for everything else.
fn denormalise_key(k: &str) -> String {
    for (raw, ir) in CAMEL_KEY_MAP {
        if k == *ir {
            return raw.to_string();
        }
    }
    // Title-case each underscore word.
    k.split('_')
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(first) => first.to_ascii_uppercase().to_string() + c.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Project an IR [`SubtitleStyle`] back onto a [`StyleBase`] for
/// [`style_row_to_string`]. Inverse of [`style_base_to_ir`].
fn ir_to_style_base(style: &SubtitleStyle) -> StyleBase {
    let mut b = StyleBase::default();
    if let Some(f) = &style.font_family {
        b.font_name = f.clone();
    }
    if let Some(s) = style.font_size {
        b.font_size = s as f64;
    }
    if let Some(c) = style.primary_color {
        b.primary = Rgba::from_core(c);
    }
    if let Some(c) = style.outline_color {
        b.outline_color = Rgba::from_core(c);
    }
    if let Some(c) = style.back_color {
        b.shadow_color = Rgba::from_core(c);
    }
    b.bold = style.bold;
    b.italic = style.italic;
    b.underline = style.underline;
    b.strike = style.strike;
    if let Some(o) = style.outline {
        b.border = o as f64;
    }
    if let Some(s) = style.shadow {
        b.shadow = s as f64;
    }
    b.alignment = align_to_numpad(style.align);
    b
}

/// Map the IR horizontal [`TextAlign`] back onto an ASS numpad value
/// (bottom row). Inverse direction of [`numpad_to_align`] — the IR only
/// models the horizontal axis, so the bottom band (1/2/3) is used.
fn align_to_numpad(a: TextAlign) -> u8 {
    match a {
        TextAlign::Left | TextAlign::Start => 1,
        TextAlign::Center => 2,
        TextAlign::Right | TextAlign::End => 3,
    }
}

/// Emit an IR segment tree as an ASS Dialogue `Text` field — the inverse
/// of [`push_span_segments`].
///
/// Style nodes wrap their run in an opening override block and a closing
/// reset block (`{\b1}…{\b0}`); colour nodes use `{\c&H..&}…{\c}`;
/// karaoke nodes prepend `{\k<cs>}`. Line breaks become `\N`. Unmodelled
/// `Segment::Raw` is emitted verbatim. The result parses back through
/// [`crate::ass_resolve`] to the same styled spans.
pub fn segments_to_ass_text(segments: &[Segment]) -> String {
    let mut out = String::new();
    emit_segments(segments, &mut out);
    out
}

fn emit_segments(segments: &[Segment], out: &mut String) {
    for seg in segments {
        match seg {
            Segment::Text(s) => out.push_str(s),
            Segment::LineBreak => out.push_str("\\N"),
            Segment::Bold(c) => {
                out.push_str("{\\b1}");
                emit_segments(c, out);
                out.push_str("{\\b0}");
            }
            Segment::Italic(c) => {
                out.push_str("{\\i1}");
                emit_segments(c, out);
                out.push_str("{\\i0}");
            }
            Segment::Underline(c) => {
                out.push_str("{\\u1}");
                emit_segments(c, out);
                out.push_str("{\\u0}");
            }
            Segment::Strike(c) => {
                out.push_str("{\\s1}");
                emit_segments(c, out);
                out.push_str("{\\s0}");
            }
            Segment::Color { rgb, children } => {
                out.push_str(&format!(
                    "{{\\c&H{:02X}{:02X}{:02X}&}}",
                    rgb.2, rgb.1, rgb.0
                ));
                emit_segments(children, out);
                out.push_str("{\\c}");
            }
            Segment::Karaoke { cs, children } => {
                out.push_str(&format!("{{\\k{cs}}}"));
                emit_segments(children, out);
            }
            // The IR Font / Voice / Class / Timestamp variants have no
            // direct ASS spelling the resolver round-trips; emit their
            // inner text without markup so no content is lost.
            Segment::Font { children, .. }
            | Segment::Voice { children, .. }
            | Segment::Class { children, .. } => emit_segments(children, out),
            Segment::Timestamp { .. } => {}
            Segment::Raw(s) => out.push_str(s),
        }
    }
}
