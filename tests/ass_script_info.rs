//! Integration test for the public `ass_script_info` accessor surface.
//!
//! Mirrors the kind of IR an `oxideav-ass` decode is expected to
//! populate: lowercase / snake-cased `[Script Info]` keys per the
//! crate-level documented contract.

use oxideav_subtitle::{
    ass_script_info::{script_info_keys, AssScriptInfo, Collisions, WrapStyle},
    script_info, SourceFormat, SubtitleTrack,
};

fn ass_track(meta: &[(&str, &str)]) -> SubtitleTrack {
    let mut t = SubtitleTrack::new().with_source(SourceFormat::AssOrSsa);
    for (k, v) in meta {
        t.metadata.push((k.to_string(), v.to_string()));
    }
    t
}

#[test]
fn full_aegisub_default_script_info_is_typed_through_accessor() {
    let t = ass_track(&[
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

#[test]
fn non_ass_source_is_still_safe_to_call() {
    // A WebVTT track has no Script Info keys; the accessor returns
    // a fully-defaulted struct without error.
    let mut t = SubtitleTrack::new().with_source(SourceFormat::WebVtt);
    t.metadata
        .push(("header".to_string(), "Kind: subtitles".to_string()));
    let info = script_info(&t);
    assert_eq!(info, AssScriptInfo::default());
    // Foreign-format keys aren't dropped — they stay in metadata.
    assert!(t.metadata.iter().any(|(k, _)| k == "header"));
}

#[test]
fn recognised_keys_match_doc_table() {
    // Sanity-check the published key set against what the
    // documentation table on `crate::ass_script_info` enumerates.
    let keys = script_info_keys();
    let want = [
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
    ];
    assert_eq!(keys, want.as_slice());
}

#[test]
fn wrap_style_round_trips_through_as_u8() {
    for v in [
        WrapStyle::SmartEven,
        WrapStyle::EndOfLine,
        WrapStyle::None,
        WrapStyle::SmartLower,
    ] {
        let s = v.as_u8().to_string();
        assert_eq!(WrapStyle::from_value(&s), Some(v));
    }
}

#[test]
fn collisions_other_carries_freeform_value() {
    let info = script_info(&ass_track(&[("collisions", "vendor-specific")]));
    assert_eq!(
        info.collisions,
        Some(Collisions::Other("vendor-specific".into()))
    );
    assert_eq!(
        info.collisions.as_ref().map(|c| c.as_str()),
        Some("vendor-specific")
    );
}
