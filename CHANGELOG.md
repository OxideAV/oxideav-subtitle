# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `\t(...)` animated-transform evaluation in `ass_anim`
  (`animate_style_at` / `transform_factor` / `collect_transforms`).
  `transform_factor` computes the acceleration-curve factor
  `((t-t1)/(t2-t1)).clamp(0,1)^accel` (accel `1` linear, `(0,1)`
  fast-then-slow, `>1` slow-then-fast), defaulting the window to the
  whole line when the `\t` carried no times. `animate_style_at`
  interpolates the animatable `ResolvedStyle` fields (font size, x/y
  scale, spacing, x/y/z rotation, x/y border, x/y shadow, edge/gaussian
  blur, and the four primary/secondary/outline/shadow colours + alpha)
  between the pre-`\t` state and the `\t`-modified state, snapping
  non-animatable fields at the window end; stacked `\t(...)` blocks
  compose. Verified against the reference worked examples
  (`\t(0,5000,\frz3600)`, `\1c&HFF0000&\t(\1c&H0000FF&)`,
  `\fscx0\fscy0\t(0,500,\fscx100\fscy100)`).

- ASS / SSA time-varying override-tag evaluator (`ass_anim` module,
  re-exported `fade_alpha_at` / `position_at` / `FadeAlpha`). Given a
  resolved `LineLayout`, a line-relative time `t`, and the line's
  on-screen duration, `fade_alpha_at` evaluates `\fad(<fadein>,<fadeout>)`
  (ramp `255→0`, hold, `0→255` against the duration) and the five-part
  `\fade(a1,a2,a3,t1,t2,t3,t4)` schedule into the ASS-convention alpha
  multiplier (`0` visible … `255` invisible); `position_at` evaluates
  `\move(x1,y1,x2,y2[,t1,t2])` constant-speed interpolation (stationary
  before `t1` / after `t2`, both-zero or absent times spanning the whole
  line) and falls back to the static `\pos`. Both derived from the
  Aegisub override-tag reference.

- Whole-document ASS writer (`ass_script::write`, re-exported as
  `write_ass`) + an IR-segment → Dialogue-`Text` emitter
  (`segments_to_ass_text`). `write` emits the three canonical sections
  (`[Script Info]`, `[V4+ Styles]`, `[Events]`) each with the canonical
  `Format:` header, rebuilding the `[Script Info]` key spellings (the
  camelCase exceptions reverse exactly, other keys Title-Case their
  underscore words) and converting microseconds back to centiseconds.
  `parse(write(track))` is a semantic round-trip over metadata, styles,
  and cue timing + plain text. `segments_to_ass_text` is the inverse of
  the parser's segment wrapping — bold/italic/underline/strike wrap in
  `{\b1}…{\b0}` etc., colour in `{\c&H..&}…{\c}`, karaoke prepends
  `{\k<cs>}`, line breaks become `\N`.

- Whole-document ASS / SSA parser (`ass_script::parse`, re-exported as
  `parse_ass`). Reads a `.ass` / `.ssa` byte stream into a
  `SubtitleTrack`, tying the four ASS helper layers together:
  `[Script Info]` keys land in `track.metadata` under the
  lowercase-snake-case IR convention (`script_info` reads them back
  typed); `[V4+ Styles]` / `[V4 Styles]` rows become IR
  `SubtitleStyle`s; and `[Events]` `Dialogue:` rows become
  `SubtitleCue`s whose bodies are resolved against their style row into
  IR `Segment`s (bold / italic / underline / strike / primary-colour /
  karaoke wrapping, `\N` → `LineBreak`). `Comment:` rows are skipped,
  times convert centiseconds → microseconds, and the parser never
  fails (malformed rows are dropped).

- ASS / SSA `[Events]` `Dialogue:` / `Comment:` line parsing
  (`ass_event` module). `parse_event` maps a row onto an `AssEvent`
  (`layer`/`marked`, start/end in centiseconds, style, name, margins,
  effect, verbatim text) **by Format header name**, with the final
  positional field absorbing embedded commas per "the last field will
  always be the Text field". `parse_time` / `fmt_time` codec the ASS
  `H:MM:SS.cc` timestamp (accepting either a `.` or a `:` before the
  hundredths, one or two hundredth digits, emitting the canonical dotted
  form). `event_to_string` round-trips an `AssEvent` back to a row.
  `DEFAULT_EVENT_FORMAT` is the canonical field order for header-less
  rows.

