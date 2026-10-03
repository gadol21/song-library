//! A small ISO-BMFF (MP4) reader and writer.
//!
//! Reading covers regular files and fragmented (DASH) ones such as YouTube's separate video and audio streams.
//! Writing produces a regular MP4 with the index first ("fast start"), from tracks whose sample-description box
//! is either copied from a source file (remuxing) or built for freshly encoded H.264 / AAC.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

// ---------------------------------------------------------------- reading

#[derive(Clone, Debug)]
pub struct Sample {
    /// Absolute position of the sample's data in the file
    pub offset: u64,
    pub size: u32,
    /// Decode duration, in the track's timescale
    pub duration: u32,
    /// Composition time offset (presentation minus decode time)
    pub cts: i32,
    pub sync: bool,
}

#[derive(Clone, Debug)]
pub struct Track {
    pub id: u32,
    /// "vide" or "soun"
    pub handler: [u8; 4],
    pub timescale: u32,
    /// The stsd box's single sample entry (e.g. the whole avc1 or mp4a box), copied as is when remuxing
    pub sample_entry: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// Edit list: (segment duration in the movie timescale, media time in the track timescale)
    pub edits: Vec<(u64, i64)>,
    pub mdhd_duration: u64,
    pub samples: Vec<Sample>,
}

impl Track {
    pub fn is_video(&self) -> bool {
        &self.handler == b"vide"
    }
    pub fn is_audio(&self) -> bool {
        &self.handler == b"soun"
    }
    pub fn media_duration(&self) -> u64 {
        let total: u64 = self.samples.iter().map(|s| s.duration as u64).sum();
        if total > 0 {
            total
        } else {
            self.mdhd_duration
        }
    }
}

pub struct Movie {
    pub timescale: u32,
    pub mvhd_duration: u64,
    pub tracks: Vec<Track>,
}

struct Reader;

fn rd_u32(d: &[u8], at: usize) -> u32 {
    d.get(at..at + 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]])).unwrap_or(0)
}
fn rd_u64(d: &[u8], at: usize) -> u64 {
    d.get(at..at + 8).map(|b| u64::from_be_bytes(b.try_into().unwrap())).unwrap_or(0)
}

/// Iterate the child boxes of a container body: (type, body start, body end, box start) relative to `d`.
fn children(d: &[u8]) -> Vec<([u8; 4], usize, usize, usize)> {
    let mut out = Vec::new();
    let mut at = 0usize;
    while at + 8 <= d.len() {
        let size32 = rd_u32(d, at) as u64;
        let kind: [u8; 4] = d[at + 4..at + 8].try_into().unwrap();
        let (header, size) = match size32 {
            1 => (16usize, rd_u64(d, at + 8)),
            0 => (8usize, (d.len() - at) as u64),
            n => (8usize, n),
        };
        if size < header as u64 || at as u64 + size > d.len() as u64 {
            break;
        }
        out.push((kind, at + header, at + size as usize, at));
        at += size as usize;
    }
    out
}

fn find<'a>(d: &'a [u8], kind: &[u8; 4]) -> Option<&'a [u8]> {
    children(d).into_iter().find(|c| &c.0 == kind).map(|(_, s, e, _)| &d[s..e])
}

