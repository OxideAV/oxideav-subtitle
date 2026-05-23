//! WebVTT parser and writer.
//!
//! WebVTT file structure:
//!
//! ```text
//! WEBVTT [optional trailing text]
//! <blank>
//! STYLE
//! ::cue(.yellow) { color: yellow }
//! <blank>
//! REGION
//! id:foo
//! width:40%
//! <blank>
//! [cue-id]
//! 00:00:01.000 --> 00:00:03.500 line:90% position:50% align:center
//! <v Alice>Hello <c.yellow>world</c></v>
//! <blank>
//! ...
//! ```
//!
//! We parse best-effort: unknown CSS properties are dropped, unknown
//! inline tags fall through to [`Segment::Raw`].

use oxideav_core::{CuePosition, Error, Result, Segment, SubtitleCue, SubtitleStyle, TextAlign};

use crate::ir::{SourceFormat, SubtitleTrack};

/// Parse a UTF-8 WebVTT payload into a track.
pub fn parse(bytes: &[u8]) -> Result<SubtitleTrack> {
    let text = crate::encoding::decode_subtitle_text(bytes);
    let mut lines_iter = text.split('\n').map(|l| l.trim_end_matches('\r'));
    let header = match lines_iter.next() {
        Some(v) => v,
        None => return Err(Error::invalid("WebVTT: empty input")),
    };
    if !header.starts_with("WEBVTT") {
        return Err(Error::invalid("WebVTT: missing WEBVTT signature"));
    }
    let header_trailing = header["WEBVTT".len()..].trim().to_string();

    let mut track = SubtitleTrack {
        source: Some(SourceFormat::WebVtt),
        ..SubtitleTrack::default()
    };
    if !header_trailing.is_empty() {
        track.metadata.push(("header".into(), header_trailing));
    }

    // Group subsequent lines into blocks separated by blank lines.
    let remaining: Vec<&str> = lines_iter.collect();
    let blocks = split_blocks(&remaining);

    let mut extradata = String::new();
    extradata.push_str(header);
    extradata.push('\n');

    for block in &blocks {
        if block.is_empty() {
            continue;
        }
        let first = block[0].trim();
        let first_lc = first.to_ascii_lowercase();
        if first_lc.starts_with("note") {
            // NOTE block — skip.
            continue;
        }
        if first_lc == "style" {
            for style in parse_style_block(&block[1..]) {
                track.styles.push(style);
            }
            // Re-emit into extradata for remuxing.
            extradata.push('\n');
            for line in block {
                extradata.push_str(line);
                extradata.push('\n');
            }
            continue;
        }
        if first_lc == "region" {
            if let Some(region) = parse_region_block(&block[1..]) {
                track.styles.push(region);
            }
            extradata.push('\n');
            for line in block {
                extradata.push_str(line);
                extradata.push('\n');
            }
            continue;
        }
        // Otherwise: a cue. May have an optional id line, then a timing line.
        parse_cue_block(block, &mut track);
    }

    track.extradata = extradata.into_bytes();
    Ok(track)
}

/// Re-emit a track as WebVTT bytes. If the track has extradata from a
/// prior parse we re-use the verbatim header; otherwise we synthesise a
/// minimal `WEBVTT\n` prelude and re-emit `STYLE` blocks from the styles
/// table.
pub fn write(track: &SubtitleTrack) -> Vec<u8> {
    let mut out = String::new();

    if !track.extradata.is_empty() {
        out.push_str(&String::from_utf8_lossy(&track.extradata));
        if !out.ends_with('\n') {
            out.push('\n');
        }
    } else {
        out.push_str("WEBVTT");
        if let Some(h) = track.metadata.iter().find(|(k, _)| k == "header") {
            if !h.1.is_empty() {
                out.push(' ');
                out.push_str(&h.1);
            }
        }
        out.push('\n');

        // Re-emit STYLE blocks.
        for s in &track.styles {
            // Skip any styles whose name looks region-ish (starts with
            // `region:`) — those were produced by a WebVTT REGION block
            // and we don't round-trip them here.
            if s.name.starts_with("region:") {
                continue;
            }
            out.push_str("\nSTYLE\n");
            out.push_str(&format!("::cue(.{}) {{\n", s.name));
            if let Some((r, g, b, _)) = s.primary_color {
                out.push_str(&format!("  color: rgb({}, {}, {});\n", r, g, b));
            }
            if let Some(fam) = &s.font_family {
                out.push_str(&format!("  font-family: {};\n", fam));
            }
            if let Some(sz) = s.font_size {
                out.push_str(&format!("  font-size: {}px;\n", sz));
            }
            if s.bold {
                out.push_str("  font-weight: bold;\n");
            }
            if s.italic {
                out.push_str("  font-style: italic;\n");
            }
            if s.underline || s.strike {
                out.push_str("  text-decoration:");
                if s.underline {
                    out.push_str(" underline");
                }
                if s.strike {
                    out.push_str(" line-through");
                }
                out.push_str(";\n");
            }
            out.push_str("}\n");
        }
    }

    for (idx, cue) in track.cues.iter().enumerate() {
        let extras = track
            .metadata
            .iter()
            .find(|(k, _)| k.strip_prefix("vtt_cue_extra.") == Some(idx.to_string().as_str()))
            .map(|(_, v)| v.as_str());
        out.push('\n');
        out.push_str(&format_timing_line_with_extras(cue, extras));
        out.push('\n');
        out.push_str(&render_segments(&cue.segments));
        out.push('\n');
    }

    out.into_bytes()
}

