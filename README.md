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
  region surfaces as `ttml_region.<id>` carrying the full attribute
  list in canonical spec order (`origin`, `extent`, `padding`,
  `backgroundColor`, `color`, `displayAlign`, `textAlign`, …,
  `itts:forcedDisplay`, `itts:fillLineGap`).
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
