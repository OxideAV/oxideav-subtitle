# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0](https://github.com/OxideAV/oxideav-subtitle/compare/v0.0.5...v0.1.0) - 2026-05-03

### Other

- promote to 0.1

## [0.0.5](https://github.com/OxideAV/oxideav-subtitle/compare/v0.0.4...v0.0.5) - 2026-05-03

### Other

- replace never-match regex with semver_check = false
- drop nested [workspace] + [patch.crates-io] (umbrella sweep)
- optional oxideav-scribe TTF rendering path
- migrate to centralized OxideAV/.github reusable workflows
- adopt slim VideoFrame shape
- pin release-plz to patch-only bumps

### Added

- `Compositor::with_face` (and `set_face` / `clear_face` / `has_face`)
  to render subtitles via an `oxideav_scribe::Face` instead of the
  embedded 8×16 bitmap font. The Scribe path uses
  `render_text_wrapped` for shaping + word-wrap and composites each
  line with straight-alpha "over" from `oxideav_pixfmt::over_straight`.
- `Compositor::font_size_px` field (default 20.0) — only consumed by
  the Scribe TTF path.
- `RenderedSubtitleDecoder::with_face` builder + new
  `make_rendered_decoder_with_face` factory for one-call wrapper
  construction.
- New `oxideav-scribe` and `oxideav-pixfmt` dependencies.

### Deferred (round 2)

- Italic synthesis, per-run colour, font-fallback chain, and outline
  drawing on the Scribe TTF path. The bitmap-font path is unaffected
  and continues to honour all of these as before.

## [0.0.4](https://github.com/OxideAV/oxideav-subtitle/compare/v0.0.3...v0.0.4) - 2026-04-25

### Other

- use CodecParameters::subtitle() builder
- drop oxideav-codec/oxideav-container shims, import from oxideav-core
- bump oxideav-container dep to "0.1"
- drop Cargo.lock — this crate is a library
- bump to oxideav-core 0.1.1 + codec 0.1.1
- migrate register() to CodecInfo builder
- bump oxideav-core + oxideav-codec deps to "0.1"
