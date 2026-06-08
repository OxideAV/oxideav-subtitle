//! Typed accessor for the ASS / SSA `[Script Info]` block carried in
//! [`SubtitleTrack::metadata`].
//!
//! Parsing of `.ass` / `.ssa` files themselves lives in the sibling
//! `oxideav-ass` crate. This module is a read-side helper: when an
//! `oxideav-ass` decode populates the shared IR, the `[Script Info]`
//! key/value pairs are written into `SubtitleTrack::metadata` under
//! the lowercase-snake-case normalisation documented on
//! [`SubtitleTrack`]'s `metadata` field. Downstream code that wants
//! typed `PlayResX` / `WrapStyle` / `ScaledBorderAndShadow` access
//! routes through [`script_info`] instead of repeating the lowercase
//! key + string-to-number boilerplate.
//!
//! The canonical key set and value-shape rules come from the SSA v4
//! script-format specification mirrored at
//! `docs/subtitles/ass/ass-specs-tcax.html` (the HTML transcription of
//! the original Kotus / CS.LOVE `.doc` distributed with SubStation
//! Alpha) and the Aegisub override-tag reference mirrored at
//! `docs/subtitles/ass/aegisub-ass-tags.html` (used to validate
//! `WrapStyle`'s 0..=3 range — the Aegisub `\q` tag mirrors the
//! `[Script Info] WrapStyle:` default at the per-cue level).
//!
//! ## Canonical key contract
//!
//! When an ASS source populates the IR, every `[Script Info]`
//! key/value pair is stored verbatim except the key is lowercased,
//! ASCII spaces become underscores, and leading / trailing whitespace
//! is trimmed. The reverse-direction writer in `oxideav-ass`
//! reconstitutes the original mixed-case spelling on emit. The keys
//! recognised by [`script_info`] are:
//!
//! | IR key                          | `[Script Info]` field    | Type           |
//! |---------------------------------|--------------------------|----------------|
//! | `title`                         | `Title`                  | `String`       |
//! | `original_script`               | `Original Script`        | `String`       |
//! | `original_translation`          | `Original Translation`   | `String`       |
//! | `original_editing`              | `Original Editing`       | `String`       |
//! | `original_timing`               | `Original Timing`        | `String`       |
//! | `synch_point`                   | `Synch Point`            | `String`       |
//! | `script_updated_by`             | `Script Updated By`      | `String`       |
//! | `update_details`                | `Update Details`         | `String`       |
//! | `script_type`                   | `ScriptType`             | `String`       |
//! | `collisions`                    | `Collisions`             | `Collisions`   |
//! | `play_res_x`                    | `PlayResX`               | `u32`          |
//! | `play_res_y`                    | `PlayResY`               | `u32`          |
//! | `play_depth`                    | `PlayDepth`              | `u32`          |
//! | `timer`                         | `Timer`                  | `f64` percent  |
//! | `wrap_style`                    | `WrapStyle`              | `WrapStyle`    |
//! | `scaled_border_and_shadow`      | `ScaledBorderAndShadow`  | `bool`         |
//!
//! Unknown / unrecognised keys are left in `metadata` for the caller
//! to inspect; this accessor only surfaces the spec-defined set.

use crate::SubtitleTrack;

/// Wrap-style enum mirroring the `[Script Info] WrapStyle:` field
/// (and the per-cue `\q` override tag).
///
/// Per `ass-specs-tcax.html`:
///
/// * `0` — smart wrapping, lines are evenly broken.
/// * `1` — end-of-line word wrapping, only `\N` breaks.
/// * `2` — no word wrapping, `\n` `\N` both break.
/// * `3` — same as `0`, but the lower line gets wider.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WrapStyle {
    /// `0` — smart wrapping, lines evenly broken.
    SmartEven,
    /// `1` — end-of-line wrapping; only `\N` breaks the line.
    EndOfLine,
    /// `2` — no automatic wrapping; both `\n` and `\N` break.
    None,
    /// `3` — smart wrapping, lower line gets wider.
    SmartLower,
}

impl WrapStyle {
    /// Parse the numeric value carried in the IR metadata string. The
    /// SSA spec only defines `0..=3`; anything outside is rejected.
    pub fn from_value(s: &str) -> Option<Self> {
        match s.trim() {
            "0" => Some(WrapStyle::SmartEven),
            "1" => Some(WrapStyle::EndOfLine),
            "2" => Some(WrapStyle::None),
            "3" => Some(WrapStyle::SmartLower),
            _ => None,
        }
    }

    /// Numeric form for re-emit (`0..=3`).
    pub fn as_u8(self) -> u8 {
        match self {
            WrapStyle::SmartEven => 0,
            WrapStyle::EndOfLine => 1,
            WrapStyle::None => 2,
            WrapStyle::SmartLower => 3,
        }
    }
}

