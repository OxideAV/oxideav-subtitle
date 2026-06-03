# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- WebVTT §4.1 comment-block (`NOTE …`) round-trip. The parser now
  captures every comment block — single-line, multi-line, or the
  bare-token form (`NOTE` followed by a newline) — into per-block
  `vtt_note.<idx>` track metadata holding the verbatim body and a
  paired `vtt_note_pos.<idx>` recording the cue index the block
  preceded (so `0` means "before the first cue", `N` means "after the
  last cue", `k` means "between cue `k-1` and `k`"). Both write paths
  honour the capture: the verbatim-extradata path re-emits the block
  in its original byte position because it is now included in the
  saved extradata; the synthesised (no-extradata) write path
  reconstructs the same interleaving from the metadata, so a parse →
  drop-extradata → write → parse cycle round-trips every NOTE in its
  original position. Comment-block detection is case-sensitive per
  spec — only first-line tokens of `NOTE`, `NOTE ` (space) or
  `NOTE\t` (tab) qualify; lowercase `note` or longer identifiers like
  `Notebook` fall through to the cue-block code path (previous
  behaviour silently swallowed `notebook` as a comment, which was a
  latent bug). The W3C §1.5 worked example with three NOTE blocks
  (heading, mid-stream `check next cue`, trailing `end of file`)
  round-trips byte-stable through both writer paths.

### Changed

- `src/srt.rs`: trimmed a decorative comparison clause from the
  `escape_text` doc comment. The behavioural statement (keep `<`
  unescaped so the writer doesn't accidentally smuggle a recognised
  inline tag) is preserved; only the trailing attribution to an
  unrelated implementation is dropped. Pure prose change, no
  behaviour or test delta.

### Fixed

- SRT parser tolerates three classes of malformed-but-recoverable
  input that real-world files exhibit: (a) leading preamble — junk
  lines or a PEM-style armoured envelope before the first cue, with
  or without a blank separator; previously aborted the entire parse
  because the first non-blank line didn't satisfy the strict
  index-then-timing sequence. (b) duplicate-index rows such as
  `N\nN\n<timing>` that some batch editors emit when re-numbering
  with a buggy template; previously consumed the second copy as the
  timing line, failed to parse, and dropped the cue. (c) cue bodies
  whose middle line is whitespace-only, e.g. `"A\n   \nB"`;
  previously the body terminator used `lines[i].trim().is_empty()`,
  treating the whitespace-only line as a cue boundary and dropping
  `B`. The new loop forward-scans for the timing line within the
  current non-blank block (so anything before it is discardable
  preamble) and terminates the body only on a TRULY empty line or a
  new timing line (so whitespace-only continuation lines survive,
  and two cues with no intervening blank are split correctly). The
  `bytes_to_cue` single-cue helper picks up the same forward-scan
  preamble tolerance. Covered by 8 new
  `srt::tests::{leading_garbage_line_without_blank_is_skipped,
  two_leading_garbage_lines_without_blank_are_skipped,
  pem_style_armoured_prefix_is_tolerated,
  duplicate_index_line_does_not_drop_the_cue,
  whitespace_only_internal_line_stays_in_body,
  two_cues_with_no_blank_between_are_split_on_timing_line,
  trailing_crlf_blank_lines_are_tolerated,
  missing_trailing_newline_is_tolerated,
  bytes_to_cue_skips_pem_preamble}` unit tests plus
  `tests/srt_parse.rs::end_to_end_robustness_recovers_pem_preamble_dup_index_and_embedded_blanks`
  integration test.

### Added

- WebVTT `STYLE` block §8.2.1 property + selector coverage now matches
  the spec. The parser previously accepted only `::cue(.class)`
  selectors (silently dropping bare `::cue`, `::cue(#id)`, and
  `::cue(<element>)`) and only seven of the eleven properties §8.2.1
  lists as applying to `::cue`. The new encoding tags styles as
  `"::cue"` (bare), `"#<id>"`, `"::cue(<elem>)"`, or `"<class.chain>"`
  (historical `::cue(.x)` shape preserved for back-compat). The four
  spec-listed properties without a `SubtitleStyle` field — `opacity`,
  `visibility`, `text-shadow`, `outline`, `white-space`,
  `text-combine-upright`, `ruby-position`, `line-height` — ride a
  per-style `vtt_style.<name>.<property>` metadata channel in canonical
  spec order, mirroring the existing `vtt_region.<id>` /
  `ttml_style_extra.<id>` round-trip pattern. The synthesised writer
  (no-extradata path) reconstructs the original `::cue(...)` selector
  string and re-emits the extras deterministically so a parse → write →
  parse cycle is byte-stable. The `background-color` property, which
  parses into `SubtitleStyle.back_color`, is now also re-emitted by the
  synthesised writer (previously dropped on the floor). Covered by 12
  new `webvtt::tests::{cue_bare_selector_with_no_argument_round_trips,
  cue_id_selector_round_trips, cue_type_selector_round_trips,
  class_chain_selector_keeps_dot_chain_in_style_name,
  opacity_visibility_text_shadow_outline_round_trip_via_metadata,
  white_space_text_combine_ruby_position_line_height_round_trip,
  background_color_round_trips_to_back_color_field,
  unknown_property_is_silently_dropped,
  extras_emit_in_canonical_order_regardless_of_source_order,
  parse_style_block_existing_test_still_works,
  multiple_style_blocks_each_with_extras,
  synthesised_write_full_roundtrip_is_byte_stable}` unit tests plus
  `tests/webvtt_parse.rs::style_block_full_property_round_trip_end_to_end`
  integration test.

## [0.1.2](https://github.com/OxideAV/oxideav-subtitle/compare/v0.1.1...v0.1.2) - 2026-05-29

### Other

- advance inline-text accumulators by full UTF-8 codepoints
- per-cue TTI + GSI DSC/CCT/LC/TCS round-trip via metadata
- IMSC 1.2 layout + parameters + extended styling round-trip
- §3.5 cue-payload inline markup round-trip + UTF-8 fix
- parse all five REGION settings + round-trip via vtt_region.<id>
- preserve vertical / line+position align suffixes / region cue settings
- shared UTF-16 BOM + classic-Mac CR-only tolerance

### Fixed

- Multi-byte UTF-8 cue text adjacent to a tag boundary is no longer
  corrupted into Latin-1 mojibake in the SubRip (`srt`), MicroDVD
  (`microdvd`), and RealText (`realtext`) inline-markup parsers. Each
  parser's plain-text accumulator advanced one *byte* at a time via
  `byte as char`, so a codepoint such as `é` (`0xC3 0xA9`) or `漢`
  (`0xE6 0xBC 0xA2`) sitting next to a `<i>` / `{y:i}` / `</font>`
  delimiter was split into its individual continuation bytes and
  re-emitted as `Ã©` / `æ¼¢`. The accumulators now advance one full
  UTF-8 codepoint at a time (matching the WebVTT fix already in place).
  Covered by new `srt::tests::multibyte_text_around_tags_round_trips`,
  `microdvd::tests::multibyte_text_around_tags_round_trips`, and
  `realtext::tests::multibyte_text_around_tags_round_trips`.

### Added

- EBU STL per-cue TTI field round-trip preservation. SGN (subtitle group
  number), SN (subtitle number), CS (cumulative status), VP (vertical
  position), and JC (justification code) now survive a parse → write
  loop via per-cue `ebu_tti.<idx>.<field>` track metadata; previously
  the writer hardcoded SGN=0, SN=index, CS=0, VP=`mnr-1`, JC=0x02 on
  every row regardless of the source's values. Extension-block
  membership (EBN != 0xFF / != 0x00) is recorded as
  `ebu_tti.<idx>.ext.<ebn>` on the parent cue so a continuation row's
  EBN can be replayed. Comment-flagged TTI rows (CF == 1), previously
  silently dropped, now ride alongside as `ebu_tti.comment.<n>.*`
  entries (SGN / SN / EBN / CS / TCI / TCO / VP / JC + `tf_hex` of the
  raw 112-byte text field) and are re-emitted after the playable cues
  on write so they survive byte-faithfully. GSI bytes DSC / CCT / LC /
  TCS likewise round-trip through `dsc` / `cct` / `lc` / `tcs`
  track-level metadata; previously the writer always emitted DSC='1',
  CCT='00', LC='00', TCS='1'. Programmatic tracks (no metadata
  populated) still fall back to the same first-cut defaults so existing
  call-sites are unaffected. Covered by 7 new
  `ebu_stl::tests::{parse_captures_tti_fields_into_per_cue_metadata,
  write_replays_captured_tti_fields_byte_exact,
  write_uses_safe_defaults_for_programmatic_track,
  comment_flagged_rows_round_trip_via_metadata,
  extension_block_marker_recorded_on_parent_cue,
  dsc_cct_lc_tcs_round_trip_through_metadata, hex_helpers_round_trip}`
  unit tests plus
  `tests/ebu_stl_parse.rs::per_cue_tti_fields_round_trip_end_to_end`
  and `comment_flag_row_survives_parse_write_parse` integration tests.
- TTML / IMSC 1.2 layout regions, parameter attributes, and extended
  styling now parse + round-trip end-to-end. `<head><layout><region
  xml:id="X" tts:.../></layout></head>` definitions (IMSC1 §7) ride as
  per-region `ttml_region.<id>` track metadata carrying the canonical
  attribute order (`origin`, `extent`, `padding`, `backgroundColor`,
  `color`, `displayAlign`, `textAlign`, …, `itts:forcedDisplay`,
  `itts:fillLineGap`). `<p region="X">` cue-region references survive
  as per-cue `ttml_cue_region.<idx>`. `tts:textAlign` on a `<style>`
  maps to `SubtitleStyle.align`; IR-unmodelled `tts:*` / `itts:*`
  styling attributes (`displayAlign`, `extent`, `origin`, `padding`,
  `lineHeight`, `opacity`, `textOutline`, `textShadow`, `writingMode`,
  `wrapOption`, `direction`, `rubyAlign`, `shear`, `showBackground`,
  `visibility`, `display`, `disparity`, `fontSelectionStrategy`,
  `position`, `itts:forcedDisplay`, `itts:fillLineGap`) survive as
  `ttml_style_extra.<id>` in canonical order. `<tt>` parameter
  attributes (`ttp:frameRate`, `ttp:tickRate`, `ttp:timeBase`,
  `ttp:profile`, `ttp:cellResolution`, `ttp:frameRateMultiplier`,
  `ttp:displayAspectRatio`, `ttp:contentProfiles`) and IMSC1 extension
  parameters (`ittp:aspectRatio`, `ittp:activeArea`,
  `ittp:progressivelyDecodable`) round-trip as `ttml_param.<name>`,
  with the writer reinstating `xmlns:ttp` / `xmlns:ittp` /
  `xmlns:itts` only when the corresponding namespace is in use.
  Covered by 14 new `ttml::tests::imsc1_*` / `ttp_*` /
  `hhmmssff_*` / `cue_region_*` unit tests plus two integration tests
  in `tests/ttml_parse.rs::full_imsc1_document_parses_and_round_trips`
  and `imsc1_region_without_cue_ref_still_round_trips`.
- TTML timing previously dropped on the floor now decodes when the
  source supplies the matching `<tt>` parameter: `HH:MM:SS:FF`
  clock-time frames against `ttp:frameRate` (00:00:01:05 at 25 fps
  ⇒ 1.2 s, not 1.0 s); `<n>f` offset-time frames against
  `ttp:frameRate`; `<n>t` offset-time ticks against `ttp:tickRate`.
  Without the matching parameter the frame / tick component is
  dropped (legacy behaviour preserved for back-compat).
- WebVTT cue payload inline markup (WebVTT §3.5) now has full byte-stable
  parse/emit round-trip coverage: `<v Speaker>` voice spans preserve the
  annotation (and `<v>` with no annotation re-emits cleanly); `<c.foo.bar>`
  class chains keep the full dot-joined name and bare `<c>` no longer
  re-emits as the invalid `<c.>`; `<lang xx-YY>...</lang>` language spans
  preserve the BCP 47 annotation including subtag chains like
  `zh-Hant-HK`; `<ruby>base<rt>annotation</rt></ruby>` ruby spans handle
  multiple base+rt pairs and tolerate the spec-permitted implicit final
  `</rt>` (the writer normalises to explicit). A stray `<rt>` outside
  `<ruby>` is preserved as Raw rather than swallowing the rest of the cue.
  Covered by 13 new `webvtt::tests::inline_*` unit tests plus
  `tests/webvtt_parse.rs::cue_payload_inline_markup_round_trips_end_to_end`
  and `cue_payload_language_span_with_bcp47_tag`.
- Fixed a latent UTF-8 mishandling bug in the cue-payload inline-text
  accumulator: previously `text_buf.push(byte as char)` advanced one byte
  at a time, so a multi-byte codepoint (`à`, `漢`, `みん`, …) adjacent to
  a tag boundary was emitted as mojibake on the parsed-out side. The
  accumulator now advances by full UTF-8 codepoints.
- WebVTT `REGION` definition blocks (WebVTT §4.3) now parse all five
  region settings — `width`, `lines`, `regionanchor`, `viewportanchor`,
  and `scroll` — with case-sensitive names and §6.2 value validation
  (percentages `0..=100` with a `%`, `lines` digits-only, anchors as
  `<pct>,<pct>` tuples, `scroll:up` only); malformed values are dropped.
  Previously only `id` + `width` were read and the rest were lost on the
  synthesised write path. The geometry settings the IR `SubtitleStyle`
  can't model are captured verbatim (re-serialised in canonical spec
  order) in a per-region `vtt_region.<id>` track-metadata entry, and the
  writer rebuilds a complete REGION block from style + metadata when the
  track has no verbatim parse extradata, so all five settings round-trip
  through the synthesised path. Covered by 6 new webvtt unit tests plus
  `tests/webvtt_parse.rs::full_region_block_round_trips_through_synthesised_write`.
