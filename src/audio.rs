//! Audio without FFmpeg: decoding (symphonia), MP3 encoding (LAME), AAC encoding (FDK) and file lengths.
//!
//! Lengths are computed the way ffprobe reported them (the Python app asked ffprobe), because they reach the AI
//! prompts ("It is 3:41 long") and the export timings.

use std::fs::File;
use std::path::Path;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

use crate::mp4;

/// Decoded audio: interleaved samples in [-1, 1].
pub struct Pcm {
    pub rate: u32,
    pub channels: usize,
    pub samples: Vec<f32>,
}

impl Pcm {
    pub fn frames(&self) -> usize {
        self.samples.len() / self.channels.max(1)
    }
}

/// Decode a whole audio file (mp3, m4a/aac, wav, flac, ogg vorbis, aiff, alac). Encoder delay and padding are
/// removed, as FFmpeg does, so the audio starts exactly at its first real sample.
pub fn decode(path: &Path) -> Result<Pcm, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let format_opts = FormatOptions { enable_gapless: true, ..Default::default() };
    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &format_opts, &MetadataOptions::default())
        .map_err(|e| format!("unsupported audio format ({})", e))?;
    let mut format = probed.format;
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL && t.codec_params.sample_rate.is_some())
        .ok_or("no audio track")?;
    let track_id = track.id;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| format!("unsupported audio codec ({})", e))?;

    let mut rate = track.codec_params.sample_rate.unwrap_or(44100);
    let mut channels = 0usize;
    let mut samples: Vec<f32> = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(SymError::ResetRequired) => break,
            Err(e) => return Err(e.to_string()),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(SymError::DecodeError(_)) => continue,
            Err(e) => return Err(e.to_string()),
        };
        let spec = *decoded.spec();
        rate = spec.rate;
        let ch = spec.channels.count();
        if channels == 0 {
            channels = ch;
        }
        let mut buf = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
        buf.copy_interleaved_ref(decoded);
        let frames = buf.samples().len() / ch;
        // Gapless trimming
        let start = (packet.trim_start as usize).min(frames);
        let end = frames.saturating_sub(packet.trim_end as usize).max(start);
        let data = &buf.samples()[start * ch..end * ch];
        if ch == channels {
            samples.extend_from_slice(data);
        } else {
            // A change of layout mid-stream: map onto the first one
            for frame in data.chunks(ch) {
                for c in 0..channels {
                    samples.push(frame[c.min(ch - 1)]);
                }
            }
        }
    }
    if channels == 0 {
        return Err("the file has no audio".into());
    }
    Ok(Pcm { rate, channels, samples })
}

/// Mono or stereo version (encoders take at most two channels): extra channels are dropped.
pub fn to_stereo_or_mono(pcm: Pcm) -> Pcm {
    if pcm.channels <= 2 {
        return pcm;
    }
    let mut samples = Vec::with_capacity(pcm.frames() * 2);
    for frame in pcm.samples.chunks(pcm.channels) {
        samples.push(frame[0]);
        samples.push(frame[1]);
    }
    Pcm { rate: pcm.rate, channels: 2, samples }
}

// ---------------------------------------------------------------- MP3

fn lame_bitrate(kbps: u32) -> mp3lame_encoder::Bitrate {
    use mp3lame_encoder::Bitrate::*;
    match kbps {
        0..=8 => Kbps8,
        9..=16 => Kbps16,
        17..=24 => Kbps24,
        25..=32 => Kbps32,
        33..=40 => Kbps40,
        41..=48 => Kbps48,
        49..=64 => Kbps64,
        65..=80 => Kbps80,
        81..=96 => Kbps96,
        97..=112 => Kbps112,
        113..=128 => Kbps128,
        129..=160 => Kbps160,
        161..=192 => Kbps192,
        193..=224 => Kbps224,
        225..=256 => Kbps256,
        _ => Kbps320,
    }
}