/// Collision-prevention mode mirroring the `[Script Info] Collisions:`
/// field.
///
/// Per spec, two values are documented (case-insensitive): `Normal`
/// (subtitles stack as close to the bottom margin as possible) and
/// `Reverse` (the opposite stacking direction). Any other free-form
/// value is preserved verbatim via [`Collisions::Other`] so the
/// accessor never drops information.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Collisions {
    Normal,
    Reverse,
    Other(String),
}

impl Collisions {
    /// Parse the case-insensitive value carried in IR metadata.
    pub fn from_value(s: &str) -> Self {
        let t = s.trim();
        if t.eq_ignore_ascii_case("normal") {
            Collisions::Normal
        } else if t.eq_ignore_ascii_case("reverse") {
            Collisions::Reverse
        } else {
            Collisions::Other(t.to_string())
        }
    }

    /// Canonical mixed-case spelling for re-emit. `Other` strings are
    /// returned unchanged.
    pub fn as_str(&self) -> &str {
        match self {
            Collisions::Normal => "Normal",
            Collisions::Reverse => "Reverse",
            Collisions::Other(s) => s.as_str(),
        }
    }
}

/// Typed view of the `[Script Info]` keys an ASS-source track carries
/// in [`SubtitleTrack::metadata`].
///
/// Every field is `Option`-shaped because the SSA spec marks all
/// fields except `ScriptType` and the `PlayRes*` pair as optional.
/// Even those are missing on real-world files often enough that we
/// don't force them either; callers decide what to do with `None`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AssScriptInfo {
    pub title: Option<String>,
    pub original_script: Option<String>,
    pub original_translation: Option<String>,
    pub original_editing: Option<String>,
    pub original_timing: Option<String>,
    pub synch_point: Option<String>,
    pub script_updated_by: Option<String>,
    pub update_details: Option<String>,
    pub script_type: Option<String>,
    pub collisions: Option<Collisions>,
    pub play_res_x: Option<u32>,
    pub play_res_y: Option<u32>,
    pub play_depth: Option<u32>,
    /// Timer speed as a percentage (e.g. `100.0` means 100%).
    pub timer: Option<f64>,
    pub wrap_style: Option<WrapStyle>,
    pub scaled_border_and_shadow: Option<bool>,
}

/// Read the SSA `[Script Info]` view out of a track. Keys this
/// accessor doesn't recognise stay in `metadata` for the caller's own
/// inspection.
///
/// Returns a fully-defaulted (`None`-valued) `AssScriptInfo` for a
/// track with no Script Info entries — the call never fails.
pub fn script_info(track: &SubtitleTrack) -> AssScriptInfo {
    let mut out = AssScriptInfo::default();
    for (k, v) in &track.metadata {
        match k.as_str() {
            "title" => out.title = Some(v.clone()),
            "original_script" => out.original_script = Some(v.clone()),
            "original_translation" => out.original_translation = Some(v.clone()),
            "original_editing" => out.original_editing = Some(v.clone()),
            "original_timing" => out.original_timing = Some(v.clone()),
            "synch_point" => out.synch_point = Some(v.clone()),
            "script_updated_by" => out.script_updated_by = Some(v.clone()),
            "update_details" => out.update_details = Some(v.clone()),
            "script_type" => out.script_type = Some(v.clone()),
            "collisions" => out.collisions = Some(Collisions::from_value(v)),
            "play_res_x" => out.play_res_x = v.trim().parse().ok(),
            "play_res_y" => out.play_res_y = v.trim().parse().ok(),
            "play_depth" => out.play_depth = v.trim().parse().ok(),
            "timer" => out.timer = parse_timer(v),
            "wrap_style" => out.wrap_style = WrapStyle::from_value(v),
            "scaled_border_and_shadow" => {
                out.scaled_border_and_shadow = parse_yes_no(v);
            }
            _ => {}
        }
    }
    out
}

/// Lowercase IR-keys this accessor recognises. Useful to filter the
/// `metadata` vector down to "everything Script Info doesn't claim".
pub fn script_info_keys() -> &'static [&'static str] {
    &[
        "title",
        "original_script",
        "original_translation",
        "original_editing",
        "original_timing",
        "synch_point",
        "script_updated_by",
        "update_details",
        "script_type",
        "collisions",
        "play_res_x",
        "play_res_y",
        "play_depth",
        "timer",
        "wrap_style",
        "scaled_border_and_shadow",
    ]
}

/// Parse the `Timer:` percentage. The tcax-mirrored spec gives the
/// example `100.0000` (four-fraction-digit form Aegisub writes), but
/// integer / shorter-fraction forms appear in the wild. Trailing
/// `%` is tolerated, the value is parsed as `f64`.
fn parse_timer(s: &str) -> Option<f64> {
    let t = s.trim().trim_end_matches('%').trim();
    t.parse().ok()
}