impl Reader {
    fn parse_trak(&self, trak: &[u8]) -> Option<Track> {
        let tkhd = find(trak, b"tkhd")?;
        let version = tkhd[0];
        let id = if version == 1 { rd_u32(tkhd, 20) } else { rd_u32(tkhd, 12) };
        let (w_at, h_at) = if version == 1 { (88, 92) } else { (76, 80) };
        let width = rd_u32(tkhd, w_at) >> 16;
        let height = rd_u32(tkhd, h_at) >> 16;

        let mut edits = Vec::new();
        if let Some(elst) = find(trak, b"edts").and_then(|e| find(e, b"elst")) {
            let v = elst[0];
            let count = rd_u32(elst, 4) as usize;
            let mut at = 8;
            for _ in 0..count {
                if v == 1 {
                    edits.push((rd_u64(elst, at), rd_u64(elst, at + 8) as i64));
                    at += 20;
                } else {
                    edits.push((rd_u32(elst, at) as u64, rd_u32(elst, at + 4) as i32 as i64));
                    at += 12;
                }
            }
        }

        let mdia = find(trak, b"mdia")?;
        let mdhd = find(mdia, b"mdhd")?;
        let (timescale, mdhd_duration) =
            if mdhd[0] == 1 { (rd_u32(mdhd, 20), rd_u64(mdhd, 24)) } else { (rd_u32(mdhd, 12), rd_u32(mdhd, 16) as u64) };
        let hdlr = find(mdia, b"hdlr")?;
        let handler: [u8; 4] = hdlr.get(8..12)?.try_into().ok()?;
        let stbl = find(find(mdia, b"minf")?, b"stbl")?;
        let stsd = find(stbl, b"stsd")?;
        let entry = children(&stsd[8..]).into_iter().next()?;
        let sample_entry = stsd[8 + entry.3..8 + entry.2].to_vec();

        // Sample tables (empty in a fragmented file)
        let mut sizes = Vec::new();
        if let Some(stsz) = find(stbl, b"stsz") {
            let fixed = rd_u32(stsz, 4);
            let count = rd_u32(stsz, 8) as usize;
            for i in 0..count {
                sizes.push(if fixed != 0 { fixed } else { rd_u32(stsz, 12 + i * 4) });
            }
        }
        let mut durations = Vec::new();
        if let Some(stts) = find(stbl, b"stts") {
            for i in 0..rd_u32(stts, 4) as usize {
                let (n, delta) = (rd_u32(stts, 8 + i * 8), rd_u32(stts, 12 + i * 8));
                durations.extend(std::iter::repeat(delta).take(n as usize));
            }
        }
        let mut ctts = Vec::new();
        if let Some(c) = find(stbl, b"ctts") {
            for i in 0..rd_u32(c, 4) as usize {
                let (n, off) = (rd_u32(c, 8 + i * 8), rd_u32(c, 12 + i * 8) as i32);
                ctts.extend(std::iter::repeat(off).take(n as usize));
            }
        }
        let sync: Option<std::collections::HashSet<u32>> =
            find(stbl, b"stss").map(|s| (0..rd_u32(s, 4) as usize).map(|i| rd_u32(s, 8 + i * 4)).collect());
        let mut chunk_offsets = Vec::new();
        if let Some(stco) = find(stbl, b"stco") {
            chunk_offsets = (0..rd_u32(stco, 4) as usize).map(|i| rd_u32(stco, 8 + i * 4) as u64).collect();
        } else if let Some(co64) = find(stbl, b"co64") {
            chunk_offsets = (0..rd_u32(co64, 4) as usize).map(|i| rd_u64(co64, 8 + i * 8)).collect();
        }
        let mut stsc = Vec::new();
        if let Some(s) = find(stbl, b"stsc") {
            stsc = (0..rd_u32(s, 4) as usize).map(|i| (rd_u32(s, 8 + i * 12), rd_u32(s, 12 + i * 12))).collect::<Vec<_>>();
        }

        let mut samples = Vec::with_capacity(sizes.len());
        let mut index = 0usize;
        for (c, &chunk_offset) in chunk_offsets.iter().enumerate() {
            let chunk_no = c as u32 + 1;
            let per_chunk = stsc.iter().rev().find(|(first, _)| *first <= chunk_no).map(|e| e.1).unwrap_or(0);
            let mut offset = chunk_offset;
            for _ in 0..per_chunk {
                if index >= sizes.len() {
                    break;
                }
                samples.push(Sample {
                    offset,
                    size: sizes[index],
                    duration: durations.get(index).copied().unwrap_or(0),
                    cts: ctts.get(index).copied().unwrap_or(0),
                    sync: sync.as_ref().map_or(true, |s| s.contains(&(index as u32 + 1))),
                });
                offset += sizes[index] as u64;
                index += 1;
            }
        }
        Some(Track { id, handler, timescale, sample_entry, width, height, edits, mdhd_duration, samples })
    }

