//! Subtitle compositor: turns a [`SubtitleCue`] into an RGBA bitmap
//! suitable for compositing as a video plane.
//!
//! Two render back-ends are supported:
//!
//! * **Bitmap font** (always available) — the embedded 8×16 face from
//!   [`crate::font::BitmapFont`]. No external assets, no TTF parsing.
//!   Use [`Compositor::new`] with no face.
//! * **TrueType via `oxideav-scribe` + `oxideav-raster`** (gated behind
//!   the default-on `text` cargo feature) — `oxideav-scribe` shapes
//!   each styled run into a tree of glyph paths via
//!   [`oxideav_scribe::Shaper::shape_to_paths`]; the resulting nodes are
//!   placed into an [`oxideav_core::VectorFrame`] at the correct
//!   per-line / per-glyph offsets and rasterised end-to-end by
//!   [`oxideav_raster::Renderer`]. Use [`Compositor::with_face`] passing
//!   a [`oxideav_scribe::FaceChain`] (single-face chains are fine).
//!
//! Pipeline (bitmap-font path):
//!
//! 1. Walk the segment tree, flattening it into a stream of styled runs
//!    (text, face, italic, color).
//! 2. Word-wrap those runs into lines that each fit within `width` pixels
//!    — breaking at spaces first, hard-breaking only when a single word
//!    is larger than the line.
//! 3. Stack the lines from the bottom of the canvas upwards (honouring
//!    `bottom_margin_px`) and horizontally centre-align each line — unless
//!    the cue's own `positioning.align` tells us to left/right-align.
//! 4. Blit each glyph with a black outline drawn first (4 one-pixel
//!    offsets in `outline_color`) and the run's foreground on top.
//!    Italic renders via a per-row horizontal shear of `cell_w / 4`
//!    pixels across the glyph height.
//!
//! Pipeline (Scribe + Raster path):
//!
//! 1. Flatten the segment tree to a stream of styled runs (same
//!    `flatten_segments` used by the bitmap path).
//! 2. Split into logical lines on `\n` boundaries and word-wrap each
//!    logical line greedily so it fits within `width` — measurement uses
//!    [`oxideav_scribe::Shaper::shape`] on per-piece text so kerning /
//!    ligatures count toward the budget.
//! 3. For every visual line, walk its (style, text) pieces and shape
//!    each piece via [`oxideav_scribe::Shaper::shape_to_paths`]; wrap
//!    every glyph node in a `Group` whose `transform` places the glyph at
//!    `(line_x + glyph_x, line_baseline + glyph_y)` on the canvas, and
//!    whose `fill` is repainted to the run's colour.
//! 4. Push every wrapped glyph node into the root group of one
//!    [`oxideav_core::VectorFrame`] sized to the canvas, then call
//!    [`oxideav_raster::Renderer::render`] to rasterise the whole cue
//!    in one pass. The resulting RGBA frame is straight-alpha; we
//!    composite it onto the caller's destination buffer with
//!    [`oxideav_pixfmt::over_straight`].
//!
//! The output is always a fresh RGBA `Vec<u8>` of size `width*height*4`,
//! starting zeroed (fully transparent). A paired [`Compositor::render_into`]
//! reuses a caller-provided buffer.
//!
//! Intentional non-features (left for later):
//!
//! * Bitmap-font path: no TrueType shaping; no CJK; no BiDi; no
//!   combining marks beyond Latin-1 precomposed.
//! * Scribe + raster path: no synthesised italic on runs marked italic
//!   (the Renderer rasterises the upright outline; round-2 will swap
//!   to a `render_text_styled`-equivalent vector path); no font-fallback
//!   beyond the explicit `FaceChain` the caller provides; no outline
//!   drawing. Per-run colour IS honoured.
//! * No animation / karaoke timing (the Karaoke segment is rendered as
//!   plain text).
//! * No absolute positioning (ASS `\pos`, WebVTT `x%,y%`). Everything
//!   sits at the bottom-centre (or bottom-left / bottom-right).

use oxideav_core::{Segment, SubtitleCue, TextAlign};

use crate::font::BitmapFont;

#[cfg(feature = "text")]
use oxideav_core::{
    Group, Node, Paint, PathNode, Rgba as CoreRgba, TimeBase, Transform2D, VectorFrame,
};
#[cfg(feature = "text")]
use oxideav_scribe::{FaceChain, Shaper};

