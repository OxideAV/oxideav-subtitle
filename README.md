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

## License

MIT — see [LICENSE](LICENSE).