    /// Samples described by one movie fragment.
    fn parse_moof(&self, moof: &[u8], moof_start: u64, tracks: &mut [Track], trex: &[(u32, u32, u32, u32)]) {
        for (kind, s, e, _) in children(moof) {
            if &kind != b"traf" {
                continue;
            }
            let traf = &moof[s..e];
            let Some(tfhd) = find(traf, b"tfhd") else { continue };
            let flags = rd_u32(tfhd, 0) & 0xFFFFFF;
            let track_id = rd_u32(tfhd, 4);
            let defaults = trex.iter().find(|t| t.0 == track_id).copied().unwrap_or((track_id, 0, 0, 0));
            let mut at = 8;
            let mut base = moof_start;
            if flags & 0x1 != 0 {
                base = rd_u64(tfhd, at);
                at += 8;
            }
            if flags & 0x2 != 0 {
                at += 4;
            }
            let mut def_dur = defaults.1;
            let mut def_size = defaults.2;
            let mut def_flags = defaults.3;
            if flags & 0x8 != 0 {
                def_dur = rd_u32(tfhd, at);
                at += 4;
            }
            if flags & 0x10 != 0 {
                def_size = rd_u32(tfhd, at);
                at += 4;
            }
            if flags & 0x20 != 0 {
                def_flags = rd_u32(tfhd, at);
            }
            let Some(track) = tracks.iter_mut().find(|t| t.id == track_id) else { continue };
            let mut next_offset = base;
            for (k, ts, te, _) in children(traf) {
                if &k != b"trun" {
                    continue;
                }
                let trun = &traf[ts..te];
                let tf = rd_u32(trun, 0) & 0xFFFFFF;
                let version = trun[0];
                let count = rd_u32(trun, 4) as usize;
                let mut p = 8;
                let mut offset = next_offset;
                if tf & 0x1 != 0 {
                    offset = (base as i64 + rd_u32(trun, p) as i32 as i64) as u64;
                    p += 4;
                }
                let mut first_flags = None;
                if tf & 0x4 != 0 {
                    first_flags = Some(rd_u32(trun, p));
                    p += 4;
                }
                for i in 0..count {
                    let mut dur = def_dur;
                    let mut size = def_size;
                    let mut sflags = if i == 0 { first_flags.unwrap_or(def_flags) } else { def_flags };
                    let mut cts = 0i32;
                    if tf & 0x100 != 0 {
                        dur = rd_u32(trun, p);
                        p += 4;
                    }
                    if tf & 0x200 != 0 {
                        size = rd_u32(trun, p);
                        p += 4;
                    }
                    if tf & 0x400 != 0 {
                        sflags = rd_u32(trun, p);
                        p += 4;
                    }
                    if tf & 0x800 != 0 {
                        let raw = rd_u32(trun, p);
                        cts = if version == 0 { raw as i32 } else { raw as i32 };
                        p += 4;
                    }
                    // sample_is_non_sync_sample is bit 16 of the sample flags
                    let sync = sflags & 0x0001_0000 == 0;
                    track.samples.push(Sample { offset, size, duration: dur, cts, sync });
                    offset += size as u64;
                }
                next_offset = offset;
            }
        }
    }
}

/// Read the structure of an MP4 file (sample positions only; data is read on demand).
pub fn read_movie(path: &Path) -> Result<Movie, String> {
    let data = std::fs::read(path).map_err(|e| e.to_string())?;
    let reader = Reader;
    let top = children(&data);
    let moov = top.iter().find(|c| &c.0 == b"moov").ok_or("not an MP4 file (no moov box)")?;
    let moov_body = &data[moov.1..moov.2];
    let mvhd = find(moov_body, b"mvhd").ok_or("no mvhd box")?;
    let (timescale, mvhd_duration) =
        if mvhd[0] == 1 { (rd_u32(mvhd, 20), rd_u64(mvhd, 24)) } else { (rd_u32(mvhd, 12), rd_u32(mvhd, 16) as u64) };
    let mut tracks: Vec<Track> = children(moov_body)
        .into_iter()
        .filter(|c| &c.0 == b"trak")
        .filter_map(|(_, s, e, _)| reader.parse_trak(&moov_body[s..e]))
        .collect();
    let trex: Vec<(u32, u32, u32, u32)> = find(moov_body, b"mvex")
        .map(|mvex| {
            children(mvex)
                .into_iter()
                .filter(|c| &c.0 == b"trex")
                .map(|(_, s, _, _)| {
                    let t = &mvex[s..];
                    (rd_u32(t, 4), rd_u32(t, 12), rd_u32(t, 16), rd_u32(t, 20))
                })
                .collect()
        })
        .unwrap_or_default();
    for (kind, s, e, start) in &top {
        if kind == b"moof" {
            reader.parse_moof(&data[*s..*e], *start as u64, &mut tracks, &trex);
        }
    }
    Ok(Movie { timescale, mvhd_duration, tracks })
}