- ASS / SSA serialization (`ass_emit` module) — the inverse of the
  resolve + style-row parsers. `serialize_line` emits a Dialogue `Text`
  field from a `ResolvedLine` as a *minimal* override stream (diffing
  each span's `ResolvedStyle` against the running state, prefixed by the
  whole-line layout tags), giving a semantic round-trip: re-resolving
  the output against the same base reproduces the same spans + layout.
  `style_row_to_string` emits a `Style:` row for a chosen field order
  (round-trips through `parse_style_row`), and `color_to_string` emits
  the `&HaaBBGGRR&` wire form. `ResolvedStyle::from_base` is now public.

- ASS / SSA `Style:` row parsing (`ass_style_row` module) into a
  `StyleBase` for resolution. `parse_format` reads the `[V4+ Styles]` /
  `[V4 Styles]` `Format:` header; `parse_style_row` maps each `Style:`
  value onto a field **by header name** (per the spec: the Format line
  "defines how SSA will interpret the Style definition lines … even if
  the field order is changed"), so the SSA v4 layout (`TertiaryColour`,
  `AlphaLevel`, legacy 1..=11 alignment) and the ASS V4+ layout
  (`OutlineColour`, numpad alignment) parse uniformly.
  `DEFAULT_V4PLUS_FORMAT` is the canonical V4+ field order for rows with
  no header. `parse_color` decodes the `&H[aa]bbggrr&` wire form (BGR
  byte order, optional alpha high byte inverted to straight alpha) and
  the bare decimal long-integer form. Bold parses as a boolean or an
  explicit weight.

- ASS / SSA override-tag **style resolution** (`ass_resolve` module).
  `resolve_line` folds a Dialogue `Text` override stream over a base
  `Style:` row into a `ResolvedLine`: a sequence of `ResolvedSpan`s
  (visible-text run + the fully `ResolvedStyle` in effect) plus a
  `LineLayout` carrying the whole-line property tags (`\pos`, `\move`,
  `\org`, `\fad`/`\fade`, `\clip`/`\iclip`, `\an`/`\a`). The fold
  applies bold/weight, italic/underline/strike, the four-component
  colour + alpha channels (`\c`/`\1c`–`\4c`, `\alpha`/`\1a`–`\4a`,
  inverting the ASS `00`-opaque/`FF`-transparent convention to straight
  alpha), font name/size/scale/spacing/encoding, the three rotation
  axes, per-axis border + shadow, and the `\be`/`\blur` families, with
  every tag's parameterless form resetting that field to the base.
  `\k` karaoke beats ride each span's `karaoke_cs`; `\t(...)` is left
  for separate animation. `StyleBase::from_style` seeds the base from an
  `oxideav_core::SubtitleStyle`.

- TTML round-trips the full TTML2 §10.2 styling-attribute vocabulary.
  The `<style>`, `<region>`, and inline `<p>` extras channels now carry
  every §10.2 styling attribute the IR does not otherwise model — adding
  `tts:backgroundClip` / `backgroundExtent` / `backgroundImage` /
  `backgroundOrigin` / `backgroundPosition` / `backgroundRepeat`,
  `tts:border`, `tts:bpd` / `ipd`, `tts:fontKerning`, `tts:fontShear`,
  `tts:fontVariant`, `tts:letterSpacing`, `tts:lineShear`,
  `tts:luminanceGain`, `tts:ruby` / `rubyPosition` / `rubyReserve`,
  `tts:textCombine`, `tts:textEmphasis`, `tts:textOrientation`, and
  `tts:zIndex` to the set that previously survived a parse → write
  cycle. The canonical attribute order on the synthesised `<style>` /
  `<region>` / `<p>` now follows the spec's §10.2 numbering
  (§10.2.2 `tts:backgroundClip` … §10.2.52 `tts:zIndex`) so re-emit
  stays byte-stable. Previously these attributes were dropped on the
  re-emit. The seven `SubtitleStyle`-modelled names continue to
  round-trip through the IR fields rather than the verbatim extras.

### Fixed

- TTML inline `tts:backgroundColor` on a `<p>` is now preserved through
  a parse → write cycle. At the `<style>` / `<region>` level the
  attribute maps onto `SubtitleStyle.back_color`, but the inline-styling
  path has no per-run background segment, so an inline
  `tts:backgroundColor` was previously dropped on re-emit. It now rides
  the per-cue `ttml_p_extra.<idx>` channel (in its canonical §10.2.3
  position), alongside the rest of the inline §10.2 vocabulary.

