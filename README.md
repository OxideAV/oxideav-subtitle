# oxideav-subtitle

Subtitle codecs + containers for oxideav — SRT, WebVTT, MicroDVD, MPL2,
MPsub, VPlayer, PJS, AQTitle, JACOsub, RealText, SubViewer 1/2, TTML,
SAMI, EBU STL — plus a text-to-RGBA compositor and a
`RenderedSubtitleDecoder` wrapper.

Part of the [oxideav](https://github.com/OxideAV/oxideav-workspace) framework — a pure-Rust media transcoding and streaming stack. Codec, container, and filter crates are implemented from the spec (no C codec libraries linked or wrapped, no `*-sys` crates). Optional hardware-engine crates (`oxideav-videotoolbox` / `-audiotoolbox` / `-vaapi` / `-vdpau` / `-nvidia` / `-vulkan-video`) bridge to OS APIs via runtime `libloading`; pass `--no-hwaccel` (or omit the `hwaccel` feature) to opt out.

## Usage

```toml
[dependencies]
oxideav-subtitle = "0.0"
```

## Text-to-RGBA rendering

The `Compositor` rasterises a `SubtitleCue` into a straight-alpha RGBA
buffer suitable for plane-compositing onto a video frame. Two
back-ends are supported:

### Bitmap-font path (default)

```rust
use oxideav_subtitle::Compositor;
let comp = Compositor::new(640, 480);
let buf = comp.render(&cue); // Vec<u8>, 640*480*4 bytes
```

Uses the embedded 8×16 bitmap font (Latin-1 only). No external
assets, no TTF parsing — always available.

### TrueType path via `oxideav-scribe` + `oxideav-raster`

Gated behind the default-on `text` cargo feature. Disable via
`default-features = false` to drop the scribe + raster dep tree if you
only need the `BitmapFont` path.

```rust
use oxideav_subtitle::Compositor;
use oxideav_scribe::{Face, FaceChain};

let bytes = std::fs::read("DejaVuSans.ttf")?;
let face  = Face::from_ttf_bytes(bytes)?;
let chain = FaceChain::new(face); // .push_fallback(cjk).push_fallback(emoji) for fallbacks
let mut comp = Compositor::with_face(640, 480, chain);
comp.font_size_px = 24.0;
let buf = comp.render(&cue);
```

Glyphs are anti-aliased and shaped (kerning, GSUB ligatures, face-chain
fallback via the
[`oxideav-scribe`](https://github.com/OxideAV/oxideav-scribe) pipeline)
into vector path nodes, gathered into one `oxideav_core::VectorFrame`,
and rasterised end-to-end by
[`oxideav-raster`](https://github.com/OxideAV/oxideav-raster). The
result is composited onto the canvas with straight-alpha "over" from
[`oxideav-pixfmt`](https://github.com/OxideAV/oxideav-pixfmt). Per-run
colour from `Segment::Color` is honoured.

### Wrapper decoder

```rust
use oxideav_subtitle::{make_rendered_decoder, make_rendered_decoder_with_face};
// Bitmap-font path:
let video_dec = make_rendered_decoder(srt_decoder, 640, 480);
// Scribe + Raster TTF path (requires the `text` feature):
let video_dec = make_rendered_decoder_with_face(srt_decoder, 640, 480, chain);
// Or builder form:
let video_dec = RenderedSubtitleDecoder::new(srt_decoder, 640, 480).with_face(chain);
```

### Limitations on the Scribe + Raster path

Italic / weight synthesis on runs marked italic + bold currently render
upright/regular (they go through `Shaper::shape_to_paths`, which uses
the upright outline). `Segment::Font.family` is ignored — the explicit
`FaceChain` the caller installed is always used. Outline smear isn't
replicated on this path.

The bitmap-font path is unaffected by these and continues to honour
bold / italic / per-run colour as before.

## ASS / SSA Dialogue-text override-tag tokenizer

The `ass_tags` module tokenizes the per-event Dialogue `Text` payload
(file-level `.ass` / `.ssa` parsing lives in the sibling `oxideav-ass`
crate) per the SSA v4 spec's Appendix A "Style override codes" and the
Aegisub override-tag reference, both staged under `docs/subtitles/ass/`:

```rust
use oxideav_subtitle::ass_tags::{tokenize, emit, plain_text};
use oxideav_subtitle::{AssTag, AssToken, WrapStyle};

let toks = tokenize("There is a {\\b1}bold {\\b0}word here\\Nsecond line");
assert_eq!(emit(&toks), "There is a {\\b1}bold {\\b0}word here\\Nsecond line");
let visible = plain_text(&toks, Some(WrapStyle::SmartEven));
```

* `{...}` override blocks split into per-tag items; "several overrides
  within one set of braces" are kept in order.
* The four boolean style flags the IR can model are typed —
  `\b` (`Bold(Option<u32>)`, including the `\b<weight>` 100..900 form),
  `\i` / `\u` / `\s` (`Option<bool>`, with the parameterless
  reset-to-style form as `None`). The exact-prefix + digits-only match
  means `\bord`, `\be`, `\blur`, `\shad`, and `\iclip` can't be
  mistaken for flag forms (`\bord` / `\shad` are typed by the
  border / shadow family below, not as `\b` / `\s`).
* The colour / alpha family is typed. `\c` / `\1c`–`\4c&H<bbggrr>&`
  parse to `AssTag::Color { target, short, hex }` — `target` is the
  `AssColorTarget` component (primary / secondary fill, border,
  shadow), `short` keeps the `\c`-abbreviation-of-`\1c` spelling
  distinct so emit stays byte-stable, and `hex` is the verbatim digit
  run ("Leading zeroes are not required" — `&HFF&` is pure red since
  codes are hexadecimal in **Blue Green Red** order).
  `\alpha` / `\1a`–`\4a&H<aa>&` parse to `AssTag::Alpha` the same way
  (`target: None` = all components; `00` opaque, `FF` fully
  transparent). The bare `\c` / `\alpha` forms are the
  reset-to-style shape (`hex: None`). `decode_bgr_hex` /
  `decode_alpha_hex` turn a digit run into `(r, g, b)` / `u8`.
  Off-shape parameters — a missing closing `&`, an over-long digit
  run, `\5c`, the `\clip` prefix cousin — stay verbatim untyped.
* The alignment pair is typed. `\an1`–`\an9` parse to
  `AssTag::AlignNumpad` ("numpad" layout: 1/2/3 bottom, 4/5/6 middle,
  7/8/9 top); legacy `\a` parses to `AssTag::AlignLegacy` over the
  documented value set (1–3, +4 "Toptitle" → 5–7, +8 "Midtitle" →
  9–11, and the explicit `\a0` reset). `legacy_align_to_numpad`
  converts a legacy value to its numpad equivalent for renderers that
  only speak `\an`. Parameterless `\an` / `\a` are the reset-to-style
  shape; `\a4` / `\a8` / `\an0` stay verbatim untyped.
* The karaoke family is typed: `\k` (instant fill switch), `\K` and
  `\kf` (left-to-right sweep — identical effects whose distinct
  spellings are preserved via `AssKaraokeKind`), and `\ko` (outline)
  parse to `AssTag::Karaoke { kind, centisec }`, durations in
  hundredths of seconds. The undocumented `\kt` and bare no-duration
  forms stay verbatim.
* The three line-positioning functions are typed: `\pos(x,y)`,
  `\org(x,y)`, and both `\move` arities (`x1,y1,x2,y2` plus the
  optional `t1,t2` millisecond animation window, kept distinct from
  the 4-argument spelling even when `0,0`). Coordinates are integers
  in script-resolution pixels. Only canonically-spelled integers are
  typed — embedded spaces, leading zeroes, a `+` sign, `-0`, or an
  off-arity argument list keep the whole tag verbatim so emit stays
  byte-stable.
* The font-metric / rotation family is typed: `\fn` (`FontName`,
  arbitrary verbatim string — font names carry spaces, e.g.
  `\fnCourier New`), `\fs` (`FontSize`), `\fscx` / `\fscy`
  (`FontScale { x_axis }`), `\fsp` (`FontSpacing`), `\fe`
  (`FontEncoding`), and `\frx` / `\fry` / `\frz` plus the bare `\fr`
  ("defaults to `\frz`", kept distinct via `bare` so emit is
  byte-stable) as `Rotation { axis, bare }`. The numeric runs are kept
  verbatim as strings — sizes / scales / spacings / angles are commonly
  fractional or negative (`\fs28.5`, `\fscx200`, `\fsp-2`, `\frz-30.5`)
  — and `decode_decimal` turns one into an `f64`. Only a canonical
  decimal (optional `-`, digits, at most one interior `.`) is typed; a
  `+` sign, a bare or trailing `.`, an embedded space, a `%`, or a
  digit-grouping cousin keeps the whole tag verbatim. The exact-prefix
  match means `\fad` / `\fade` / `\be` / `\blur` can't be mistaken for
  `\fs*` / `\fr*` forms, and `\fsc*` / `\fsp` are matched ahead of the
  shorter `\fs`. Parameterless `\fn` … `\fr` are the reset-to-style
  shape (`None`).
* The border / shadow family is typed: `\bord` / `\xbord` / `\ybord`
  parse to `AssTag::Border { axis, size }` ("Change the width of the
  border around the text … Set the size to 0 to disable the border
  entirely", `\xbord` / `\ybord` "set the border size in X and Y
  direction separately") and `\shad` / `\xshad` / `\yshad` to
  `AssTag::Shadow { axis, depth }` ("Set the distance from the text to
  position the shadow"), both over the new `AssBorderAxis`
  (`Both` / `X` / `Y`, re-exported at the crate root). Widths and depths
  are kept verbatim as strings — they "can have decimal places"
  (`\bord3.7`); `decode_decimal` turns one into an `f64`. The spec bars
  a negative border ("Border width cannot be negative") and a negative
  combined `\shad` ("distance can not be negative with this tag"), so a
  `-`-signed value there stays verbatim `AssTag::Other`; the per-axis
  `\xshad` / `\yshad` accept a negative ("unlike `\shad`, you can set
  the distance negative … to position the shadow to the top or left").
  Exact-prefix matching keeps the `\b` / `\s` toggles and the `\be`
  blur cousin distinct, and the axis-prefixed `\xbord` … `\yshad` are
  checked ahead of the combined forms. Parameterless `\bord` … `\yshad`
  are the reset-to-style shape (`None`).
* The edge-blur family is typed: `\be` and `\blur` parse to
  `AssTag::Blur { kind, strength }` over the new `AssBlurKind`
  (`Edge` / `Gaussian`, re-exported at the crate root). A `\be` strength
  "is the number of times to apply the regular effect" and "must be an
  integer number", so it accepts only a canonical non-negative integer
  run; `\blur` "uses a more advanced algorithm" and "Unlike `\be`, the
  strength can be non-integer here", so it accepts a non-negative decimal
  (`decode_decimal` turns one into an `f64`). Neither strength is
  meaningfully negative — a `-`-signed value, a decimal `\be`, a `+`
  sign, or a leading zero keeps the whole tag verbatim — and `\blur` is
  matched ahead of `\be`, both after the `\bord` family so `\bord` is
  never mistaken for a `\b` toggle. Parameterless `\be` / `\blur` are the
  reset-to-style shape (`None`).
* The clip family is typed: `\clip` / `\iclip` parse to
  `AssTag::Clip { inverse, shape }` over the new `AssClipShape`
  (re-exported at the crate root). The rectangle shape
  `\clip(x1,y1,x2,y2)` keeps "only the part of the line that is inside
  the rectangle" — its four coordinates "must be integers" so a
  non-integer coordinate keeps the whole tag verbatim — and `\iclip`
  "has the opposite effect" (`inverse: true`, hide inside the box). The
  vector-drawing shape `\clip(<drawing>)` / `\clip(<scale>,<drawing>)`
  clips against a `\p`-style path; the optional integer `scale` ("If the
  scale is not specified it is assumed to be 1") is carried distinctly
  and the drawing-command run rides through verbatim, so emit stays
  byte-stable. The argument list is disambiguated by its top-level comma
  count (four integers → rectangle; one argument → unscaled drawing; an
  integer scale + drawing → scaled), and a drawing arm requires an
  actual command letter so a bare two-coordinate list (`\clip(50,50)`)
  stays an untyped `AssTag::Other`. Off-shape arities, a non-integer
  scale, an empty argument list, or trailing text after the close paren
  also stay verbatim. The exact-prefix paren match keeps `\iclip` /
  `\clip` distinct from the `\i` toggle and `\c` colour.
* The fade family is typed: `\fad` / `\fade` parse to
  `AssTag::Fade(spec)` over the new `AssFadeSpec` (re-exported at the
  crate root). The simple `\fad(<fadein>,<fadeout>)` "produces a
  fade-in and fade-out effect. The fadein and fadeout times are given
  in milliseconds" (either may be 0 "to not have any fade effect on
  that end") and types to `AssFadeSpec::Simple { fadein, fadeout }`
  from two non-negative integers. The complex
  `\fade(<a1>,<a2>,<a3>,<t1>,<t2>,<t3>,<t4>)` performs "a five-part
  fade using three alpha values … and four times" — the alphas "are
  given in decimal and are between 0 and 255" (`u8`) and the times "are
  given in milliseconds after the start of the line", all seven
  "required" — and types to `AssFadeSpec::Complex { a1, a2, a3, t1, t2,
  t3, t4 }`. Each value is canonically spelled (`\fad`'s two ms, `\fade`'s
  three 0–255 alphas + four ms), so a wrong arity, a signed or
  non-integer value, an alpha above 255, or trailing text after the
  close paren keeps the whole tag verbatim `AssTag::Other`. The `\fade`
  arm is tried ahead of `\fad`, and the exact-prefix paren match keeps
  both distinct from the `\f*` font-metric family.
* The animated-transform tag is typed: `\t(...)` parses to
  `AssTag::Transform { t1, t2, accel, modifiers }` across the four
  documented arities — `\t(<modifiers>)`, `\t(<accel>,<modifiers>)`,
  `\t(<t1>,<t2>,<modifiers>)`, and `\t(<t1>,<t2>,<accel>,<modifiers>)`.
  The leading numeric arguments are separated from the *style modifiers*
  ("other override tags as specified in this reference") at the first
  top-level backslash, and the modifiers run is parsed recursively into
  nested `AssTag` values that re-emit through the same per-tag emitter,
  so a `\t(0,1000,\fscx200\fscy200)` round-trips byte-stably with both
  `\fscx` / `\fscy` typed as `FontScale`. `t1` / `t2` are non-negative
  integer milliseconds "relative to the start time of the line" (always
  present or absent together); `accel` is a non-negative decimal kept
  verbatim ("can be non-integer", `1` is linear) — `decode_decimal`
  turns it into an `f64`. Off-shape spellings — a `\t()` with no
  modifiers, a leading-argument count outside 1–3, a signed or
  non-integer time, a negative or non-canonical accel, trailing text
  after the close paren — stay an untyped `AssTag::Other`. The nested
  rectangle `\clip` (the only animatable clip form per the reference)
  rides through as a recursively-parsed `Clip` modifier.
* The drawing-mode family is typed: `\p<0/1/..>` parses to
  `AssTag::Drawing(u32)` ("Setting this tag to 1 or above enables drawing
  mode … the value … will be interpreted as the scale, in `2^(value-1)`
  mode"; `\p0` disables it) and `\pbo<y>` to `AssTag::BaselineOffset(i32)`
  (the "Y offset to all coordinates", which may be negative, e.g.
  `\pbo-50`). `\pbo` is matched ahead of `\p`, and `\pos` / `\pos(...)`
  are unaffected. Only canonical integer levels / offsets type — a `+`
  sign, a non-integer, a negative level, or the bare `\p` / `\pbo`
  (no documented value) keeps the whole tag verbatim `AssTag::Other`.
  `drawing_scale_divisor` maps a level to its coordinate divisor
  (`\p2` → 2, `\p4` → 8).
* Every other tag is preserved verbatim as `AssTag::Other`, and non-tag
  text inside a block becomes `AssTag::Comment`, so
  `emit(&tokenize(s)) == s` byte-for-byte on every input (unterminated
  `{` and unrecognised backslash sequences stay literal text).
* The mid-text escapes `\n` (soft break), `\N` (hard break), and `\h`
  (non-breaking hard space) are their own tokens; `plain_text` maps
  them against the script's `WrapStyle` (from the `[Script Info]`
  accessor) — `\n` breaks only in wrap mode 2 and is a regular space
  otherwise, `\N` always breaks, `\h` becomes U+00A0.

With the drawing-mode `\p` / `\pbo` toggles now typed, the override-tag
tokenizer covers every tag in the Aegisub reference that carries
structured arguments. The `\p` vector-path command stream itself is
decoded by a dedicated parser:

```rust
use oxideav_subtitle::ass_tags::{parse_drawing, emit_drawing};
use oxideav_subtitle::DrawCmd;

// The spec's "Square" example: m 0 0 l 100 0 100 100 0 100
let cmds = parse_drawing("m 0 0 l 100 0 100 100 0 100").unwrap();
assert_eq!(cmds[0], DrawCmd::Move(0.0, 0.0));
assert_eq!(emit_drawing(&cmds), "m 0 0 l 100 0 100 100 0 100");
```

`parse_drawing` / `emit_drawing` decode the verbatim command run that
appears between `\p<level>` and `\p0` (and inside the vector-overload
`\clip(<drawing>)` form) into a structured `DrawCmd` list — `m` move,
`n` move-no-close, `l` line (one or more segments), `b` cubic Bézier
(three control points per curve), `s` cubic b-spline (≥3 points), `p`
spline-extend, `c` close-spline. Coordinates parse as decimals (commonly
fractional under a `\p2`+ subpixel scale, possibly negative); the spec's
square / rounded-square / circle examples decode exactly, and a
malformed stream returns `None`.

## ASS / SSA `[Script Info]` typed accessor

`script_info(&track)` reads the SSA `[Script Info]` keys an ASS-source
track carries in `SubtitleTrack::metadata` into a typed
`AssScriptInfo` view — `PlayResX` / `PlayResY` / `PlayDepth` as `u32`,
`Timer` as `f64` percent, `WrapStyle` and `Collisions` as enums,
`ScaledBorderAndShadow` as `bool` — leaving unknown keys untouched.

## WebVTT signature and timestamp strictness

The WebVTT parser enforces the §4.1 file-signature production and the
§3.3 timestamp production literally rather than accepting near-misses.

* The file signature is the byte string `WEBVTT` followed by either a
  line terminator (no trailing text) or a single U+0020 SPACE / U+0009
  TAB and then the optional header trailing text. A signature like
  `WEBVTTHEADER` (no separator) is rejected with a missing-signature
  error instead of silently treating `HEADER` as trailing metadata.
* Cue timings are accepted only in the two §3.3 canonical shapes
  `MM:SS.fff` and `HH:MM:SS.fff`. Minutes and seconds must be exactly
  two ASCII digits in the range `0..=59`; the fractional component must
  be exactly three ASCII digits separated by a `.`; the optional hours
  component, when present, must be two or more ASCII digits.
  Non-canonical forms (`0:00:01.000`, `00:00:1.000`, `00:00:01`,
  `00:00:01.00`, `00:60:01.000`, `00:00:60.000`, …) make the cue block
  fail to recognise a timing line, and the cue is dropped rather than
  silently turning a malformed offset into a wrong-but-plausible value.

A UTF-8 BOM on the file's first byte still works because the shared
`encoding::decode_subtitle_text` helper strips it before the parser
sees the signature line.

## WebVTT §5 default cue-component classes

The WebVTT §5.1 table reserves eight foreground class names (`white`,
`lime`, `cyan`, `red`, `yellow`, `magenta`, `blue`, `black`) and §5.2
mirrors them as eight `bg_*` background class names, each carrying a
fully-opaque `rgba(R,G,B,1)` presentational-hint colour. The parser
keeps `Segment::Class::name` as the verbatim dot-chain
(`"yellow.bg_blue.magenta.bg_black"`) so existing call-sites continue to
round-trip authored chains byte-stable; a downstream renderer that needs
the resolved colours calls

```rust
use oxideav_subtitle::webvtt::{default_class_color, resolve_default_class_colors,
                               DefaultClassKind};

// Single-class lookup — returns the kind + RGBA, or None for names
// outside §5.1 / §5.2.
let (kind, rgba) = default_class_color("yellow").unwrap();
assert_eq!(kind, DefaultClassKind::Foreground);
assert_eq!(rgba, (255, 255, 0, 0xff));

// Whole-chain resolution honours the §5.2 cascade rule — within each
// presentational target, the last matching class wins. The spec
// worked example `<c.yellow.bg_blue.magenta.bg_black>` resolves to
// "magenta text on a black background":
let (fg, bg) =
    resolve_default_class_colors("yellow.bg_blue.magenta.bg_black");
assert_eq!(fg, Some((255, 0, 255, 0xff)));
assert_eq!(bg, Some((0,   0,   0, 0xff)));
```

Name matching is **case-sensitive** per spec — `Yellow`, `YELLOW`, and
`BG_BLUE` are unrecognised author class names that fall through to
`::cue(.Yellow)` STYLE rules instead of the default colour. Mixed
chains where author classes (`warning`, `chapter-2`) sit alongside §5
defaults are tolerated: the unrecognised classes are skipped and the
§5 ones still resolve. An author-only chain returns `(None, None)` so
the caller knows to defer entirely to author-supplied STYLE rules. The
resolver is a pure presentational-hint computation — it never overrides
an explicit `::cue(...)` STYLE rule the author defined for the same
name (e.g. `::cue(.yellow) { color: cyan }` per the §5 closing note);
the renderer composes the two layers in its own cascade.

## WebVTT cue settings

The WebVTT timing line's cue settings (WebVTT §3.5) are parsed into the
unified `CuePosition` where they fit — `position`, `line`, `size`, and
`align`. The settings the IR has no field for are preserved verbatim
through a parse → write round-trip via per-cue `vtt_cue_extra.<idx>`
track metadata:

* `vertical:rl` / `vertical:lr` — vertical writing direction.
* the `,start` / `,center` / `,end` alignment suffix on `line`.
* the `,line-left` / `,center` / `,line-right` suffix on `position`.
* a `region:<id>` reference.

A `line` offset given as a bare (possibly negative) line number is kept
distinct from a percentage offset, so `line:-1` survives without
acquiring a spurious `%`. The structured `CuePosition` keeps carrying
the numeric offset / size / align for downstream consumers either way.
The single-cue packet codec path (`cue_to_bytes` / `bytes_to_cue`) has
no track context, so these extras are a track-level write feature.

Per the WebVTT §6.3 cue-settings algorithm, individual settings whose
value isn't a valid WebVTT percentage (§4.4: digits, optional
`.digits`, then `%`, numerically `0..=100`) are dropped while the cue
itself still parses. That covers `position`, `size`, and the
percentage variant of `line`; the `line` line-number variant accepts
only the spec production `[-]?digits[.digits]?`. An unrecognised
`,<align>` suffix on `position` or `line` drops the whole setting
rather than silently keeping the numeric part.

## WebVTT cue payload inline markup

The §3.5 cue components are parsed into the structured `Segment` tree and
re-emitted byte-stably:

* `<b>` / `<i>` / `<u>` — bold / italic / underline.
* `<v Speaker>...</v>` — voice span; the annotation rides on
  `Segment::Voice::name` and survives the round-trip. `<v>` with no
  annotation is also tolerated and re-emits without a spurious space.
* `<c.class.chain>...</c>` — class span; the full dot-chain (e.g.
  `<c.foo.bar.baz>`) is kept as one `Segment::Class::name`. Bare `<c>`
  (no annotation) round-trips as `<c>` rather than the invalid `<c.>`.
* `<lang xx-YY>...</lang>` — language span; the BCP 47 annotation
  (including subtag chains like `zh-Hant-HK`) is preserved verbatim.
  Nested `<lang>` spans round-trip through Raw-bracket flattening.
* `<ruby>base<rt>annotation</rt></ruby>` — ruby spans, including
  multiple base+rt pairs in a single `<ruby>`. Per §3.5 the final
  `</rt>` may be omitted; the parser handles the implicit close and
  the writer normalises to explicit `</rt>`.
* `<00:00:01.500>` — inline cue timestamp.
* Any other tag falls through to `Segment::Raw` so a re-emit to WebVTT
  is faithful.

A latent UTF-8 bug in the inline text accumulator was fixed in the same
change: previously a multi-byte codepoint (`à`, `漢`, etc.) in cue text
adjacent to a tag boundary was sliced byte-by-byte and re-emitted as
mojibake; the accumulator now advances by full codepoints.

## WebVTT cue-text character references

The WebVTT §6.4 cue-text tokenizer transitions to the "HTML character
reference in data state" on `&` in the cue body. The parser now decodes
those references in the three shapes the spec admits:

* Decimal — `&#NNNN;` (one or more ASCII digits, semicolon-terminated).
* Hex — `&#xNNNN;` / `&#XNNNN;` (one or more ASCII hex digits).
* Named — the eight HTML5.1 names subtitle authoring tools emit in
  practice: `&amp;`, `&lt;`, `&gt;`, `&nbsp;`, `&lrm;`, `&rlm;`,
  `&quot;`, `&apos;`. The §4.2.5 examples that the WebVTT spec itself
  cites by name (`&lrm;` for U+200E, the `&#x2068;` / `&#x2069;` bidi
  isolate pair) decode to their target codepoints rather than
  passing through as literal byte sequences.

Per HTML5.1, numeric references that name U+0000 or a surrogate-range
codepoint (U+D800 .. U+DFFF) map to U+FFFD REPLACEMENT CHARACTER, as do
out-of-range scalars above U+10FFFF. A malformed reference — no
terminating `;`, an unknown name, missing digits — falls back to the
literal `&` byte per the §6.4 "If nothing is returned, append a U+0026
AMPERSAND character" branch, so a stray `& Co.` in cue text no longer
parses as the start of an entity.

The reciprocal writer side encodes any `&`, `<`, or `>` that appears
inside a `Segment::Text` as `&amp;` / `&lt;` / `&gt;`. The three bytes
are the §4.2.2 reserved tokens that re-enter the tokenizer, so a parse
→ write → parse round-trip on text containing them reproduces the
original user-visible string (`Tom & Jerry <3 hearts > rocks` survives
through `&amp;` / `&lt;` / `&gt;` on the wire).

## WebVTT STYLE blocks

The WebVTT `STYLE` block (`::cue(...) { … }`) parses both the selector and
the eleven CSS properties WebVTT §8.2.1 lists as applying to the `::cue`
pseudo-element. Selectors are recognised in all five spec forms:

* `::cue` — surfaces as a style named `::cue`.
* `::cue(.class)` / `::cue(.a.b.c)` — historical convention; the dot chain
  becomes the style name (`"a.b.c"`) so `track.style("a.b.c")` keeps
  working.
* `::cue(#id)` — surfaces as a style named `#id`.
* `::cue(<elem>)` (e.g. `::cue(b)`, `::cue(i)`, `::cue(c)`, `::cue(v)`,
  `::cue(lang)`, `::cue(ruby)`, `::cue(rt)`) — wrapped as `::cue(elem)` so
  it can't collide with a class named the same letter.
* Anything more exotic (compound / attribute / `:past`/`:future`) is kept
  verbatim as `::cue(<raw>)`.

Properties with a `SubtitleStyle` field (`color`, `background-color`,
`font-family`, `font-size`, `font-weight`, `font-style`,
`text-decoration`) populate those fields. The seven §8.2.1 properties
with no IR home — `opacity`, `visibility`, `text-shadow`, `outline`,
`white-space`, `text-combine-upright`, `ruby-position`, `line-height` —
ride a per-style `vtt_style.<name>.<property>` track-metadata channel in
canonical spec order, mirroring the proven `vtt_region.<id>` /
`ttml_style_extra.<id>` pattern. The synthesised (no-extradata) writer
reconstructs the original `::cue(...)` selector and re-emits the extras
deterministically, so a parse → write → parse cycle is byte-stable for
both the selector and the full property set. Properties §8.2.1 does not
list (e.g. `cursor`, `display`) are silently dropped per the spec's
"other properties set on the pseudo-element must be ignored" clause.

## WebVTT REGION blocks

`REGION` definition blocks (WebVTT §4.3) are parsed for all five region
settings: `width`, `lines`, `regionanchor`, `viewportanchor`, and
`scroll`. Names are matched **case-sensitively** and each value is
validated per the §6.2 algorithm — percentages must carry a `%` and lie
in `0..=100`, `lines` is ASCII digits only, the two anchors are
`<pct>,<pct>` tuples, and `scroll` must be exactly `up`; malformed
values are dropped.

The region surfaces in `track.styles` as a `region:<id>` style (with
`width` mirrored into `margin_r` as a rough integer hint). Because the
unified `SubtitleStyle` has no fields for the geometry settings, the
full settings list is captured verbatim — re-serialised in canonical
spec order — in a per-region `vtt_region.<id>` track-metadata entry.
When a track carries verbatim parse extradata the original REGION block
round-trips byte-for-byte; when the track was built programmatically (no
extradata) the writer reconstructs a complete REGION block from the
style + `vtt_region.<id>` metadata, so all five settings survive the
synthesised write path too.

## WebVTT cue identifiers

WebVTT §3.4 lets a cue carry an optional identifier on the line immediately
before the cue timings line. The IR `SubtitleCue` has no `id` field, so the
parser captures every per-cue identifier into a `vtt_cue_id.<idx>`
track-metadata entry, mirroring the proven `vtt_cue_extra.<idx>` channel.
The synthesised (no-extradata) writer prepends the captured identifier on
its own line before the timing line, so a parse → write → parse cycle is
byte-stable for cue ids — including the spec's two interop shapes:

* Textual ids (`intro`, `chapter-2`, `warn`) used by the WebVTT §8.2.1
  `::cue(#id)` selector — already worked at the STYLE end; now the
  identifier also re-emits on the cue itself.
* Numeric ids (`1`, `42`, …) carried over from SRT-style authoring tools
  that recycle the cue index. Per §3.4 any sequence that doesn't contain
  `-->` qualifies, so a bare digit is preserved verbatim rather than
  mistaken for part of the timing line.

The id slot is per-cue, so a track with a mix of identified and
unidentified cues round-trips cue-for-cue. When a NOTE comment block sits
between two identified cues the writer interleaves both: NOTE first
(against its own `vtt_note_pos.<idx>` slot), then a blank separator, then
the next cue's identifier, then the timing line. Empty identifier strings
are skipped at write time so a stray blank line cannot sneak in. The
single-cue codec path (`cue_to_bytes` / `bytes_to_cue`) has no track
context and therefore neither emits nor consumes the id field — that path
remains a pure timing+body wire shape.

## WebVTT NOTE comment blocks

WebVTT §4.1 comment blocks (`NOTE …`) round-trip end-to-end. The parser
captures each block verbatim into per-block `vtt_note.<idx>` track
metadata together with a `vtt_note_pos.<idx>` recording the cue index
the block preceded (`0` before the first cue, `N` after the last cue,
`k` between cue `k-1` and `k`). The verbatim-extradata writer keeps each
block in its original byte position because the captured block is also
appended to the saved extradata; the synthesised (no-extradata) writer
reconstructs the same interleaving from the metadata. Both single-line
(`NOTE foo`), bare-token (`NOTE` alone on the line followed by body
lines), and multi-line bodies survive. Comment-block detection is
case-sensitive per spec — only first-line tokens of exactly `NOTE`,
`NOTE ` (space), or `NOTE\t` (tab) qualify, so a cue id like `Notebook`
no longer accidentally lights up the comment-block code path.

## TTML / IMSC 1.2

The TTML parser handles core TTML v1, TTML v2, and the IMSC 1.2 profile.
What the unified IR can model maps directly:

* `tts:textAlign` on a `<style>` lands on `SubtitleStyle.align` (the
  `justify` value has no IR home and falls through to the extras path
  below).
* `tts:color` / `tts:backgroundColor` / `tts:fontFamily` /
  `tts:fontSize` / `tts:fontWeight` / `tts:fontStyle` /
  `tts:textDecoration` continue to populate `SubtitleStyle` fields.

The IMSC1 features that don't fit existing IR fields are captured as
track-level metadata so a parse → write round-trip is byte-faithful:

* `<head><layout><region xml:id="X" tts:.../></layout></head>` — every
  region surfaces as `ttml_region.<id>` carrying the full TTML2 §10.2
  styling-attribute vocabulary in canonical spec order
  (§10.2.2 `tts:backgroundClip` … §10.2.52 `tts:zIndex`), followed by
  the `style` style-reference attribute and the IMSC `itts:forcedDisplay`
  / `itts:fillLineGap` extension attributes.
* `<p region="X">` cue-region references ride alongside the cue in
  `ttml_cue_region.<idx>`.
* Inline `tts:*` styling attributes on `<p>` (TTML2 §8.1.5 — "An author
  may associate a set of style properties with a `p` element by means
  of either the `style` attribute or inline style attributes or a
  combination thereof"). IR-modelled attrs (`tts:color`,
  `tts:fontFamily`, `tts:fontSize`, `tts:fontWeight`, `tts:fontStyle`,
  `tts:textDecoration`) wrap the cue's content with the equivalent
  `Segment::Bold` / `Italic` / `Underline` / `Strike` / `Color` /
  `Font` segments. IR-unmodelled inline attrs (the same
  `displayAlign` / `lineHeight` / `opacity` / `textShadow` / … list as
  `<style>` extras above, plus `tts:textAlign` in any value) ride a
  per-cue `ttml_p_extra.<idx>` metadata channel in canonical spec
  order. A parse → write → parse cycle is byte-stable for the inline-
  styled `<p>`; the `xmlns:itts` namespace binding is regrown on `<tt>`
  whenever a `ttml_p_extra` carries an IMSC1 `itts:*` attribute.
* IR-unmodelled `tts:*` / `itts:*` attributes on `<style>` — the full
  TTML2 §10.2 styling-attribute vocabulary minus the seven
  `SubtitleStyle`-modelled names, i.e. `tts:backgroundClip` /
  `backgroundExtent` / `backgroundImage` / `backgroundOrigin` /
  `backgroundPosition` / `backgroundRepeat`, `border`, `bpd` / `ipd`,
  `direction`, `disparity`, `display`, `displayAlign`, `extent`,
  `fontKerning`, `fontSelectionStrategy`, `fontShear`, `fontVariant`,
  `letterSpacing`, `lineHeight`, `lineShear`, `luminanceGain`,
  `opacity`, `origin`, `overflow`, `padding`, `position`, `ruby` /
  `rubyAlign` / `rubyPosition` / `rubyReserve`, `shear`,
  `showBackground`, `textCombine`, `textEmphasis`, `textOrientation`,
  `textOutline`, `textShadow`, `unicodeBidi`, `visibility`,
  `wrapOption`, `writingMode`, `zIndex`, plus `tts:textAlign` in its
  `justify` value and `itts:forcedDisplay` / `itts:fillLineGap` —
  survive as `ttml_style_extra.<id>` in canonical §10.2 order.
* `<tt>` parameter attributes — `ttp:frameRate`, `ttp:tickRate`,
  `ttp:timeBase`, `ttp:profile`, `ttp:cellResolution`,
  `ttp:frameRateMultiplier`, `ttp:displayAspectRatio`,
  `ttp:contentProfiles`, plus the IMSC1 extension parameters
  `ittp:aspectRatio`, `ittp:activeArea`,
  `ittp:progressivelyDecodable` — round-trip via
  `ttml_param.<name>`. The writer rebuilds the `xmlns:ttp` /
  `xmlns:ittp` / `xmlns:itts` declarations on `<tt>` only when the
  corresponding namespace is in use.

Timing previously dropped on the floor now decodes when the document
supplies the matching `<tt>` parameter:

* `HH:MM:SS:FF` clock-time frames against `ttp:frameRate`
  (e.g. `00:00:01:05` at 25 fps = 1.2 s, instead of 1.0 s).
* `<n>f` offset-time frames against `ttp:frameRate`.
* `<n>t` offset-time ticks against `ttp:tickRate`.

Without the matching parameter on `<tt>`, the frame / tick component is
silently dropped (legacy behaviour preserved for back-compat).

### TTML2 §12.2.4 timeContainer (par / seq) timing

`<body>` / `<div>` / `<p>` form nested time containers. The default
container is **parallel** (`par`): every child's `begin` is resolved
relative to the container's begin point, so siblings overlap. A
`timeContainer="seq"` container is **sequential**: each child's interval
is resolved relative to the *end* of its preceding sibling (the
container begin for the first child), chaining cues end-to-begin. Each
container is an independent time base, so a `seq` `<div>` nested inside a
`par` `<body>` chains its own children while being positioned by the
outer `par` rules.

* A `begin` on `<body>` shifts the whole document's time base.
* A `dur` on a `<div>` / `<body>` container fixes its interval span
  (§12.2.2) regardless of child content, which advances the next
  sibling in a `seq` parent.
* `end` on a `<p>` is resolved against the same syncbase as its `begin`.

Resolved cue times are stored absolute, so a re-emit writes a plain
`par` body and re-parsing reproduces the same intervals.

### TTML2 §12.2.4 timed inline `<span>` reveal

A `<span begin="…">` inside a `<p>` is a timed span: its content
becomes visible at a cue-relative time. The parser surfaces this as a
leading `Segment::Timestamp` progressive-reveal marker carrying the
absolute reveal time — the same marker the WebVTT inline
cue-timestamp (`<00:00:01.500>`) path produces, so a renderer staggers
both uniformly. Nested timed spans sync against their outer span's
begin. The writer regroups a `Timestamp` plus the run that follows it
into a timed `<span begin="HH:MM:SS.mmm">`, so a parse → write → parse
cycle preserves every reveal offset. Untimed styled spans emit no
marker.

### TTML2 §8.2.10 `xml:space` whitespace handling

A `<p>` cue's inline text is normalised per its resolved `xml:space`
mode rather than carried with the authored line-formatting whitespace.

* **`default` (collapse)** — the initial value when no `xml:space` is
  present (§8.1.1: "If no `xml:space` attribute is specified upon the
  `tt` element, then it must be considered as if the attribute had been
  specified with a value of `default`"). Authored linefeeds are treated
  as spaces (`linefeed-treatment="treat-as-space"`), a horizontal tab
  counts as a single space, runs of whitespace collapse to one space
  (`white-space-collapse="true"`), and whitespace that surrounds a cue
  edge or a `<br/>` line-break boundary is dropped
  (`white-space-treatment="ignore-if-surrounding-linefeed"` +
  `suppress-at-line-break="auto"`). So a multi-line indented
  `<p>\n  Hello   there\n  wide world\n</p>` parses to
  `"Hello there wide world"` instead of a segment run carrying the
  inter-tag indentation.
* **`preserve`** — keeps the text exactly as authored.

The mode is inherited from the nearest ancestor (`tt` / `body` / `div` /
`p` / `span`) that specifies it. A `preserve` `<p>` containing a
`<span xml:space="default">` collapses only the span, and a `default`
`<p>` containing a `<span xml:space="preserve">` keeps only that span
verbatim; the collapse threads a single boundary state across the whole
cue so a trailing space before a span and a leading space after it
collapse to one. A cue captured in `preserve` mode rides a per-cue
`ttml_cue_xml_space.<idx>` track-metadata entry, and the writer re-emits
`xml:space="preserve"` on the `<p>` so the verbatim text survives a
parse → write → parse round-trip.

## Input encoding tolerance

Every text-subtitle parser in this crate routes its raw bytes through
the shared `encoding::decode_subtitle_text` helper, which transparently
accepts:

* **UTF-8** (the canonical encoding for every format), with or without
  a leading `EF BB BF` BOM.
* **UTF-16 LE with BOM** (`FF FE …`), commonly emitted by YouTube's
  SRT export and various Windows authoring tools.
* **UTF-16 BE with BOM** (`FE FF …`).

Line endings are normalised to LF before parsing, so files saved with
DOS (`\r\n`), Unix (`\n`), or **classic Mac OS** (`\r`-only) line
terminators are all handled identically. WebVTT §4 explicitly lists all
three as valid line terminators; the legacy formats (SRT, MicroDVD,
MPL2, …) have no formal spec and the consensus interop behaviour is to
accept the same matrix.

Invalid byte sequences in any decode path are replaced with U+FFFD —
we never reject a file because a single byte was malformed.

Once decoded to UTF-8, the inline-markup tokenisers in every text parser
advance one full codepoint at a time. An earlier byte-at-a-time
accumulator in the SubRip, MicroDVD, and RealText parsers split a
multi-byte codepoint (`é`, `漢`, …) adjacent to a tag boundary into its
Latin-1 continuation bytes, re-emitting it as mojibake; that path now
matches the WebVTT tokeniser and round-trips such text byte-for-byte.

## SRT structural tolerance

SRT in the wild routinely diverges from the on-paper
`index → timing → text → blank` template. The parser absorbs three
recoverable shapes without losing cues:

* **Leading preamble** — junk lines or a PEM-style armoured envelope
  before the first cue, with or without an intervening blank.
* **Duplicate-index rows** — `N\nN\n<timing>` patterns from buggy
  re-numbering templates. The second copy is absorbed into the
  index-line slot rather than killing the cue.
* **Whitespace-only continuation lines** inside a cue body, e.g.
  `"A\n   \nB"`. The body terminator triggers only on a truly empty
  line or on a new timing line — whitespace-only lines remain body
  content, and two cues with no intervening blank are still split.

Combined with the encoding tolerance above, the parse is
forward-progress-preserving: any cue with a parseable timing line is
recovered even if surrounding rows are malformed.

## License

MIT — see [LICENSE](LICENSE).
