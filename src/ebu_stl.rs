//! EBU STL (EBU Tech 3264 / ISO 18041) subtitle parser + writer.
//!
//! Binary format:
//!
//! * **GSI block** — 1024 bytes of header metadata (code page, disk
//!   format code, frame rate, totals, max row/col).
//! * **TTI blocks** — 128 bytes each, one per subtitle line (extension
//!   blocks via `EBN != 0xFF` are merged into a single cue).
//!
//! GSI fields handled:
//! * `CPN` (code page 3 bytes — interpreted as ASCII/Latin-1 fallback)
//! * `DFC` (8 bytes, disk format code, e.g. `STL25.01` → 25 fps)
//! * `DSC` (1 byte, display standard code — preserved through metadata)
//! * `CCT` (2 bytes, character code table)
//! * `LC` (2 bytes, language code)
//! * `TNB` (total TTI blocks, 5-digit ASCII)
//! * `TNS` (total subtitles, 5-digit ASCII)
//! * `MNC` (max chars per row, 2-digit ASCII)
//! * `MNR` (max rows, 2-digit ASCII)
//! * `TCS` (1 byte, time code status)
//!
//! Each TTI:
//! * SGN (1), SN (2 LE), EBN (1), CS (1)
//! * TCI (4: HH MM SS FF)
//! * TCO (4: HH MM SS FF)
//! * VP (1), JC (1), CF (1)
//! * TF (112) — text field with control codes
//!
//! Text-field control codes (§ Tech 3264):
//! * `0x00..=0x07` — italic/underline/boxing on/off (0x80/0x81 italic
//!   on/off; 0x82/0x83 underline on/off; 0x84/0x85 boxing on/off —
//!   mapping per EBU ARIB rework here uses 0x80..0x87).
//! * `0x8A` — CRLF (line break)
//! * `0x8F` — pad (trailing, ignored)
//!
//! Colour codes are emitted as [`Segment::Raw`] for lossless round-trip
//! in this first cut.
//!
//! ## Round-trip preservation
//!
//! Per-cue TTI fields outside the unified IR — SGN / SN / CS / VP / JC —
//! ride alongside each cue as `ebu_tti.<idx>.<field>` track metadata so
//! a parse → write loop reproduces them byte-faithfully. Extension-block
//! membership (EBN values other than 0xFF / 0x00) is recorded as
//! `ebu_tti.<idx>.ext.<ebn>=1` on the parent cue. Comment-flagged TTI
//! rows (`CF == 1`) survive on the side via `ebu_tti.comment.<n>.*`
//! entries (SGN / SN / EBN / CS / TCI / TCO / VP / JC + a `tf_hex` of
//! the raw 112-byte text field) and are re-emitted after the playable
//! cues on write; they do not enter `track.cues` because they're not
//! playable. GSI bytes DSC / CCT / LC / TCS likewise round-trip
//! through `dsc` / `cct` / `lc` / `tcs` track-level metadata.
//!
//! Text mode only — bitmap subtitles (DFC `STL*.22`) are not decoded.

use oxideav_core::{Error, Result, Segment, SubtitleCue};

use crate::ir::{SourceFormat, SubtitleTrack};

pub const GSI_SIZE: usize = 1024;
pub const TTI_SIZE: usize = 128;
/// Codec id string.
pub const CODEC_ID: &str = "ebu_stl";