/// Bottom-centered subtitle renderer.
///
/// Defaults to the embedded bitmap font; call [`Compositor::with_face`]
/// (or [`Compositor::set_face`]) to substitute a Scribe + Raster TTF
/// back-end for high-quality anti-aliased text. The TTF path is gated
/// behind the default-on `text` cargo feature; embedders who only need
/// the bitmap-font path can opt out via `default-features = false`.
pub struct Compositor {
    pub width: u32,
    pub height: u32,
    /// Default foreground RGBA. Runs without an explicit `Color` use this.
    pub default_color: [u8; 4],
    /// Outline RGBA drawn underneath every glyph (bitmap-font path only).
    pub outline_color: [u8; 4],
    /// Distance between baselines of consecutive lines, in pixels.
    pub line_height_px: u32,
    /// Spacing between the bottom edge of the canvas and the baseline of
    /// the last line.
    pub bottom_margin_px: u32,
    /// Outline thickness (0..=2). Larger values are clamped in `render`.
    /// Only honoured by the bitmap-font path.
    pub outline_px: u32,
    /// Nominal font size in pixels for the Scribe + Raster path. Ignored
    /// on the bitmap-font path (the bitmap face has a fixed cell size).
    pub font_size_px: f32,
    /// Optional TrueType face chain. When set (and the `text` feature is
    /// enabled), [`Compositor::render_into`] switches to the Scribe +
    /// Raster back-end; when `None`, the bitmap-font path is used.
    #[cfg(feature = "text")]
    face: Option<FaceChain>,
}

