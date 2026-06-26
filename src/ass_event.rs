//! Parse / serialize an ASS / SSA `[Events]` `Dialogue:` (and `Comment:`)
//! line into a typed [`AssEvent`].
//!
//! Per the SSA v4 script-format specification (mirrored at
//! `docs/subtitles/ass/ass-specs-tcax.html`), the `[Events]` section is
//! introduced by a `Format:` line and every event is a comma-separated
//! row whose interpretation follows that header. The documented field
//! list is:
//!
//! > Marked, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect,
//! > Text
//!
//! "The last field will always be the Text field, so that it can contain
//! commas." The first field is `Marked` in classic SSA (`Marked=0` /
//! `Marked=1`) and `Layer` ("any integer") in ASS V4+ — both occupy the
//! same leading slot, so the parser keeps whichever the header named.
//!
//! Times are in `Hrs:Mins:Secs:hundredths` ("Note that there is a single
//! digit for the hours!"), i.e. `H:MM:SS.cc`. The parser accepts either a
//! dot or a colon before the hundredths (the spec's example uses a colon;
//! real-world ASS uses a dot) and the writer emits the canonical dotted
//! `H:MM:SS.cc` form.
//!
//! The default field order is [`DEFAULT_EVENT_FORMAT`]; a `Format:` line
//! parsed with [`crate::ass_style_row::parse_format`] can remap the
//! columns by name.

/// Centiseconds (hundredths of a second), the resolution of an ASS event
/// timestamp.
pub type CentiSec = i64;

/// The canonical ASS V4+ `[Events]` `Format:` field order, used when an
/// event row has no preceding `Format:` line to map against.
pub const DEFAULT_EVENT_FORMAT: &[&str] = &[
    "Layer", "Start", "End", "Style", "Name", "MarginL", "MarginR", "MarginV", "Effect", "Text",
];

/// A parsed `[Events]` row — a `Dialogue:` or `Comment:` line.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AssEvent {
    /// `true` for a `Comment:` row (not rendered), `false` for
    /// `Dialogue:`.
    pub comment: bool,
    /// The leading `Layer` (ASS) / `Marked` (SSA) field, kept verbatim as
    /// the integer it carries (`0` when absent).
    pub layer: i64,
    /// Start time in centiseconds.
    pub start_cs: CentiSec,
    /// End time in centiseconds.
    pub end_cs: CentiSec,
    /// `Style` name referenced from the `[V4+ Styles]` table.
    pub style: String,
    /// `Name` / actor field.
    pub name: String,
    pub margin_l: i32,
    pub margin_r: i32,
    pub margin_v: i32,
    /// `Effect` field (e.g. `Scroll up;...`, `Banner;...`), kept verbatim.
    pub effect: String,
    /// The raw `Text` field (override blocks + visible text), kept exactly
    /// as written so [`crate::ass_tags::tokenize`] / [`crate::ass_resolve`]
    /// can take it from here.
    pub text: String,
}

/// Parse a `Dialogue:` / `Comment:` event row against an ordered field
/// list (a [`crate::ass_style_row::parse_format`] result or
/// [`DEFAULT_EVENT_FORMAT`]).
///
/// The leading `Dialogue:` / `Comment:` keyword selects
/// [`AssEvent::comment`]; if neither keyword is present the row is parsed
/// as a `Dialogue`. The value list is split into at most `format.len()`
/// fields so the final `Text` field absorbs any embedded commas, matching
/// the spec's "The last field will always be the Text field".
///
/// Returns `None` only when the format list is empty or the row carries
/// no timing field that parses. Unknown header names are skipped; a
/// missing or malformed numeric field falls back to `0`.
pub fn parse_event<S: AsRef<str>>(line: &str, format: &[S]) -> Option<AssEvent> {
    if format.is_empty() {
        return None;
    }
    let t = line.trim_start();
    let (comment, body) = if let Some(rest) = strip_kw(t, "Dialogue") {
        (false, rest)
    } else if let Some(rest) = strip_kw(t, "Comment") {
        (true, rest)
    } else {
        (false, t)
    };

    let n = format.len();
    let values: Vec<&str> = body.splitn(n, ',').collect();

    let mut ev = AssEvent {
        comment,
        ..Default::default()
    };
    let mut saw_timing = false;

    for (i, field) in format.iter().enumerate() {
        let raw = match values.get(i) {
            Some(v) => v.trim(),
            None => continue,
        };
        match field.as_ref() {
            // Layer (ASS) / Marked (SSA) share the leading slot. `Marked`
            // may be written `Marked=1`; take the integer after `=`.
            "Layer" | "Marked" => {
                let v = raw.rsplit('=').next().unwrap_or(raw);
                ev.layer = v.trim().parse().unwrap_or(0);
            }
            "Start" => {
                if let Some(cs) = parse_time(raw) {
                    ev.start_cs = cs;
                    saw_timing = true;
                }
            }
            "End" => {
                if let Some(cs) = parse_time(raw) {
                    ev.end_cs = cs;
                    saw_timing = true;
                }
            }
            "Style" => ev.style = raw.to_string(),
            "Name" | "Actor" => ev.name = raw.to_string(),
            "MarginL" => ev.margin_l = raw.parse().unwrap_or(0),
            "MarginR" => ev.margin_r = raw.parse().unwrap_or(0),
            "MarginV" => ev.margin_v = raw.parse().unwrap_or(0),
            "Effect" => ev.effect = raw.to_string(),
            "Text" => {
                // Text is the final field — take the verbatim remainder
                // (which `splitn` already kept whole, commas included).
                ev.text = values.get(i).copied().unwrap_or("").to_string();
            }
            _ => {}
        }
    }

    if !saw_timing {
        return None;
    }
    Some(ev)
}