/// Parsed view of the per-cue `vtt_cue_extra.<idx>` metadata string. Carries
/// the WebVTT §3.5 settings the IR can't model so the writer can re-emit
/// them: vertical direction, the line/position alignment suffixes, and the
/// region reference.
#[derive(Default)]
struct CueExtras {
    vertical: Option<String>,
    /// True when the `line` offset is a percentage (re-attach `%`); false when
    /// it is a line number. Defaults to `false`, but the writer treats a
    /// non-integer / non-negative offset with no recorded flag as a percentage
    /// for back-compat with cues that carry positioning but no extras.
    line_is_pct: bool,
    line_align: Option<String>,
    position_align: Option<String>,
    region: Option<String>,
}

fn parse_cue_extras(s: &str) -> CueExtras {
    let mut e = CueExtras::default();
    for tok in s.split_whitespace() {
        let (k, v) = match tok.split_once(':') {
            Some(kv) => kv,
            None => continue,
        };
        match k {
            "vertical" => e.vertical = Some(v.to_string()),
            "line-pct" => e.line_is_pct = v == "1",
            "line-align" => e.line_align = Some(v.to_string()),
            "position-align" => e.position_align = Some(v.to_string()),
            "region" => e.region = Some(v.to_string()),
            _ => {}
        }
    }
    e
}

