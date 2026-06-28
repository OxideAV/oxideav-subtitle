//! ASS / SSA override-tag style-resolution tests.

use oxideav_subtitle::ass_resolve::{
    resolve_line, ClipRegion, Move, ResolvedLine, Rgba, StyleBase,
};

fn base() -> StyleBase {
    StyleBase::default()
}

fn texts(line: &ResolvedLine) -> Vec<String> {
    line.spans.iter().map(|s| s.text.clone()).collect()
}

#[test]
fn plain_text_one_span_base_style() {
    let r = resolve_line("Hello world", &base());
    assert_eq!(texts(&r), vec!["Hello world"]);
    assert_eq!(r.spans[0].style.font_name, "Arial");
    assert_eq!(r.spans[0].style.font_size, 18.0);
    assert!(!r.spans[0].style.bold);
    assert!(!r.spans[0].style.italic);
    assert_eq!(r.layout.alignment, None);
}

#[test]
fn bold_toggle_splits_spans() {
    let r = resolve_line("a{\\b1}b{\\b0}c", &base());
    assert_eq!(texts(&r), vec!["a", "b", "c"]);
    assert!(!r.spans[0].style.bold);
    assert!(r.spans[1].style.bold);
    assert!(!r.spans[2].style.bold);
}

#[test]
fn bold_reset_restores_base() {
    let mut b = base();
    b.bold = true;
    // \b1 then \b (no arg) resets to the base (bold = true here).
    let r = resolve_line("{\\b0}x{\\b}y", &b);
    assert!(!r.spans[0].style.bold);
    assert!(r.spans[1].style.bold, "bare \\b resets to base bold=true");
}

#[test]
fn explicit_weight_sets_bold_threshold() {
    let r = resolve_line("{\\b700}bold{\\b400}normal", &base());
    assert!(r.spans[0].style.bold);
    assert_eq!(r.spans[0].style.weight, Some(700));
    assert!(!r.spans[1].style.bold);
    assert_eq!(r.spans[1].style.weight, Some(400));
}

#[test]
fn italic_underline_strike() {
    let r = resolve_line("{\\i1\\u1\\s1}styled", &base());
    let s = &r.spans[0].style;
    assert!(s.italic && s.underline && s.strike);
}

#[test]
fn primary_color_decoded_bgr() {
    // &H0000FF& is BGR -> r=FF, g=00, b=00 = pure red.
    let r = resolve_line("{\\c&H0000FF&}red", &base());
    assert_eq!(
        r.spans[0].style.primary,
        Rgba {
            r: 0xFF,
            g: 0,
            b: 0,
            a: 255
        }
    );
}

#[test]
fn color_reset_restores_base_primary() {
    let mut b = base();
    b.primary = Rgba {
        r: 1,
        g: 2,
        b: 3,
        a: 200,
    };
    let r = resolve_line("{\\c&H0000FF&}x{\\c}y", &b);
    // reset keeps the base RGB but leaves alpha untouched (alpha is a
    // separate channel: \c only touches RGB).
    assert_eq!(r.spans[1].style.primary.r, 1);
    assert_eq!(r.spans[1].style.primary.g, 2);
    assert_eq!(r.spans[1].style.primary.b, 3);
}

#[test]
fn alpha_inverts_ass_convention() {
    // \1a&HFF& = fully transparent in ASS => straight alpha 0.
    let r = resolve_line("{\\1a&HFF&}x", &base());
    assert_eq!(r.spans[0].style.primary.a, 0);
    // \1a&H00& = opaque => 255.
    let r2 = resolve_line("{\\1a&H00&}x", &base());
    assert_eq!(r2.spans[0].style.primary.a, 255);
}

#[test]
fn alpha_all_components() {
    // \alpha sets all four at once.
    let r = resolve_line("{\\alpha&H80&}x", &base());
    let s = &r.spans[0].style;
    assert_eq!(s.primary.a, 0x7F);
    assert_eq!(s.secondary.a, 0x7F);
    assert_eq!(s.outline_color.a, 0x7F);
    assert_eq!(s.shadow_color.a, 0x7F);
}

#[test]
fn color_targets_each_component() {
    let r = resolve_line("{\\2c&H112233&\\3c&H445566&\\4c&H778899&}x", &base());
    let s = &r.spans[0].style;
    // BGR &H112233& -> r=33 g=22 b=11
    assert_eq!(
        (s.secondary.r, s.secondary.g, s.secondary.b),
        (0x33, 0x22, 0x11)
    );
    assert_eq!(
        (s.outline_color.r, s.outline_color.g, s.outline_color.b),
        (0x66, 0x55, 0x44)
    );
    assert_eq!(
        (s.shadow_color.r, s.shadow_color.g, s.shadow_color.b),
        (0x99, 0x88, 0x77)
    );
}

