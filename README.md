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

## License

MIT — see [LICENSE](LICENSE).
