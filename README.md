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

### Round-1 deferrals on the Scribe + Raster path

Italic / weight synthesis on runs marked italic + bold currently render
upright/regular (they go through `Shaper::shape_to_paths`, which uses
the upright outline; round 2 will route those through a
`render_text_styled`-equivalent vector path or a paired italic / bold
`Face`). `Segment::Font.family` is ignored — the explicit `FaceChain`
the caller installed is always used. Outline smear isn't replicated on
this path.

The bitmap-font path is unaffected by these and continues to honour
bold / italic / per-run colour as before.

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
  region surfaces as `ttml_region.<id>` carrying the full attribute
  list in canonical spec order (`origin`, `extent`, `padding`,
  `backgroundColor`, `color`, `displayAlign`, `textAlign`, …,
  `itts:forcedDisplay`, `itts:fillLineGap`).
* `<p region="X">` cue-region references ride alongside the cue in
  `ttml_cue_region.<idx>`.
* IR-unmodelled `tts:*` / `itts:*` attributes on `<style>` —
  `displayAlign`, `extent`, `origin`, `padding`, `lineHeight`,
  `opacity`, `textOutline`, `textShadow`, `writingMode`, `wrapOption`,
  `direction`, `rubyAlign`, `shear`, `showBackground`, `visibility`,
  `display`, `disparity`, `fontSelectionStrategy`, `position`,
  `itts:forcedDisplay`, `itts:fillLineGap` — survive as
  `ttml_style_extra.<id>` in canonical order.
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

## License

MIT — see [LICENSE](LICENSE).