#[test]
fn font_name_and_size() {
    let r = resolve_line("{\\fnComic Sans\\fs36}x{\\fn\\fs}y", &base());
    assert_eq!(r.spans[0].style.font_name, "Comic Sans");
    assert_eq!(r.spans[0].style.font_size, 36.0);
    // reset both
    assert_eq!(r.spans[1].style.font_name, "Arial");
    assert_eq!(r.spans[1].style.font_size, 18.0);
}

#[test]
fn font_scale_per_axis() {
    let r = resolve_line("{\\fscx200\\fscy50}x", &base());
    assert_eq!(r.spans[0].style.scale_x, 200.0);
    assert_eq!(r.spans[0].style.scale_y, 50.0);
}

#[test]
fn spacing_and_encoding() {
    let r = resolve_line("{\\fsp3\\fe128}x", &base());
    assert_eq!(r.spans[0].style.spacing, 3.0);
    assert_eq!(r.spans[0].style.encoding, 128);
}

#[test]
fn rotation_all_axes() {
    let r = resolve_line("{\\frx10\\fry20\\frz30}x", &base());
    let s = &r.spans[0].style;
    assert_eq!(s.angle_x, 10.0);
    assert_eq!(s.angle_y, 20.0);
    assert_eq!(s.angle_z, 30.0);
}

#[test]
fn bare_fr_is_z_axis() {
    let r = resolve_line("{\\fr45}x", &base());
    assert_eq!(r.spans[0].style.angle_z, 45.0);
}

#[test]
fn border_and_shadow_axes() {
    let r = resolve_line("{\\bord4\\xbord1\\ybord2\\shad3\\xshad5\\yshad6}x", &base());
    let s = &r.spans[0].style;
    // \bord4 sets both then \xbord/\ybord override per-axis.
    assert_eq!(s.border_x, 1.0);
    assert_eq!(s.border_y, 2.0);
    assert_eq!(s.shadow_x, 5.0);
    assert_eq!(s.shadow_y, 6.0);
}

#[test]
fn blur_families() {
    let r = resolve_line("{\\be2\\blur3.5}x", &base());
    assert_eq!(r.spans[0].style.blur_be, 2.0);
    assert_eq!(r.spans[0].style.blur_gauss, 3.5);
}

#[test]
fn pos_move_org_layout() {
    let r = resolve_line("{\\pos(100,200)}x", &base());
    assert_eq!(r.layout.pos, Some((100, 200)));
    let r2 = resolve_line("{\\move(1,2,3,4)}x", &base());
    assert_eq!(
        r2.layout.mv,
        Some(Move {
            x1: 1,
            y1: 2,
            x2: 3,
            y2: 4,
            times: None
        })
    );
    let r3 = resolve_line("{\\move(1,2,3,4,10,20)}x", &base());
    assert_eq!(
        r3.layout.mv,
        Some(Move {
            x1: 1,
            y1: 2,
            x2: 3,
            y2: 4,
            times: Some((10, 20))
        })
    );
    let r4 = resolve_line("{\\org(50,60)}x", &base());
    assert_eq!(r4.layout.org, Some((50, 60)));
}

#[test]
fn alignment_numpad_and_legacy() {
    let r = resolve_line("{\\an7}x", &base());
    assert_eq!(r.layout.alignment, Some(7));
    // legacy \a5 = toptitle-left -> numpad 7.
    let r2 = resolve_line("{\\a5}x", &base());
    assert_eq!(r2.layout.alignment, Some(7));
}

#[test]
fn clip_rectangle_and_drawing() {
    let r = resolve_line("{\\clip(0,0,100,50)}x", &base());
    assert_eq!(
        r.layout.clip,
        Some(ClipRegion::Rectangle {
            inverse: false,
            x1: 0,
            y1: 0,
            x2: 100,
            y2: 50
        })
    );
    let r2 = resolve_line("{\\iclip(m 0 0 l 10 0 10 10)}x", &base());
    match r2.layout.clip {
        Some(ClipRegion::Drawing { inverse, .. }) => assert!(inverse),
        other => panic!("expected drawing clip, got {other:?}"),
    }
}

#[test]
fn karaoke_beat_rides_following_run() {
    let r = resolve_line("{\\k50}ka{\\k30}ra", &base());
    assert_eq!(texts(&r), vec!["ka", "ra"]);
    assert_eq!(r.spans[0].karaoke_cs, Some(50));
    assert_eq!(r.spans[1].karaoke_cs, Some(30));
}

#[test]
fn hard_break_and_soft_break_in_text() {
    let r = resolve_line("a\\Nb\\nc", &base());
    // single span, \N -> newline, \n -> space
    assert_eq!(r.spans.len(), 1);
    assert_eq!(r.spans[0].text, "a\nb c");
}