fn split_blocks<'a>(lines: &'a [&'a str]) -> Vec<Vec<&'a str>> {
    let mut blocks: Vec<Vec<&'a str>> = Vec::new();
    let mut current: Vec<&'a str> = Vec::new();
    for l in lines {
        if l.trim().is_empty() {
            if !current.is_empty() {
                blocks.push(std::mem::take(&mut current));
            }
        } else {
            current.push(l);
        }
    }
    if !current.is_empty() {
        blocks.push(current);
    }
    blocks
}

fn parse_style_block(lines: &[&str]) -> Vec<SubtitleStyle> {
    // Minimal CSS parser: look for `::cue(.name) { k: v; ... }` rules.
    let joined = lines.join("\n");
    let mut styles: Vec<SubtitleStyle> = Vec::new();
    let mut i = 0;
    let bytes = joined.as_bytes();
    while i < bytes.len() {
        // Skip whitespace.
        while i < bytes.len() && (bytes[i] as char).is_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        // Expect `::cue(.name)`. If not, skip to next `}` to resync.
        let rest = &joined[i..];
        let start_marker = rest.find("::cue");
        if start_marker.is_none() {
            break;
        }
        let cue_idx = i + start_marker.unwrap();
        i = cue_idx + "::cue".len();
        // Optional `(.name)` or `(#id)` or `()`.
        let mut class_name = String::new();
        if i < bytes.len() && bytes[i] == b'(' {
            let end = joined[i..].find(')').map(|p| i + p);
            if let Some(end) = end {
                let inner = joined[i + 1..end].trim();
                if let Some(name) = inner.strip_prefix('.') {
                    class_name = name.to_string();
                }
                i = end + 1;
            }
        }
        // Find `{` and `}`.
        let brace_open = joined[i..].find('{').map(|p| i + p);
        if brace_open.is_none() {
            break;
        }
        let brace_open = brace_open.unwrap();
        let brace_close = joined[brace_open..].find('}').map(|p| brace_open + p);
        if brace_close.is_none() {
            break;
        }
        let brace_close = brace_close.unwrap();
        let body = &joined[brace_open + 1..brace_close];
        let mut style = SubtitleStyle::new(if class_name.is_empty() {
            "default".into()
        } else {
            class_name
        });
        for decl in body.split(';') {
            let decl = decl.trim();
            if decl.is_empty() {
                continue;
            }
            if let Some(colon) = decl.find(':') {
                let key = decl[..colon].trim().to_ascii_lowercase();
                let val = decl[colon + 1..].trim();
                apply_css_prop(&mut style, &key, val);
            }
        }
        styles.push(style);
        i = brace_close + 1;
    }
    styles
}

fn apply_css_prop(style: &mut SubtitleStyle, key: &str, val: &str) {
    match key {
        "color" => {
            if let Some(rgba) = parse_css_color(val) {
                style.primary_color = Some(rgba);
            }
        }
        "background-color" | "background" => {
            if let Some(rgba) = parse_css_color(val) {
                style.back_color = Some(rgba);
            }
        }
        "font-family" => {
            style.font_family = Some(
                val.trim_matches(|c: char| c == '"' || c == '\'')
                    .to_string(),
            );
        }
        "font-size" => {
            let num: String = val
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect();
            if let Ok(v) = num.parse::<f32>() {
                style.font_size = Some(v);
            }
        }
        "font-weight"
            if val.eq_ignore_ascii_case("bold") || val == "700" || val == "800" || val == "900" =>
        {
            style.bold = true;
        }
        "font-style"
            if val.eq_ignore_ascii_case("italic") || val.eq_ignore_ascii_case("oblique") =>
        {
            style.italic = true;
        }
        "text-decoration" => {
            let lc = val.to_ascii_lowercase();
            if lc.contains("underline") {
                style.underline = true;
            }
            if lc.contains("line-through") || lc.contains("strike") {
                style.strike = true;
            }
        }
        _ => {}
    }
}

fn parse_region_block(lines: &[&str]) -> Option<SubtitleStyle> {
    let mut id = String::new();
    let mut width = None;
    for line in lines {
        let line = line.trim();
        if let Some((k, v)) = line.split_once(':') {
            let k = k.trim().to_ascii_lowercase();
            let v = v.trim();
            match k.as_str() {
                "id" => id = v.to_string(),
                "width" => {
                    let num: String = v
                        .chars()
                        .take_while(|c| c.is_ascii_digit() || *c == '.')
                        .collect();
                    width = num.parse::<f32>().ok();
                }
                _ => {}
            }
        }
    }
    if id.is_empty() {
        return None;
    }
    let mut s = SubtitleStyle::new(format!("region:{id}"));
    if let Some(w) = width {
        // Stash the width into margin_r as a rough hint.
        s.margin_r = Some(w as i32);
    }
    Some(s)
}

fn parse_cue_block(block: &[&str], track: &mut SubtitleTrack) {
    let mut iter = block.iter().peekable();
    let first = **iter.peek().unwrap();
    let (timing_line, skip_first) = if first.contains("-->") {
        (first, 1)
    } else {
        // Optional id line; next must be timing.
        if block.len() < 2 {
            return;
        }
        (block[1], 2)
    };

    let parsed = match parse_timing_full(timing_line) {
        Some(v) => v,
        None => return,
    };

    let body_lines: Vec<&str> = block.iter().skip(skip_first).copied().collect();
    let body = body_lines.join("\n");
    let segments = parse_vtt_inline(&body);
    let cue_idx = track.cues.len();
    // `CuePosition` can carry `position`/`line`/`size`/`align`, but the
    // WebVTT §3.5 settings list also admits `vertical:rl|lr`, an optional
    // `,start|,center|,end` alignment suffix on `line`, a
    // `,line-left|,center|,line-right` suffix on `position`, and a
    // `region:<id>` reference — none of which have a home in the IR. We
    // stash those verbatim, keyed by cue index, so the track-level writer
    // can re-emit them faithfully.
    if !parsed.extras.is_empty() {
        track
            .metadata
            .push((format!("vtt_cue_extra.{cue_idx}"), parsed.extras));
    }
    track.cues.push(SubtitleCue {
        start_us: parsed.start_us,
        end_us: parsed.end_us,
        style_ref: None,
        positioning: parsed.position,
        segments,
    });
}

/// Outcome of parsing a `... --> ...` timing line plus its trailing cue
/// settings (WebVTT §3.5).
struct ParsedTiming {
    start_us: i64,
    end_us: i64,
    /// Structured positioning the IR can model: `position`/`line`/`size`/`align`.
    position: Option<CuePosition>,
    /// Cue settings the IR cannot model, captured verbatim in spec order so
    /// the track writer can re-append them. Holds `vertical:rl|lr`, the
    /// `,start|,center|,end` alignment suffix on `line`, the
    /// `,line-left|,center|,line-right` suffix on `position`, and any
    /// `region:<id>` reference. Space-separated, no leading space.
    extras: String,
}

fn parse_timing_and_settings(line: &str) -> Option<(i64, i64, Option<CuePosition>)> {
    let p = parse_timing_full(line)?;
    Some((p.start_us, p.end_us, p.position))
}

fn parse_timing_full(line: &str) -> Option<ParsedTiming> {
    let mid = line.find("-->")?;
    let (l, r) = line.split_at(mid);
    let rest = &r[3..];
    let lhs = l.trim();
    // Split rhs into timestamp + settings.
    let parts: Vec<&str> = rest.split_whitespace().collect();
    if parts.is_empty() {
        return None;
    }
    let rhs_ts = parts[0];
    let start_us = parse_vtt_timestamp(lhs)?;
    let end_us = parse_vtt_timestamp(rhs_ts)?;

    let mut pos: Option<CuePosition> = None;
    // Unmodeled settings, gathered in spec order regardless of the order they
    // appeared in the source so re-emission is canonical: vertical, line
    // alignment suffix, position alignment suffix, region.
    let mut vertical: Option<&str> = None;
    let mut line_suffix: Option<String> = None;
    let mut line_is_pct = false;
    let mut position_suffix: Option<String> = None;
    let mut region: Option<&str> = None;
    for setting in parts.iter().skip(1) {
        let (k, v) = match setting.split_once(':') {
            Some(kv) => kv,
            None => continue,
        };
        let k_lc = k.to_ascii_lowercase();
        match k_lc.as_str() {
            "line" => {
                let cp = pos.get_or_insert_with(CuePosition::default);
                // `line:<offset>[,<align>]` — `<offset>` is a percentage or a
                // (possibly negative) line number; the IR holds the numeric
                // offset in `y`, but loses whether a `%` was present and the
                // alignment suffix, so both go to `extras`.
                let (offset, suffix) = split_setting_suffix(v);
                cp.y = parse_signed_number(offset);
                if offset.contains('%') {
                    line_is_pct = true;
                }
                if let Some(s) = suffix {
                    if matches!(s.to_ascii_lowercase().as_str(), "start" | "center" | "end") {
                        line_suffix = Some(s.to_ascii_lowercase());
                    }
                }
            }
            "position" => {
                let cp = pos.get_or_insert_with(CuePosition::default);
                let (offset, suffix) = split_setting_suffix(v);
                let num: String = offset
                    .chars()
                    .take_while(|c| c.is_ascii_digit() || *c == '.')
                    .collect();
                cp.x = num.parse::<f32>().ok();
                if let Some(s) = suffix {
                    if matches!(
                        s.to_ascii_lowercase().as_str(),
                        "line-left" | "center" | "line-right"
                    ) {
                        position_suffix = Some(s.to_ascii_lowercase());
                    }
                }
            }
            "size" => {
                let cp = pos.get_or_insert_with(CuePosition::default);
                let num: String = v
                    .chars()
                    .take_while(|c| c.is_ascii_digit() || *c == '.')
                    .collect();
                cp.size = num.parse::<f32>().ok();
            }
            "align" => {
                let cp = pos.get_or_insert_with(CuePosition::default);
                cp.align = match v.to_ascii_lowercase().as_str() {
                    "start" => TextAlign::Start,
                    "middle" | "center" => TextAlign::Center,
                    "end" => TextAlign::End,
                    "left" => TextAlign::Left,
                    "right" => TextAlign::Right,
                    _ => TextAlign::Start,
                };
            }
            "vertical" => {
                let v_lc = v.to_ascii_lowercase();
                if v_lc == "rl" {
                    vertical = Some("rl");
                } else if v_lc == "lr" {
                    vertical = Some("lr");
                }
            }
            "region" if !v.is_empty() => {
                region = Some(v);
            }
            _ => {}
        }
    }

    let mut extras = String::new();
    if let Some(v) = vertical {
        push_setting(&mut extras, &format!("vertical:{v}"));
    }
    // The `y` offset is a line number unless the source carried a `%`. Record
    // the percentage flag (1 = percentage, 0 = bare line number) whenever a
    // `line` offset is present so the writer re-attaches `%` correctly and
    // doesn't fall back to its programmatic-cue percentage default.
    if pos.as_ref().and_then(|p| p.y).is_some() {
        push_setting(
            &mut extras,
            if line_is_pct {
                "line-pct:1"
            } else {
                "line-pct:0"
            },
        );
    }
    if let Some(s) = &line_suffix {
        push_setting(&mut extras, &format!("line-align:{s}"));
    }
    if let Some(s) = &position_suffix {
        push_setting(&mut extras, &format!("position-align:{s}"));
    }
    if let Some(r) = region {
        push_setting(&mut extras, &format!("region:{r}"));
    }

    Some(ParsedTiming {
        start_us,
        end_us,
        position: pos,
        extras,
    })
}

/// Split a cue-setting value into its leading value and an optional
/// `,<suffix>` alignment component (WebVTT §3.5 line/position settings).
fn split_setting_suffix(v: &str) -> (&str, Option<&str>) {
    match v.split_once(',') {
        Some((head, tail)) => (head, Some(tail)),
        None => (v, None),
    }
}

/// Parse a `line` offset: a percentage or a (possibly negative) line number.
fn parse_signed_number(s: &str) -> Option<f32> {
    let neg = s.starts_with('-');
    let body = s.strip_prefix('-').unwrap_or(s);
    let num: String = body
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    let val = num.parse::<f32>().ok()?;
    Some(if neg { -val } else { val })
}

fn push_setting(buf: &mut String, item: &str) {
    if !buf.is_empty() {
        buf.push(' ');
    }
    buf.push_str(item);
}

/// Parse `HH:MM:SS.mmm` or `MM:SS.mmm` into microseconds.
fn parse_vtt_timestamp(s: &str) -> Option<i64> {
    let (hms, ms) = match s.find('.') {
        Some(i) => (&s[..i], &s[i + 1..]),
        None => (s, "000"),
    };
    let parts: Vec<&str> = hms.split(':').collect();
    let (h, m, sec) = match parts.len() {
        3 => (
            parts[0].parse::<u32>().ok()?,
            parts[1].parse::<u32>().ok()?,
            parts[2].parse::<u32>().ok()?,
        ),
        2 => (
            0u32,
            parts[0].parse::<u32>().ok()?,
            parts[1].parse::<u32>().ok()?,
        ),
        _ => return None,
    };
    let ms_val: u32 = if ms.is_empty() { 0 } else { ms.parse().ok()? };
    Some(
        (h as i64) * 3_600_000_000
            + (m as i64) * 60_000_000
            + (sec as i64) * 1_000_000
            + (ms_val as i64) * 1_000,
    )
}

fn format_vtt_ts(us: i64) -> String {
    let us = us.max(0);
    let ms_total = us / 1_000;
    let ms = (ms_total % 1_000) as u32;
    let s_total = ms_total / 1_000;
    let s = (s_total % 60) as u32;
    let m = ((s_total / 60) % 60) as u32;
    let h = (s_total / 3_600) as u32;
    format!("{:02}:{:02}:{:02}.{:03}", h, m, s, ms)
}

fn format_timing_line(cue: &SubtitleCue) -> String {
    format_timing_line_with_extras(cue, None)
}

/// Render the `start --> end` timing line plus cue settings. `extras` carries
/// the unmodeled WebVTT §3.5 settings (vertical / line-align / position-align
/// / region) captured at parse time, keyed off `track.metadata`. Settings are
/// emitted in spec order: vertical, line, position, size, align, region.
fn format_timing_line_with_extras(cue: &SubtitleCue, extras: Option<&str>) -> String {
    let had_extras = extras.is_some();
    let extras = extras.map(parse_cue_extras).unwrap_or_default();
    let mut s = format!(
        "{} --> {}",
        format_vtt_ts(cue.start_us),
        format_vtt_ts(cue.end_us)
    );
    if let Some(v) = &extras.vertical {
        s.push_str(&format!(" vertical:{v}"));
    }
    if let Some(p) = &cue.positioning {
        if let Some(y) = p.y {
            // The `line` offset is a percentage or a (possibly negative) line
            // number. We re-attach `%` when the source carried it
            // (`line_is_pct`), or — for back-compat with cues that have
            // positioning but no parsed extras — when the value isn't an
            // integer line number.
            let as_pct = extras.line_is_pct || !had_extras || y.fract() != 0.0;
            s.push_str(" line:");
            if as_pct {
                s.push_str(&format!("{y}%"));
            } else {
                s.push_str(&format!("{}", y as i64));
            }
            if let Some(a) = &extras.line_align {
                s.push(',');
                s.push_str(a);
            }
        }
        if let Some(x) = p.x {
            s.push_str(&format!(" position:{x}%"));
            if let Some(a) = &extras.position_align {
                s.push(',');
                s.push_str(a);
            }
        }
        if let Some(sz) = p.size {
            s.push_str(&format!(" size:{sz}%"));
        }
        match p.align {
            TextAlign::Center => s.push_str(" align:center"),
            TextAlign::End => s.push_str(" align:end"),
            TextAlign::Left => s.push_str(" align:left"),
            TextAlign::Right => s.push_str(" align:right"),
            TextAlign::Start => {}
        }
    }
    if let Some(r) = &extras.region {
        s.push_str(&format!(" region:{r}"));
    }
    s
}

// ---------------------------------------------------------------------------
// WebVTT inline parser.

fn parse_vtt_inline(body: &str) -> Vec<Segment> {
    let mut p = VttParser { src: body, pos: 0 };
    p.parse_until(None)
}

struct VttParser<'a> {
    src: &'a str,
    pos: usize,
}