/// Parse an EBU STL file into a track.
///
/// Per-cue TTI fields that the unified IR doesn't model — SGN
/// (subtitle group number), SN (subtitle number), CS (cumulative status),
/// EBN (extension block number), VP (vertical position), JC
/// (justification code), CF (comment flag) — survive a parse → write
/// round-trip through per-cue `ebu_tti.<idx>.<field>` track metadata
/// entries. Comment-flagged TTI rows (`CF == 1`) ride alongside as
/// `ebu_tti.<idx>.cf=1` plus the decoded text in `ebu_tti.<idx>.cf_text`
/// so the writer can re-emit them in the same position; they do not
/// appear in `track.cues` because they are not playable content.
pub fn parse(bytes: &[u8]) -> Result<SubtitleTrack> {
    if bytes.len() < GSI_SIZE {
        return Err(Error::invalid("EBU STL: truncated GSI header"));
    }
    let gsi = &bytes[..GSI_SIZE];

    let dfc = std::str::from_utf8(&gsi[3..11])
        .unwrap_or("STL25.01")
        .trim();
    let fps = fps_from_dfc(dfc);

    let cpn = std::str::from_utf8(&gsi[0..3]).unwrap_or("850").trim();
    let dsc = gsi[11];
    let cct = std::str::from_utf8(&gsi[12..14]).unwrap_or("00").trim();
    let lc = std::str::from_utf8(&gsi[14..16]).unwrap_or("00").trim();
    let mnc = ascii_u32(&gsi[248..250]).unwrap_or(40);
    let mnr = ascii_u32(&gsi[250..252]).unwrap_or(23);
    let tnb = ascii_u32(&gsi[238..243]).unwrap_or(0);
    let tns = ascii_u32(&gsi[243..248]).unwrap_or(0);
    let tcs = gsi[252];

    let mut track = SubtitleTrack {
        source: Some(SourceFormat::Srt),
        ..SubtitleTrack::default()
    };
    track
        .metadata
        .push(("source_format".into(), "ebu_stl".into()));
    track.metadata.push(("dfc".into(), dfc.to_string()));
    track.metadata.push(("cpn".into(), cpn.to_string()));
    if dsc != 0 && dsc != b' ' {
        track
            .metadata
            .push(("dsc".into(), (dsc as char).to_string()));
    }
    track.metadata.push(("cct".into(), cct.to_string()));
    track.metadata.push(("lc".into(), lc.to_string()));
    track.metadata.push(("mnc".into(), mnc.to_string()));
    track.metadata.push(("mnr".into(), mnr.to_string()));
    track.metadata.push(("tnb".into(), tnb.to_string()));
    track.metadata.push(("tns".into(), tns.to_string()));
    if tcs != 0 && tcs != b' ' {
        track
            .metadata
            .push(("tcs".into(), (tcs as char).to_string()));
    }

    // Walk TTI records.
    let tail = &bytes[GSI_SIZE..];
    if tail.len() % TTI_SIZE != 0 && tail.len() / TTI_SIZE == 0 {
        return Err(Error::invalid("EBU STL: TTI section empty / not aligned"));
    }

    // Cue index for the metadata keys we attach per TTI row. Tracks the
    // SubtitleCue we just pushed (or merged into); used so the writer can
    // re-emit the same SGN / SN / CS / VP / JC / EBN values.
    let mut cue_idx: i64 = -1;
    // Index for comment-flagged TTI rows. They never become cues, so they
    // are keyed separately under `ebu_tti.comment.<idx>.*`.
    let mut comment_idx: u32 = 0;

    let mut i = 0;
    while i + TTI_SIZE <= tail.len() {
        let rec = &tail[i..i + TTI_SIZE];
        i += TTI_SIZE;

        let sgn = rec[0];
        let sn = u16::from_le_bytes([rec[1], rec[2]]);
        let ebn = rec[3];
        let cs = rec[4];
        let tci = (rec[5], rec[6], rec[7], rec[8]);
        let tco = (rec[9], rec[10], rec[11], rec[12]);
        let vp = rec[13];
        let jc = rec[14];
        let cf = rec[15];
        let tf = &rec[16..128];

        if cf != 0 {
            // Comment-flagged row. Preserve through track metadata so the
            // writer can re-emit at the same byte offset; not playable so
            // it doesn't enter `track.cues`.
            let prefix = format!("ebu_tti.comment.{comment_idx}");
            track
                .metadata
                .push((format!("{prefix}.sgn"), sgn.to_string()));
            track
                .metadata
                .push((format!("{prefix}.sn"), sn.to_string()));
            track
                .metadata
                .push((format!("{prefix}.ebn"), ebn.to_string()));
            track
                .metadata
                .push((format!("{prefix}.cs"), cs.to_string()));
            track
                .metadata
                .push((format!("{prefix}.tci"), tc_to_string(tci)));
            track
                .metadata
                .push((format!("{prefix}.tco"), tc_to_string(tco)));
            track
                .metadata
                .push((format!("{prefix}.vp"), vp.to_string()));
            track
                .metadata
                .push((format!("{prefix}.jc"), jc.to_string()));
            track
                .metadata
                .push((format!("{prefix}.tf_hex"), bytes_to_hex(tf)));
            comment_idx += 1;
            continue;
        }

        let start_us = tc_to_us(tci, fps);
        let end_us = tc_to_us(tco, fps);

        // Extension blocks carry continuation text of the prior cue
        // (EBN values 0x00..0xFE = extension index; 0xFF = last / only).
        if ebn != 0xFF && ebn != 0x00 {
            // Continuation — merge into previous cue.
            if let Some(last) = track.cues.last_mut() {
                let addl = decode_text_field(tf, cpn);
                extend_segments(&mut last.segments, addl);
                // Note the extension EBN under the parent cue so write can
                // emit `ebn != 0xFF` on the continuation row(s).
                if cue_idx >= 0 {
                    let key = format!("ebu_tti.{cue_idx}.ext.{ebn}");
                    track.metadata.push((key, "1".into()));
                }
                continue;
            }
        }

        let segments = decode_text_field(tf, cpn);
        track.cues.push(SubtitleCue {
            start_us,
            end_us,
            style_ref: None,
            positioning: None,
            segments,
        });
        cue_idx += 1;
        let prefix = format!("ebu_tti.{cue_idx}");
        track
            .metadata
            .push((format!("{prefix}.sgn"), sgn.to_string()));
        track
            .metadata
            .push((format!("{prefix}.sn"), sn.to_string()));
        track
            .metadata
            .push((format!("{prefix}.cs"), cs.to_string()));
        track
            .metadata
            .push((format!("{prefix}.vp"), vp.to_string()));
        track
            .metadata
            .push((format!("{prefix}.jc"), jc.to_string()));
    }

    Ok(track)
}