- TTML `xml:space` whitespace handling (TTML2 §8.2.10). The `<p>` cue
  parser now normalizes inline text per the resolved whitespace mode
  instead of carrying authored line-formatting verbatim. In the
  `default` (collapse) mode — the initial value when no `xml:space` is
  present — authored linefeeds and horizontal tabs become single spaces,
  runs of whitespace collapse to one space, and whitespace that surrounds
  a cue edge or a `<br/>` line-break boundary is dropped
  ("ignore-if-surrounding-linefeed"). The `preserve` mode keeps the text
  verbatim. The mode is inherited from the nearest ancestor (`tt` /
  `body` / `div` / `p` / `span`) that specifies it, so a `preserve`
  `<p>` with a `default` `<span>` collapses only the span, and a
  `default` `<p>` with a `preserve` `<span>` keeps only that span
  verbatim. Previously a multi-line indented `<p>` produced segments
  full of inter-tag indentation whitespace. A cue captured in `preserve`
  mode records its mode in `ttml_cue_xml_space.<idx>` track metadata, and
  the writer re-emits `xml:space="preserve"` on the `<p>` so the verbatim
  text survives a parse → write → parse round-trip; a `default`-mode cue
  carries no such metadata and the writer emits no `xml:space` attribute.
- ASS / SSA typed drawing-mode override tags in `ass_tags`. `\p<0/1/..>`
  parses to `AssTag::Drawing(u32)` (the level "might be any integer
  larger than zero, and will be interpreted as the scale, in
  `2^(value-1)` mode"; `\p0` disables drawing) and `\pbo<y>` to
  `AssTag::BaselineOffset(i32)` (the "Y offset to all coordinates", which
  may be negative). `\pbo` is matched ahead of `\p`, and `\pos` /
  `\pos(...)` are unaffected. Only canonical integer levels / offsets
  type — a `+` sign, a non-integer, or a negative level keeps the whole
  tag verbatim `AssTag::Other`, and the bare `\p` / `\pbo` forms (no
  documented value) likewise stay verbatim, so `emit` is byte-stable.
  `drawing_scale_divisor` maps a level to its coordinate divisor
  (`\p2` → 2, `\p4` → 8).
- ASS / SSA `\p` vector drawing-command stream parser in `ass_tags`.
  `parse_drawing` / `emit_drawing` decode the verbatim command run that
  appears between `\p<level>` and `\p0` (and inside the vector-overload
  `\clip(<drawing>)` form) into a structured `DrawCmd` list — `m` move,
  `n` move-no-close, `l` line (one or more segments), `b` cubic Bézier
  (three control points per curve), `s` cubic b-spline (≥3 points),
  `p` spline-extend, `c` close-spline — per the Aegisub drawing-command
  reference. Coordinates parse as decimals (commonly fractional under a
  `\p2`+ subpixel scale, possibly negative); the spec's worked square /
  rounded-square / circle examples decode exactly. A malformed stream
  (leading non-letter token, non-decimal coordinate, a `b` whose point
  count isn't a positive multiple of three, an `l`/`s` with too few
  points, an unknown command letter) returns `None`.
- ASS / SSA typed `\t(...)` animated-transform override tag in
  `ass_tags`. `\t` parses to `AssTag::Transform { t1, t2, accel,
  modifiers }` across the four documented arities — `\t(<modifiers>)`,
  `\t(<accel>,<modifiers>)`, `\t(<t1>,<t2>,<modifiers>)`, and
  `\t(<t1>,<t2>,<accel>,<modifiers>)`. Per the Aegisub override-tag
  reference the *style modifiers* "are other override tags as specified
  in this reference"; they are parsed recursively into nested `AssTag`
  values and re-emit through the same per-tag emitter (factored out of
  `emit` as `emit_tag`), so a `\t(0,1000,\fscx200\fscy200)` round-trips
  byte-stably with both `\fscx` / `\fscy` typed as `FontScale`. The
  leading numbers are split from the modifiers at the first top-level
  backslash: `t1` / `t2` are non-negative integer milliseconds
  "relative to the start time of the line" (present or absent together),
  and `accel` is a non-negative decimal kept verbatim ("can be
  non-integer", `1` is linear) decoded with `decode_decimal`. Off-shape
  spellings — a `\t()` with no modifiers, a leading-argument count
  outside 1–3, a signed or non-integer time, a negative or
  non-canonical accel, trailing text after the close paren — stay an
  untyped `AssTag::Other` so `emit` is byte-stable. The rewritten test
  `transform_t1_t2_modifiers_types` (formerly asserting the verbatim
  `AssTag::Other`) plus three new tests cover all four arities, the
  off-shape verbatim set, and the nested rectangle `\clip` modifier.
- ASS / SSA typed fade override tags in `ass_tags`. `\fad` and `\fade`
  parse to `AssTag::Fade(spec)` over the new `AssFadeSpec` enum
  (re-exported at the crate root). Per the Aegisub override-tag
  reference the simple `\fad(<fadein>,<fadeout>)` "produces a fade-in
  and fade-out effect. The fadein and fadeout times are given in
  milliseconds" (either may be 0 "to not have any fade effect on that
  end") and types to `AssFadeSpec::Simple { fadein, fadeout }` from two
  non-negative integers. The complex
  `\fade(<a1>,<a2>,<a3>,<t1>,<t2>,<t3>,<t4>)` performs "a five-part fade
  using three alpha values … and four times": the alphas "are given in
  decimal and are between 0 and 255" (`u8`), the times "are given in
  milliseconds after the start of the line", and "all seven parameters
  are required" → `AssFadeSpec::Complex { a1, a2, a3, t1, t2, t3, t4 }`.
  Each value is canonically spelled, so a wrong arity, a signed or
  non-integer value, an alpha above 255, or trailing text after the
  close paren keeps the whole tag verbatim `AssTag::Other` and `emit`
  stays byte-stable. The `\fade` arm is tried ahead of `\fad`, and the
  exact-prefix paren match keeps both distinct from the `\f*`
  font-metric family.
- ASS / SSA typed clip override tags in `ass_tags`. `\clip` and `\iclip`
  parse to `AssTag::Clip { inverse, shape }` over the new `AssClipShape`
  enum (re-exported at the crate root). Per the Aegisub override-tag
  reference the rectangle shape `\clip(<x1>,<y1>,<x2>,<y2>)` keeps "only
  the part of the line that is inside the rectangle" and its coordinates
  "must be integers"; `\iclip` "has the opposite effect" (`inverse:
  true`). The vector-drawing shape `\clip(<drawing commands>)` /
  `\clip(<scale>,<drawing commands>)` clips against a `\p`-style path,
  with the optional integer scale ("If the scale is not specified it is
  assumed to be 1") carried distinctly and the command run preserved
  verbatim so `emit` stays byte-stable. The argument list is
  disambiguated by its top-level comma count (four integers → rectangle;
  one argument → unscaled drawing; an integer scale + drawing →
  scaled), and a drawing arm requires an actual command letter so a bare
  two-coordinate list stays an untyped `AssTag::Other`. Off-shape
  arities, a non-integer rectangle coordinate or scale, an empty
  argument list, and trailing text after the close paren all stay
  verbatim. Exact-prefix paren matching keeps `\iclip` / `\clip`
  distinct from the `\i` toggle and `\c` colour.
- ASS / SSA typed edge-blur override tags in `ass_tags`. `\be` and
  `\blur` parse to `AssTag::Blur { kind, strength }` over the new
  `AssBlurKind` enum (`Edge` for `\be`, `Gaussian` for `\blur`,
  re-exported at the crate root). Per the Aegisub reference a `\be`
  strength "is the number of times to apply the regular effect" and
  "must be an integer number", so `\be` accepts only a canonical
  non-negative integer run; `\blur` "uses a more advanced algorithm" and
  "Unlike `\be`, the strength can be non-integer here", so it accepts a
  non-negative decimal (`decode_decimal` turns one into an `f64`).
  Neither strength is meaningfully negative, so a `-`-signed value, a
  decimal `\be`, a `+` sign, or a leading zero keeps the whole tag
  verbatim `AssTag::Other` and `emit` stays byte-stable. Parameterless
  `\be` / `\blur` are the documented reset-to-style form (`None`).
  `\blur` is matched ahead of `\be`, and both run after the `\bord`
  family so `\bord` is never mistaken for a `\b` toggle.
- ASS / SSA typed border and shadow override tags in `ass_tags`.
  `\bord` / `\xbord` / `\ybord` parse to `AssTag::Border { axis, size }`
  and `\shad` / `\xshad` / `\yshad` to `AssTag::Shadow { axis, depth }`,
  both over the new `AssBorderAxis` enum (`Both` / `X` / `Y`,
  re-exported at the crate root). The width / depth runs are preserved
  verbatim as strings — they "can have decimal places" (`\bord3.7`) —
  and `decode_decimal` turns one into an `f64`. Per the Aegisub
  reference the border "cannot be negative" and the combined `\shad`
  "distance can not be negative with this tag", so a `-`-signed value
  there keeps the whole tag verbatim `AssTag::Other`; the per-axis
  `\xshad` / `\yshad` accept a negative depth ("unlike `\shad`, you can
  set the distance negative"). The axis-prefixed forms are matched ahead
  of the combined `\bord` / `\shad`, and exact-prefix matching keeps the
  `\b` / `\s` style toggles and the `\be` blur cousin distinct.
  Parameterless `\bord` … `\yshad` are the reset-to-style shape
  (`None`). `emit` stays byte-stable.
- ASS / SSA typed font-metric and rotation override tags in `ass_tags`.
  `\fn` parses to `AssTag::FontName` (an arbitrary verbatim string, so
  font names with spaces like `\fnCourier New` round-trip), `\fs` to
  `AssTag::FontSize`, `\fscx` / `\fscy` to `AssTag::FontScale { x_axis }`,
  `\fsp` to `AssTag::FontSpacing`, `\fe` to `AssTag::FontEncoding`, and
  `\frx` / `\fry` / `\frz` plus the bare `\fr` (which "defaults to
  `\frz`", kept distinct via a `bare` flag) to
  `AssTag::Rotation { axis, bare }` over the new `AssRotationAxis` enum
  (re-exported at the crate root). The numeric runs are preserved
  verbatim as strings — sizes, scales, spacings, and angles are commonly
  fractional or negative — and the new `decode_decimal` turns one into
  an `f64`. Only a canonically-spelled decimal (optional `-`, ASCII
  digits, at most one interior `.`) is typed; a `+` sign, a bare or
  trailing `.`, an embedded space, a `%`, or a digit-grouping cousin
  keeps the whole tag verbatim `AssTag::Other`, and exact-prefix
  matching (with `\fsc*` / `\fsp` checked ahead of the shorter `\fs`)
  means `\fad` / `\fade` / `\be` / `\blur` / `\bord` are never
  mis-typed. Parameterless `\fn` … `\fr` are the documented
  reset-to-style shape (`None`). `emit` stays byte-stable for every
  input.
- ASS / SSA typed alignment, karaoke, and positioning override tags in
  `ass_tags`. `\an1`–`\an9` parse to `AssTag::AlignNumpad` and legacy
  `\a` to `AssTag::AlignLegacy` over the documented value set (1–3,
  +4 "Toptitle", +8 "Midtitle", explicit `\a0` reset), with
  `legacy_align_to_numpad` mapping legacy values onto the numpad
  layout. The karaoke family — `\k` instant, `\K` / `\kf` left-to-right
  sweep (identical effects, distinct spellings preserved through the
  new `AssKaraokeKind`, re-exported at the crate root), `\ko`
  outline — parses to `AssTag::Karaoke { kind, centisec }` with
  durations in hundredths of seconds. The three line-positioning
  functions parse to `AssTag::Pos` / `AssTag::Org` (integer
  script-resolution coordinates) and `AssTag::Move` (both arities;
  the optional `t1,t2` millisecond animation window stays distinct
  from the 4-argument spelling even when `0,0`). Only
  canonically-spelled parameters are typed — embedded spaces, leading
  zeroes, `+` signs, `-0`, off-arity argument lists, the undocumented
  `\kt`, and the bare no-duration karaoke forms all stay verbatim
  `AssTag::Other`, so `emit` remains byte-stable for every input.

- ASS / SSA typed colour / alpha override tags in `ass_tags`. The
  `\c` / `\1c`–`\4c&H<bbggrr>&` family parses to
  `AssTag::Color { target, short, hex }` and
  `\alpha` / `\1a`–`\4a&H<aa>&` to `AssTag::Alpha { target, hex }`,
  with the four components (primary / secondary fill, border, shadow)
  typed as `AssColorTarget` (re-exported at the crate root) and
  `target: None` for the all-components `\alpha` (per the SSA spec it
  "defaults to `\1a`"). `hex` carries the verbatim digit run — the
  spec's "Leading zeroes are not required" means `&HFF&` (pure red)
  and `&H0000FF&` differ on the wire — and `short` keeps the `\c`
  abbreviation of `\1c` distinct, so `emit` stays byte-stable for
  every typed spelling. The bare `\c` / `\alpha` reset-to-style forms
  are `hex: None`. New `decode_bgr_hex` (hexadecimal **Blue Green
  Red** order, "the opposite order of HTML color codes" → `(r, g, b)`)
  and `decode_alpha_hex` (`00` opaque, `FF` fully transparent)
  helpers decode the runs. Only the canonical `&H<digits>&` parameter
  shape is typed: a missing closing `&`, an empty or over-long digit
  run (> 6 colour / > 2 alpha), non-hex digits, `\5c`, and the
  `\clip` prefix cousin all stay verbatim `AssTag::Other`.

- ASS / SSA Dialogue-text override-tag tokenizer (`ass_tags` module,
  `AssToken` / `AssTag` re-exported at the crate root). `tokenize`
  splits a per-event `Text` payload into plain-text runs, `{...}`
  override blocks, and the three mid-text escapes (`\n` soft break,
  `\N` hard break, `\h` non-breaking hard space) per the SSA v4 spec's
  Appendix A "Style override codes"
  (`docs/subtitles/ass/ass-specs-tcax.html`) and the Aegisub
  override-tag reference (`docs/subtitles/ass/aegisub-ass-tags.html`).
  Within a block, each `\tag` body extends to the next backslash at
  parenthesis depth zero, so a `\t(0,1000,\fscx200\fscy200)` transform
  whose argument list carries nested modifiers stays one tag. The four
  boolean style flags the IR `Segment` tree can model are typed:
  `\b` as `Bold(Option<u32>)` covering off / on / the `\b<weight>`
  100..900 form, and `\i` / `\u` / `\s` as `Option<bool>` with the
  parameterless reset-to-style-default form (spec: "Any style modifier
  followed by no recognizable parameter resets to the default") as
  `None`. The exact-prefix + digits-only-remainder name match keeps
  `\bord` / `\be` / `\blur` / `\shad` / `\iclip` out of the flag arms.
  Everything else — colours, positioning, karaoke, drawing-mode `\p` —
  is preserved verbatim as `AssTag::Other`, and non-tag text inside a
  block (the reference's "unrecognized text within override blocks is
  silently ignored, so they are also commonly used for inline
  comments") as `AssTag::Comment`, so the reciprocal `emit` reproduces
  the original text byte-for-byte on every input: an unrecognised flag
  parameter (`\i2`), an unterminated `{`, and a backslash before
  anything other than `n` / `N` / `h` all stay literal. `plain_text`
  strips a token stream to the user-visible text, mapping the escapes
  against the script's `WrapStyle` (from the previous step's
  `[Script Info]` accessor): `\n` breaks only in wrap mode 2 ("Both
  `\n` and `\N` force line breaks") and is a regular space otherwise,
  `\N` always breaks, `\h` becomes U+00A0 NO-BREAK SPACE. Covered by
  20 unit tests in `ass_tags::tests` (including the spec's own bold /
  italic / karaoke / drawing worked examples) and 2 integration tests
  in `tests/ass_tags.rs`.
- Typed accessor for the SSA / ASS `[Script Info]` block carried in
  `SubtitleTrack::metadata`, exposed as the `ass_script_info` module
  with public re-exports at the crate root: `script_info(&track)`
  returns an `AssScriptInfo` view, and `script_info_keys()` enumerates
  the sixteen lowercase IR keys it recognises. The accessor only reads
  the IR (parsing of `.ass` / `.ssa` files themselves lives in the
  sibling `oxideav-ass` crate); it covers the spec-defined fields per
  the SSA v4 format specification (mirrored at
  `docs/subtitles/ass/ass-specs-tcax.html`) plus the `WrapStyle`
  default whose 0..=3 numeric range the Aegisub override-tag reference
  (`docs/subtitles/ass/aegisub-ass-tags.html`) documents alongside the
  per-cue `\q` tag. `WrapStyle` is exposed as a typed enum
  (`SmartEven` / `EndOfLine` / `None` / `SmartLower`) with
  `from_value` / `as_u8` round-trip helpers; `Collisions` distinguishes
  the spec-named `Normal` / `Reverse` modes and preserves any
  vendor-specific freeform value via a third `Other(String)` variant.
  `PlayResX` / `PlayResY` / `PlayDepth` are parsed as `u32`; `Timer`
  as `f64` percent with `100.0000` (Aegisub default), integer, and
  trailing-`%` shapes all tolerated; `ScaledBorderAndShadow` accepts
  the spec `yes` / `no` plus `true` / `false` / `1` / `0` aliases
  case-insensitively. Unknown keys are left in `metadata` untouched so
  the accessor never drops information. Covered by 16 unit tests in
  `ass_script_info::tests` and 5 integration tests in
  `tests/ass_script_info.rs`.
- WebVTT §5 default cue-component class colour resolution helpers
  `webvtt::default_class_color` and `webvtt::resolve_default_class_colors`,
  plus the `webvtt::DefaultClassKind` enum (`Foreground` /
  `Background`). The §5.1 / §5.2 tables together reserve sixteen class
  names — eight foreground (`white`, `lime`, `cyan`, `red`, `yellow`,
  `magenta`, `blue`, `black`) and eight matching `bg_*` background
  variants — each carrying a fully-opaque `rgba(R,G,B,1)` presentational
  hint that the spec also lets author `::cue(...)` STYLE rules override.
  `default_class_color` is a single-name lookup returning
  `Option<(DefaultClassKind, (u8,u8,u8,u8))>` with case-sensitive
  matching per spec, so `<c.Yellow>` and `<c.BG_BLUE>` remain
  unrecognised author classes rather than aliasing the §5 defaults.
  `resolve_default_class_colors` consumes the dot-chain the parser
  already stores on `Segment::Class::name` (e.g.
  `"yellow.bg_blue.magenta.bg_black"`) and returns
  `(Option<fg>, Option<bg>)` after applying the §5.2 cascade rule —
  within each presentational target, the last matching class in
  appearance order wins. The §5 worked example
  `<c.yellow.bg_blue.magenta.bg_black>` resolves to magenta-on-black,
  matching the spec's explicit narration. Author-defined classes mixed
  into the chain (`<c.warning.yellow.bg_black>`) are skipped rather
  than rejecting the chain, and an author-only chain returns
  `(None, None)` so the caller knows to defer entirely to author
  STYLE rules. Empty / stray dot segments (`<c.yellow..bg_blue>`,
  leading / trailing dots) are tolerated. Covered by 9 new
  `webvtt::tests::{default_class_color_resolves_all_eight_foreground_names,
  default_class_color_resolves_all_eight_background_names,
  default_class_color_is_case_sensitive,
  resolve_chain_applies_cascade_within_each_target,
  resolve_chain_two_classes_text_and_background_only,
  resolve_chain_skips_unrecognised_author_classes,
  resolve_chain_with_no_default_classes_returns_none_none,
  resolve_chain_tolerates_empty_dot_segments,
  resolve_chain_only_foreground_or_only_background,
  resolve_chain_against_class_segment_name_round_trip}`
  unit tests plus
  `tests/webvtt_parse.rs::default_cue_component_classes_5_resolve_through_class_segment_name`
  integration test.

### Changed

- WebVTT §6.4 cue-text tokenizer now decodes HTML character references
  in the data state (the spec's "U+0026 AMPERSAND → HTML character
  reference in data state" transition). The decoder recognises the
  three reference shapes the spec admits — decimal `&#NN…;`, hex
  `&#xNN…;` / `&#XNN…;`, and named — with the named-reference table
  covering the eight HTML5.1 names that subtitle authoring tools emit
  in practice: `&amp;`, `&lt;`, `&gt;`, `&nbsp;`, `&lrm;`, `&rlm;`,
  `&quot;`, `&apos;`. The §4.2.5 examples that the WebVTT spec itself
  cites — `&lrm;` (U+200E LEFT-TO-RIGHT MARK) and the bidi-isolate pair
  `&#x2068;` / `&#x2069;` — now decode to their target codepoints
  rather than passing through as literal byte sequences. Per HTML5.1,
  numeric references that name U+0000 or a surrogate-range codepoint
  map to U+FFFD REPLACEMENT CHARACTER. Malformed references (no
  terminating `;`, unknown name, missing digits) fall back to the
  literal `&` byte per the §6.4 "If nothing is returned, append a
  U+0026 AMPERSAND character" branch, so a stray `& Co.` in cue text
  no longer parses as the start of an entity. The reciprocal writer
  side encodes any `&`, `<`, or `>` byte that appears inside a
  `Segment::Text` as `&amp;` / `&lt;` / `&gt;`, so a parse → write →
  parse round-trip on text that contains the three §4.2.2-reserved
  bytes (`Tom & Jerry <3 hearts > rocks`) reproduces the original
  user-visible string.
- TTML2 §8.1.5 inline `tts:*` styling attributes on `<p>` content
  elements are now honoured. The IR-modelled attrs
  (`tts:color`, `tts:fontFamily`, `tts:fontSize`, `tts:fontWeight`,
  `tts:fontStyle`, `tts:textDecoration`) on `<p>` wrap the cue's
  content with the equivalent `Segment::Bold` / `Italic` / `Underline`
  / `Strike` / `Color` / `Font` segments, mirroring the spec's
  position that inline styling on `<p>` is "available for style
  inheritance by descendant content elements". The IR-unmodelled
  inline attrs (`tts:textAlign`, `tts:displayAlign`, `tts:lineHeight`,
  `tts:opacity`, `tts:textOutline`, `tts:textShadow`,
  `tts:writingMode`, `tts:wrapOption`, `tts:direction`,
  `tts:rubyAlign`, `tts:shear`, `tts:showBackground`,
  `tts:visibility`, `tts:display`, `tts:disparity`,
  `tts:fontSelectionStrategy`, `tts:position`, `itts:forcedDisplay`,
  `itts:fillLineGap`) survive the round-trip via per-cue
  `ttml_p_extra.<idx>` track metadata in canonical spec order, so a
  parse → write → parse cycle is byte-stable on the inline-styled
  `<p>`. The synthesised writer also widens the `xmlns:itts`
  emission test to cover `ttml_p_extra` so an inline
  `itts:forcedDisplay` on a `<p>` re-emits with a valid namespace
  binding. Previously every inline `tts:*` attribute on `<p>` was
  silently dropped — only the `style="ref"` attribute and `<span>`
  inline styling were honoured.
- WebVTT §6.3 `position` / `size` / `line` cue-setting parsing now drops
  individual settings whose value is not a valid WebVTT percentage
  (§4.4: one or more ASCII digits, optionally followed by a U+002E DOT
  and one or more ASCII digits, then a U+0025 PERCENT SIGN, numerically
  in `0..=100`) — matching the spec's "jump to the step labeled next
  setting" branch for malformed values. Previously the parser kept the
  leading digit prefix of a bare `position:50` and accepted
  out-of-range values like `size:120%`; both are now discarded while
  the cue itself still parses. The `line` setting's line-number
  variant likewise rejects values that don't match the spec
  production `[-]?digits[.digits]?`, and the `,<align>` suffix of
  `line` and `position` drops the whole setting when the suffix isn't
  one of the spec's recognised keywords. The existing percentage and
  negative-line-number round-trips are unchanged.
- WebVTT §4.1 file-signature validation now enforces the spec's literal
  shape: the `WEBVTT` byte string must be followed by either a line
  terminator or a single U+0020 SPACE / U+0009 TAB and then the
  optional header trailing text. The previous lenient implementation
  used a bare `starts_with("WEBVTT")` check, so a file beginning with
  `WEBVTTHEADER` was accepted and the `HEADER` suffix silently became
  trailing-text metadata; the strict check rejects that input with the
  same missing-signature error a non-WEBVTT file produces. The two
  valid separators (SPACE and TAB) round-trip alongside the empty
  separator unchanged. A UTF-8 BOM on the file's first byte continues
  to work because the shared `encoding::decode_subtitle_text` helper
  strips it before the signature check.
- WebVTT §3.3 timestamp parsing tightened to only accept the two
  canonical shapes the spec defines: `MM:SS.fff` and `HH:MM:SS.fff`.
  Minutes and seconds must be exactly two ASCII digits each in the
  range `0..=59`; the fractional component must be exactly three ASCII
  digits separated from the seconds by a `.`; the optional hours
  component, when present, must be two or more ASCII digits. The
  previous parser accepted single-digit minutes / seconds, an empty
  fractional component (with `.000` as a silent default), and
  out-of-range minutes / seconds, so a malformed timing line such as
  `0:00:01.000 --> 00:00:02.000` (single-digit hours) or
  `00:60:01.000 --> 00:60:02.000` (minutes > 59) would silently parse
  into a wrong offset. The strict parser rejects those cases; the
  containing cue block fails to recognise a timing line and the cue
  is dropped instead of carrying a quietly-wrong timestamp. Covered by
  14 new `webvtt::tests::{signature_with_no_separator_is_rejected,
  signature_with_tab_separator_keeps_trailing_text,
  signature_with_space_separator_keeps_trailing_text,
  bare_signature_parses_with_no_trailing_metadata,
  signature_with_utf8_bom_is_accepted,
  timestamp_with_single_digit_minutes_is_rejected,
  timestamp_with_single_digit_seconds_is_rejected,
  timestamp_with_missing_fraction_is_rejected,
  timestamp_with_two_digit_fraction_is_rejected,
  timestamp_with_four_digit_fraction_is_rejected,
  timestamp_with_out_of_range_minutes_is_rejected,
  timestamp_with_out_of_range_seconds_is_rejected,
  timestamp_with_one_digit_hours_is_rejected,
  timestamp_three_digit_hours_is_accepted,
  timestamp_mm_ss_fff_short_form_is_accepted}` unit tests plus
  `tests/webvtt_parse.rs::strict_signature_and_timestamp_validation_end_to_end`
  integration test.

### Added

- WebVTT §3.4 cue-identifier round-trip. The parser now captures every
  per-cue identifier (the optional line immediately before the cue
  timings line) into a `vtt_cue_id.<idx>` track-metadata entry, mirroring
  the existing `vtt_cue_extra.<idx>` and `vtt_note.<idx>` channels. The
  synthesised (no-extradata) writer prepends the captured identifier on
  its own line ahead of the timing line so a parse → drop-extradata →
  write → parse cycle reproduces the original identifier byte-for-byte.
  Both textual ids (used by the §8.2.1 `::cue(#id)` style selector) and
  numeric ids (carried over from SRT-style authoring tools) survive;
  empty identifiers are skipped at write time so a stray blank line
  cannot sneak in. Cues that lack an identifier round-trip unchanged
  (no spurious id metadata written, no spurious id line emitted). When a
  NOTE comment block sits between two identified cues the writer
  interleaves both deterministically — NOTE block, blank separator, next
  cue's identifier, then the timing line.
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