impl<'a> VttParser<'a> {
    fn parse_until(&mut self, stop_tag: Option<&str>) -> Vec<Segment> {
        let mut out: Vec<Segment> = Vec::new();
        let mut text_buf = String::new();
        let bytes = self.src.as_bytes();
        while self.pos < bytes.len() {
            let byte = bytes[self.pos];
            if byte == b'<' {
                let tag_end = match self.src[self.pos..].find('>') {
                    Some(e) => e,
                    None => {
                        text_buf.push_str(&self.src[self.pos..]);
                        self.pos = bytes.len();
                        continue;
                    }
                };
                let tag = &self.src[self.pos + 1..self.pos + tag_end];
                // Timestamp `<00:00:01.500>`.
                if let Some(us) = parse_vtt_timestamp(tag.trim()) {
                    if !text_buf.is_empty() {
                        out.push(Segment::Text(std::mem::take(&mut text_buf)));
                    }
                    out.push(Segment::Timestamp { offset_us: us });
                    self.pos += tag_end + 1;
                    continue;
                }
                // Closing tag.
                if let Some(stop) = stop_tag {
                    let close = format!("/{}", stop);
                    if tag.eq_ignore_ascii_case(&close) {
                        if !text_buf.is_empty() {
                            out.push(Segment::Text(std::mem::take(&mut text_buf)));
                        }
                        self.pos += tag_end + 1;
                        return out;
                    }
                }
                // Generic closing tag (e.g. `</c>` outside its own scope).
                if tag.starts_with('/') {
                    // Skip — we're not under that open tag.
                    if !text_buf.is_empty() {
                        out.push(Segment::Text(std::mem::take(&mut text_buf)));
                    }
                    self.pos += tag_end + 1;
                    continue;
                }
                // Opening tag — figure out which.
                let (name, rest) = match tag.find(|c: char| c.is_whitespace() || c == '.') {
                    Some(i) => (&tag[..i], &tag[i..]),
                    None => (tag, ""),
                };
                let name_lc = name.to_ascii_lowercase();
                if !text_buf.is_empty() {
                    out.push(Segment::Text(std::mem::take(&mut text_buf)));
                }
                self.pos += tag_end + 1;
                match name_lc.as_str() {
                    "b" | "i" | "u" => {
                        let children = self.parse_until(Some(&name_lc));
                        out.push(match name_lc.as_str() {
                            "b" => Segment::Bold(children),
                            "i" => Segment::Italic(children),
                            _ => Segment::Underline(children),
                        });
                    }
                    "v" => {
                        let speaker = rest.trim().to_string();
                        let children = self.parse_until(Some("v"));
                        out.push(Segment::Voice {
                            name: speaker,
                            children,
                        });
                    }
                    "c" => {
                        // `<c.name.other>` — treat dot-separated classes by stacking.
                        let name = if let Some(stripped) = rest.strip_prefix('.') {
                            stripped.trim().to_string()
                        } else {
                            rest.trim().to_string()
                        };
                        let children = self.parse_until(Some("c"));
                        out.push(Segment::Class { name, children });
                    }
                    "lang" | "ruby" | "rt" => {
                        // We collapse these to their children (good enough for text).
                        let children = self.parse_until(Some(&name_lc));
                        // Preserve as Raw wrapper to avoid silent drop of the tag on
                        // re-emit.
                        out.push(Segment::Raw(format!("<{}>", tag)));
                        out.extend(children);
                        out.push(Segment::Raw(format!("</{}>", name_lc)));
                    }
                    _ => {
                        out.push(Segment::Raw(format!("<{}>", tag)));
                    }
                }
            } else {
                text_buf.push(byte as char);
                self.pos += 1;
            }
        }
        if !text_buf.is_empty() {
            out.push(Segment::Text(text_buf));
        }
        out
    }
}