/// Write a track as EBU STL bytes.
///
/// Per-cue TTI fields are taken from `track.metadata` entries the
/// parser populated (`ebu_tti.<idx>.sgn`, `.sn`, `.cs`, `.vp`, `.jc`)
/// so a parse → write round-trip preserves SGN / SN / CS / VP / JC.
/// When those entries are absent (programmatic track construction) the
/// writer falls back to the same defaults the first cut used:
/// SGN=0, SN=index, CS=0, VP=`mnr - 1`, JC=0x02 (centered), EBN=0xFF,
/// CF=0. Comment-flagged rows preserved by the parser as
/// `ebu_tti.comment.<n>.*` are re-emitted in order after the playable
/// cues so they survive byte-faithfully on a flat round-trip; for a
/// strict positional round-trip (interleaved with playable rows) use
/// the higher-level container layer.
pub fn write(track: &SubtitleTrack) -> Result<Vec<u8>> {
    let dfc = meta_or(&track.metadata, "dfc", "STL25.01");
    let cpn = meta_or(&track.metadata, "cpn", "850");
    let mnc: u32 = meta_or(&track.metadata, "mnc", "40").parse().unwrap_or(40);
    let mnr: u32 = meta_or(&track.metadata, "mnr", "23").parse().unwrap_or(23);
    let dsc = meta_or(&track.metadata, "dsc", "1");
    let cct = meta_or(&track.metadata, "cct", "00");
    let lc = meta_or(&track.metadata, "lc", "00");
    let tcs = meta_or(&track.metadata, "tcs", "1");
    let fps = fps_from_dfc(&dfc);

    let mut gsi = [0x20u8; GSI_SIZE];
    write_ascii_fixed(&mut gsi[0..3], &cpn, 3);
    write_ascii_fixed(&mut gsi[3..11], &dfc, 8);
    // DSC — one byte; default '1' (teletext latin) preserved if metadata absent.
    gsi[11] = dsc.as_bytes().first().copied().unwrap_or(b'1');
    write_ascii_fixed(&mut gsi[12..14], &cct, 2);
    write_ascii_fixed(&mut gsi[14..16], &lc, 2);

    // Count playable + comment TTI rows to write.
    let comment_count = count_comment_rows(&track.metadata);
    let tti_rows = track.cues.len() as u32 + comment_count;
    write_ascii_fixed(&mut gsi[238..243], &format!("{:05}", tti_rows), 5);
    write_ascii_fixed(&mut gsi[243..248], &format!("{:05}", track.cues.len()), 5);
    write_ascii_fixed(&mut gsi[248..250], &format!("{:02}", mnc), 2);
    write_ascii_fixed(&mut gsi[250..252], &format!("{:02}", mnr), 2);
    // TCS — one byte; default '1' (hh:mm:ss:ff).
    gsi[252] = tcs.as_bytes().first().copied().unwrap_or(b'1');

    let mut out = Vec::with_capacity(GSI_SIZE + (tti_rows as usize) * TTI_SIZE);
    out.extend_from_slice(&gsi);

    for (idx, cue) in track.cues.iter().enumerate() {
        let mut tti = [0u8; TTI_SIZE];
        let prefix = format!("ebu_tti.{idx}");
        // SGN.
        tti[0] = meta_u8(&track.metadata, &format!("{prefix}.sgn"), 0);
        // SN — preserve original where present, else default to ordinal index.
        let sn = match meta_get(&track.metadata, &format!("{prefix}.sn")) {
            Some(s) => s.parse::<u16>().unwrap_or(idx as u16),
            None => idx as u16,
        };
        let sn_bytes = sn.to_le_bytes();
        tti[1] = sn_bytes[0];
        tti[2] = sn_bytes[1];
        tti[3] = 0xFF; // EBN = last (we don't split extensions here)
        tti[4] = meta_u8(&track.metadata, &format!("{prefix}.cs"), 0);
        let tci = us_to_tc(cue.start_us, fps);
        tti[5] = tci.0;
        tti[6] = tci.1;
        tti[7] = tci.2;
        tti[8] = tci.3;
        let tco = us_to_tc(cue.end_us, fps);
        tti[9] = tco.0;
        tti[10] = tco.1;
        tti[11] = tco.2;
        tti[12] = tco.3;
        tti[13] = meta_u8(
            &track.metadata,
            &format!("{prefix}.vp"),
            (mnr as u8).saturating_sub(1),
        );
        tti[14] = meta_u8(&track.metadata, &format!("{prefix}.jc"), 0x02);
        tti[15] = 0; // CF — playable row.
                     // Fill text field.
        let encoded = encode_text_field(&cue.segments, &cpn);
        let copy_len = encoded.len().min(112);
        tti[16..16 + copy_len].copy_from_slice(&encoded[..copy_len]);
        // Pad with 0x8F.
        for b in &mut tti[16 + copy_len..128] {
            *b = 0x8F;
        }
        out.extend_from_slice(&tti);
    }

    // Append comment-flagged rows the parser stashed in metadata.
    for n in 0..comment_count {
        let mut tti = [0x8Fu8; TTI_SIZE];
        let prefix = format!("ebu_tti.comment.{n}");
        tti[0] = meta_u8(&track.metadata, &format!("{prefix}.sgn"), 0);
        let sn: u16 = meta_get(&track.metadata, &format!("{prefix}.sn"))
            .and_then(|s| s.parse::<u16>().ok())
            .unwrap_or(0);
        let sn_bytes = sn.to_le_bytes();
        tti[1] = sn_bytes[0];
        tti[2] = sn_bytes[1];
        tti[3] = meta_u8(&track.metadata, &format!("{prefix}.ebn"), 0xFF);
        tti[4] = meta_u8(&track.metadata, &format!("{prefix}.cs"), 0);
        if let Some(s) = meta_get(&track.metadata, &format!("{prefix}.tci")) {
            let tc = parse_tc_string(&s);
            tti[5] = tc.0;
            tti[6] = tc.1;
            tti[7] = tc.2;
            tti[8] = tc.3;
        }
        if let Some(s) = meta_get(&track.metadata, &format!("{prefix}.tco")) {
            let tc = parse_tc_string(&s);
            tti[9] = tc.0;
            tti[10] = tc.1;
            tti[11] = tc.2;
            tti[12] = tc.3;
        }
        tti[13] = meta_u8(&track.metadata, &format!("{prefix}.vp"), 0);
        tti[14] = meta_u8(&track.metadata, &format!("{prefix}.jc"), 0);
        tti[15] = 1; // CF — comment.
        if let Some(s) = meta_get(&track.metadata, &format!("{prefix}.tf_hex")) {
            if let Some(buf) = hex_to_bytes(&s) {
                let n = buf.len().min(112);
                tti[16..16 + n].copy_from_slice(&buf[..n]);
                // Pad any remaining bytes with 0x8F.
                for b in &mut tti[16 + n..128] {
                    *b = 0x8F;
                }
            }
        }
        out.extend_from_slice(&tti);
    }

    Ok(out)
}

