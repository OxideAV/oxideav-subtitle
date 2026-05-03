//! Standalone-subtitle codecs + containers for oxideav.
//!
//! Hosts the lightweight text subtitle formats, their cross-format
//! converters, and a text-to-RGBA rendering stack (bitmap font +
//! compositor + `RenderedSubtitleDecoder` wrapper).
//!
//! ASS/SSA lives in its own sibling crate `oxideav-ass` because advanced
//! rendering (animated tags, sub-pixel positioning, karaoke playback)
//! needs substantial work that shouldn't clutter this hub. Bitmap-native
//! subtitle formats (PGS, DVB, VobSub) live in `oxideav-sub-image`.
//!
//! | Format        | Codec id      | Container name | Extensions       |
//! |---------------|---------------|----------------|------------------|
//! | SubRip        | `subrip`      | `srt`          | `.srt`           |
//! | WebVTT        | `webvtt`      | `webvtt`       | `.vtt`           |
//! | MicroDVD      | `microdvd`    | `microdvd`     | `.sub`, `.txt`   |
//! | MPL2          | `mpl2`        | `mpl2`         | `.mpl`           |
//! | MPsub         | `mpsub`       | `mpsub`        | `.sub`           |
//! | VPlayer       | `vplayer`     | `vplayer`      | `.txt`, `.vpl`   |
//! | PJS           | `pjs`         | `pjs`          | `.pjs`           |
//! | AQTitle       | `aqtitle`     | `aqtitle`      | `.aqt`           |
//! | JACOsub       | `jacosub`     | `jacosub`      | `.jss`, `.js`    |
//! | RealText      | `realtext`    | `realtext`     | `.rt`            |
//! | SubViewer 1   | `subviewer1`  | `subviewer1`   | `.sub`           |
//! | SubViewer 2   | `subviewer2`  | `subviewer2`   | `.sub`           |
//! | TTML          | `ttml`        | `ttml`         | `.ttml`, `.dfxp`, `.xml` |
//! | SAMI          | `sami`        | `sami`         | `.smi`, `.sami`  |
//! | EBU STL       | `ebu_stl`     | `ebu_stl`      | `.stl`           |
//!
//! Shared-extension conflicts (several formats use `.sub` / `.txt`) are
//! resolved content-first: every container ships a probe that scores the
//! first few KB of input and the registry picks the highest-scoring match.
//!
//! ## Cross-format conversion
//!
//! * [`transform::srt_to_webvtt`]
//! * [`transform::webvtt_to_srt`]
//!
//! Converters touching ASS live in the `oxideav-ass` crate.
//!
//! ## Text → RGBA rendering
//!
//! Any subtitle decoder that produces `Frame::Subtitle` can be wrapped in
//! a [`RenderedSubtitleDecoder`] that produces `Frame::Video(Rgba)` at a
//! caller-specified canvas size. Cue dedup means the wrapper emits at most
//! one frame per visible-state change.
//!
//! In-container subtitle tracks (MKV / MP4 sub streams) are out of scope —
//! this crate deals with standalone files only.

pub mod aqtitle;
pub mod codec;
pub mod compositor;
pub mod container;
pub mod ebu_stl;
pub mod font;
pub mod ir;
pub mod jacosub;
pub mod microdvd;
pub mod mpl2;
pub mod mpsub;
pub mod pjs;
pub mod realtext;
pub mod render;
pub mod sami;
pub mod srt;
pub mod subviewer1;
pub mod subviewer2;
pub mod transform;
pub mod ttml;
pub mod vplayer;
pub mod webvtt;

use oxideav_core::ContainerRegistry;
use oxideav_core::{CodecCapabilities, CodecId, MediaType};
use oxideav_core::{CodecInfo, CodecRegistry};

pub use compositor::Compositor;
pub use font::BitmapFont;
pub use ir::{SourceFormat, SubtitleTrack};
pub use render::{make_rendered_decoder, make_rendered_decoder_with_face, RenderedSubtitleDecoder};
pub use transform::{srt_to_webvtt, webvtt_to_srt};

/// Shape a shared capability set for a subtitle codec. Every text
/// subtitle registered here is `decode=true, encode=true, intra_only=true,
/// lossless=true, media_type=Subtitle`.
fn subtitle_caps(impl_name: &str) -> CodecCapabilities {
    CodecCapabilities {
        decode: false,
        encode: false,
        media_type: MediaType::Subtitle,
        intra_only: true,
        lossy: false,
        lossless: true,
        hardware_accelerated: false,
        implementation: impl_name.into(),
        max_width: None,
        max_height: None,
        max_bitrate: None,
        max_sample_rate: None,
        max_channels: None,
        priority: 100,
        accepted_pixel_formats: Vec::new(),
    }
}

/// Register all text subtitle codecs (decoders + encoders). Each
/// format's `make_decoder` / `make_encoder` lives in its own module and
/// is registered independently — no dispatch branch to maintain here.
pub fn register_codecs(reg: &mut CodecRegistry) {
    // SRT + WebVTT share codec.rs's dispatcher (legacy).
    for (id, impl_name) in [
        (codec::SRT_CODEC_ID, "subrip_sw"),
        (codec::WEBVTT_CODEC_ID, "webvtt_sw"),
    ] {
        reg.register(
            CodecInfo::new(CodecId::new(id))
                .capabilities(subtitle_caps(impl_name))
                .decoder(codec::make_decoder)
                .encoder(codec::make_encoder),
        );
    }

    // Per-format factories for the rest.
    macro_rules! register_text {
        ($module:ident, $impl_name:literal) => {
            reg.register(
                CodecInfo::new(CodecId::new($module::CODEC_ID))
                    .capabilities(subtitle_caps($impl_name))
                    .decoder($module::make_decoder)
                    .encoder($module::make_encoder),
            );
        };
    }
    register_text!(microdvd, "microdvd_sw");
    register_text!(mpl2, "mpl2_sw");
    register_text!(mpsub, "mpsub_sw");
    register_text!(vplayer, "vplayer_sw");
    register_text!(pjs, "pjs_sw");
    register_text!(aqtitle, "aqtitle_sw");
    register_text!(jacosub, "jacosub_sw");
    register_text!(realtext, "realtext_sw");
    register_text!(subviewer1, "subviewer1_sw");
    register_text!(subviewer2, "subviewer2_sw");
    register_text!(ttml, "ttml_sw");
    register_text!(sami, "sami_sw");
    register_text!(ebu_stl, "ebu_stl_sw");
}

/// Register the text subtitle containers (demuxers + muxers + probes).
pub fn register_containers(reg: &mut ContainerRegistry) {
    container::register(reg);
}

/// Convenience combined registration.
pub fn register(codecs: &mut CodecRegistry, containers: &mut ContainerRegistry) {
    register_codecs(codecs);
    register_containers(containers);
}