fn render_segments(segments: &[Segment]) -> String {
    let mut out = String::new();
    append_segments(segments, &mut out);
    out
}

fn append_segments(segments: &[Segment], out: &mut String) {
    for seg in segments {
        match seg {
            Segment::Text(s) => out.push_str(s),
            Segment::LineBreak => out.push('\n'),
            Segment::Bold(c) => {
                out.push_str("<b>");
                append_segments(c, out);
                out.push_str("</b>");
            }
            Segment::Italic(c) => {
                out.push_str("<i>");
                append_segments(c, out);
                out.push_str("</i>");
            }
            Segment::Underline(c) => {
                out.push_str("<u>");
                append_segments(c, out);
                out.push_str("</u>");
            }
            Segment::Strike(c) => {
                // WebVTT doesn't have a native strike tag — use a class.
                out.push_str("<c.strike>");
                append_segments(c, out);
                out.push_str("</c>");
            }
            Segment::Color { children, .. } | Segment::Font { children, .. } => {
                // WebVTT inline color / font is spec-limited to classes.
                append_segments(children, out);
            }
            Segment::Voice { name, children } => {
                if name.is_empty() {
                    out.push_str("<v>");
                } else {
                    out.push_str(&format!("<v {}>", name));
                }
                append_segments(children, out);
                out.push_str("</v>");
            }
            Segment::Class { name, children } => {
                out.push_str(&format!("<c.{}>", name));
                append_segments(children, out);
                out.push_str("</c>");
            }
            Segment::Karaoke { children, .. } => append_segments(children, out),
            Segment::Timestamp { offset_us } => {
                out.push('<');
                out.push_str(&format_vtt_ts(*offset_us));
                out.push('>');
            }
            Segment::Raw(s) => out.push_str(s),
        }
    }
}