/// Probe score — needs the full GSI header to be confident.
pub fn probe(buf: &[u8]) -> u8 {
    looks_like_ebu_stl(buf)
}

pub fn looks_like_ebu_stl(buf: &[u8]) -> u8 {
    if buf.len() < 16 {
        return 0;
    }
    // CPN: 3 ASCII digits (e.g. "850", "437").
    let cpn_ok = buf[..3].iter().all(|&b| b.is_ascii_digit());
    // DFC begins with "STL".
    let dfc_ok = &buf[3..6] == b"STL";
    let mut score = 0u8;
    if cpn_ok {
        score += 25;
    }
    if dfc_ok {
        score += 70;
    }
    // DSC is an ASCII char.
    if buf.len() >= 12 && buf[11].is_ascii() && !buf[11].is_ascii_control() {
        score = score.saturating_add(5);
    }
    score.min(100)
}

pub fn make_decoder(
    params: &oxideav_core::CodecParameters,
) -> Result<Box<dyn oxideav_core::Decoder>> {
    crate::codec::make_decoder(params)
}

pub fn make_encoder(
    params: &oxideav_core::CodecParameters,
) -> Result<Box<dyn oxideav_core::Encoder>> {
    crate::codec::make_encoder(params)
}

// ---------------------------------------------------------------------------
// Cue <-> bytes helpers (used by the codec wiring).

pub(crate) fn cue_to_bytes(cue: &SubtitleCue) -> Vec<u8> {
    // For the per-packet transport, emit a single TTI row (128 bytes).
    let fps = 25.0; // default if not known
    let mut tti = [0u8; TTI_SIZE];
    tti[3] = 0xFF; // EBN = last
    let tci = us_to_tc(cue.start_us, fps);
    tti[5] = tci.0;
    tti[6] = tci.1;
    tti[7] = tci.2;
    tti[8] = tci.3;
    let tco = us_to_tc(cue.end_us, fps);
    tti[9] = tco.0;
    tti[10] = tco.1;
    tti[11] = tco.2;
    tti[12] = tco.3;
    tti[14] = 0x02; // centered
    let encoded = encode_text_field(&cue.segments, "850");
    let copy_len = encoded.len().min(112);
    tti[16..16 + copy_len].copy_from_slice(&encoded[..copy_len]);
    for b in &mut tti[16 + copy_len..128] {
        *b = 0x8F;
    }
    tti.to_vec()
}

pub(crate) fn bytes_to_cue(bytes: &[u8]) -> Result<SubtitleCue> {
    if bytes.len() < TTI_SIZE {
        return Err(Error::invalid("EBU STL TTI: short"));
    }
    let rec = &bytes[..TTI_SIZE];
    let tci = (rec[5], rec[6], rec[7], rec[8]);
    let tco = (rec[9], rec[10], rec[11], rec[12]);
    let tf = &rec[16..128];
    let fps = 25.0;
    Ok(SubtitleCue {
        start_us: tc_to_us(tci, fps),
        end_us: tc_to_us(tco, fps),
        style_ref: None,
        positioning: None,
        segments: decode_text_field(tf, "850"),
    })
}

// ---------------------------------------------------------------------------
// Text-field codec.

fn decode_text_field(tf: &[u8], _cpn: &str) -> Vec<Segment> {
    // Flat run-based decoder. Each time a style flips we close the
    // current run (text) and push it wrapped in the appropriate
    // Segment::Italic / Underline nests. Colour / unknown control bytes
    // are emitted as Raw so a round-trip can replay them.
    let mut segs: Vec<Segment> = Vec::new();
    let mut run = String::new();
    let mut stack_italic = false;
    let mut stack_underline = false;

    fn flush(buf: &mut String, it: bool, un: bool, out: &mut Vec<Segment>) {
        if buf.is_empty() {
            return;
        }
        let text = std::mem::take(buf);
        let mut node: Vec<Segment> = vec![Segment::Text(text)];
        if un {
            node = vec![Segment::Underline(node)];
        }
        if it {
            node = vec![Segment::Italic(node)];
        }
        out.extend(node);
    }

    for &b in tf {
        if b == 0x8F {
            // Trailing pad — stop.
            break;
        }
        if b == 0x8A {
            // Line break.
            flush(&mut run, stack_italic, stack_underline, &mut segs);
            segs.push(Segment::LineBreak);
            continue;
        }
        // Style toggles (one common mapping — 0x80..0x87 are attribute
        // start/end in Tech 3264; we map italic + underline + boxing).
        match b {
            0x80 => {
                // Italic on.
                flush(&mut run, stack_italic, stack_underline, &mut segs);
                stack_italic = true;
                continue;
            }
            0x81 => {
                // Italic off.
                flush(&mut run, stack_italic, stack_underline, &mut segs);
                stack_italic = false;
                continue;
            }
            0x82 => {
                // Underline on.
                flush(&mut run, stack_italic, stack_underline, &mut segs);
                stack_underline = true;
                continue;
            }
            0x83 => {
                // Underline off.
                flush(&mut run, stack_italic, stack_underline, &mut segs);
                stack_underline = false;
                continue;
            }
            _ => {}
        }
        // Colour codes 0x00..0x07 (teletext) — preserve raw for round-trip.
        if b <= 0x07 {
            flush(&mut run, stack_italic, stack_underline, &mut segs);
            segs.push(Segment::Raw(format!("\\x{:02X}", b)));
            continue;
        }
        // Other control codes 0x08..0x1F, 0x84..0x8F (excluding the ones
        // handled above).
        if b < 0x20 || (0x80..=0x9F).contains(&b) {
            flush(&mut run, stack_italic, stack_underline, &mut segs);
            segs.push(Segment::Raw(format!("\\x{:02X}", b)));
            continue;
        }
        // Printable — Latin-1 style (our simplified CCIR-1 interpretation
        // for this first cut).
        run.push(b as char);
    }
    flush(&mut run, stack_italic, stack_underline, &mut segs);
    segs
}