/// Serialize an [`AssEvent`] into a `Dialogue:` / `Comment:` row for the
/// given field order. The result parses back through [`parse_event`] to
/// an equal [`AssEvent`] (the `Text` field's verbatim bytes survive,
/// times re-emit in canonical `H:MM:SS.cc` form).
pub fn event_to_string<S: AsRef<str>>(ev: &AssEvent, format: &[S]) -> String {
    let mut out = String::from(if ev.comment {
        "Comment: "
    } else {
        "Dialogue: "
    });
    for (i, field) in format.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        let cell: String = match field.as_ref() {
            "Layer" | "Marked" => ev.layer.to_string(),
            "Start" => fmt_time(ev.start_cs),
            "End" => fmt_time(ev.end_cs),
            "Style" => ev.style.clone(),
            "Name" | "Actor" => ev.name.clone(),
            "MarginL" => ev.margin_l.to_string(),
            "MarginR" => ev.margin_r.to_string(),
            "MarginV" => ev.margin_v.to_string(),
            "Effect" => ev.effect.clone(),
            "Text" => ev.text.clone(),
            _ => String::new(),
        };
        out.push_str(&cell);
    }
    out
}

/// Parse an ASS `H:MM:SS.cc` timestamp into centiseconds.
///
/// Accepts a single- or multi-digit hour, two-digit minutes / seconds,
/// and two-digit hundredths separated by either `.` or `:` (the spec's
/// example writes `0:00:00:00` but real-world ASS uses
/// `0:00:00.00`). One- or two-digit hundredths are both tolerated.
/// Returns `None` on a malformed value.
pub fn parse_time(s: &str) -> Option<CentiSec> {
    let t = s.trim();
    // Split off the hundredths separator first (`.` preferred, else the
    // final `:`).
    let (hms, cc) = if let Some(idx) = t.find('.') {
        (&t[..idx], &t[idx + 1..])
    } else {
        // Use the last colon as the hundredths separator only if there
        // are at least three colons (H:MM:SS:cc).
        let colon_count = t.bytes().filter(|&b| b == b':').count();
        if colon_count >= 3 {
            let idx = t.rfind(':')?;
            (&t[..idx], &t[idx + 1..])
        } else {
            (t, "")
        }
    };

    let mut parts = hms.split(':');
    let h: i64 = parts.next()?.trim().parse().ok()?;
    let m: i64 = parts.next()?.trim().parse().ok()?;
    let sec: i64 = parts.next()?.trim().parse().ok()?;
    if parts.next().is_some() {
        return None; // too many colons in the H:MM:SS portion
    }
    if !(0..60).contains(&m) || !(0..60).contains(&sec) {
        return None;
    }
    let centi: i64 = if cc.is_empty() {
        0
    } else {
        if cc.len() > 2 || !cc.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        // `5` means 50 cs, `05` means 5 cs.
        let v: i64 = cc.parse().ok()?;
        if cc.len() == 1 {
            v * 10
        } else {
            v
        }
    };
    Some(((h * 3600 + m * 60 + sec) * 100) + centi)
}

/// Emit centiseconds as the canonical ASS `H:MM:SS.cc` timestamp (single
/// hour digit floor, zero-padded minutes / seconds / hundredths).
pub fn fmt_time(cs: CentiSec) -> String {
    let cs = cs.max(0);
    let centi = cs % 100;
    let total_secs = cs / 100;
    let s = total_secs % 60;
    let total_mins = total_secs / 60;
    let m = total_mins % 60;
    let h = total_mins / 60;
    format!("{h}:{m:02}:{s:02}.{centi:02}")
}

/// Strip a leading `Keyword:` token (case-insensitive). Returns the body
/// trimmed of leading whitespace, or `None` if the keyword is absent.
fn strip_kw<'a>(t: &'a str, keyword: &str) -> Option<&'a str> {
    if t.len() >= keyword.len() && t[..keyword.len()].eq_ignore_ascii_case(keyword) {
        let rest = t[keyword.len()..].trim_start();
        return rest.strip_prefix(':').map(|r| r.trim_start());
    }
    None
}