// ---------------------------------------------------------------------------

/// CSS color parser — accepts `#RGB`, `#RRGGBB`, `rgb(r,g,b)`,
/// `rgba(r,g,b,a)`, and named colors. Returns RGBA with an opaque alpha
/// when the source has no alpha.
fn parse_css_color(s: &str) -> Option<(u8, u8, u8, u8)> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix('#') {
        if hex.len() == 3 {
            let r = u8::from_str_radix(&hex[0..1].repeat(2), 16).ok()?;
            let g = u8::from_str_radix(&hex[1..2].repeat(2), 16).ok()?;
            let b = u8::from_str_radix(&hex[2..3].repeat(2), 16).ok()?;
            return Some((r, g, b, 255));
        }
        if hex.len() == 6 {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            return Some((r, g, b, 255));
        }
        if hex.len() == 8 {
            let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
            let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
            let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
            let a = u8::from_str_radix(&hex[6..8], 16).ok()?;
            return Some((r, g, b, a));
        }
        return None;
    }
    if let Some(rest) = s.strip_prefix("rgb(").and_then(|r| r.strip_suffix(')')) {
        let parts: Vec<&str> = rest.split(',').map(|p| p.trim()).collect();
        if parts.len() == 3 {
            let r: u8 = parts[0].parse().ok()?;
            let g: u8 = parts[1].parse().ok()?;
            let b: u8 = parts[2].parse().ok()?;
            return Some((r, g, b, 255));
        }
    }
    if let Some(rest) = s.strip_prefix("rgba(").and_then(|r| r.strip_suffix(')')) {
        let parts: Vec<&str> = rest.split(',').map(|p| p.trim()).collect();
        if parts.len() == 4 {
            let r: u8 = parts[0].parse().ok()?;
            let g: u8 = parts[1].parse().ok()?;
            let b: u8 = parts[2].parse().ok()?;
            let a: f32 = parts[3].parse().ok()?;
            return Some((r, g, b, (a.clamp(0.0, 1.0) * 255.0) as u8));
        }
    }
    match s.to_ascii_lowercase().as_str() {
        "black" => Some((0, 0, 0, 255)),
        "white" => Some((255, 255, 255, 255)),
        "red" => Some((255, 0, 0, 255)),
        "green" => Some((0, 128, 0, 255)),
        "lime" => Some((0, 255, 0, 255)),
        "blue" => Some((0, 0, 255, 255)),
        "yellow" => Some((255, 255, 0, 255)),
        "cyan" | "aqua" => Some((0, 255, 255, 255)),
        "magenta" | "fuchsia" => Some((255, 0, 255, 255)),
        _ => None,
    }
}