fn encode_text_field(segs: &[Segment], _cpn: &str) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    let mut italic = false;
    let mut underline = false;
    walk_encode(segs, &mut out, &mut italic, &mut underline);
    // If styles are still open, close them.
    if italic {
        out.push(0x81);
    }
    if underline {
        out.push(0x83);
    }
    out
}

fn walk_encode(segs: &[Segment], out: &mut Vec<u8>, italic: &mut bool, underline: &mut bool) {
    for s in segs {
        match s {
            Segment::Text(t) => {
                for c in t.chars() {
                    if (c as u32) <= 0xFF {
                        let b = c as u8;
                        // Avoid emitting a control byte as text.
                        if b < 0x20 || (0x80..=0x9F).contains(&b) {
                            // Replace with '?'.
                            out.push(b'?');
                        } else {
                            out.push(b);
                        }
                    } else {
                        out.push(b'?');
                    }
                }
            }
            Segment::LineBreak => out.push(0x8A),
            Segment::Italic(c) => {
                if !*italic {
                    out.push(0x80);
                    *italic = true;
                }
                walk_encode(c, out, italic, underline);
                out.push(0x81);
                *italic = false;
            }
            Segment::Bold(c) => walk_encode(c, out, italic, underline),
            Segment::Underline(c) => {
                if !*underline {
                    out.push(0x82);
                    *underline = true;
                }
                walk_encode(c, out, italic, underline);
                out.push(0x83);
                *underline = false;
            }
            Segment::Strike(c)
            | Segment::Color { children: c, .. }
            | Segment::Font { children: c, .. }
            | Segment::Voice { children: c, .. }
            | Segment::Class { children: c, .. }
            | Segment::Karaoke { children: c, .. } => {
                walk_encode(c, out, italic, underline);
            }
            Segment::Timestamp { .. } => {}
            Segment::Raw(r) => {
                // Support the `\xNN` placeholder we emit on decode.
                if let Some(rest) = r.strip_prefix("\\x") {
                    if rest.len() == 2 {
                        if let Ok(v) = u8::from_str_radix(rest, 16) {
                            out.push(v);
                            continue;
                        }
                    }
                }
                // Otherwise, write printable chars from the string.
                for c in r.chars() {
                    if (c as u32) <= 0xFF && ((c as u8) >= 0x20) {
                        out.push(c as u8);
                    }
                }
            }
        }
    }
}

/// Merge a new segment list onto the end of an existing one, inserting a
/// line break between them (used for extension blocks).
fn extend_segments(dst: &mut Vec<Segment>, mut addl: Vec<Segment>) {
    if !dst.is_empty() {
        dst.push(Segment::LineBreak);
    }
    dst.append(&mut addl);
}

// ---------------------------------------------------------------------------
// Timecode helpers.

fn fps_from_dfc(dfc: &str) -> f32 {
    // DFC examples: "STL25.01", "STL30.01", "STL24.01".
    let trimmed = dfc.trim();
    if trimmed.starts_with("STL") && trimmed.len() >= 5 {
        let body = &trimmed[3..];
        let num: String = body.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(v) = num.parse::<u32>() {
            return v as f32;
        }
    }
    25.0
}

fn tc_to_us(tc: (u8, u8, u8, u8), fps: f32) -> i64 {
    let (h, m, s, f) = tc;
    let fps = fps.max(1.0);
    let frame_us = 1_000_000.0 / fps;
    (h as i64) * 3_600_000_000
        + (m as i64) * 60_000_000
        + (s as i64) * 1_000_000
        + (f as i64) * (frame_us as i64)
}

fn us_to_tc(us: i64, fps: f32) -> (u8, u8, u8, u8) {
    let us = us.max(0);
    let fps = fps.max(1.0);
    let total_s = us / 1_000_000;
    let remain_us = us - total_s * 1_000_000;
    let frames = ((remain_us as f64) * (fps as f64) / 1_000_000.0).floor() as i64;
    let h = (total_s / 3_600) as u8;
    let m = ((total_s / 60) % 60) as u8;
    let s = (total_s % 60) as u8;
    let f = (frames as u8).min((fps as u8).saturating_sub(1).max(24));
    (h, m, s, f)
}

// ---------------------------------------------------------------------------
// GSI helpers.

fn ascii_u32(bytes: &[u8]) -> Option<u32> {
    let s = std::str::from_utf8(bytes).ok()?.trim();
    s.parse::<u32>().ok()
}

fn write_ascii_fixed(dst: &mut [u8], s: &str, len: usize) {
    let bytes = s.as_bytes();
    let copy_len = bytes.len().min(len);
    for (i, b) in dst.iter_mut().enumerate().take(len) {
        *b = if i < copy_len { bytes[i] } else { b' ' };
    }
}

fn meta_or(meta: &[(String, String)], key: &str, fallback: &str) -> String {
    meta.iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.clone())
        .unwrap_or_else(|| fallback.to_string())
}

fn meta_get(meta: &[(String, String)], key: &str) -> Option<String> {
    meta.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
}

/// Parse a metadata-stored u8 as decimal; falls back to `default` on
/// missing or unparseable entries.
fn meta_u8(meta: &[(String, String)], key: &str, default: u8) -> u8 {
    meta_get(meta, key)
        .and_then(|s| s.parse::<u8>().ok())
        .unwrap_or(default)
}

/// Count `ebu_tti.comment.<n>.*` entries by finding the highest contiguous
/// `n` for which any keyed subfield exists.
fn count_comment_rows(meta: &[(String, String)]) -> u32 {
    let mut n: u32 = 0;
    loop {
        let prefix = format!("ebu_tti.comment.{n}.");
        if meta.iter().any(|(k, _)| k.starts_with(&prefix)) {
            n += 1;
        } else {
            return n;
        }
    }
}