/// Length in seconds as FFmpeg reports it: the longest track, after its edit list.
pub fn duration_seconds(movie: &Movie) -> f64 {
    let mut longest: f64 = 0.0;
    for t in &movie.tracks {
        if t.timescale == 0 {
            continue;
        }
        let media = t.media_duration() as f64 / t.timescale as f64;
        let secs = match t.edits.iter().filter(|e| e.1 >= 0).map(|e| e.0).sum::<u64>() {
            0 => {
                // Without a usable edit list the media time starting point still counts
                let skip = t.edits.iter().find(|e| e.1 >= 0).map(|e| e.1 as f64 / t.timescale as f64).unwrap_or(0.0);
                media - skip
            }
            edited if movie.timescale > 0 => (edited as f64 / movie.timescale as f64).min(media),
            _ => media,
        };
        longest = longest.max(secs);
    }
    longest
}

pub fn read_sample(file: &mut File, s: &Sample) -> std::io::Result<Vec<u8>> {
    let mut buf = vec![0u8; s.size as usize];
    file.seek(SeekFrom::Start(s.offset))?;
    file.read_exact(&mut buf)?;
    Ok(buf)
}

// ---------------------------------------------------------------- writing

pub struct OutTrack {
    pub handler: [u8; 4],
    pub timescale: u32,
    pub sample_entry: Vec<u8>,
    pub width: u32,
    pub height: u32,
    /// Edit list to write: (segment duration in the movie timescale, media time in the track timescale)
    pub edits: Vec<(u64, i64)>,
    pub samples: Vec<OutSample>,
}

pub struct OutSample {
    pub data: Vec<u8>,
    pub duration: u32,
    pub cts: i32,
    pub sync: bool,
}

