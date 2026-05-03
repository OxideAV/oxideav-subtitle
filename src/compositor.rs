//! Subtitle compositor: turns a [`SubtitleCue`] into an RGBA bitmap
//! suitable for compositing as a video plane.
//!
//! Two render back-ends are supported:
//!
//! * **Bitmap font** (default) — the embedded 8×16 face from
//!   [`crate::font::BitmapFont`]. No external assets, no TTF parsing,
//!   always available. Use [`Compositor::new`].
//! * **TrueType font via `oxideav-scribe`** — anti-aliased, properly
//!   shaped glyphs from a real TTF face (kerning, ligatures, full
//!   Unicode `cmap`). Use [`Compositor::with_face`] passing an
//!   `oxideav_scribe::Face` constructed from a `.ttf` byte slice.
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
//! Pipeline (Scribe TTF path):
//!
//! 1. Flatten the segment tree to a single text string per cue
//!    (line-breaks preserved as `\n`).
//! 2. Hand the string to
//!    [`oxideav_scribe::render_text_wrapped`] — Scribe runs its own
//!    shaper + word-wrap, returning one straight-alpha
//!    [`RgbaBitmap`](oxideav_scribe::RgbaBitmap) per output line.
//! 3. Stack the resulting line bitmaps bottom-up, honouring
//!    `bottom_margin_px`, and alpha-composite each onto the canvas
//!    using straight-alpha "over". Outline / italic / per-run colours
//!    are documented round-2 work; round 1 uses
//!    `default_color` for the whole cue.
//!
//! The output is always a fresh RGBA `Vec<u8>` of size `width*height*4`,
//! starting zeroed (fully transparent). A paired [`Compositor::render_into`]
//! reuses a caller-provided buffer.
//!
//! Intentional non-features (left for later):
//!
//! * Bitmap-font path: no TrueType shaping; no CJK; no BiDi; no
//!   combining marks beyond Latin-1 precomposed.
//! * Scribe path (round 1): no synthesised italic — runs marked italic
//!   render upright (round 2 will pass an italic Face or shear-fake);
//!   no font-fallback chain — `Segment::Font.family` is ignored, the
//!   single Compositor face is always used; no per-run colour
//!   override (one cue, one colour); no outline drawing.
//! * No animation / karaoke timing (the Karaoke segment is rendered as
//!   plain text).
//! * No absolute positioning (ASS `\pos`, WebVTT `x%,y%`). Everything
//!   sits at the bottom-centre (or bottom-left / bottom-right).

use oxideav_core::{Segment, SubtitleCue, TextAlign};

use crate::font::BitmapFont;