/// Constant-bitrate MP3, with the LAME header (length, encoder delay and padding) like FFmpeg's libmp3lame wrote.
pub fn encode_mp3(pcm: &Pcm, kbps: u32) -> Result<Vec<u8>, String> {
    use mp3lame_encoder::{Builder, FlushGap, InterleavedPcm, MonoPcm};
    let err = |e: &dyn std::fmt::Debug| format!("MP3 encoding failed: {:?}", e);
    let mut builder = Builder::new().ok_or("MP3 encoder unavailable")?;
    builder.set_num_channels(pcm.channels as u8).map_err(|e| err(&e))?;
    builder.set_sample_rate(pcm.rate).map_err(|e| err(&e))?;
    builder.set_brate(lame_bitrate(kbps)).map_err(|e| err(&e))?;
    builder.set_to_write_vbr_tag(true).map_err(|e| err(&e))?;
    let mut encoder = builder.build().map_err(|e| err(&e))?;

    let mut out: Vec<u8> = Vec::new();
    const CHUNK_FRAMES: usize = 1152 * 64;
    for chunk in pcm.samples.chunks(CHUNK_FRAMES * pcm.channels) {
        let frames = chunk.len() / pcm.channels;
        out.reserve(mp3lame_encoder::max_required_buffer_size(frames));
        if pcm.channels == 1 {
            encoder.encode_to_vec(MonoPcm(chunk), &mut out).map_err(|e| err(&e))?;
        } else {
            encoder.encode_to_vec(InterleavedPcm(chunk), &mut out).map_err(|e| err(&e))?;
        }
    }
    out.reserve(7200);
    encoder.flush_to_vec::<FlushGap>(&mut out).map_err(|e| err(&e))?;
    // LAME wrote an empty frame first; fill in the header now that the totals are known
    let mut tag = Vec::with_capacity(encoder.lame_tag_size().max(1));
    if encoder.lame_tag_encode_to_vec(&mut tag).is_some() && tag.len() <= out.len() {
        out[..tag.len()].copy_from_slice(&tag);
    }
    Ok(out)
}

/// The file as a constant-bitrate MP3.
pub fn convert_to_mp3(path: &Path, kbps: u32) -> Result<Vec<u8>, String> {
    let pcm = to_stereo_or_mono(decode(path)?);
    encode_mp3(&pcm, kbps)
}

// ---------------------------------------------------------------- AAC

/// AAC-LC encoder (FDK), producing raw access units for an MP4 track.
pub struct AacEncoder {
    handle: fdk_aac_sys::HANDLE_AACENCODER,
    pub channels: usize,
    pub rate: u32,
    /// Priming samples the encoder adds at the start (to skip with an edit list)
    pub delay: u32,
    /// AudioSpecificConfig for the MP4 sample description
    pub config: Vec<u8>,
    max_out: usize,
}

unsafe impl Send for AacEncoder {}

impl AacEncoder {
    pub fn new(rate: u32, channels: usize, bitrate: u32) -> Result<AacEncoder, String> {
        use fdk_aac_sys as sys;
        let mut handle: sys::HANDLE_AACENCODER = std::ptr::null_mut();
        let check = |r: sys::AACENC_ERROR, what: &str| {
            if r == sys::AACENC_ERROR_AACENC_OK {
                Ok(())
            } else {
                Err(format!("AAC encoder: {} failed ({})", what, r))
            }
        };
        unsafe {
            check(sys::aacEncOpen(&mut handle, 0, channels as u32), "open")?;
                        check(sys::aacEncoder_SetParam(handle, sys::AACENC_PARAM_AACENC_AOT, sys::AUDIO_OBJECT_TYPE_AOT_AAC_LC as u32), "AOT")?;
            check(sys::aacEncoder_SetParam(handle, sys::AACENC_PARAM_AACENC_SAMPLERATE, rate), "sample rate")?;
            check(sys::aacEncoder_SetParam(handle, sys::AACENC_PARAM_AACENC_CHANNELMODE, channels as u32), "channel mode")?;
            check(sys::aacEncoder_SetParam(handle, sys::AACENC_PARAM_AACENC_CHANNELORDER, 1), "channel order")?;
            check(sys::aacEncoder_SetParam(handle, sys::AACENC_PARAM_AACENC_BITRATEMODE, 0), "bitrate mode")?;
            check(sys::aacEncoder_SetParam(handle, sys::AACENC_PARAM_AACENC_BITRATE, bitrate), "bitrate")?;
            check(sys::aacEncoder_SetParam(handle, sys::AACENC_PARAM_AACENC_TRANSMUX, 0), "transport")?;
            check(sys::aacEncoder_SetParam(handle, sys::AACENC_PARAM_AACENC_AFTERBURNER, 1), "afterburner")?;
            check(sys::aacEncEncode(handle, std::ptr::null(), std::ptr::null(), std::ptr::null(), std::ptr::null_mut()), "init")?;
            let mut info: sys::AACENC_InfoStruct = std::mem::zeroed();
            check(sys::aacEncInfo(handle, &mut info), "info")?;
            Ok(AacEncoder {
                handle,
                channels,
                rate,
                delay: info.nDelay,
                config: info.confBuf[..info.confSize as usize].to_vec(),
                max_out: (info.maxOutBufBytes as usize).max(8192),
            })
        }
    }

