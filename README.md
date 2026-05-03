# oxideav-subtitle

Subtitle codecs + containers for oxideav — SRT, WebVTT, MicroDVD, MPL2,
MPsub, VPlayer, PJS, AQTitle, JACOsub, RealText, SubViewer 1/2, TTML,
SAMI, EBU STL — plus a text-to-RGBA compositor and a
`RenderedSubtitleDecoder` wrapper.

Part of the [oxideav](https://github.com/OxideAV/oxideav-workspace) framework — a
100% pure Rust media transcoding and streaming stack. No C libraries, no FFI
wrappers, no `*-sys` crates.

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

### TrueType path via `oxideav-scribe`

```rust
use oxideav_subtitle::Compositor;
use oxideav_scribe::Face;

let bytes = std::fs::read("DejaVuSans.ttf")?;
let face  = Face::from_ttf_bytes(bytes)?;
let mut comp = Compositor::with_face(640, 480, face);
comp.font_size_px = 24.0;
let buf = comp.render(&cue);
```

Glyphs are anti-aliased and shaped (kerning, GSUB ligatures via the
[`oxideav-scribe`](https://github.com/OxideAV/oxideav-scribe) pipeline)
before being alpha-composited onto the canvas with straight-alpha "over"
from [`oxideav-pixfmt`](https://github.com/OxideAV/oxideav-pixfmt).

### Wrapper decoder

```rust
use oxideav_subtitle::{make_rendered_decoder, make_rendered_decoder_with_face};
// Bitmap-font path:
let video_dec = make_rendered_decoder(srt_decoder, 640, 480);
// Scribe TTF path:
let video_dec = make_rendered_decoder_with_face(srt_decoder, 640, 480, face);
// Or builder form:
let video_dec = RenderedSubtitleDecoder::new(srt_decoder, 640, 480).with_face(face);
```

### Round-1 deferrals on the Scribe TTF path

Round-1 of the Scribe integration delivers crisp anti-aliased glyphs
but intentionally simplifies styling. Enhancements landing in round 2:

* **Italic** — runs marked italic currently render upright. Round 2
  will accept a paired italic `Face`, and/or shear-fake one as a
  fallback.
* **Per-run colour** — the whole cue renders in `default_color`.
  Round 2 will shape one Scribe run per styled segment and composite
  each with its own `Color { rgb }`.
* **Font-fallback chain** — `Segment::Font.family` is ignored; the
  single Compositor face is always used. Round 2 will accept a
  `Vec<Face>` and pick the first one whose `cmap` covers each glyph.
* **Outline** — the bitmap-font path's outline smear isn't replicated.

The bitmap-font path is unaffected by these and continues to honour
bold / italic / per-run colour as before.

## License

MIT — see [LICENSE](LICENSE).
