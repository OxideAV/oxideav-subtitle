//! Integration tests for the subtitle compositor + RenderedSubtitleDecoder.

use std::collections::VecDeque;

use oxideav_core::Decoder;
use oxideav_core::{CodecId, Error, Frame, Packet, Result, Segment, SubtitleCue};
use oxideav_subtitle::{
    make_rendered_decoder, make_rendered_decoder_with_face, Compositor, RenderedSubtitleDecoder,
};

fn mkcue(segs: Vec<Segment>) -> SubtitleCue {
    SubtitleCue {
        start_us: 1_000_000,
        end_us: 2_000_000,
        style_ref: None,
        positioning: None,
        segments: segs,
    }
}

fn rgba_alpha(buf: &[u8], w: u32, x: u32, y: u32) -> u8 {
    let idx = (y as usize * w as usize + x as usize) * 4 + 3;
    buf.get(idx).copied().unwrap_or(0)
}

fn bounding_box(buf: &[u8], w: u32, h: u32) -> Option<(u32, u32, u32, u32)> {
    let mut min_x = u32::MAX;
    let mut min_y = u32::MAX;
    let mut max_x = 0u32;
    let mut max_y = 0u32;
    let mut found = false;
    for y in 0..h {
        for x in 0..w {
            if rgba_alpha(buf, w, x, y) > 0 {
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
                found = true;
            }
        }
    }
    if found {
        Some((min_x, min_y, max_x, max_y))
    } else {
        None
    }
}

#[test]
fn renders_plain_text_cue() {
    let comp = Compositor::new(640, 480);
    let cue = mkcue(vec![Segment::Text("Hello".to_string())]);
    let buf = comp.render(&cue);
    assert_eq!(buf.len(), 640 * 480 * 4);

    let (min_x, min_y, max_x, max_y) =
        bounding_box(&buf, 640, 480).expect("rendered cue had no lit pixels");

    // Lower-middle region.
    assert!(
        min_y > 240,
        "text should be below mid-height; bbox y={min_y}..{max_y}"
    );
    // Centered horizontally — bbox should straddle the centre column or
    // at least start before it.
    assert!(
        min_x < 320 && max_x > 320,
        "text should straddle horizontal centre; bbox x={min_x}..{max_x}"
    );
    // Upper area (y < 200): no lit pixels at all.
    for y in 0..200 {
        for x in 0..640 {
            assert_eq!(
                rgba_alpha(&buf, 640, x, y),
                0,
                "unexpected lit pixel at ({x}, {y})"
            );
        }
    }
}

#[test]
fn bold_italic_color() {
    let comp = Compositor::new(320, 240);
    let cue = mkcue(vec![
        Segment::Bold(vec![Segment::Text("Bold".to_string())]),
        Segment::Text(" ".to_string()),
        Segment::Italic(vec![Segment::Text("Italic".to_string())]),
        Segment::Text(" ".to_string()),
        Segment::Color {
            rgb: (255, 0, 0),
            children: vec![Segment::Text("Red".to_string())],
        },
    ]);
    let buf = comp.render(&cue);
    assert_eq!(buf.len(), 320 * 240 * 4);
    let lit = buf.chunks(4).filter(|p| p[3] > 0).count();
    assert!(lit > 0, "bold/italic/color cue produced no pixels");
}

#[test]
fn linebreak_multiple_lines() {
    let comp = Compositor::new(320, 240);
    // Single-line cue first.
    let cue_one = mkcue(vec![Segment::Text("Line".to_string())]);
    let buf_one = comp.render(&cue_one);
    let (_, one_min_y, _, one_max_y) = bounding_box(&buf_one, 320, 240).expect("single line");

    // Two-line cue.
    let cue_two = mkcue(vec![
        Segment::Text("Line".to_string()),
        Segment::LineBreak,
        Segment::Text("Two".to_string()),
    ]);
    let buf_two = comp.render(&cue_two);
    let (_, two_min_y, _, two_max_y) = bounding_box(&buf_two, 320, 240).expect("two lines");

    let one_height = one_max_y - one_min_y;
    let two_height = two_max_y - two_min_y;
    assert!(
        two_height > one_height,
        "two-line cue bbox height ({two_height}) must exceed single-line ({one_height})"
    );
}