    /// Feed interleaved 16-bit samples (None flushes); returns the access units produced.
    fn feed(&mut self, input: Option<&[i16]>) -> Result<Vec<Vec<u8>>, String> {
        use fdk_aac_sys as sys;
        let mut units = Vec::new();
        let mut offset = 0usize;
        loop {
            let remaining: &[i16] = match input {
                Some(data) => &data[offset..],
                None => &[],
            };
            if input.is_some() && remaining.is_empty() {
                break;
            }
            let mut out = vec![0u8; self.max_out];
            let mut in_ptr = remaining.as_ptr() as *mut std::ffi::c_void;
            let mut in_id: i32 = sys::AACENC_BufferIdentifier_IN_AUDIO_DATA as i32;
            let mut in_size: i32 = (remaining.len() * 2) as i32;
            let mut in_el: i32 = 2;
            let in_desc = sys::AACENC_BufDesc { numBufs: 1, bufs: &mut in_ptr, bufferIdentifiers: &mut in_id, bufSizes: &mut in_size, bufElSizes: &mut in_el };
            let mut out_ptr = out.as_mut_ptr() as *mut std::ffi::c_void;
            let mut out_id: i32 = sys::AACENC_BufferIdentifier_OUT_BITSTREAM_DATA as i32;
            let mut out_size: i32 = out.len() as i32;
            let mut out_el: i32 = 1;
            let out_desc = sys::AACENC_BufDesc { numBufs: 1, bufs: &mut out_ptr, bufferIdentifiers: &mut out_id, bufSizes: &mut out_size, bufElSizes: &mut out_el };
            let in_args = sys::AACENC_InArgs { numInSamples: if input.is_some() { remaining.len() as i32 } else { -1 }, numAncBytes: 0 };
            let mut out_args: sys::AACENC_OutArgs = unsafe { std::mem::zeroed() };
            let r = unsafe { sys::aacEncEncode(self.handle, &in_desc, &out_desc, &in_args, &mut out_args) };
            if r == sys::AACENC_ERROR_AACENC_ENCODE_EOF {
                break;
            }
            if r != sys::AACENC_ERROR_AACENC_OK {
                return Err(format!("AAC encoding failed ({})", r));
            }
            if out_args.numOutBytes > 0 {
                out.truncate(out_args.numOutBytes as usize);
                units.push(out);
            }
            offset += out_args.numInSamples as usize;
            if input.is_some() && out_args.numInSamples == 0 && out_args.numOutBytes == 0 {
                break;
            }
            if input.is_none() && out_args.numOutBytes == 0 {
                break;
            }
        }
        Ok(units)
    }

    /// Encode a whole signal: interleaved f32 samples in [-1, 1] -> AAC access units (1024 samples each).
    pub fn encode_all(&mut self, samples: &[f32]) -> Result<Vec<Vec<u8>>, String> {
        let pcm16: Vec<i16> = samples.iter().map(|s| (s.clamp(-1.0, 1.0) * 32767.0).round() as i16).collect();
        let mut units = Vec::new();
        for chunk in pcm16.chunks(1024 * self.channels * 16) {
            units.extend(self.feed(Some(chunk))?);
        }
        units.extend(self.feed(None)?);
        Ok(units)
    }
}

impl Drop for AacEncoder {
    fn drop(&mut self) {
        unsafe {
            fdk_aac_sys::aacEncClose(&mut self.handle);
        }
    }
}

// ---------------------------------------------------------------- lengths, as ffprobe reported them

/// Seconds as ffprobe printed them: whole microseconds, shown with six decimals.
fn microseconds(num: u128, den: u128) -> f64 {
    if den == 0 {
        return 0.0;
    }
    let us = (num * 1_000_000 + den / 2) / den;
    format!("{}.{:06}", us / 1_000_000, us % 1_000_000).parse().unwrap_or(0.0)
}

const MP3_BITRATES: [[u32; 15]; 2] = [
    [0, 32, 40, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320],
    [0, 8, 16, 24, 32, 40, 48, 56, 64, 80, 96, 112, 128, 144, 160],
];

struct Mp3Header {
    mpeg1: bool,
    rate: u32,
    kbps: u32,
    mono: bool,
    samples_per_frame: u32,
}

fn parse_mp3_header(h: u32) -> Option<Mp3Header> {
    if h >> 21 != 0x7FF {
        return None;
    }
    let version = (h >> 19) & 3; // 3 = MPEG-1, 2 = MPEG-2, 0 = MPEG-2.5
    let layer = (h >> 17) & 3; // 1 = layer III
    let br = ((h >> 12) & 15) as usize;
    let sr = (h >> 10) & 3;
    if version == 1 || layer != 1 || br == 0 || br == 15 || sr == 3 {
        return None;
    }
    let base = [44100, 48000, 32000][sr as usize];
    let rate = match version {
        3 => base,
        2 => base / 2,
        _ => base / 4,
    };
    let mpeg1 = version == 3;
    Some(Mp3Header {
        mpeg1,
        rate,
        kbps: MP3_BITRATES[if mpeg1 { 0 } else { 1 }][br],
        mono: (h >> 6) & 3 == 3,
        samples_per_frame: if mpeg1 { 1152 } else { 576 },
    })
}