- WebVTT cue settings the unified IR can't model are now preserved
  through a parse → write round-trip: the `vertical:rl|lr` writing
  direction, the `,start|,center|,end` alignment suffix on `line`, the
  `,line-left|,center|,line-right` suffix on `position`, and a
  `region:<id>` reference (WebVTT §3.5). They ride alongside the cue in
  per-cue `vtt_cue_extra.<idx>` track metadata, so the structured
  `CuePosition` (offset / size / align) keeps working unchanged while
  the full settings list survives. A `line` offset given as a bare
  (possibly negative) line number is now distinguished from a percentage
  offset and re-emitted without a spurious `%`.
- New crate-private `encoding` module with a `decode_subtitle_text`
  helper that sniffs UTF-8 / UTF-16 LE / UTF-16 BE BOMs and normalises
  CRLF / lone-CR line endings to LF. Every parser (SRT, WebVTT,
  MicroDVD, MPL2, MPsub, VPlayer, PJS, AQTitle, JACOsub, RealText,
  SubViewer 1/2, TTML, SAMI) routes through it, so a UTF-16-with-BOM
  file (the format YouTube's SRT export emits) and a classic-Mac
  CR-only file are now both accepted everywhere instead of producing
  a single-line garbled track.
- `tests/encoding_tolerance.rs` — 11 integration tests covering
  UTF-16 LE / UTF-16 BE / Mac CR-only / DOS CRLF / mixed-newline /
  surrogate-pair / odd-tail-byte cases on the four common parsers
  (SRT, WebVTT, MicroDVD, MPL2). 16 unit tests in `encoding.rs`
  cover the helper directly (BMP + supplementary-plane decode,
  CR-inside-text, multibyte UTF-8 preservation, etc.).

### Changed

- Removed 13 copy-pasted local `strip_bom` / `decode_utf8_lossy_stripping_bom`
  helpers (one per parser module) in favour of the shared
  `encoding::decode_subtitle_text`. Behaviour is a strict superset of
  the previous per-parser helpers: every previously-accepted file
  still parses identically; UTF-16 BOMs and CR-only line endings now
  also work.


## [0.1.1](https://github.com/OxideAV/oxideav-subtitle/compare/v0.1.0...v0.1.1) - 2026-05-06

### Other

- reframe FFI claim — HW-engine crates use OS FFI by necessity
- drop stale REGISTRARS / with_all_features intra-doc links
- drop dead `linkme` dep
- registry calls: rename make_decoder/make_encoder → first_decoder/first_encoder
- auto-register via oxideav_core::register! macro (linkme distributed slice)
- unify entry point on register(&mut RuntimeContext) ([#502](https://github.com/OxideAV/oxideav-subtitle/pull/502))
- cargo fmt the new make_rendered_decoder_with_face import order
- shape via scribe → rasterise via oxideav-raster ([#355](https://github.com/OxideAV/oxideav-subtitle/pull/355))
- bump oxideav-scribe pin to 0.1
- release v0.1.0

### Changed

- Compositor TTF path now goes shape (oxideav-scribe `Shaper::shape_to_paths`)
  → vector scene (`oxideav_core::VectorFrame`) → rasterise
  (`oxideav_raster::Renderer`). Per-run colour from the SubtitleCue's
  `Segment::Color` is honoured (the previous round-1 path forced the whole
  cue to one colour). The bitmap-font path is unchanged.
- `Compositor::with_face` and `RenderedSubtitleDecoder::with_face` now take
  an `oxideav_scribe::FaceChain` instead of a single `Face`, so callers
  can install fallback faces (CJK / emoji) alongside the primary. Wrap a
  single face with `FaceChain::new(face)` to migrate.

### Added

- `text` cargo feature (default-on): gates the TTF rendering path. When
  disabled, the crate drops both `oxideav-scribe` and `oxideav-raster`
  from its dep tree and only the `BitmapFont` fallback is available;
  `Compositor::with_face` / `set_face` / `clear_face` /
  `make_rendered_decoder_with_face` are compiled out. Mirrors the
  `oxideav-svg` `text` feature pattern so embedders that only need the
  bitmap path can opt out via `default-features = false`.
- `oxideav-raster = "0.1"` (optional, gated behind `text`) — vector
  scene rasteriser used by the new TTF path.
- `tests/render.rs::srt_round_trip_renders_through_both_paths` —
  feeds an SRT cue through `srt::parse` then renders via both the
  bitmap-font path and (when the `text` feature is enabled and the
  DejaVu fixture is present) the Scribe + Raster path, verifying both
  produce non-zero pixel output.

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