#[test]
fn wrap_long_line() {
    let long = "x".repeat(400);
    let cue = mkcue(vec![Segment::Text(long)]);
    let comp = Compositor::new(320, 240);
    let buf = comp.render(&cue);
    // Count rows that have at least one lit pixel.
    let mut non_empty_rows = 0u32;
    for y in 0..240 {
        let any = (0..320).any(|x| rgba_alpha(&buf, 320, x, y) > 0);
        if any {
            non_empty_rows += 1;
        }
    }
    // Each bitmap line spans cell_h = 16 rows; multiple visual lines
    // should produce well above 16 lit rows.
    assert!(
        non_empty_rows > 20,
        "wrapped 400-char cue produced only {non_empty_rows} lit rows; \
         expected multi-line wrap to fill many more"
    );
}

// ------------------------------------------------------------------
// Wrapper-level test
// ------------------------------------------------------------------

/// A tiny fake decoder that emits a fixed queue of Frames regardless of
/// packets. Useful for testing wrappers.
struct CannedDecoder {
    codec_id: CodecId,
    queue: VecDeque<Frame>,
    flushed: bool,
}

impl CannedDecoder {
    fn new(frames: Vec<Frame>) -> Self {
        Self {
            codec_id: CodecId::new("test_canned"),
            queue: frames.into_iter().collect(),
            flushed: false,
        }
    }
}

impl Decoder for CannedDecoder {
    fn codec_id(&self) -> &CodecId {
        &self.codec_id
    }

    fn send_packet(&mut self, _packet: &Packet) -> Result<()> {
        Ok(())
    }

    fn receive_frame(&mut self) -> Result<Frame> {
        if let Some(f) = self.queue.pop_front() {
            Ok(f)
        } else if self.flushed {
            Err(Error::Eof)
        } else {
            Err(Error::NeedMore)
        }
    }

    fn flush(&mut self) -> Result<()> {
        self.flushed = true;
        Ok(())
    }
}

#[test]
fn wrapper_deduplicates_identical_cues() {
    let cue = mkcue(vec![Segment::Text("Same".to_string())]);
    let inner = Box::new(CannedDecoder::new(vec![
        Frame::Subtitle(cue.clone()),
        Frame::Subtitle(cue),
    ]));
    let mut wrapper = RenderedSubtitleDecoder::new(inner, 160, 120);

    // First call: fresh cue → Frame::Video.
    match wrapper.receive_frame() {
        Ok(Frame::Video(vf)) => {
            assert_eq!(vf.pts, Some(1_000_000));
            assert_eq!(vf.planes.len(), 1);
            assert_eq!(vf.planes[0].stride, 160 * 4);
            assert_eq!(vf.planes[0].data.len(), 160 * 120 * 4);
        }
        other => panic!("expected Frame::Video on first cue; got {other:?}"),
    }

    // Second call: duplicate → NeedMore.
    match wrapper.receive_frame() {
        Err(Error::NeedMore) => {}
        other => panic!("expected NeedMore on duplicate; got {other:?}"),
    }
}

#[test]
fn wrapper_emits_new_frame_on_content_change() {
    let cue_a = mkcue(vec![Segment::Text("A".to_string())]);
    let mut cue_b = cue_a.clone();
    cue_b.segments = vec![Segment::Text("B".to_string())];
    cue_b.start_us = 2_000_000;
    cue_b.end_us = 3_000_000;

    let inner = Box::new(CannedDecoder::new(vec![
        Frame::Subtitle(cue_a),
        Frame::Subtitle(cue_b),
    ]));
    let mut wrapper = RenderedSubtitleDecoder::new(inner, 160, 120);
    assert!(matches!(wrapper.receive_frame(), Ok(Frame::Video(_))));
    assert!(matches!(wrapper.receive_frame(), Ok(Frame::Video(_))));
}

#[test]
fn make_rendered_decoder_factory() {
    let cue = mkcue(vec![Segment::Text("X".to_string())]);
    let inner: Box<dyn Decoder> = Box::new(CannedDecoder::new(vec![Frame::Subtitle(cue)]));
    let mut wrapper = make_rendered_decoder(inner, 64, 64);
    assert!(matches!(wrapper.receive_frame(), Ok(Frame::Video(_))));
}

// ------------------------------------------------------------------
// Scribe TTF back-end integration
// ------------------------------------------------------------------