/// Length of an MP3 as FFmpeg computes it: from the Xing/Info (or VBRI) frame count, minus the LAME tag's encoder
/// delay and padding; without a frame count, from the file size and bit rate.
fn mp3_duration(d: &[u8]) -> Option<f64> {
    let mut start = 0usize;
    while d.len() >= start + 10 && &d[start..start + 3] == b"ID3" {
        let size = ((d[start + 6] as usize & 0x7F) << 21) | ((d[start + 7] as usize & 0x7F) << 14) | ((d[start + 8] as usize & 0x7F) << 7) | (d[start + 9] as usize & 0x7F);
        start += 10 + size + if d[start + 5] & 0x10 != 0 { 10 } else { 0 };
    }
    let data_offset = start;
    // First frame header (allow some junk before it)
    let mut at = start;
    let header = loop {
        if at + 4 > d.len() || at > start + 64 * 1024 {
            return None;
        }
        let h = u32::from_be_bytes(d[at..at + 4].try_into().unwrap());
        if let Some(hdr) = parse_mp3_header(h) {
            break hdr;
        }
        at += 1;
    };
    let side = match (header.mpeg1, header.mono) {
        (true, false) => 32,
        (true, true) => 17,
        (false, false) => 17,
        (false, true) => 9,
    };
    let rd = |p: usize| d.get(p..p + 4).map(|b| u32::from_be_bytes(b.try_into().unwrap()));
    let xing = at + 4 + side;
    let tag = d.get(xing..xing + 4);
    if tag == Some(b"Xing") || tag == Some(b"Info") {
        let flags = rd(xing + 4)?;
        let mut p = xing + 8;
        let mut frames = None;
        if flags & 1 != 0 {
            frames = rd(p);
            p += 4;
        }
        if flags & 2 != 0 {
            p += 4;
        }
        if flags & 4 != 0 {
            p += 100;
        }
        if flags & 8 != 0 {
            p += 4;
        }
        if let Some(frames) = frames.filter(|f| *f > 0) {
            let mut samples = frames as u128 * header.samples_per_frame as u128;
            // LAME / FFmpeg tag: 12-bit encoder delay and padding
            let encoder = d.get(p..p + 4).unwrap_or(&[]);
            if encoder == b"LAME" || encoder == b"Lavf" || encoder == b"Lavc" || encoder == b"L3.9" {
                if let Some(b) = d.get(p + 21..p + 24) {
                    let v = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
                    let (delay, padding) = ((v >> 12) as u128, (v & 0xFFF) as u128);
                    samples = samples.saturating_sub(delay + padding);
                }
            }
            return Some(microseconds(samples, header.rate as u128));
        }
    }
    // VBRI header (Fraunhofer), always 32 bytes after the frame header
    if d.get(at + 36..at + 40) == Some(b"VBRI") {
        if let Some(frames) = rd(at + 36 + 14).filter(|f| *f > 0) {
            return Some(microseconds(frames as u128 * header.samples_per_frame as u128, header.rate as u128));
        }
    }
    // Constant bit rate: estimate from the size
    let bytes = d.len().saturating_sub(data_offset) as u128;
    Some(microseconds(bytes * 8, header.kbps as u128 * 1000))
}

fn looks_like_mp3(d: &[u8]) -> bool {
    if d.starts_with(b"ID3") {
        return true;
    }
    d.len() >= 4 && parse_mp3_header(u32::from_be_bytes(d[..4].try_into().unwrap())).is_some()
}

/// Length of an audio or video file in seconds as ffprobe reported it; 0.0 if it can't be read.
pub fn probe_duration(path: &Path) -> f64 {
    let Ok(data) = std::fs::read(path) else { return 0.0 };
    let seconds = if data.get(4..8) == Some(b"ftyp") || data.get(4..8) == Some(b"moov") {
        mp4::read_movie(path).ok().map(|m| {
            let s = mp4::duration_seconds(&m);
            microseconds((s * 1e9).round() as u128, 1_000_000_000)
        })
    } else if looks_like_mp3(&data) {
        mp3_duration(&data)
    } else {
        None
    };
    match seconds {
        Some(s) if s > 0.0 => s,
        _ => symphonia_duration(path).unwrap_or(0.0),
    }
}

fn symphonia_duration(path: &Path) -> Option<f64> {
    let file = File::open(path).ok()?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let probed = symphonia::default::get_probe().format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default()).ok()?;
    let track = probed.format.tracks().iter().find(|t| t.codec_params.codec != CODEC_TYPE_NULL)?;
    let frames = track.codec_params.n_frames?;
    let rate = track.codec_params.sample_rate?;
    Some(microseconds(frames as u128, rate as u128))
}