impl Compositor {
    /// Construct a Compositor that renders via the embedded 8×16
    /// bitmap font. The bitmap path has no external dependencies and
    /// always produces output, but is unscaled and ASCII / Latin-1 only.
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            default_color: [255, 255, 255, 255],
            outline_color: [0, 0, 0, 255],
            line_height_px: 20,
            bottom_margin_px: 24,
            outline_px: 1,
            font_size_px: 20.0,
            #[cfg(feature = "text")]
            face: None,
        }
    }

    /// Construct a Compositor that renders via the supplied
    /// [`FaceChain`]. Glyphs are TrueType-shaped (kerning, ligatures,
    /// face-chain fallback), rasterised by `oxideav-raster`'s scanline
    /// engine, and composited with straight-alpha "over". The
    /// bitmap-font path remains available — drop the chain with
    /// [`Compositor::clear_face`] to revert.
    ///
    /// Available only when the `text` cargo feature is enabled (it is
    /// by default).
    #[cfg(feature = "text")]
    pub fn with_face(width: u32, height: u32, face: FaceChain) -> Self {
        let mut comp = Self::new(width, height);
        comp.face = Some(face);
        comp
    }

    /// Replace the active face chain. Pass `None` to fall back to the
    /// bitmap font.
    ///
    /// Available only when the `text` cargo feature is enabled.
    #[cfg(feature = "text")]
    pub fn set_face(&mut self, face: Option<FaceChain>) {
        self.face = face;
    }

    /// Drop the active face chain and revert to the bitmap-font path.
    ///
    /// Available only when the `text` cargo feature is enabled.
    #[cfg(feature = "text")]
    pub fn clear_face(&mut self) {
        self.face = None;
    }

    /// True when the Scribe + Raster TTF path is active (i.e. a
    /// [`FaceChain`] is set and the `text` feature is enabled).
    pub fn has_face(&self) -> bool {
        #[cfg(feature = "text")]
        {
            self.face.is_some()
        }
        #[cfg(not(feature = "text"))]
        {
            false
        }
    }

    /// Render a cue into a freshly-allocated RGBA buffer of
    /// `width * height * 4` bytes (all pixels initially transparent).
    pub fn render(&self, cue: &SubtitleCue) -> Vec<u8> {
        let mut buf = vec![0u8; (self.width as usize) * (self.height as usize) * 4];
        self.render_into(cue, &mut buf);
        buf
    }

    /// Render a cue into a pre-allocated RGBA buffer. The buffer is
    /// cleared to transparent first, so callers can reuse it across
    /// frames without manual zeroing.
    pub fn render_into(&self, cue: &SubtitleCue, dst: &mut [u8]) {
        // Zero the canvas.
        let required = (self.width as usize) * (self.height as usize) * 4;
        if dst.len() < required {
            return;
        }
        for b in dst[..required].iter_mut() {
            *b = 0;
        }

        // Honour cue-level alignment. Default: Center.
        let align = cue
            .positioning
            .as_ref()
            .map(|p| p.align)
            .unwrap_or(TextAlign::Center);

        // Branch on render back-end.
        #[cfg(feature = "text")]
        {
            if let Some(face) = self.face.as_ref() {
                self.render_into_scribe(face, cue, dst, align);
                return;
            }
        }
        self.render_into_bitmap(cue, dst, align);
    }

    fn render_into_bitmap(&self, cue: &SubtitleCue, dst: &mut [u8], align: TextAlign) {
        // 1. Flatten segments into runs.
        let runs = flatten_segments(&cue.segments, RunStyle::default_with(self.default_color));

        // 2. Wrap runs into lines by greedy word-break.
        let regular = BitmapFont::default_regular();
        let cell_w = regular.cell_w;
        let max_cols = (self.width / cell_w).max(1);
        let lines = wrap_runs(&runs, max_cols as usize);
        if lines.is_empty() {
            return;
        }

        // 3. Position lines from bottom up.
        let cell_h = regular.cell_h;
        let bearing_y = regular.bearing_y;
        let line_h = self.line_height_px.max(cell_h);
        let outline = self.outline_px.min(2);
        let last_baseline = self
            .height
            .saturating_sub(self.bottom_margin_px)
            .saturating_sub((cell_h - bearing_y).min(cell_h));
        let last_baseline = last_baseline as i32;

        // 4. Blit each line.
        let n_lines = lines.len();
        for (i, line) in lines.iter().enumerate() {
            let baseline = last_baseline - ((n_lines - 1 - i) as i32) * line_h as i32;
            let line_width_px = measure_line(line, cell_w) as i32;
            let x = match align {
                TextAlign::Left | TextAlign::Start => 8,
                TextAlign::Right | TextAlign::End => self.width as i32 - line_width_px - 8,
                TextAlign::Center => (self.width as i32 - line_width_px) / 2,
            };
            self.draw_line(line, dst, x, baseline, outline);
        }
    }

    /// Scribe + Raster path. See module docs for the pipeline.
    ///
    /// `face` is the caller-supplied [`FaceChain`]; `cue` is the
    /// subtitle cue to render; `dst` is the destination RGBA8
    /// straight-alpha buffer (already zeroed by the caller); `align`
    /// is the per-line horizontal alignment.
    #[cfg(feature = "text")]
    fn render_into_scribe(
        &self,
        face: &FaceChain,
        cue: &SubtitleCue,
        dst: &mut [u8],
        align: TextAlign,
    ) {
        // 1. Flatten the segment tree to per-style runs (preserves per-run
        //    colour). The bitmap path uses the same flattener.
        let runs = flatten_segments(&cue.segments, RunStyle::default_with(self.default_color));
        if runs.is_empty() {
            return;
        }

        let size_px = if self.font_size_px.is_finite() && self.font_size_px > 0.0 {
            self.font_size_px
        } else {
            20.0
        };
        let side_margin: u32 = 8;
        let max_text_w = self.width.saturating_sub(side_margin * 2);
        if max_text_w == 0 {
            return;
        }

        // 2. Group runs into "logical lines" (split on \n inside a run's
        //    text), then word-wrap each logical line into "visual lines"
        //    that fit within `max_text_w` after shaping with the primary
        //    face. Each visual line is a `Vec<StyledPiece>`.
        let logical = split_logical_lines(&runs);
        let mut visual: Vec<Vec<StyledPiece>> = Vec::new();
        for line in logical {
            let wrapped = wrap_logical_line(&line, face, size_px, max_text_w as f32);
            visual.extend(wrapped);
        }
        if visual.is_empty() {
            return;
        }

        // 3. Vertical layout — stack from the bottom up. Use the
        //    user-configured line height when it exceeds the face's
        //    natural one; otherwise honour the face metric so descenders
        //    don't stomp the next line.
        let face_line_h = face.primary().line_height_px(size_px).ceil() as u32;
        let face_descent_abs = (-face.primary().descent_px(size_px)).ceil().max(0.0) as u32;
        let line_h = self.line_height_px.max(face_line_h.max(1));

        let n_lines = visual.len();
        // Bottom of the *last* (lowest) line: descent below baseline must
        // sit just above the configured bottom_margin_px.
        let last_baseline = self
            .height
            .saturating_sub(self.bottom_margin_px)
            .saturating_sub(face_descent_abs);

        // 4. Build a VectorFrame: one root Group containing every glyph
        //    node, pre-translated to its absolute canvas position and
        //    repainted with its run's colour.
        let mut root = Group::default();
        for (i, line_pieces) in visual.iter().enumerate() {
            // Per-line measurement: sum of piece widths (each piece's
            // shaped run width including its own kerning).
            let piece_widths: Vec<f32> = line_pieces
                .iter()
                .map(|p| measure_piece(face, &p.text, size_px))
                .collect();
            let line_w_px: f32 = piece_widths.iter().sum();

            let line_x = match align {
                TextAlign::Left | TextAlign::Start => side_margin as f32,
                TextAlign::Right | TextAlign::End => {
                    (self.width as f32 - line_w_px - side_margin as f32).max(side_margin as f32)
                }
                TextAlign::Center => ((self.width as f32 - line_w_px) / 2.0).max(0.0),
            };
            let baseline_y =
                last_baseline.saturating_sub(((n_lines - 1 - i) as u32) * line_h) as f32;
            // baseline_y is the canvas y-coordinate of the glyph's pen
            // origin. Face::glyph_node bakes size_px scale + Y-flip so
            // glyph ink rises into negative local y from the pen origin.

            let mut pen_x = line_x;
            for (piece, piece_w) in line_pieces.iter().zip(piece_widths.iter()) {
                if piece.text.is_empty() {
                    continue;
                }
                let glyphs = Shaper::shape_to_paths(face, &piece.text, size_px);
                let fill = Paint::Solid(rgba_to_core(piece.style.color));
                for (_face_idx, node, glyph_xform) in glyphs {
                    // glyph_xform = translate(target_x, y_offset) in
                    // run-local coords. Compose with the line origin.
                    let absolute = Transform2D::translate(pen_x, baseline_y).compose(&glyph_xform);
                    let painted = repaint_node(node, &fill);
                    root.children.push(Node::Group(Group {
                        transform: absolute,
                        children: vec![painted],
                        ..Group::default()
                    }));
                }
                pen_x += *piece_w;
            }
        }

        if root.children.is_empty() {
            return;
        }

        // 5. Rasterise the whole scene in one pass.
        let frame = VectorFrame {
            width: self.width as f32,
            height: self.height as f32,
            view_box: None,
            root,
            pts: None,
            time_base: TimeBase::new(1, 1),
        };
        let renderer = oxideav_raster::Renderer::new(self.width, self.height);
        let rendered = renderer.render(&frame);
        let plane = match rendered.planes.first() {
            Some(p) => p,
            None => return,
        };

        // 6. Composite the rasterised RGBA over the destination buffer
        //    with straight-alpha "over". Renderer's output is straight
        //    alpha, matching pixfmt::over_straight's expectation.
        composite_straight_over(dst, &plane.data, self.width, self.height);
    }

    fn draw_line(&self, line: &Line, dst: &mut [u8], start_x: i32, baseline: i32, outline: u32) {
        let regular = BitmapFont::default_regular();
        let bold = BitmapFont::default_bold();
        let mut x = start_x;
        for piece in &line.pieces {
            let font = if piece.style.bold { bold } else { regular };
            let shear = if piece.style.italic {
                font.cell_w as f32 / 4.0
            } else {
                0.0
            };
            for ch in piece.text.chars() {
                // Draw outline first (4-offset smear).
                if outline > 0 {
                    for dy in -(outline as i32)..=(outline as i32) {
                        for dx in -(outline as i32)..=(outline as i32) {
                            if dx == 0 && dy == 0 {
                                continue;
                            }
                            // Only cardinal + diagonals at exactly `outline` distance
                            // to keep the outline sharp, not bloomed.
                            if dx.abs().max(dy.abs()) != outline as i32 {
                                continue;
                            }
                            font.draw_glyph_sheared(
                                ch,
                                dst,
                                self.width,
                                self.height,
                                x + dx,
                                baseline + dy,
                                self.outline_color,
                                shear,
                            );
                        }
                    }
                }
                // Foreground on top.
                font.draw_glyph_sheared(
                    ch,
                    dst,
                    self.width,
                    self.height,
                    x,
                    baseline,
                    piece.style.color,
                    shear,
                );
                x += font.cell_w as i32;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Scribe + Raster path helpers
// ---------------------------------------------------------------------------

/// One styled piece on a visual line. The `text` is plain (no `\n`).
#[cfg(feature = "text")]
#[derive(Clone, Debug)]
struct StyledPiece {
    text: String,
    style: RunStyle,
}

/// Split a Vec<Run> on embedded `\n` characters. Each output `Vec<Run>`
/// is one logical line (paragraph) carrying its style breaks. Empty
/// trailing lines are dropped.
#[cfg(feature = "text")]
fn split_logical_lines(runs: &[Run]) -> Vec<Vec<Run>> {
    let mut out: Vec<Vec<Run>> = vec![Vec::new()];
    for run in runs {
        let mut iter = run.text.split('\n').peekable();
        while let Some(piece) = iter.next() {
            if !piece.is_empty() {
                out.last_mut().unwrap().push(Run {
                    text: piece.to_string(),
                    style: run.style,
                });
            }
            if iter.peek().is_some() {
                out.push(Vec::new());
            }
        }
    }
    while out.last().map(|l| l.is_empty()).unwrap_or(false) {
        out.pop();
    }
    out
}

/// Greedy word-wrap of a logical line into one or more visual lines that
/// each fit within `max_width` after shaping with `face`.
///
/// Tokenisation matches the bitmap-path `tokenise`: alternating
/// space-runs and word-runs, each carrying the run's style. We measure
/// each token by shaping its text via the primary face — close enough
/// to the rendered width for greedy wrapping (the actual rendering uses
/// the same shape on the per-piece text).
#[cfg(feature = "text")]
fn wrap_logical_line(
    runs: &[Run],
    face: &FaceChain,
    size_px: f32,
    max_width: f32,
) -> Vec<Vec<StyledPiece>> {
    let tokens = tokenise(runs);
    let mut out: Vec<Vec<StyledPiece>> = Vec::new();
    let mut current: Vec<StyledPiece> = Vec::new();
    let mut current_w = 0.0_f32;
    for tok in tokens {
        if tok.text.is_empty() {
            continue;
        }
        let tok_w = measure_piece(face, &tok.text, size_px);
        // Skip leading whitespace at the start of a wrapped line.
        if current.is_empty() && tok.is_space {
            continue;
        }
        if !current.is_empty() && current_w + tok_w > max_width {
            // Wrap before this token.
            trim_trailing_space_pieces(&mut current);
            out.push(std::mem::take(&mut current));
            current_w = 0.0;
            if tok.is_space {
                continue;
            }
        }
        // Hard-break a single oversized word.
        if tok_w > max_width && !tok.is_space {
            for chunk in hard_break_by_width(face, &tok.text, size_px, max_width) {
                if !current.is_empty() {
                    trim_trailing_space_pieces(&mut current);
                    out.push(std::mem::take(&mut current));
                    current_w = 0.0;
                }
                let chunk_w = measure_piece(face, &chunk, size_px);
                push_piece(
                    &mut current,
                    StyledPiece {
                        text: chunk,
                        style: tok.style,
                    },
                );
                current_w += chunk_w;
                if current_w >= max_width {
                    out.push(std::mem::take(&mut current));
                    current_w = 0.0;
                }
            }
            continue;
        }
        push_piece(
            &mut current,
            StyledPiece {
                text: tok.text,
                style: tok.style,
            },
        );
        current_w += tok_w;
    }
    if !current.is_empty() {
        trim_trailing_space_pieces(&mut current);
        if !current.is_empty() {
            out.push(current);
        }
    }
    out
}

#[cfg(feature = "text")]
fn push_piece(line: &mut Vec<StyledPiece>, piece: StyledPiece) {
    if let Some(last) = line.last_mut() {
        if same_style(&last.style, &piece.style) {
            last.text.push_str(&piece.text);
            return;
        }
    }
    line.push(piece);
}

#[cfg(feature = "text")]
fn trim_trailing_space_pieces(line: &mut Vec<StyledPiece>) {
    while let Some(last) = line.last_mut() {
        let trimmed = last.text.trim_end_matches([' ', '\t']);
        if trimmed.is_empty() {
            line.pop();
        } else if trimmed.len() != last.text.len() {
            last.text = trimmed.to_string();
            break;
        } else {
            break;
        }
    }
}

#[cfg(feature = "text")]
fn measure_piece(face: &FaceChain, text: &str, size_px: f32) -> f32 {
    if text.is_empty() {
        return 0.0;
    }
    match face.shape(text, size_px) {
        Ok(glyphs) => oxideav_scribe::run_width(&glyphs),
        Err(_) => 0.0,
    }
}

/// Hard-break a long word into character chunks each <= `max_width`.
#[cfg(feature = "text")]
fn hard_break_by_width(face: &FaceChain, text: &str, size_px: f32, max_width: f32) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    for ch in text.chars() {
        let mut candidate = cur.clone();
        candidate.push(ch);
        let w = measure_piece(face, &candidate, size_px);
        if w > max_width && !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
        cur.push(ch);
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Recursively repaint every `PathNode`'s fill with `paint`. The default
/// fill from `Face::glyph_node` is opaque black; subtitle runs carry a
/// (per-run) colour we want to apply instead.
#[cfg(feature = "text")]
fn repaint_node(node: Node, paint: &Paint) -> Node {
    match node {
        Node::Path(PathNode {
            path,
            stroke,
            fill_rule,
            ..
        }) => Node::Path(PathNode {
            path,
            fill: Some(paint.clone()),
            stroke,
            fill_rule,
        }),
        Node::Group(mut g) => {
            g.children = g
                .children
                .into_iter()
                .map(|c| repaint_node(c, paint))
                .collect();
            Node::Group(g)
        }
        other => other,
    }
}

#[cfg(feature = "text")]
fn rgba_to_core(c: [u8; 4]) -> CoreRgba {
    CoreRgba::new(c[0], c[1], c[2], c[3])
}

/// Composite a straight-alpha RGBA8 source bitmap onto a straight-alpha
/// RGBA8 destination of the same dimensions. Skips fully-transparent
/// source pixels. Blend is Porter-Duff "over" via
/// [`oxideav_pixfmt::over_straight`].
#[cfg(feature = "text")]
fn composite_straight_over(dst: &mut [u8], src: &[u8], width: u32, height: u32) {
    let n = (width as usize) * (height as usize);
    if dst.len() < n * 4 || src.len() < n * 4 {
        return;
    }
    for i in 0..n {
        let off = i * 4;
        let s = [src[off], src[off + 1], src[off + 2], src[off + 3]];
        if s[3] == 0 {
            continue;
        }
        let d = [dst[off], dst[off + 1], dst[off + 2], dst[off + 3]];
        let out = oxideav_pixfmt::over_straight(s, d);
        dst[off] = out[0];
        dst[off + 1] = out[1];
        dst[off + 2] = out[2];
        dst[off + 3] = out[3];
    }
}

// ---------------------------------------------------------------------------
// Run / line model (shared by both back-ends)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
struct RunStyle {
    bold: bool,
    italic: bool,
    color: [u8; 4],
}

impl RunStyle {
    fn default_with(color: [u8; 4]) -> Self {
        Self {
            bold: false,
            italic: false,
            color,
        }
    }
}

#[derive(Clone, Debug)]
struct Run {
    text: String,
    style: RunStyle,
}

#[derive(Clone, Debug, Default)]
struct Line {
    pieces: Vec<Run>,
}

fn flatten_segments(segments: &[Segment], style: RunStyle) -> Vec<Run> {
    let mut out: Vec<Run> = Vec::new();
    walk(segments, style, &mut out);
    // Collapse adjacent runs that have the same style, to keep lines tidy.
    let mut merged: Vec<Run> = Vec::with_capacity(out.len());
    for run in out {
        if let Some(last) = merged.last_mut() {
            if same_style(&last.style, &run.style) {
                last.text.push_str(&run.text);
                continue;
            }
        }
        merged.push(run);
    }
    merged
}

fn same_style(a: &RunStyle, b: &RunStyle) -> bool {
    a.bold == b.bold && a.italic == b.italic && a.color == b.color
}

fn walk(segments: &[Segment], style: RunStyle, out: &mut Vec<Run>) {
    for seg in segments {
        match seg {
            Segment::Text(s) => {
                out.push(Run {
                    text: s.clone(),
                    style,
                });
            }
            Segment::LineBreak => {
                // Encode as a newline character; the wrapper splits on it.
                out.push(Run {
                    text: "\n".to_string(),
                    style,
                });
            }
            Segment::Bold(children) => {
                let mut s = style;
                s.bold = true;
                walk(children, s, out);
            }
            Segment::Italic(children) => {
                let mut s = style;
                s.italic = true;
                walk(children, s, out);
            }
            Segment::Underline(children) | Segment::Strike(children) => {
                // No glyph-level support — render as plain text.
                walk(children, style, out);
            }
            Segment::Color { rgb, children } => {
                let mut s = style;
                s.color = [rgb.0, rgb.1, rgb.2, 255];
                walk(children, s, out);
            }
            Segment::Font { children, .. } => {
                walk(children, style, out);
            }
            Segment::Voice { name, children } => {
                out.push(Run {
                    text: format!("{name}: "),
                    style,
                });
                walk(children, style, out);
            }
            Segment::Class { children, .. } => {
                walk(children, style, out);
            }
            Segment::Karaoke { children, .. } => {
                // Future: could highlight the active beat. For now, plain text.
                walk(children, style, out);
            }
            Segment::Timestamp { .. } => {
                // Nothing visible.
            }
            Segment::Raw(s) => {
                out.push(Run {
                    text: s.clone(),
                    style,
                });
            }
        }
    }
}

/// Greedy word-wrap. `max_cols` is in *glyph cells* (assumes fixed-width
/// font). Splits first on embedded `\n`, then on spaces. Words longer
/// than `max_cols` are hard-broken.
fn wrap_runs(runs: &[Run], max_cols: usize) -> Vec<Line> {
    // Step 1: split at \n to get raw logical lines, each still a Vec<Run>.
    let mut raw_lines: Vec<Vec<Run>> = vec![Vec::new()];
    for run in runs {
        // Split the text on \n, preserving styles.
        let mut iter = run.text.split('\n').peekable();
        while let Some(piece) = iter.next() {
            if !piece.is_empty() {
                raw_lines.last_mut().unwrap().push(Run {
                    text: piece.to_string(),
                    style: run.style,
                });
            }
            if iter.peek().is_some() {
                raw_lines.push(Vec::new());
            }
        }
    }

    // Step 2: for each logical line, wrap to max_cols.
    let mut out: Vec<Line> = Vec::new();
    for logical in raw_lines {
        // Walk the runs, emitting tokens (word | space) with styles. Greedy
        // accumulate into the current visual line; on overflow, start a new one.
        let tokens = tokenise(&logical);
        let mut current = Line::default();
        let mut current_cols = 0usize;
        for tok in tokens {
            let tok_cols = visible_cols(&tok.text);
            if tok_cols == 0 {
                continue;
            }
            if current_cols == 0 && tok.is_space {
                // Skip leading whitespace on a wrap.
                continue;
            }
            if current_cols + tok_cols > max_cols && current_cols > 0 {
                // Wrap.
                out.push(std::mem::take(&mut current));
                current_cols = 0;
                if tok.is_space {
                    continue;
                }
            }
            // Word larger than a line: hard-break across multiple lines.
            if tok_cols > max_cols && !tok.is_space {
                for chunk in hard_break(&tok.text, max_cols) {
                    if current_cols > 0 {
                        out.push(std::mem::take(&mut current));
                    }
                    append_run(
                        &mut current,
                        Run {
                            text: chunk,
                            style: tok.style,
                        },
                    );
                    current_cols = visible_cols(&current.pieces.last().unwrap().text);
                    if current_cols >= max_cols {
                        out.push(std::mem::take(&mut current));
                        current_cols = 0;
                    }
                }
                continue;
            }
            append_run(
                &mut current,
                Run {
                    text: tok.text,
                    style: tok.style,
                },
            );
            current_cols += tok_cols;
        }
        out.push(current);
    }
    // Prune trailing spaces and fully-empty lines at the tail.
    for line in out.iter_mut() {
        trim_trailing_space(line);
    }
    while out.last().map(is_empty_line).unwrap_or(false) {
        out.pop();
    }
    out
}

fn append_run(line: &mut Line, run: Run) {
    if let Some(last) = line.pieces.last_mut() {
        if same_style(&last.style, &run.style) {
            last.text.push_str(&run.text);
            return;
        }
    }
    line.pieces.push(run);
}

fn trim_trailing_space(line: &mut Line) {
    while let Some(last) = line.pieces.last_mut() {
        let trimmed = last.text.trim_end_matches(' ').to_string();
        if trimmed.is_empty() {
            line.pieces.pop();
        } else {
            last.text = trimmed;
            break;
        }
    }
}

fn is_empty_line(line: &Line) -> bool {
    line.pieces.iter().all(|r| r.text.is_empty())
}

#[derive(Clone, Debug)]
struct Token {
    text: String,
    is_space: bool,
    style: RunStyle,
}

fn tokenise(runs: &[Run]) -> Vec<Token> {
    let mut out: Vec<Token> = Vec::new();
    for run in runs {
        let mut buf = String::new();
        let mut buf_is_space: Option<bool> = None;
        for ch in run.text.chars() {
            let is_sp = ch == ' ' || ch == '\t';
            match buf_is_space {
                None => {
                    buf.push(ch);
                    buf_is_space = Some(is_sp);
                }
                Some(prev) if prev == is_sp => buf.push(ch),
                Some(_) => {
                    out.push(Token {
                        text: std::mem::take(&mut buf),
                        is_space: buf_is_space.unwrap(),
                        style: run.style,
                    });
                    buf.push(ch);
                    buf_is_space = Some(is_sp);
                }
            }
        }
        if !buf.is_empty() {
            out.push(Token {
                text: buf,
                is_space: buf_is_space.unwrap_or(false),
                style: run.style,
            });
        }
    }
    out
}

fn visible_cols(s: &str) -> usize {
    // Each char is one cell (fixed-width bitmap font). Tabs count as 1
    // for now; control chars count as 0.
    s.chars().filter(|c| !c.is_control() || *c == '\t').count()
}

fn hard_break(s: &str, max_cols: usize) -> Vec<String> {
    if max_cols == 0 {
        return vec![s.to_string()];
    }
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut cols = 0usize;
    for ch in s.chars() {
        if cols >= max_cols {
            out.push(std::mem::take(&mut cur));
            cols = 0;
        }
        cur.push(ch);
        cols += 1;
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn measure_line(line: &Line, cell_w: u32) -> u32 {
    let cols: usize = line.pieces.iter().map(|r| visible_cols(&r.text)).sum();
    cols as u32 * cell_w
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxideav_core::{CuePosition, Segment, SubtitleCue, TextAlign};

    fn make_cue(segs: Vec<Segment>) -> SubtitleCue {
        SubtitleCue {
            start_us: 0,
            end_us: 1_000_000,
            style_ref: None,
            positioning: None,
            segments: segs,
        }
    }

    #[test]
    fn renders_plain_text() {
        let comp = Compositor::new(320, 240);
        let cue = make_cue(vec![Segment::Text("Hi".to_string())]);
        let buf = comp.render(&cue);
        assert_eq!(buf.len(), 320 * 240 * 4);
        // Some pixel somewhere has alpha > 0.
        assert!(buf.chunks(4).any(|p| p[3] > 0), "no lit pixels");
    }

    #[test]
    fn alignment_right() {
        let mut cue = make_cue(vec![Segment::Text("X".to_string())]);
        cue.positioning = Some(CuePosition {
            align: TextAlign::Right,
            ..Default::default()
        });
        let comp = Compositor::new(200, 100);
        let buf = comp.render(&cue);
        // Right-aligned: lit pixel should exist in right half.
        let lit_right = (0..buf.len() / 4)
            .filter(|i| buf[i * 4 + 3] > 0)
            .filter(|i| {
                let x = i % 200;
                x > 100
            })
            .count();
        assert!(lit_right > 0, "no lit pixels on right side");
    }

    #[test]
    fn handles_empty_cue() {
        let comp = Compositor::new(64, 32);
        let cue = make_cue(vec![]);
        let buf = comp.render(&cue);
        assert!(buf.iter().all(|&b| b == 0));
    }

    #[test]
    fn no_face_means_bitmap_path() {
        let comp = Compositor::new(160, 80);
        assert!(!comp.has_face());
    }

    /// Smoke test: a Compositor built `with_face` from the DejaVu fixture
    /// renders some lit pixels for a simple cue.
    #[cfg(feature = "text")]
    #[test]
    fn scribe_path_renders_text() {
        let bytes = match std::fs::read("../oxideav-ttf/tests/fixtures/DejaVuSans.ttf") {
            Ok(b) => b,
            Err(_) => return, // fixture missing — skip
        };
        let face =
            oxideav_scribe::Face::from_ttf_bytes(bytes).expect("failed to parse DejaVuSans.ttf");
        let chain = oxideav_scribe::FaceChain::new(face);
        let mut comp = Compositor::with_face(320, 240, chain);
        comp.font_size_px = 24.0;
        assert!(comp.has_face());
        let cue = make_cue(vec![Segment::Text("Hello".into())]);
        let buf = comp.render(&cue);
        assert_eq!(buf.len(), 320 * 240 * 4);
        let lit = buf.chunks(4).filter(|p| p[3] > 0).count();
        assert!(lit > 0, "scribe path produced no lit pixels");
        // Lit pixels must sit in the lower half of the canvas (bottom-stack).
        let lit_lower = (0..buf.len() / 4)
            .filter(|i| buf[i * 4 + 3] > 0)
            .filter(|i| (i / 320) >= 120)
            .count();
        assert!(
            lit_lower > 0,
            "no lit pixels in lower half of canvas; got {lit} lit total"
        );
    }
}