pub(crate) fn looks_like_webvtt(buf: &[u8]) -> bool {
    let stripped = if buf.starts_with(&[0xEF, 0xBB, 0xBF]) {
        &buf[3..]
    } else {
        buf
    };
    stripped.starts_with(b"WEBVTT")
}

/// Serialise one cue to its on-wire form — timing line + body.
pub(crate) fn cue_to_bytes(cue: &SubtitleCue) -> Vec<u8> {
    let mut s = String::new();
    s.push_str(&format_timing_line(cue));
    s.push('\n');
    s.push_str(&render_segments(&cue.segments));
    s.into_bytes()
}

pub(crate) fn bytes_to_cue(bytes: &[u8]) -> Result<SubtitleCue> {
    let text = crate::encoding::decode_subtitle_text(bytes);
    let mut lines: Vec<&str> = text.split('\n').map(|l| l.trim_end_matches('\r')).collect();
    while lines.first().map(|l| l.trim().is_empty()).unwrap_or(false) {
        lines.remove(0);
    }
    if lines.is_empty() {
        return Err(Error::invalid("WebVTT: empty cue"));
    }
    // Skip optional id line.
    let first = lines[0];
    let timing_idx = if first.contains("-->") { 0 } else { 1 };
    if lines.len() <= timing_idx {
        return Err(Error::invalid("WebVTT: cue has no timing line"));
    }
    let (start_us, end_us, pos) = parse_timing_and_settings(lines[timing_idx].trim())
        .ok_or_else(|| Error::invalid("WebVTT: bad cue timing"))?;
    let body_lines: Vec<&str> = lines[timing_idx + 1..].to_vec();
    let body = body_lines.join("\n");
    let segments = parse_vtt_inline(body.trim_end());
    Ok(SubtitleCue {
        start_us,
        end_us,
        style_ref: None,
        positioning: pos,
        segments,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal() {
        let src = "WEBVTT\n\n00:00:01.000 --> 00:00:02.000\nhello\n";
        let t = parse(src.as_bytes()).unwrap();
        assert_eq!(t.cues.len(), 1);
        assert_eq!(t.cues[0].start_us, 1_000_000);
    }

    #[test]
    fn parse_style_block() {
        let src = "WEBVTT\n\nSTYLE\n::cue(.yellow) { color: yellow; font-weight: bold; }\n\n00:00:01.000 --> 00:00:02.000\n<c.yellow>hi</c>\n";
        let t = parse(src.as_bytes()).unwrap();
        assert!(t.styles.iter().any(|s| s.name == "yellow" && s.bold));
    }

    #[test]
    fn parse_voice() {
        let src = "WEBVTT\n\n00:00:01.000 --> 00:00:02.000\n<v Alice>hi</v>\n";
        let t = parse(src.as_bytes()).unwrap();
        match &t.cues[0].segments[0] {
            Segment::Voice { name, .. } => assert_eq!(name, "Alice"),
            other => panic!("expected voice: {other:?}"),
        }
    }

    #[test]
    fn looks_like() {
        assert!(looks_like_webvtt(b"WEBVTT\n"));
        assert!(!looks_like_webvtt(b"1\n00:00:01,000"));
    }

    #[test]
    fn parses_vertical_setting() {
        let src = "WEBVTT\n\n00:00:01.000 --> 00:00:02.000 vertical:rl\nhi\n";
        let t = parse(src.as_bytes()).unwrap();
        assert!(t
            .metadata
            .iter()
            .any(|(k, v)| k == "vtt_cue_extra.0" && v.contains("vertical:rl")));
        let out = String::from_utf8(write(&t)).unwrap();
        assert!(out.contains("vertical:rl"), "round-trip: {out}");
    }

    #[test]
    fn roundtrips_line_position_align_suffixes() {
        let src = "WEBVTT\n\n00:00:01.000 --> 00:00:02.000 line:80%,end position:30%,line-left align:start\nhi\n";
        let t = parse(src.as_bytes()).unwrap();
        let extra = t
            .metadata
            .iter()
            .find(|(k, _)| k == "vtt_cue_extra.0")
            .map(|(_, v)| v.as_str())
            .unwrap();
        assert!(extra.contains("line-align:end"), "extras: {extra}");
        assert!(
            extra.contains("position-align:line-left"),
            "extras: {extra}"
        );
        let out = String::from_utf8(write(&t)).unwrap();
        assert!(out.contains("line:80%,end"), "round-trip: {out}");
        assert!(out.contains("position:30%,line-left"), "round-trip: {out}");
    }

    #[test]
    fn roundtrips_negative_line_number() {
        // A bare (non-percentage) negative line number must survive without a
        // spurious `%`.
        let src = "WEBVTT\n\n00:00:01.000 --> 00:00:02.000 line:-1\nhi\n";
        let t = parse(src.as_bytes()).unwrap();
        assert_eq!(t.cues[0].positioning.as_ref().unwrap().y, Some(-1.0));
        let out = String::from_utf8(write(&t)).unwrap();
        assert!(
            out.contains("line:-1\n") || out.contains("line:-1 "),
            "round-trip: {out}"
        );
        assert!(
            !out.contains("line:-1%"),
            "negative line number must not gain %: {out}"
        );
    }

    #[test]
    fn roundtrips_region_reference() {
        let src = "WEBVTT\n\nREGION\nid:fred\nwidth:40%\n\n00:00:01.000 --> 00:00:02.000 region:fred align:start\nhi\n";
        let t = parse(src.as_bytes()).unwrap();
        let out = String::from_utf8(write(&t)).unwrap();
        assert!(out.contains("region:fred"), "round-trip: {out}");
    }

    #[test]
    fn percentage_line_keeps_percent() {
        let src = "WEBVTT\n\n00:00:01.000 --> 00:00:02.000 line:90%\nhi\n";
        let t = parse(src.as_bytes()).unwrap();
        let out = String::from_utf8(write(&t)).unwrap();
        assert!(out.contains("line:90%"), "round-trip: {out}");
    }
}