#[test]
fn hard_space_is_nbsp() {
    let r = resolve_line("a\\hb", &base());
    assert_eq!(r.spans[0].text, "a\u{00A0}b");
}

#[test]
fn transform_does_not_corrupt_running_state() {
    // \t carries nested modifiers but must not change the static state.
    let r = resolve_line("a{\\t(\\frz360)}b", &base());
    assert_eq!(texts(&r), vec!["a", "b"]);
    assert_eq!(r.spans[1].style.angle_z, 0.0);
}

#[test]
fn from_style_seeds_base() {
    let mut st = oxideav_core::SubtitleStyle::new("Default");
    st.bold = true;
    st.font_size = Some(40.0);
    st.font_family = Some("Verdana".into());
    st.primary_color = Some((10, 20, 30, 255));
    let b = StyleBase::from_style(&st);
    let r = resolve_line("x", &b);
    let s = &r.spans[0].style;
    assert!(s.bold);
    assert_eq!(s.font_size, 40.0);
    assert_eq!(s.font_name, "Verdana");
    assert_eq!((s.primary.r, s.primary.g, s.primary.b), (10, 20, 30));
}

#[test]
fn override_block_with_no_text_emits_no_span() {
    let r = resolve_line("{\\b1}", &base());
    assert!(r.spans.is_empty());
}

#[test]
fn multiple_pos_last_wins() {
    let r = resolve_line("{\\pos(1,1)}x{\\pos(9,9)}y", &base());
    assert_eq!(r.layout.pos, Some((9, 9)));
}

#[test]
fn comment_and_unknown_tags_ignored() {
    // A leading non-backslash run is a Comment; \xyz is an unrecognised
    // tag (Other). Both fold as no-ops, so the \b1 still takes effect and
    // the visible text is just "bold". (Each tag body runs to the next
    // backslash, so the recognised \b1 must be backslash-delimited.)
    let r = resolve_line("{a comment\\b1\\xyz}bold", &base());
    assert!(r.spans[0].style.bold);
    assert_eq!(r.spans[0].text, "bold");
}

#[test]
fn reset_bare_cancels_overrides() {
    // \b1 turns bold on; \r resets the running style back to the base, so
    // the text after \r is not bold.
    use oxideav_subtitle::ass_resolve::resolve_line;
    let r = resolve_line("{\\b1}bold{\\r}plain", &base());
    assert!(r.spans[0].style.bold, "first span bold");
    assert!(!r.spans[1].style.bold, "post-reset span not bold");
}

#[test]
fn reset_named_swaps_active_base() {
    // \rBig switches the active base to one whose font size is 72; a later
    // parameterless \fs (reset-to-style) then resolves against the swapped
    // base rather than the line base.
    use oxideav_subtitle::ass_resolve::{resolve_tokens_with_styles, StyleBase};
    use oxideav_subtitle::ass_tags::tokenize;

    let line_base = StyleBase {
        font_size: 18.0,
        ..StyleBase::default()
    };
    let toks = tokenize("{\\fs40}A{\\rBig}B{\\fs}C");
    let r = resolve_tokens_with_styles(&toks, &line_base, |name| {
        (name == "Big").then(|| StyleBase {
            font_size: 72.0,
            ..StyleBase::default()
        })
    });
    // "A" under the explicit \fs40.
    assert_eq!(r.spans[0].text, "A");
    assert_eq!(r.spans[0].style.font_size, 40.0);
    // "B" after \rBig — reset to the swapped base (72).
    assert_eq!(r.spans[1].text, "B");
    assert_eq!(r.spans[1].style.font_size, 72.0);
    // "C" after a parameterless \fs — reset-to-style against the *swapped*
    // base, so 72 not the line's 18.
    assert_eq!(r.spans[2].text, "C");
    assert_eq!(r.spans[2].style.font_size, 72.0);
}

#[test]
fn reset_named_unknown_falls_back_to_line_base() {
    // An unknown \r<style> name resets to the line base, not an error.
    use oxideav_subtitle::ass_resolve::resolve_line;
    let r = resolve_line("{\\b1}x{\\rNope}y", &base());
    assert!(r.spans[0].style.bold);
    assert!(!r.spans[1].style.bold);
}

#[test]
fn wrap_mode_lands_on_layout() {
    use oxideav_subtitle::ass_resolve::resolve_line;
    use oxideav_subtitle::WrapStyle;
    let r = resolve_line("{\\q2}no wrap here", &base());
    assert_eq!(r.layout.wrap, Some(WrapStyle::None));
    // A line without \q leaves wrap None (defer to script-level WrapStyle).
    let r2 = resolve_line("plain", &base());
    assert_eq!(r2.layout.wrap, None);
}