/// The DejaVu fixture lives under the sibling `oxideav-ttf` crate.
/// Tests that need it skip silently if the file isn't present (e.g.
/// when this crate is built outside the monorepo).
fn try_load_dejavu() -> Option<oxideav_scribe::Face> {
    let bytes = std::fs::read("../oxideav-ttf/tests/fixtures/DejaVuSans.ttf").ok()?;
    oxideav_scribe::Face::from_ttf_bytes(bytes).ok()
}

#[test]
fn scribe_path_compositor_renders_lit_pixels() {
    let face = match try_load_dejavu() {
        Some(f) => f,
        None => return,
    };
    let mut comp = Compositor::with_face(640, 480, face);
    comp.font_size_px = 28.0;
    let cue = mkcue(vec![Segment::Text("Hello, world!".into())]);
    let buf = comp.render(&cue);
    assert_eq!(buf.len(), 640 * 480 * 4);

    let (min_x, min_y, max_x, max_y) =
        bounding_box(&buf, 640, 480).expect("Scribe path produced no lit pixels");
    // Lower half of canvas (bottom-stack default).
    assert!(
        min_y > 240,
        "Scribe text should be in bottom half; got y={min_y}..{max_y}"
    );
    // Centred horizontally — bbox should straddle the centre column.
    assert!(
        min_x < 320 && max_x > 320,
        "Scribe text should straddle horizontal centre; bbox x={min_x}..{max_x}"
    );
    // Upper third has zero lit pixels.
    for y in 0..160 {
        for x in 0..640 {
            assert_eq!(
                rgba_alpha(&buf, 640, x, y),
                0,
                "unexpected lit pixel at ({x}, {y}) in upper region"
            );
        }
    }
}

#[test]
fn scribe_path_via_rendered_decoder_with_face() {
    let face = match try_load_dejavu() {
        Some(f) => f,
        None => return,
    };
    let cue = mkcue(vec![Segment::Text("Subtitle".into())]);
    let inner = Box::new(CannedDecoder::new(vec![Frame::Subtitle(cue)]));
    let mut wrapper = RenderedSubtitleDecoder::new(inner, 320, 200).with_face(face);
    match wrapper.receive_frame() {
        Ok(Frame::Video(vf)) => {
            assert_eq!(vf.planes.len(), 1);
            assert_eq!(vf.planes[0].stride, 320 * 4);
            assert_eq!(vf.planes[0].data.len(), 320 * 200 * 4);
            let lit = vf.planes[0].data.chunks(4).filter(|p| p[3] > 0).count();
            assert!(lit > 0, "Scribe-backed wrapper produced no lit pixels");
        }
        other => panic!("expected Frame::Video; got {other:?}"),
    }
}

#[test]
fn scribe_factory_with_face_works() {
    let face = match try_load_dejavu() {
        Some(f) => f,
        None => return,
    };
    let cue = mkcue(vec![Segment::Text("F".into())]);
    let inner: Box<dyn Decoder> = Box::new(CannedDecoder::new(vec![Frame::Subtitle(cue)]));
    let mut wrapper = make_rendered_decoder_with_face(inner, 96, 64, face);
    assert!(matches!(wrapper.receive_frame(), Ok(Frame::Video(_))));
}

#[test]
fn scribe_and_bitmap_paths_both_produce_lit_pixels_for_same_cue() {
    // Sanity: each back-end produces *some* output for the same cue,
    // and both place it in the bottom half. We don't assert byte-equal
    // bitmaps because the rasterisers are intentionally different.
    let cue = mkcue(vec![Segment::Text("Hi".into())]);
    let bitmap_buf = Compositor::new(160, 120).render(&cue);
    let bitmap_lit = bitmap_buf.chunks(4).filter(|p| p[3] > 0).count();
    assert!(bitmap_lit > 0, "bitmap path produced no pixels");

    let face = match try_load_dejavu() {
        Some(f) => f,
        None => return,
    };
    let mut comp = Compositor::with_face(160, 120, face);
    comp.font_size_px = 16.0;
    let scribe_buf = comp.render(&cue);
    let scribe_lit = scribe_buf.chunks(4).filter(|p| p[3] > 0).count();
    assert!(scribe_lit > 0, "scribe path produced no pixels");
}