fn bx(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len() + 8);
    out.extend_from_slice(&((body.len() + 8) as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(body);
    out
}

fn full(kind: &[u8; 4], version: u8, flags: u32, body: &[u8]) -> Vec<u8> {
    let mut b = vec![version, (flags >> 16) as u8, (flags >> 8) as u8, flags as u8];
    b.extend_from_slice(body);
    bx(kind, &b)
}

trait Be {
    fn u16(&mut self, v: u16);
    fn u32(&mut self, v: u32);
    fn u64(&mut self, v: u64);
}
impl Be for Vec<u8> {
    fn u16(&mut self, v: u16) {
        self.extend_from_slice(&v.to_be_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.extend_from_slice(&v.to_be_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.extend_from_slice(&v.to_be_bytes());
    }
}

const MATRIX: [u32; 9] = [0x10000, 0, 0, 0, 0x10000, 0, 0, 0, 0x40000000];
const MOVIE_TIMESCALE: u32 = 1000;

fn rescale(v: u64, from: u32, to: u32) -> u64 {
    ((v as u128 * to as u128 + from as u128 / 2) / from as u128) as u64
}

fn track_duration_in_movie(t: &OutTrack) -> u64 {
    if !t.edits.is_empty() {
        return t.edits.iter().map(|e| e.0).sum();
    }
    rescale(t.samples.iter().map(|s| s.duration as u64).sum(), t.timescale, MOVIE_TIMESCALE)
}

fn trak_box(t: &OutTrack, id: u32, chunk_offsets: &[u64]) -> Vec<u8> {
    let media_duration: u64 = t.samples.iter().map(|s| s.duration as u64).sum();
    let movie_duration = track_duration_in_movie(t);
    let video = &t.handler == b"vide";

    // Version 1: 64-bit times and duration
    let mut tkhd = Vec::new();
    tkhd.u64(0); // creation time
    tkhd.u64(0); // modification time
    tkhd.u32(id);
    tkhd.u32(0);
    tkhd.u64(movie_duration);
    tkhd.u64(0); // reserved
    tkhd.u16(0); // layer
    tkhd.u16(0); // alternate group
    tkhd.u16(if video { 0 } else { 0x0100 }); // volume
    tkhd.u16(0);
    for m in MATRIX {
        tkhd.u32(m);
    }
    tkhd.u32(t.width << 16);
    tkhd.u32(t.height << 16);
    let tkhd = full(b"tkhd", 1, 3, &tkhd);

    let edts = if t.edits.is_empty() {
        Vec::new()
    } else {
        let mut elst = Vec::new();
        elst.u32(t.edits.len() as u32);
        for (dur, media_time) in &t.edits {
            elst.u64(*dur);
            elst.u64(*media_time as u64);
            elst.u16(1);
            elst.u16(0);
        }
        bx(b"edts", &full(b"elst", 1, 0, &elst))
    };

    let mut mdhd = Vec::new();
    mdhd.u64(0);
    mdhd.u64(0);
    mdhd.u32(t.timescale);
    mdhd.u64(media_duration);
    mdhd.u16(0x55c4); // language "und"
    mdhd.u16(0);
    let mdhd = full(b"mdhd", 1, 0, &mdhd);

    let mut hdlr = Vec::new();
    hdlr.u32(0);
    hdlr.extend_from_slice(&t.handler);
    hdlr.extend_from_slice(&[0u8; 12]);
    hdlr.extend_from_slice(if video { b"VideoHandler\0" } else { b"SoundHandler\0" });
    let hdlr = full(b"hdlr", 0, 0, &hdlr);

    let media_header = if video { full(b"vmhd", 0, 1, &[0u8; 8]) } else { full(b"smhd", 0, 0, &[0u8; 4]) };
    let dref = full(b"dref", 0, 0, &[&1u32.to_be_bytes()[..], &full(b"url ", 0, 1, &[])].concat());
    let dinf = bx(b"dinf", &dref);

    let mut stsd = Vec::new();
    stsd.u32(1);
    stsd.extend_from_slice(&t.sample_entry);
    let stsd = full(b"stsd", 0, 0, &stsd);

    // Run-length tables
    let mut stts_entries: Vec<(u32, u32)> = Vec::new();
    for s in &t.samples {
        match stts_entries.last_mut() {
            Some((n, d)) if *d == s.duration => *n += 1,
            _ => stts_entries.push((1, s.duration)),
        }
    }
    let mut stts = Vec::new();
    stts.u32(stts_entries.len() as u32);
    for (n, d) in stts_entries {
        stts.u32(n);
        stts.u32(d);
    }
    let stts = full(b"stts", 0, 0, &stts);

    let ctts = if t.samples.iter().any(|s| s.cts != 0) {
        let mut entries: Vec<(u32, i32)> = Vec::new();
        for s in &t.samples {
            match entries.last_mut() {
                Some((n, c)) if *c == s.cts => *n += 1,
                _ => entries.push((1, s.cts)),
            }
        }
        let mut b = Vec::new();
        b.u32(entries.len() as u32);
        for (n, c) in entries {
            b.u32(n);
            b.u32(c as u32);
        }
        full(b"ctts", 1, 0, &b)
    } else {
        Vec::new()
    };

    let stss = if t.samples.iter().all(|s| s.sync) {
        Vec::new()
    } else {
        let syncs: Vec<u32> = t.samples.iter().enumerate().filter(|(_, s)| s.sync).map(|(i, _)| i as u32 + 1).collect();
        let mut b = Vec::new();
        b.u32(syncs.len() as u32);
        for s in syncs {
            b.u32(s);
        }
        full(b"stss", 0, 0, &b)
    };

    // One chunk per CHUNK_SAMPLES samples
    let mut stsc = Vec::new();
    let chunks = chunk_sizes(t.samples.len());
    let mut entries: Vec<(u32, u32)> = Vec::new();
    for (i, &n) in chunks.iter().enumerate() {
        if entries.last().map(|e| e.1) != Some(n as u32) {
            entries.push((i as u32 + 1, n as u32));
        }
    }
    stsc.u32(entries.len() as u32);
    for (first, n) in entries {
        stsc.u32(first);
        stsc.u32(n);
        stsc.u32(1);
    }
    let stsc = full(b"stsc", 0, 0, &stsc);

    let mut stsz = Vec::new();
    stsz.u32(0);
    stsz.u32(t.samples.len() as u32);
    for s in &t.samples {
        stsz.u32(s.data.len() as u32);
    }
    let stsz = full(b"stsz", 0, 0, &stsz);

    let mut co64 = Vec::new();
    co64.u32(chunk_offsets.len() as u32);
    for &o in chunk_offsets {
        co64.u64(o);
    }
    let co64 = full(b"co64", 0, 0, &co64);

    let stbl = bx(b"stbl", &[stsd, stts, ctts, stss, stsc, stsz, co64].concat());
    let minf = bx(b"minf", &[media_header, dinf, stbl].concat());
    let mdia = bx(b"mdia", &[mdhd, hdlr, minf].concat());
    bx(b"trak", &[tkhd, edts, mdia].concat())
}

const CHUNK_SAMPLES: usize = 32;

fn chunk_sizes(samples: usize) -> Vec<usize> {
    let mut v = vec![CHUNK_SAMPLES; samples / CHUNK_SAMPLES];
    if samples % CHUNK_SAMPLES != 0 {
        v.push(samples % CHUNK_SAMPLES);
    }
    v
}

fn moov_box(tracks: &[OutTrack], offsets: &[Vec<u64>]) -> Vec<u8> {
    let duration = tracks.iter().map(track_duration_in_movie).max().unwrap_or(0);
    let mut mvhd = Vec::new();
    mvhd.u64(0);
    mvhd.u64(0);
    mvhd.u32(MOVIE_TIMESCALE);
    mvhd.u64(duration);
    mvhd.u32(0x00010000); // rate
    mvhd.u16(0x0100); // volume
    mvhd.extend_from_slice(&[0u8; 10]);
    for m in MATRIX {
        mvhd.u32(m);
    }
    mvhd.extend_from_slice(&[0u8; 24]);
    mvhd.u32(tracks.len() as u32 + 1);
    let mut body = full(b"mvhd", 1, 0, &mvhd);
    for (i, t) in tracks.iter().enumerate() {
        body.extend(trak_box(t, i as u32 + 1, &offsets[i]));
    }
    bx(b"moov", &body)
}

/// Write the tracks as a fast-start MP4: ftyp, moov, then the samples, interleaved chunk by chunk.
pub fn write_movie(path: &Path, tracks: &[OutTrack]) -> Result<(), String> {
    let mut ftyp = Vec::new();
    ftyp.extend_from_slice(b"isom");
    ftyp.u32(0x200);
    for brand in [b"isom", b"iso2", b"avc1", b"mp41"] {
        ftyp.extend_from_slice(brand);
    }
    let ftyp = bx(b"ftyp", &ftyp);

    // Chunk order: interleave the tracks by time, so players read the file front to back
    struct Chunk {
        track: usize,
        first: usize,
        count: usize,
        start: f64,
    }
    let mut chunks = Vec::new();
    for (ti, t) in tracks.iter().enumerate() {
        let mut first = 0;
        let mut time: u64 = 0;
        for n in chunk_sizes(t.samples.len()) {
            chunks.push(Chunk { track: ti, first, count: n, start: time as f64 / t.timescale as f64 });
            time += t.samples[first..first + n].iter().map(|s| s.duration as u64).sum::<u64>();
            first += n;
        }
    }
    chunks.sort_by(|a, b| a.start.partial_cmp(&b.start).unwrap().then(a.track.cmp(&b.track)));

    // The moov size doesn't depend on the offset values (co64 is fixed size), so measure it with placeholders
    let mut offsets: Vec<Vec<u64>> = tracks.iter().map(|t| vec![0; chunk_sizes(t.samples.len()).len()]).collect();
    let moov_len = moov_box(tracks, &offsets).len() as u64;
    let mdat_payload: u64 = tracks.iter().flat_map(|t| t.samples.iter()).map(|s| s.data.len() as u64).sum();
    let mdat_header: u64 = 16; // 64-bit size form, always
    let mut pos = ftyp.len() as u64 + moov_len + mdat_header;
    let mut chunk_index: Vec<usize> = vec![0; tracks.len()];
    for c in &chunks {
        offsets[c.track][chunk_index[c.track]] = pos;
        chunk_index[c.track] += 1;
        pos += tracks[c.track].samples[c.first..c.first + c.count].iter().map(|s| s.data.len() as u64).sum::<u64>();
    }
    let moov = moov_box(tracks, &offsets);

    let file = File::create(path).map_err(|e| e.to_string())?;
    let mut w = std::io::BufWriter::new(file);
    let io = |e: std::io::Error| e.to_string();
    w.write_all(&ftyp).map_err(io)?;
    w.write_all(&moov).map_err(io)?;
    w.write_all(&1u32.to_be_bytes()).map_err(io)?;
    w.write_all(b"mdat").map_err(io)?;
    w.write_all(&(mdat_payload + mdat_header).to_be_bytes()).map_err(io)?;
    for c in &chunks {
        for s in &tracks[c.track].samples[c.first..c.first + c.count] {
            w.write_all(&s.data).map_err(io)?;
        }
    }
    w.flush().map_err(io)?;
    Ok(())
}

/// Copy a track's samples (and its sample description and edit list) for writing into another file.
fn copy_track(path: &Path, movie: &Movie, track: &Track) -> Result<OutTrack, String> {
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    let mut samples = Vec::with_capacity(track.samples.len());
    for s in &track.samples {
        samples.push(OutSample { data: read_sample(&mut file, s).map_err(|e| e.to_string())?, duration: s.duration, cts: s.cts, sync: s.sync });
    }
    let edits = track
        .edits
        .iter()
        .map(|(dur, media_time)| (if movie.timescale > 0 { rescale(*dur, movie.timescale, MOVIE_TIMESCALE) } else { *dur }, *media_time))
        .collect();
    Ok(OutTrack {
        handler: track.handler,
        timescale: track.timescale,
        sample_entry: track.sample_entry.clone(),
        width: track.width,
        height: track.height,
        edits,
        samples,
    })
}

/// Join separately downloaded streams (e.g. YouTube's video-only and audio-only files) into one MP4 without
/// re-encoding: the first video track found and the first audio track found, like FFmpeg's `-c copy` merge.
pub fn merge_streams(files: &[std::path::PathBuf], target: &Path) -> Result<(), String> {
    let mut video: Option<OutTrack> = None;
    let mut audio: Option<OutTrack> = None;
    for path in files {
        let movie = read_movie(path).map_err(|e| format!("{}: {}", path.display(), e))?;
        for track in &movie.tracks {
            if track.is_video() && video.is_none() {
                video = Some(copy_track(path, &movie, track)?);
            } else if track.is_audio() && audio.is_none() {
                audio = Some(copy_track(path, &movie, track)?);
            }
        }
    }
    let tracks: Vec<OutTrack> = video.into_iter().chain(audio).collect();
    if tracks.is_empty() {
        return Err("no video or audio tracks to join".into());
    }
    write_movie(target, &tracks)
}

// ---------------------------------------------------------------- sample entries for freshly encoded streams

/// avc1 sample entry from the stream's SPS and PPS NAL units.
pub fn avc1_entry(width: u16, height: u16, sps: &[u8], pps: &[u8]) -> Vec<u8> {
    let mut avcc = vec![1, sps[1], sps[2], sps[3], 0xFF, 0xE1];
    avcc.u16(sps.len() as u16);
    avcc.extend_from_slice(sps);
    avcc.push(1);
    avcc.u16(pps.len() as u16);
    avcc.extend_from_slice(pps);
    let mut e = vec![0u8; 6];
    e.u16(1); // data reference index
    e.extend_from_slice(&[0u8; 16]);
    e.u16(width);
    e.u16(height);
    e.u32(0x00480000);
    e.u32(0x00480000);
    e.u32(0);
    e.u16(1); // frame count
    e.extend_from_slice(&[0u8; 32]); // compressor name
    e.u16(0x0018);
    e.u16(0xFFFF);
    e.extend(bx(b"avcC", &avcc));
    bx(b"avc1", &e)
}

fn descriptor(tag: u8, body: &[u8]) -> Vec<u8> {
    let mut d = vec![tag];
    let len = body.len();
    d.extend_from_slice(&[0x80 | ((len >> 21) & 0x7F) as u8, 0x80 | ((len >> 14) & 0x7F) as u8, 0x80 | ((len >> 7) & 0x7F) as u8, (len & 0x7F) as u8]);
    d.extend_from_slice(body);
    d
}

/// mp4a sample entry for AAC with the given AudioSpecificConfig.
pub fn mp4a_entry(channels: u16, sample_rate: u32, asc: &[u8], avg_bitrate: u32) -> Vec<u8> {
    let mut dcd = vec![0x40, 0x15, 0, 0, 0];
    dcd.u32(avg_bitrate);
    dcd.u32(avg_bitrate);
    dcd.extend(descriptor(0x05, asc));
    let mut es = Vec::new();
    es.u16(0); // ES id
    es.push(0);
    es.extend(descriptor(0x04, &dcd));
    es.extend(descriptor(0x06, &[0x02]));
    let esds = full(b"esds", 0, 0, &descriptor(0x03, &es));
    let mut e = vec![0u8; 6];
    e.u16(1);
    e.extend_from_slice(&[0u8; 8]);
    e.u16(channels);
    e.u16(16);
    e.u32(0);
    e.u32(sample_rate << 16);
    e.extend(esds);
    bx(b"mp4a", &e)
}