/// `ScaledBorderAndShadow` carries `yes` / `no` (Aegisub default
/// `yes`). Both case-insensitive. `1` / `0` and `true` / `false` are
/// tolerated for interop with non-Aegisub authoring tools that mirror
/// the equivalent VSFilter registry boolean.
fn parse_yes_no(s: &str) -> Option<bool> {
    let t = s.trim();
    if t.eq_ignore_ascii_case("yes") || t.eq_ignore_ascii_case("true") || t == "1" {
        Some(true)
    } else if t.eq_ignore_ascii_case("no") || t.eq_ignore_ascii_case("false") || t == "0" {
        Some(false)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SubtitleTrack;

    fn track_with(meta: &[(&str, &str)]) -> SubtitleTrack {
        let mut t = SubtitleTrack::new();
        for (k, v) in meta {
            t.metadata.push((k.to_string(), v.to_string()));
        }
        t
    }

    #[test]
    fn empty_metadata_yields_all_none() {
        let info = script_info(&SubtitleTrack::new());
        assert_eq!(info, AssScriptInfo::default());
        assert!(info.title.is_none());
        assert!(info.wrap_style.is_none());
        assert!(info.play_res_x.is_none());
    }

    #[test]
    fn strings_round_trip_as_is() {
        let t = track_with(&[
            ("title", "Episode 01 — Pilot"),
            ("original_script", "Hayao Miyazaki"),
            ("original_translation", "fansub-team-x"),
            ("original_editing", "ed1, ed2"),
            ("original_timing", "tm1"),
            ("synch_point", "00:00:30.00"),
            ("script_updated_by", "rev-team"),
            ("update_details", "rev1 typos; rev2 timing"),
            ("script_type", "v4.00+"),
        ]);
        let info = script_info(&t);
        assert_eq!(info.title.as_deref(), Some("Episode 01 — Pilot"));
        assert_eq!(info.original_script.as_deref(), Some("Hayao Miyazaki"));
        assert_eq!(info.original_translation.as_deref(), Some("fansub-team-x"));
        assert_eq!(info.original_editing.as_deref(), Some("ed1, ed2"));
        assert_eq!(info.original_timing.as_deref(), Some("tm1"));
        assert_eq!(info.synch_point.as_deref(), Some("00:00:30.00"));
        assert_eq!(info.script_updated_by.as_deref(), Some("rev-team"));
        assert_eq!(
            info.update_details.as_deref(),
            Some("rev1 typos; rev2 timing")
        );
        assert_eq!(info.script_type.as_deref(), Some("v4.00+"));
    }

    #[test]
    fn play_res_parses_to_u32() {
        let t = track_with(&[
            ("play_res_x", "1920"),
            ("play_res_y", "1080"),
            ("play_depth", "32"),
        ]);
        let info = script_info(&t);
        assert_eq!(info.play_res_x, Some(1920));
        assert_eq!(info.play_res_y, Some(1080));
        assert_eq!(info.play_depth, Some(32));
    }

    #[test]
    fn play_res_with_whitespace_is_tolerated() {
        let t = track_with(&[("play_res_x", "  640  "), ("play_res_y", "\t480")]);
        let info = script_info(&t);
        assert_eq!(info.play_res_x, Some(640));
        assert_eq!(info.play_res_y, Some(480));
    }

    #[test]
    fn play_res_malformed_drops_to_none() {
        let t = track_with(&[("play_res_x", "not-a-number"), ("play_res_y", "-1")]);
        let info = script_info(&t);
        assert_eq!(info.play_res_x, None);
        assert_eq!(info.play_res_y, None);
    }

    #[test]
    fn timer_parses_aegisub_default_shape() {
        let info = script_info(&track_with(&[("timer", "100.0000")]));
        assert_eq!(info.timer, Some(100.0));
    }

    #[test]
    fn timer_tolerates_integer_and_percent_suffix() {
        assert_eq!(
            script_info(&track_with(&[("timer", "120")])).timer,
            Some(120.0)
        );
        assert_eq!(
            script_info(&track_with(&[("timer", "95.5%")])).timer,
            Some(95.5)
        );
        assert_eq!(
            script_info(&track_with(&[("timer", "  50.0  ")])).timer,
            Some(50.0)
        );
    }

    #[test]
    fn wrap_style_covers_all_four_values() {
        for (s, expected) in [
            ("0", WrapStyle::SmartEven),
            ("1", WrapStyle::EndOfLine),
            ("2", WrapStyle::None),
            ("3", WrapStyle::SmartLower),
        ] {
            let info = script_info(&track_with(&[("wrap_style", s)]));
            assert_eq!(info.wrap_style, Some(expected));
            assert_eq!(expected.as_u8().to_string(), s);
        }
    }

    #[test]
    fn wrap_style_out_of_range_drops_to_none() {
        for bogus in ["4", "-1", "0x0", "smart", ""] {
            let info = script_info(&track_with(&[("wrap_style", bogus)]));
            assert!(
                info.wrap_style.is_none(),
                "expected None for {bogus:?}, got {:?}",
                info.wrap_style
            );
        }
    }

    #[test]
    fn collisions_normal_and_reverse_case_insensitive() {
        assert_eq!(
            script_info(&track_with(&[("collisions", "Normal")])).collisions,
            Some(Collisions::Normal)
        );
        assert_eq!(
            script_info(&track_with(&[("collisions", "NORMAL")])).collisions,
            Some(Collisions::Normal)
        );
        assert_eq!(
            script_info(&track_with(&[("collisions", "reverse")])).collisions,
            Some(Collisions::Reverse)
        );
        assert_eq!(Collisions::Normal.as_str(), "Normal");
        assert_eq!(Collisions::Reverse.as_str(), "Reverse");
    }

    #[test]
    fn collisions_other_preserves_payload_verbatim() {
        let info = script_info(&track_with(&[("collisions", "  Stack-Up  ")]));
        // trim happens; payload otherwise unchanged.
        assert_eq!(
            info.collisions,
            Some(Collisions::Other("Stack-Up".to_string()))
        );
        assert_eq!(
            info.collisions.as_ref().map(|c| c.as_str()),
            Some("Stack-Up")
        );
    }

    #[test]
    fn scaled_border_and_shadow_accepts_yes_no_and_aliases() {
        for yes in ["yes", "Yes", "YES", "true", "True", "1"] {
            let info = script_info(&track_with(&[("scaled_border_and_shadow", yes)]));
            assert_eq!(
                info.scaled_border_and_shadow,
                Some(true),
                "expected true for {yes:?}"
            );
        }
        for no in ["no", "NO", "false", "False", "0"] {
            let info = script_info(&track_with(&[("scaled_border_and_shadow", no)]));
            assert_eq!(
                info.scaled_border_and_shadow,
                Some(false),
                "expected false for {no:?}"
            );
        }
    }

    #[test]
    fn scaled_border_and_shadow_unknown_value_is_none() {
        let info = script_info(&track_with(&[("scaled_border_and_shadow", "maybe")]));
        assert_eq!(info.scaled_border_and_shadow, None);
    }

    #[test]
    fn unknown_keys_are_left_alone() {
        let t = track_with(&[
            ("title", "X"),
            ("custom_key", "anything"),
            ("aegisub_project_garbage", "..."),
        ]);
        let info = script_info(&t);
        assert_eq!(info.title.as_deref(), Some("X"));
        // The custom keys stay in t.metadata; the accessor doesn't drop them.
        assert!(t
            .metadata
            .iter()
            .any(|(k, _)| k == "aegisub_project_garbage"));
    }

    #[test]
    fn script_info_keys_listed_matches_recognised_keys() {
        let keys = script_info_keys();
        assert_eq!(keys.len(), 16);
        // Every listed key must round-trip through the matcher: building a
        // 1-entry track with each key and a parseable value must produce
        // some change vs. the all-default `AssScriptInfo`.
        for key in keys {
            let value = match *key {
                "play_res_x" | "play_res_y" | "play_depth" => "10",
                "timer" => "100.0",
                "wrap_style" => "0",
                "scaled_border_and_shadow" => "yes",
                "collisions" => "Normal",
                _ => "x",
            };
            let info = script_info(&track_with(&[(key, value)]));
            assert_ne!(
                info,
                AssScriptInfo::default(),
                "key {key} didn't change anything; matcher arm missing?"
            );
        }
    }

    #[test]
    fn full_realistic_script_info_round_trip() {
        // Shape of a real Aegisub-emitted [Script Info] block once
        // lowercased + snake-cased by the sibling crate's ingestor.
        let t = track_with(&[
            ("title", "Default Aegisub file"),
            ("script_type", "v4.00+"),
            ("wrap_style", "0"),
            ("scaled_border_and_shadow", "yes"),
            ("play_res_x", "640"),
            ("play_res_y", "480"),
            ("timer", "100.0000"),
            ("collisions", "Normal"),
        ]);
        let info = script_info(&t);
        assert_eq!(info.title.as_deref(), Some("Default Aegisub file"));
        assert_eq!(info.script_type.as_deref(), Some("v4.00+"));
        assert_eq!(info.wrap_style, Some(WrapStyle::SmartEven));
        assert_eq!(info.scaled_border_and_shadow, Some(true));
        assert_eq!(info.play_res_x, Some(640));
        assert_eq!(info.play_res_y, Some(480));
        assert_eq!(info.timer, Some(100.0));
        assert_eq!(info.collisions, Some(Collisions::Normal));
    }
}