fn tc_to_string(tc: (u8, u8, u8, u8)) -> String {
    format!("{:02}:{:02}:{:02}:{:02}", tc.0, tc.1, tc.2, tc.3)
}

/// Parse a `HH:MM:SS:FF` string back into a tuple; returns zeros on
/// malformed input rather than failing, matching the lenient "preserve
/// what we can" stance the rest of this module takes.
fn parse_tc_string(s: &str) -> (u8, u8, u8, u8) {
    let parts: Vec<u8> = s.split(':').filter_map(|p| p.parse::<u8>().ok()).collect();
    if parts.len() == 4 {
        (parts[0], parts[1], parts[2], parts[3])
    } else {
        (0, 0, 0, 0)
    }
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{:02X}", b));
    }
    s
}

fn hex_to_bytes(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    let bytes = s.as_bytes();
    let mut i = 0;
    while i + 2 <= bytes.len() {
        let chunk = std::str::from_utf8(&bytes[i..i + 2]).ok()?;
        let b = u8::from_str_radix(chunk, 16).ok()?;
        out.push(b);
        i += 2;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_synthetic() -> Vec<u8> {
        let mut out = vec![0x20u8; GSI_SIZE];
        out[0..3].copy_from_slice(b"850");
        out[3..11].copy_from_slice(b"STL25.01");
        out[11] = b'1';
        out[12..14].copy_from_slice(b"00");
        out[14..16].copy_from_slice(b"00");
        // TNB = 2, TNS = 2, MNC = 40, MNR = 23.
        out[238..243].copy_from_slice(b"00002");
        out[243..248].copy_from_slice(b"00002");
        out[248..250].copy_from_slice(b"40");
        out[250..252].copy_from_slice(b"23");
        out[252] = b'1';

        // Two TTI blocks.
        for (idx, (tci, tco, text)) in [
            ((0u8, 0u8, 1u8, 0u8), (0u8, 0u8, 3u8, 0u8), "Hello world"),
            ((0u8, 0u8, 5u8, 0u8), (0u8, 0u8, 7u8, 0u8), "Second line"),
        ]
        .iter()
        .enumerate()
        {
            let mut tti = [0u8; TTI_SIZE];
            tti[0] = 0;
            let sn = (idx as u16).to_le_bytes();
            tti[1] = sn[0];
            tti[2] = sn[1];
            tti[3] = 0xFF;
            tti[4] = 0;
            tti[5] = tci.0;
            tti[6] = tci.1;
            tti[7] = tci.2;
            tti[8] = tci.3;
            tti[9] = tco.0;
            tti[10] = tco.1;
            tti[11] = tco.2;
            tti[12] = tco.3;
            tti[13] = 22;
            tti[14] = 0x02;
            tti[15] = 0;
            let text_bytes = text.as_bytes();
            tti[16..16 + text_bytes.len()].copy_from_slice(text_bytes);
            for b in &mut tti[16 + text_bytes.len()..128] {
                *b = 0x8F;
            }
            out.extend_from_slice(&tti);
        }
        out
    }

    #[test]
    fn parse_synthetic() {
        let buf = build_synthetic();
        let t = parse(&buf).unwrap();
        assert_eq!(t.cues.len(), 2);
        assert_eq!(t.cues[0].start_us, 1_000_000);
        assert_eq!(t.cues[0].end_us, 3_000_000);
        match &t.cues[0].segments[0] {
            Segment::Text(s) => assert_eq!(s, "Hello world"),
            other => panic!("expected text, got {other:?}"),
        }
    }

    #[test]
    fn roundtrip_synthetic() {
        let buf = build_synthetic();
        let t = parse(&buf).unwrap();
        let out = write(&t).unwrap();
        assert_eq!(out.len(), GSI_SIZE + 2 * TTI_SIZE);
        // Parse again and check timings preserved.
        let t2 = parse(&out).unwrap();
        assert_eq!(t2.cues.len(), 2);
        assert_eq!(t2.cues[0].start_us, 1_000_000);
        assert_eq!(t2.cues[1].start_us, 5_000_000);
    }

    #[test]
    fn probe_detects() {
        let buf = build_synthetic();
        assert!(probe(&buf) > 60);
        assert_eq!(probe(b"1\n00:00:01,000"), 0);
    }

    #[test]
    fn italic_encoded_as_control_bytes() {
        let cue = SubtitleCue {
            start_us: 0,
            end_us: 1_000_000,
            style_ref: None,
            positioning: None,
            segments: vec![Segment::Italic(vec![Segment::Text("hi".into())])],
        };
        let track = SubtitleTrack {
            cues: vec![cue],
            ..SubtitleTrack::default()
        };
        let out = write(&track).unwrap();
        let tti = &out[GSI_SIZE..GSI_SIZE + TTI_SIZE];
        let tf = &tti[16..];
        assert_eq!(tf[0], 0x80, "expected italic-on at start of TF");
    }

    /// Build a buffer with a single TTI row carrying non-default SGN /
    /// SN / CS / VP / JC values, so we can confirm the parser captures
    /// them into per-cue metadata.
    fn build_with_fields(sgn: u8, sn: u16, cs: u8, vp: u8, jc: u8) -> Vec<u8> {
        let mut out = vec![0x20u8; GSI_SIZE];
        out[0..3].copy_from_slice(b"850");
        out[3..11].copy_from_slice(b"STL25.01");
        out[11] = b'1';
        out[12..14].copy_from_slice(b"00");
        out[14..16].copy_from_slice(b"00");
        out[238..243].copy_from_slice(b"00001");
        out[243..248].copy_from_slice(b"00001");
        out[248..250].copy_from_slice(b"40");
        out[250..252].copy_from_slice(b"23");
        out[252] = b'1';
        let mut tti = [0x8Fu8; TTI_SIZE];
        tti[0] = sgn;
        let sn_le = sn.to_le_bytes();
        tti[1] = sn_le[0];
        tti[2] = sn_le[1];
        tti[3] = 0xFF;
        tti[4] = cs;
        tti[5..9].copy_from_slice(&[0, 0, 1, 0]);
        tti[9..13].copy_from_slice(&[0, 0, 2, 0]);
        tti[13] = vp;
        tti[14] = jc;
        tti[15] = 0;
        // Text "Hi" at offset 16, pad bytes 0x8F follow.
        tti[16] = b'H';
        tti[17] = b'i';
        out.extend_from_slice(&tti);
        out
    }

    #[test]
    fn parse_captures_tti_fields_into_per_cue_metadata() {
        let buf = build_with_fields(0x07, 42, 0x03, 11, 0x01);
        let t = parse(&buf).unwrap();
        assert_eq!(t.cues.len(), 1);
        let get = |k: &str| {
            t.metadata
                .iter()
                .find(|(kk, _)| kk == k)
                .map(|(_, v)| v.clone())
        };
        assert_eq!(get("ebu_tti.0.sgn").as_deref(), Some("7"));
        assert_eq!(get("ebu_tti.0.sn").as_deref(), Some("42"));
        assert_eq!(get("ebu_tti.0.cs").as_deref(), Some("3"));
        assert_eq!(get("ebu_tti.0.vp").as_deref(), Some("11"));
        assert_eq!(get("ebu_tti.0.jc").as_deref(), Some("1"));
    }

    #[test]
    fn write_replays_captured_tti_fields_byte_exact() {
        let buf = build_with_fields(0x05, 0x1234, 0x02, 17, 0x03);
        let t = parse(&buf).unwrap();
        let out = write(&t).unwrap();
        let tti = &out[GSI_SIZE..GSI_SIZE + TTI_SIZE];
        assert_eq!(tti[0], 0x05, "SGN preserved");
        let sn = u16::from_le_bytes([tti[1], tti[2]]);
        assert_eq!(sn, 0x1234, "SN preserved");
        assert_eq!(tti[4], 0x02, "CS preserved");
        assert_eq!(tti[13], 17, "VP preserved");
        assert_eq!(tti[14], 0x03, "JC preserved");
    }

    #[test]
    fn write_uses_safe_defaults_for_programmatic_track() {
        // No `ebu_tti.<idx>.*` metadata — falls back to SGN=0, SN=idx,
        // CS=0, VP=mnr-1, JC=0x02. Matches the round-1 first-cut writer.
        let cue = SubtitleCue {
            start_us: 0,
            end_us: 1_000_000,
            style_ref: None,
            positioning: None,
            segments: vec![Segment::Text("hi".into())],
        };
        let track = SubtitleTrack {
            cues: vec![cue],
            ..SubtitleTrack::default()
        };
        let out = write(&track).unwrap();
        let tti = &out[GSI_SIZE..GSI_SIZE + TTI_SIZE];
        assert_eq!(tti[0], 0, "default SGN");
        assert_eq!(u16::from_le_bytes([tti[1], tti[2]]), 0, "default SN = idx");
        assert_eq!(tti[4], 0, "default CS");
        assert_eq!(tti[13], 22, "default VP = mnr - 1 = 22");
        assert_eq!(tti[14], 0x02, "default JC = centered");
    }

    #[test]
    fn comment_flagged_rows_round_trip_via_metadata() {
        // GSI + one playable + one comment TTI row.
        let mut buf = vec![0x20u8; GSI_SIZE];
        buf[0..3].copy_from_slice(b"850");
        buf[3..11].copy_from_slice(b"STL25.01");
        buf[11] = b'1';
        buf[12..16].copy_from_slice(b"0000");
        buf[238..243].copy_from_slice(b"00002");
        buf[243..248].copy_from_slice(b"00001");
        buf[248..250].copy_from_slice(b"40");
        buf[250..252].copy_from_slice(b"23");
        buf[252] = b'1';
        // Playable.
        let mut p = [0x8Fu8; TTI_SIZE];
        p[3] = 0xFF;
        p[5..9].copy_from_slice(&[0, 0, 1, 0]);
        p[9..13].copy_from_slice(&[0, 0, 2, 0]);
        p[13] = 22;
        p[14] = 0x02;
        p[15] = 0;
        p[16] = b'A';
        buf.extend_from_slice(&p);
        // Comment row.
        let mut c = [0x8Fu8; TTI_SIZE];
        c[0] = 0x42; // SGN
        c[1] = 0x99; // SN low
        c[2] = 0x00; // SN high
        c[3] = 0xFF;
        c[4] = 0x05; // CS
        c[5..9].copy_from_slice(&[0, 0, 0, 0]);
        c[9..13].copy_from_slice(&[0, 0, 0, 0]);
        c[13] = 0;
        c[14] = 0;
        c[15] = 1; // CF — comment.
        c[16] = b'X';
        c[17] = b'Y';
        c[18] = b'Z';
        buf.extend_from_slice(&c);

        let t = parse(&buf).unwrap();
        assert_eq!(t.cues.len(), 1, "comment row is not a cue");
        let get = |k: &str| {
            t.metadata
                .iter()
                .find(|(kk, _)| kk == k)
                .map(|(_, v)| v.clone())
        };
        assert_eq!(get("ebu_tti.comment.0.sgn").as_deref(), Some("66"));
        assert_eq!(get("ebu_tti.comment.0.sn").as_deref(), Some("153"));
        assert_eq!(get("ebu_tti.comment.0.cs").as_deref(), Some("5"));
        let tf_hex = get("ebu_tti.comment.0.tf_hex").unwrap();
        assert!(
            tf_hex.starts_with("58595A8F"),
            "TF hex starts XYZ + pad, got {tf_hex}"
        );

        // Write back; expect the comment row re-emitted after the
        // playable row, byte-faithful in the captured fields.
        let out = write(&t).unwrap();
        assert_eq!(out.len(), GSI_SIZE + 2 * TTI_SIZE);
        let written_comment = &out[GSI_SIZE + TTI_SIZE..GSI_SIZE + 2 * TTI_SIZE];
        assert_eq!(written_comment[15], 1, "CF flag re-emitted");
        assert_eq!(written_comment[0], 0x42, "comment SGN preserved");
        let sn = u16::from_le_bytes([written_comment[1], written_comment[2]]);
        assert_eq!(sn, 0x99, "comment SN preserved");
        assert_eq!(written_comment[4], 0x05, "comment CS preserved");
        assert_eq!(
            &written_comment[16..19],
            b"XYZ",
            "comment TF body preserved"
        );
    }

    #[test]
    fn extension_block_marker_recorded_on_parent_cue() {
        // Build a row with EBN=0 plus a continuation row with EBN=1.
        let mut buf = vec![0x20u8; GSI_SIZE];
        buf[0..3].copy_from_slice(b"850");
        buf[3..11].copy_from_slice(b"STL25.01");
        buf[11] = b'1';
        buf[12..16].copy_from_slice(b"0000");
        buf[238..243].copy_from_slice(b"00002");
        buf[243..248].copy_from_slice(b"00001");
        buf[248..250].copy_from_slice(b"40");
        buf[250..252].copy_from_slice(b"23");
        buf[252] = b'1';
        // Row 1, EBN=0 (last/only for this cue if no continuation). CF=0
        // explicitly — TTI header bytes default to 0 in real STL files,
        // not 0x8F (the TF pad byte).
        let mut row1 = [0x8Fu8; TTI_SIZE];
        row1[..16].fill(0);
        row1[3] = 0x00;
        row1[5..9].copy_from_slice(&[0, 0, 1, 0]);
        row1[9..13].copy_from_slice(&[0, 0, 3, 0]);
        row1[13] = 22;
        row1[14] = 0x02;
        row1[16] = b'A';
        buf.extend_from_slice(&row1);
        // Continuation row, EBN=1.
        let mut row2 = [0x8Fu8; TTI_SIZE];
        row2[..16].fill(0);
        row2[3] = 0x01;
        row2[5..9].copy_from_slice(&[0, 0, 1, 0]);
        row2[9..13].copy_from_slice(&[0, 0, 3, 0]);
        row2[13] = 22;
        row2[14] = 0x02;
        row2[16] = b'B';
        buf.extend_from_slice(&row2);

        let t = parse(&buf).unwrap();
        assert_eq!(t.cues.len(), 1, "extension merges into one cue");
        let has = t.metadata.iter().any(|(k, _)| k == "ebu_tti.0.ext.1");
        assert!(has, "extension EBN marker recorded on parent cue");
    }

    #[test]
    fn dsc_cct_lc_tcs_round_trip_through_metadata() {
        let mut buf = vec![0x20u8; GSI_SIZE];
        buf[0..3].copy_from_slice(b"850");
        buf[3..11].copy_from_slice(b"STL25.01");
        // DSC = '2' (open subtitling for hard-of-hearing).
        buf[11] = b'2';
        // CCT = '0A' (Latin alphabet sample).
        buf[12..14].copy_from_slice(b"0A");
        // LC = '09' (English sample).
        buf[14..16].copy_from_slice(b"09");
        buf[238..243].copy_from_slice(b"00001");
        buf[243..248].copy_from_slice(b"00001");
        buf[248..250].copy_from_slice(b"40");
        buf[250..252].copy_from_slice(b"23");
        // TCS = '2'.
        buf[252] = b'2';
        let mut tti = [0x8Fu8; TTI_SIZE];
        tti[..16].fill(0);
        tti[3] = 0xFF;
        tti[5..9].copy_from_slice(&[0, 0, 1, 0]);
        tti[9..13].copy_from_slice(&[0, 0, 2, 0]);
        tti[13] = 22;
        tti[14] = 0x02;
        tti[16] = b'X';
        buf.extend_from_slice(&tti);

        let t = parse(&buf).unwrap();
        let get = |k: &str| {
            t.metadata
                .iter()
                .find(|(kk, _)| kk == k)
                .map(|(_, v)| v.clone())
        };
        assert_eq!(get("dsc").as_deref(), Some("2"));
        assert_eq!(get("cct").as_deref(), Some("0A"));
        assert_eq!(get("lc").as_deref(), Some("09"));
        assert_eq!(get("tcs").as_deref(), Some("2"));

        let out = write(&t).unwrap();
        assert_eq!(out[11], b'2', "DSC preserved");
        assert_eq!(&out[12..14], b"0A", "CCT preserved");
        assert_eq!(&out[14..16], b"09", "LC preserved");
        assert_eq!(out[252], b'2', "TCS preserved");
    }

    #[test]
    fn hex_helpers_round_trip() {
        let raw = [0x00u8, 0x42, 0xFF, 0xA0, 0x8F, 0x80];
        let hex = bytes_to_hex(&raw);
        assert_eq!(hex, "0042FFA08F80");
        let back = hex_to_bytes(&hex).unwrap();
        assert_eq!(back, raw);
        assert!(hex_to_bytes("ZZ").is_none(), "non-hex returns None");
        assert!(hex_to_bytes("abc").is_none(), "odd length returns None");
    }
}
