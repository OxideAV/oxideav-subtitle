//! ASS / SSA `Style:` row + `Format:` header parsing tests.

use oxideav_subtitle::ass_resolve::{resolve_line, Rgba};
use oxideav_subtitle::ass_style_row::{
    parse_color, parse_format, parse_style_row, DEFAULT_V4PLUS_FORMAT,
};

#[test]
fn parse_format_line() {
    let f = parse_format("Format: Name, Fontname, Fontsize, PrimaryColour").unwrap();
    assert_eq!(f, vec!["Name", "Fontname", "Fontsize", "PrimaryColour"]);
}

#[test]
fn parse_format_without_keyword() {
    let f = parse_format("Name, Fontname").unwrap();
    assert_eq!(f, vec!["Name", "Fontname"]);
}

#[test]
fn color_ass_hex_no_alpha() {
    // &H0000FF& -> BGR -> r=FF, opaque.
    assert_eq!(
        parse_color("&H0000FF&"),
        Some(Rgba {
            r: 0xFF,
            g: 0,
            b: 0,
            a: 255
        })
    );
}

#[test]
fn color_ass_hex_with_alpha_inverted() {
    // &HFF0000FF& -> alpha byte FF (transparent) -> straight alpha 0.
    assert_eq!(
        parse_color("&HFF0000FF&"),
        Some(Rgba {
            r: 0xFF,
            g: 0,
            b: 0,
            a: 0
        })
    );
    // &H80FFFFFF& -> alpha 0x80 -> straight alpha 0x7F.
    let c = parse_color("&H80FFFFFF&").unwrap();
    assert_eq!(c.a, 0x7F);
    assert_eq!((c.r, c.g, c.b), (0xFF, 0xFF, 0xFF));
}

#[test]
fn color_decimal_long_integer() {
    // 255 decimal = 0x0000FF -> BGR red, opaque.
    assert_eq!(
        parse_color("255"),
        Some(Rgba {
            r: 0xFF,
            g: 0,
            b: 0,
            a: 255
        })
    );
}

#[test]
fn color_lowercase_and_no_trailing_amp() {
    assert_eq!(parse_color("&h00ff00"), parse_color("&H00FF00&"));
}

#[test]
fn color_rejects_garbage() {
    assert_eq!(parse_color("&HZZ&"), None);
    assert_eq!(parse_color(""), None);
}

#[test]
fn v4plus_style_row_default_format() {
    let row = "Style: Default,Arial,28,&H00FFFFFF,&H000000FF,&H00000000,&H80000000,-1,0,0,0,100,100,0,0,1,2,1,2,10,10,10,1";
    let (name, base) = parse_style_row(row, DEFAULT_V4PLUS_FORMAT, false).unwrap();
    assert_eq!(name, "Default");
    assert_eq!(base.font_name, "Arial");
    assert_eq!(base.font_size, 28.0);
    assert!(base.bold);
    assert!(!base.italic);
    assert_eq!(
        base.primary,
        Rgba {
            r: 0xFF,
            g: 0xFF,
            b: 0xFF,
            a: 255
        }
    );
    // secondary &H000000FF -> red opaque
    assert_eq!(
        base.secondary,
        Rgba {
            r: 0xFF,
            g: 0,
            b: 0,
            a: 255
        }
    );
    // back &H80000000 -> alpha 0x80 -> straight 0x7F
    assert_eq!(base.shadow_color.a, 0x7F);
    assert_eq!(base.border, 2.0); // Outline field (value 2 in the row)
    assert_eq!(base.shadow, 1.0); // Shadow field (value 1 in the row)
    assert_eq!(base.alignment, 2);
    assert_eq!(base.scale_x, 100.0);
    assert_eq!(base.encoding, 1);
}

#[test]
fn style_row_with_explicit_format_mapping() {
    // A reordered Format line must remap fields by name.
    let fmt = parse_format("Format: Name, Bold, Italic, Fontname, Fontsize").unwrap();
    let (name, base) = parse_style_row("Style: Foo,-1,-1,Times,40", &fmt, false).unwrap();
    assert_eq!(name, "Foo");
    assert!(base.bold);
    assert!(base.italic);
    assert_eq!(base.font_name, "Times");
    assert_eq!(base.font_size, 40.0);
}

#[test]
fn style_row_bold_explicit_weight() {
    let fmt = parse_format("Format: Name, Bold").unwrap();
    let (_, base) = parse_style_row("Style: W,700", &fmt, false).unwrap();
    assert!(base.bold);
    assert_eq!(base.weight, Some(700));
    let (_, base2) = parse_style_row("Style: W,400", &fmt, false).unwrap();
    assert!(!base2.bold);
    assert_eq!(base2.weight, Some(400));
}

#[test]
fn ssa_v4_legacy_alignment_and_tertiary() {
    // SSA v4: TertiaryColour is the outline colour; alignment uses the
    // legacy 1..=11 grid (5 = toptitle-left -> numpad 7).
    let fmt = parse_format("Format: Name, TertiaryColour, Alignment").unwrap();
    let (_, base) = parse_style_row("Style: S,&H0000FF00,5", &fmt, true).unwrap();
    // &H0000FF00 -> b=00 g=FF r=00 -> green
    assert_eq!(
        base.outline_color,
        Rgba {
            r: 0,
            g: 0xFF,
            b: 0,
            a: 255
        }
    );
    assert_eq!(base.alignment, 7);
}

#[test]
fn name_with_trailing_commas_in_last_field() {
    // The final field absorbs embedded commas (only the Encoding here).
    let fmt = parse_format("Format: Name, Fontname").unwrap();
    let (name, base) = parse_style_row("Style: Hello,Arial,extra,stuff", &fmt, false).unwrap();
    assert_eq!(name, "Hello");
    // Fontname is the final field -> absorbs the trailing commas.
    assert_eq!(base.font_name, "Arial,extra,stuff");
}

#[test]
fn round_trip_style_row_into_resolution() {
    // A parsed style row feeds resolution: \b resets restore the row's
    // bold, \c resets restore the row's primary colour.
    let row = "Style: D,Arial,20,&H0000FF00,&H000000FF,&H00000000,&H00000000,-1,0,0,0,100,100,0,0,1,2,0,2,10,10,10,1";
    let (_, base) = parse_style_row(row, DEFAULT_V4PLUS_FORMAT, false).unwrap();
    // primary green from the row
    let r = resolve_line("{\\c&H0000FF&}red{\\c}back", &base);
    // first span: overridden to red
    assert_eq!(
        (
            r.spans[0].style.primary.r,
            r.spans[0].style.primary.g,
            r.spans[0].style.primary.b
        ),
        (0xFF, 0, 0)
    );
    // second span: \c reset restores the row's green primary
    assert_eq!(
        (
            r.spans[1].style.primary.r,
            r.spans[1].style.primary.g,
            r.spans[1].style.primary.b
        ),
        (0, 0xFF, 0)
    );
    // bare \b resets restore the row's bold (= true)
    let r2 = resolve_line("{\\b0}x{\\b}y", &base);
    assert!(!r2.spans[0].style.bold);
    assert!(r2.spans[1].style.bold);
}

#[test]
fn missing_values_keep_defaults() {
    // Fewer values than format fields: the rest keep StyleBase defaults.
    let fmt = parse_format("Format: Name, Fontname, Fontsize, Bold").unwrap();
    let (name, base) = parse_style_row("Style: Partial,Arial", &fmt, false).unwrap();
    assert_eq!(name, "Partial");
    assert_eq!(base.font_name, "Arial");
    assert_eq!(base.font_size, 18.0); // default
    assert!(!base.bold); // default
}