/// Bottom-centered subtitle renderer.
///
/// Defaults to the embedded bitmap font; call [`Compositor::with_face`]
/// to substitute a Scribe TTF face for high-quality anti-aliased text.
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
    /// Nominal font size in pixels for the Scribe TTF path. Ignored on
    /// the bitmap-font path (the bitmap face has a fixed cell size).
    pub font_size_px: f32,
    /// Optional TrueType face. When set, [`Compositor::render_into`]
    /// switches to the Scribe rasterise + alpha-composite back-end;
    /// when `None`, the bitmap-font path is used.
    face: Option<oxideav_scribe::Face>,
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
            face: None,
        }
    }

    /// Construct a Compositor that renders via the supplied
    /// `oxideav_scribe::Face`. Glyphs are TrueType-shaped (kerning,
    /// ligatures), anti-aliased, and composited with straight-alpha
    /// over. The bitmap-font path remains available — drop the face
    /// with [`Compositor::clear_face`] to revert.
    pub fn with_face(width: u32, height: u32, face: oxideav_scribe::Face) -> Self {
        let mut comp = Self::new(width, height);
        comp.face = Some(face);
        comp
    }

    /// Replace the active face. Pass `None` to fall back to the bitmap
    /// font.
    pub fn set_face(&mut self, face: Option<oxideav_scribe::Face>) {
        self.face = face;
    }

    /// Drop the active face and revert to the bitmap-font path.
    pub fn clear_face(&mut self) {
        self.face = None;
    }

    /// True when the Scribe TTF path is active.
    pub fn has_face(&self) -> bool {
        self.face.is_some()
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
        if let Some(face) = self.face.as_ref() {
            self.render_into_scribe(face, cue, dst, align);
        } else {
            self.render_into_bitmap(cue, dst, align);
        }
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

    fn render_into_scribe(
        &self,
        face: &oxideav_scribe::Face,
        cue: &SubtitleCue,
        dst: &mut [u8],
        align: TextAlign,
    ) {
        // 1. Flatten the segment tree to a single text string. Round 1
        //    ignores per-run colour / italic / face; the whole cue
        //    renders in `default_color` upright in the supplied face.
        let text = flatten_to_text(&cue.segments);
        if text.is_empty() {
            return;
        }

        // 2. Hand off to scribe for shaping + word-wrap. Reserve a
        //    side-margin (8 px each side) so glyphs aren't flush
        //    against the canvas edge.
        let side_margin: u32 = 8;
        let max_text_w = self.width.saturating_sub(side_margin * 2);
        if max_text_w == 0 {
            return;
        }
        let size_px = if self.font_size_px.is_finite() && self.font_size_px > 0.0 {
            self.font_size_px
        } else {
            20.0
        };
        let line_bitmaps = match oxideav_scribe::render_text_wrapped(
            face,
            &text,
            size_px,
            self.default_color,
            max_text_w as f32,
        ) {
            Ok(v) => v,
            Err(_) => return,
        };
        if line_bitmaps.is_empty() {
            return;
        }

        // 3. Vertical layout — stack from the bottom up. Use the
        //    user-configured line height when it exceeds the face's
        //    natural one; otherwise honour the face metric so descenders
        //    don't stomp the next line.
        let face_line_h = face.line_height_px(size_px).ceil() as u32;
        let line_h = self.line_height_px.max(face_line_h.max(1));

        let n_lines = line_bitmaps.len();
        // The bottom edge of the last (lowest) line bitmap.
        let last_bottom = self.height.saturating_sub(self.bottom_margin_px);

        for (i, line_bm) in line_bitmaps.iter().enumerate() {
            if line_bm.is_empty() {
                continue;
            }
            // Bottom of this particular line.
            let line_bottom = last_bottom.saturating_sub(((n_lines - 1 - i) as u32) * line_h);
            let y = line_bottom as i32 - line_bm.height as i32;
            let line_w = line_bm.width as i32;
            let x = match align {
                TextAlign::Left | TextAlign::Start => side_margin as i32,
                TextAlign::Right | TextAlign::End => {
                    self.width as i32 - line_w - side_margin as i32
                }
                TextAlign::Center => (self.width as i32 - line_w) / 2,
            };
            blit_rgba_straight(
                dst,
                self.width,
                self.height,
                x,
                y,
                &line_bm.data,
                line_bm.width,
                line_bm.height,
            );
        }
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
// Scribe back-end helpers
// ---------------------------------------------------------------------------

/// Walk the segment tree and flatten it into a single plain string.
/// Style information is discarded — round 1 of the Scribe path renders
/// the whole cue in one colour / one face. Round 2 will revisit this to
/// emit one shaped run per style change and composite them with their
/// respective colours.
fn flatten_to_text(segments: &[Segment]) -> String {
    let mut out = String::new();
    walk_text(segments, &mut out);
    out
}

fn walk_text(segments: &[Segment], out: &mut String) {
    for seg in segments {
        match seg {
            Segment::Text(s) | Segment::Raw(s) => out.push_str(s),
            Segment::LineBreak => out.push('\n'),
            Segment::Bold(c)
            | Segment::Italic(c)
            | Segment::Underline(c)
            | Segment::Strike(c)
            | Segment::Color { children: c, .. }
            | Segment::Font { children: c, .. }
            | Segment::Class { children: c, .. }
            | Segment::Karaoke { children: c, .. } => walk_text(c, out),
            Segment::Voice { name, children } => {
                out.push_str(name);
                out.push_str(": ");
                walk_text(children, out);
            }
            Segment::Timestamp { .. } => {
                // Invisible — emits no characters.
            }
        }
    }
}

/// Composite a straight-alpha RGBA8 source bitmap onto a straight-alpha
/// RGBA8 destination at `(x, y)` (top-left). Pixels outside the
/// destination rectangle are clipped. Blend is Porter-Duff "over" via
/// [`oxideav_pixfmt::over_straight`].
#[allow(clippy::too_many_arguments)]
fn blit_rgba_straight(
    dst: &mut [u8],
    dst_w: u32,
    dst_h: u32,
    x: i32,
    y: i32,
    src: &[u8],
    src_w: u32,
    src_h: u32,
) {
    if dst_w == 0 || dst_h == 0 || src_w == 0 || src_h == 0 {
        return;
    }
    let dx0 = x.max(0);
    let dy0 = y.max(0);
    let dx1 = (x + src_w as i32).min(dst_w as i32);
    let dy1 = (y + src_h as i32).min(dst_h as i32);
    if dx0 >= dx1 || dy0 >= dy1 {
        return;
    }
    let sx0 = (dx0 - x) as usize;
    let sy0 = (dy0 - y) as usize;
    let blit_w = (dx1 - dx0) as usize;
    let blit_h = (dy1 - dy0) as usize;
    let dst_stride = dst_w as usize * 4;
    let src_stride = src_w as usize * 4;
    for row in 0..blit_h {
        let dst_row_off = (dy0 as usize + row) * dst_stride + (dx0 as usize) * 4;
        let src_row_off = (sy0 + row) * src_stride + sx0 * 4;
        for col in 0..blit_w {
            let so = src_row_off + col * 4;
            let s = [src[so], src[so + 1], src[so + 2], src[so + 3]];
            if s[3] == 0 {
                continue;
            }
            let dop = dst_row_off + col * 4;
            let d = [dst[dop], dst[dop + 1], dst[dop + 2], dst[dop + 3]];
            let out = oxideav_pixfmt::over_straight(s, d);
            dst[dop] = out[0];
            dst[dop + 1] = out[1];
            dst[dop + 2] = out[2];
            dst[dop + 3] = out[3];
        }
    }
}

// ---------------------------------------------------------------------------
// Run / line model
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
    fn flatten_to_text_walks_segments() {
        // Bold("Hi") + " " + Italic("there") + LineBreak + Voice("J", "yo")
        let segs = vec![
            Segment::Bold(vec![Segment::Text("Hi".into())]),
            Segment::Text(" ".into()),
            Segment::Italic(vec![Segment::Text("there".into())]),
            Segment::LineBreak,
            Segment::Voice {
                name: "J".into(),
                children: vec![Segment::Text("yo".into())],
            },
        ];
        let txt = flatten_to_text(&segs);
        assert_eq!(txt, "Hi there\nJ: yo");
    }

    #[test]
    fn no_face_means_bitmap_path() {
        let comp = Compositor::new(160, 80);
        assert!(!comp.has_face());
    }

    /// Smoke test: a Compositor built `with_face` from the DejaVu fixture
    /// renders some lit pixels for a simple cue.
    #[test]
    fn scribe_path_renders_text() {
        let bytes = match std::fs::read("../oxideav-ttf/tests/fixtures/DejaVuSans.ttf") {
            Ok(b) => b,
            Err(_) => return, // fixture missing — skip
        };
        let face =
            oxideav_scribe::Face::from_ttf_bytes(bytes).expect("failed to parse DejaVuSans.ttf");
        let mut comp = Compositor::with_face(320, 240, face);
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
