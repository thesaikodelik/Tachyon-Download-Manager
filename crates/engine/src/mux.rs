//! Joins a picture-only MP4 and a sound-only MP4/M4A into one ordinary MP4, as
//! ffmpeg's "-c copy" would: samples are copied unchanged, only the index is rebuilt.
//! Either input may be fragmented (a moof/mdat pair per few seconds, as YouTube
//! sends picture) or a plain file with one index (as it sends sound). The result is
//! never fragmented, so every player can open and seek it.

use std::fs::File;
use std::io::{self, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::Path;

/// Boxes whose payload is just more boxes.
const CONTAINERS: [&[u8; 4]; 10] = [b"moov", b"trak", b"mdia", b"minf", b"stbl", b"edts", b"dinf", b"mvex", b"moof", b"traf"];

#[derive(Clone, Debug)]
struct Mp4Box {
    kind: [u8; 4],
    body: Body,
}

#[derive(Clone, Debug)]
enum Body {
    Raw(Vec<u8>),
    Children(Vec<Mp4Box>),
}

fn bad(what: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.into())
}

fn be32(b: &[u8], at: usize) -> io::Result<u32> {
    b.get(at..at + 4).map(|s| u32::from_be_bytes(s.try_into().unwrap())).ok_or_else(|| bad("box too short"))
}

fn be64(b: &[u8], at: usize) -> io::Result<u64> {
    b.get(at..at + 8).map(|s| u64::from_be_bytes(s.try_into().unwrap())).ok_or_else(|| bad("box too short"))
}

fn parse(mut b: &[u8]) -> io::Result<Vec<Mp4Box>> {
    let mut boxes = Vec::new();
    while b.len() >= 8 {
        let mut size = be32(b, 0)? as u64;
        let kind: [u8; 4] = b[4..8].try_into().unwrap();
        let mut header = 8;
        if size == 1 {
            size = be64(b, 8)?;
            header = 16;
        } else if size == 0 {
            size = b.len() as u64;
        }
        let size = usize::try_from(size).map_err(|_| bad("box too large"))?;
        if size < header || size > b.len() {
            return Err(bad(format!("broken {} box", String::from_utf8_lossy(&kind))));
        }
        let payload = &b[header..size];
        let body = if CONTAINERS.contains(&&kind) { Body::Children(parse(payload)?) } else { Body::Raw(payload.to_vec()) };
        boxes.push(Mp4Box { kind, body });
        b = &b[size..];
    }
    Ok(boxes)
}

impl Mp4Box {
    fn raw(kind: &[u8; 4], payload: Vec<u8>) -> Self {
        Self { kind: *kind, body: Body::Raw(payload) }
    }

    fn len(&self) -> u64 {
        let payload: u64 = match &self.body {
            Body::Raw(b) => b.len() as u64,
            Body::Children(c) => c.iter().map(Mp4Box::len).sum(),
        };
        payload + 8
    }

    fn write(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&(self.len() as u32).to_be_bytes());
        out.extend_from_slice(&self.kind);
        match &self.body {
            Body::Raw(b) => out.extend_from_slice(b),
            Body::Children(c) => c.iter().for_each(|b| b.write(out)),
        }
    }

    fn children(&self) -> &[Mp4Box] {
        match &self.body {
            Body::Children(c) => c,
            Body::Raw(_) => &[],
        }
    }

    fn children_mut(&mut self) -> &mut Vec<Mp4Box> {
        match &mut self.body {
            Body::Children(c) => c,
            Body::Raw(_) => panic!("not a container"),
        }
    }

    fn payload(&self) -> &[u8] {
        match &self.body {
            Body::Raw(b) => b,
            Body::Children(_) => &[],
        }
    }

    fn payload_mut(&mut self) -> &mut Vec<u8> {
        match &mut self.body {
            Body::Raw(b) => b,
            Body::Children(_) => panic!("not a leaf"),
        }
    }

    fn child(&self, kind: &[u8; 4]) -> Option<&Mp4Box> {
        self.children().iter().find(|b| &b.kind == kind)
    }

    fn child_mut(&mut self, kind: &[u8; 4]) -> Option<&mut Mp4Box> {
        self.children_mut().iter_mut().find(|b| &b.kind == kind)
    }

    /// The box at `path` below this one.
    fn find(&self, path: &[&[u8; 4]]) -> Option<&Mp4Box> {
        path.iter().try_fold(self, |b, kind| b.child(kind))
    }

    fn find_mut(&mut self, path: &[&[u8; 4]]) -> Option<&mut Mp4Box> {
        let mut b = self;
        for kind in path {
            b = b.child_mut(kind)?;
        }
        Some(b)
    }
}

/// A top-level box of a file, located but not read.
struct TopBox {
    kind: [u8; 4],
    offset: u64,
    header: u64,
    size: u64,
}

fn scan(file: &mut File) -> io::Result<Vec<TopBox>> {
    let end = file.metadata()?.len();
    let mut boxes = Vec::new();
    let mut offset = 0;
    while offset + 8 <= end {
        file.seek(SeekFrom::Start(offset))?;
        let mut head = [0u8; 16];
        let n = file.read(&mut head)?;
        let mut size = u32::from_be_bytes(head[0..4].try_into().unwrap()) as u64;
        let kind: [u8; 4] = head[4..8].try_into().unwrap();
        let mut header = 8;
        if size == 1 && n >= 16 {
            size = u64::from_be_bytes(head[8..16].try_into().unwrap());
            header = 16;
        } else if size == 0 {
            size = end - offset;
        }
        if size < header || offset + size > end {
            return Err(bad("the file ends in the middle of a box"));
        }
        boxes.push(TopBox { kind, offset, header, size });
        offset += size;
    }
    Ok(boxes)
}

fn read_box(file: &mut File, b: &TopBox) -> io::Result<Mp4Box> {
    let mut payload = vec![0u8; (b.size - b.header) as usize];
    file.seek(SeekFrom::Start(b.offset + b.header))?;
    file.read_exact(&mut payload)?;
    let body = if CONTAINERS.contains(&&b.kind) { Body::Children(parse(&payload)?) } else { Body::Raw(payload) };
    Ok(Mp4Box { kind: b.kind, body })
}

/// One sample's index entry.
struct Sample {
    size: u32,
    duration: u32,
    sync: bool,
    cto: i32,
}

/// A track ready to be written: its trak box (index still pointing into its own
/// file), the byte ranges of its media in that file, and where each chunk starts
/// as (range number, offset in that range).
struct Track {
    trak: Mp4Box,
    ranges: Vec<(u64, u64)>,
    chunks: Vec<(usize, u64)>,
    /// Rebuilt index for a fragmented input; None keeps the input's own.
    samples: Option<(Vec<Sample>, Vec<u32>)>,
    timescale: u32,
    duration: u64,
    /// The movie timescale of the track's own file, which its edit list counts in.
    movie_timescale: u32,
}

fn full_box_version(payload: &[u8]) -> u8 {
    payload.first().copied().unwrap_or(0)
}

fn flags(payload: &[u8]) -> u32 {
    u32::from_be_bytes([0, payload[1], payload[2], payload[3]])
}

/// mdhd's timescale and duration.
fn media_times(trak: &Mp4Box) -> io::Result<(u32, u64)> {
    let mdhd = trak.find(&[b"mdia", b"mdhd"]).ok_or_else(|| bad("track without mdhd"))?.payload();
    if full_box_version(mdhd) == 1 { Ok((be32(mdhd, 20)?, be64(mdhd, 24)?)) } else { Ok((be32(mdhd, 12)?, be32(mdhd, 16)? as u64)) }
}

fn read_track(path: &Path) -> io::Result<Track> {
    let mut file = File::open(path)?;
    let top = scan(&mut file)?;
    let moov_at = top.iter().find(|b| &b.kind == b"moov").ok_or_else(|| bad("not an MP4 file (no moov)"))?;
    let moov = read_box(&mut file, moov_at)?;
    let traks: Vec<&Mp4Box> = moov.children().iter().filter(|b| &b.kind == b"trak").collect();
    let [trak] = traks.as_slice() else {
        return Err(bad("expected exactly one track"));
    };
    let mut trak = (*trak).clone();
    let (timescale, duration) = media_times(&trak)?;
    let mvhd = moov.child(b"mvhd").ok_or_else(|| bad("movie without mvhd"))?.payload();
    let movie_timescale = if full_box_version(mvhd) == 1 { be32(mvhd, 20)? } else { be32(mvhd, 12)? };

    if top.iter().any(|b| &b.kind == b"moof") {
        // Fragmented: gather every fragment's samples into one index.
        let trex = moov.find(&[b"mvex", b"trex"]).map(|b| b.payload().to_vec()).unwrap_or_default();
        let trex_default = |at: usize| if trex.len() >= at + 4 { be32(&trex, at).unwrap_or(0) } else { 0 };
        let (def_duration, def_size, def_flags) = (trex_default(12), trex_default(16), trex_default(20));
        let mut samples = Vec::new();
        let mut chunk_samples = Vec::new();
        let mut ranges = Vec::new();
        let mut chunks = Vec::new();
        for moof_at in top.iter().filter(|b| &b.kind == b"moof") {
            let moof = read_box(&mut file, moof_at)?;
            for traf in moof.children().iter().filter(|b| &b.kind == b"traf") {
                let tfhd = traf.child(b"tfhd").ok_or_else(|| bad("fragment without tfhd"))?.payload();
                let tf = flags(tfhd);
                let mut at = 8;
                let mut base = moof_at.offset;
                if tf & 0x1 != 0 {
                    base = be64(tfhd, at)?;
                    at += 8;
                }
                if tf & 0x2 != 0 {
                    at += 4;
                }
                let mut take = |bit: u32, default: u32| -> io::Result<u32> {
                    if tf & bit != 0 {
                        let v = be32(tfhd, at)?;
                        at += 4;
                        Ok(v)
                    } else {
                        Ok(default)
                    }
                };
                let duration_default = take(0x8, def_duration)?;
                let size_default = take(0x10, def_size)?;
                let flags_default = take(0x20, def_flags)?;
                let mut data = base;
                for trun in traf.children().iter().filter(|b| &b.kind == b"trun") {
                    let p = trun.payload();
                    let (version, tr) = (full_box_version(p), flags(p));
                    let count = be32(p, 4)? as usize;
                    let mut at = 8;
                    if tr & 0x1 != 0 {
                        data = base.wrapping_add_signed(be32(p, at)? as i32 as i64);
                        at += 4;
                    }
                    let mut first_flags = None;
                    if tr & 0x4 != 0 {
                        first_flags = Some(be32(p, at)?);
                        at += 4;
                    }
                    let start = data;
                    for i in 0..count {
                        let mut field = |bit: u32, default: u32| -> io::Result<u32> {
                            if tr & bit != 0 {
                                let v = be32(p, at)?;
                                at += 4;
                                Ok(v)
                            } else {
                                Ok(default)
                            }
                        };
                        let duration = field(0x100, duration_default)?;
                        let size = field(0x200, size_default)?;
                        let sample_flags = field(0x400, if i == 0 { first_flags.unwrap_or(flags_default) } else { flags_default })?;
                        let cto = field(0x800, 0)?;
                        let cto = if version == 0 { cto.min(i32::MAX as u32) as i32 } else { cto as i32 };
                        // "sample_is_non_sync_sample"
                        let sync = sample_flags & 0x0001_0000 == 0;
                        samples.push(Sample { size, duration, sync, cto });
                        data += size as u64;
                    }
                    if count > 0 {
                        chunks.push((ranges.len(), 0));
                        ranges.push((start, data - start));
                        chunk_samples.push(count as u32);
                    }
                }
            }
        }
        let total: u64 = samples.iter().map(|s| s.duration as u64).sum();
        return Ok(Track {
            trak,
            ranges,
            chunks,
            samples: Some((samples, chunk_samples)),
            timescale,
            duration: total.max(duration),
            movie_timescale,
        });
    }

    // A plain file: keep its index, point chunks into its mdat boxes.
    let mdats: Vec<&TopBox> = top.iter().filter(|b| &b.kind == b"mdat").collect();
    let ranges: Vec<(u64, u64)> = mdats.iter().map(|m| (m.offset + m.header, m.size - m.header)).collect();
    let stbl = trak.find_mut(&[b"mdia", b"minf", b"stbl"]).ok_or_else(|| bad("track without stbl"))?;
    let offsets: Vec<u64> = if let Some(stco) = stbl.child(b"stco") {
        let p = stco.payload();
        (0..be32(p, 4)? as usize).map(|i| be32(p, 8 + i * 4).map(u64::from)).collect::<io::Result<_>>()?
    } else if let Some(co64) = stbl.child(b"co64") {
        let p = co64.payload();
        (0..be32(p, 4)? as usize).map(|i| be64(p, 8 + i * 8)).collect::<io::Result<_>>()?
    } else {
        return Err(bad("track without chunk offsets"));
    };
    let chunks = offsets
        .iter()
        .map(|&o| {
            ranges
                .iter()
                .position(|&(start, len)| o >= start && o < start + len)
                .map(|i| (i, o - ranges[i].0))
                .ok_or_else(|| bad("a chunk lies outside the media data"))
        })
        .collect::<io::Result<_>>()?;
    Ok(Track { trak, ranges, chunks, samples: None, timescale, duration, movie_timescale })
}

fn full_box(version: u8, flags: u32, body: impl FnOnce(&mut Vec<u8>)) -> Vec<u8> {
    let mut p = vec![version, (flags >> 16) as u8, (flags >> 8) as u8, flags as u8];
    body(&mut p);
    p
}

/// Index boxes (stts, ctts, stss, stsz, stsc) for samples gathered from fragments.
fn index_boxes(samples: &[Sample], chunk_samples: &[u32]) -> Vec<Mp4Box> {
    fn runs<T: PartialEq + Copy>(values: impl Iterator<Item = T>) -> Vec<(u32, T)> {
        let mut out: Vec<(u32, T)> = Vec::new();
        for v in values {
            match out.last_mut() {
                Some((n, last)) if *last == v => *n += 1,
                _ => out.push((1, v)),
            }
        }
        out
    }
    let mut boxes = Vec::new();
    let stts = runs(samples.iter().map(|s| s.duration));
    boxes.push(Mp4Box::raw(b"stts", full_box(0, 0, |p| {
        p.extend((stts.len() as u32).to_be_bytes());
        for (n, d) in &stts {
            p.extend(n.to_be_bytes());
            p.extend(d.to_be_bytes());
        }
    })));
    if samples.iter().any(|s| s.cto != 0) {
        let ctts = runs(samples.iter().map(|s| s.cto));
        let version = u8::from(samples.iter().any(|s| s.cto < 0));
        boxes.push(Mp4Box::raw(b"ctts", full_box(version, 0, |p| {
            p.extend((ctts.len() as u32).to_be_bytes());
            for (n, c) in &ctts {
                p.extend(n.to_be_bytes());
                p.extend(c.to_be_bytes());
            }
        })));
    }
    if samples.iter().any(|s| !s.sync) {
        boxes.push(Mp4Box::raw(b"stss", full_box(0, 0, |p| {
            let sync: Vec<u32> = samples.iter().enumerate().filter(|(_, s)| s.sync).map(|(i, _)| i as u32 + 1).collect();
            p.extend((sync.len() as u32).to_be_bytes());
            sync.iter().for_each(|n| p.extend(n.to_be_bytes()));
        })));
    }
    boxes.push(Mp4Box::raw(b"stsz", full_box(0, 0, |p| {
        p.extend(0u32.to_be_bytes());
        p.extend((samples.len() as u32).to_be_bytes());
        samples.iter().for_each(|s| p.extend(s.size.to_be_bytes()));
    })));
    let stsc = runs(chunk_samples.iter().copied());
    boxes.push(Mp4Box::raw(b"stsc", full_box(0, 0, |p| {
        p.extend((stsc.len() as u32).to_be_bytes());
        let mut first_chunk = 1u32;
        for (n, per_chunk) in &stsc {
            p.extend(first_chunk.to_be_bytes());
            p.extend(per_chunk.to_be_bytes());
            p.extend(1u32.to_be_bytes());
            first_chunk += n;
        }
    })));
    boxes
}

/// Sets a track's ID and durations, and swaps its index for the final one with
/// chunk offsets from `chunk_offsets`.
fn finish_trak(track: &mut Track, id: u32, movie_timescale: u32, chunk_offsets: &[u64]) -> io::Result<()> {
    let trak = &mut track.trak;
    let tkhd = trak.child_mut(b"tkhd").ok_or_else(|| bad("track without tkhd"))?.payload_mut();
    let movie_duration = track.duration * movie_timescale as u64 / track.timescale.max(1) as u64;
    if full_box_version(tkhd) == 1 {
        tkhd[20..24].copy_from_slice(&id.to_be_bytes());
        tkhd[28..36].copy_from_slice(&movie_duration.to_be_bytes());
    } else {
        tkhd[12..16].copy_from_slice(&id.to_be_bytes());
        tkhd[20..24].copy_from_slice(&(movie_duration.min(u32::MAX as u64) as u32).to_be_bytes());
    }
    // Edit lists count in the movie timescale, which may differ in the new file.
    if let Some(elst) = trak.find_mut(&[b"edts", b"elst"]) {
        let from = track.movie_timescale.max(1) as u64;
        let p = elst.payload_mut();
        let v1 = full_box_version(p) == 1;
        let count = be32(p, 4)? as usize;
        let entry = if v1 { 20 } else { 12 };
        for i in 0..count {
            let at = 8 + i * entry;
            if v1 {
                let d = be64(p, at)? * movie_timescale as u64 / from;
                p[at..at + 8].copy_from_slice(&d.to_be_bytes());
            } else {
                let d = (be32(p, at)? as u64 * movie_timescale as u64 / from).min(u32::MAX as u64) as u32;
                p[at..at + 4].copy_from_slice(&d.to_be_bytes());
            }
        }
    }
    let mdhd = trak.find_mut(&[b"mdia", b"mdhd"]).ok_or_else(|| bad("track without mdhd"))?.payload_mut();
    if full_box_version(mdhd) == 1 {
        mdhd[24..32].copy_from_slice(&track.duration.to_be_bytes());
    } else {
        mdhd[16..20].copy_from_slice(&(track.duration.min(u32::MAX as u64) as u32).to_be_bytes());
    }
    let stbl = trak.find_mut(&[b"mdia", b"minf", b"stbl"]).ok_or_else(|| bad("track without stbl"))?;
    let co64 = Mp4Box::raw(b"co64", full_box(0, 0, |p| {
        p.extend((chunk_offsets.len() as u32).to_be_bytes());
        chunk_offsets.iter().for_each(|o| p.extend(o.to_be_bytes()));
    }));
    let children = stbl.children_mut();
    match &track.samples {
        Some((samples, chunk_samples)) => {
            // Only the sample descriptions survive from a fragmented file's empty index.
            children.retain(|b| &b.kind == b"stsd" || &b.kind == b"sgpd" || &b.kind == b"sbgp");
            children.extend(index_boxes(samples, chunk_samples));
        }
        None => children.retain(|b| &b.kind != b"stco" && &b.kind != b"co64"),
    }
    children.push(co64);
    Ok(())
}

/// Writes `video` and `audio` as one MP4 at `out`.
pub(crate) fn merge(video: &Path, audio: &Path, out: &Path) -> io::Result<()> {
    let mut tracks = [read_track(video)?, read_track(audio)?];
    let video_moov = {
        let mut file = File::open(video)?;
        let top = scan(&mut file)?;
        read_box(&mut file, top.iter().find(|b| &b.kind == b"moov").unwrap())?
    };
    let mut mvhd = video_moov.child(b"mvhd").ok_or_else(|| bad("movie without mvhd"))?.clone();
    let mvhd_p = mvhd.payload_mut();
    let movie_timescale = if full_box_version(mvhd_p) == 1 { be32(mvhd_p, 20)? } else { be32(mvhd_p, 12)? };
    let movie_duration = tracks
        .iter()
        .map(|t| t.duration * movie_timescale as u64 / t.timescale.max(1) as u64)
        .max()
        .unwrap_or(0);
    if full_box_version(mvhd_p) == 1 {
        mvhd_p[24..32].copy_from_slice(&movie_duration.to_be_bytes());
    } else {
        mvhd_p[16..20].copy_from_slice(&(movie_duration.min(u32::MAX as u64) as u32).to_be_bytes());
    }
    let next_id = mvhd_p.len() - 4;
    mvhd_p[next_id..].copy_from_slice(&3u32.to_be_bytes());

    let ftyp = Mp4Box::raw(b"ftyp", [&b"isom"[..], &512u32.to_be_bytes(), b"isomiso2avc1mp41"].concat());
    // The index has the same size whatever the offsets, so lay it out once with
    // zeros to learn where the media will start.
    let build = |tracks: &mut [Track; 2], offsets: &[Vec<u64>; 2]| -> io::Result<Mp4Box> {
        let mut moov = Mp4Box { kind: *b"moov", body: Body::Children(vec![mvhd.clone()]) };
        for (i, track) in tracks.iter_mut().enumerate() {
            let mut t = Track {
                trak: track.trak.clone(),
                ranges: Vec::new(),
                chunks: Vec::new(),
                samples: track.samples.take(),
                timescale: track.timescale,
                duration: track.duration,
                movie_timescale: track.movie_timescale,
            };
            finish_trak(&mut t, i as u32 + 1, movie_timescale, &offsets[i])?;
            track.samples = t.samples.take();
            moov.children_mut().push(t.trak);
        }
        Ok(moov)
    };
    let zeros = [vec![0u64; tracks[0].chunks.len()], vec![0u64; tracks[1].chunks.len()]];
    let draft = build(&mut tracks, &zeros)?;
    const MDAT_HEADER: u64 = 16;
    let media_start = ftyp.len() + draft.len() + MDAT_HEADER;

    // Where each input range lands in the output.
    let mut position = media_start;
    let mut landed: [Vec<u64>; 2] = [Vec::new(), Vec::new()];
    for (i, track) in tracks.iter().enumerate() {
        for &(_, len) in &track.ranges {
            landed[i].push(position);
            position += len;
        }
    }
    let offsets = [0, 1].map(|i| tracks[i].chunks.iter().map(|&(range, within)| landed[i][range] + within).collect::<Vec<_>>());
    let moov = build(&mut tracks, &offsets)?;

    let mut head = Vec::new();
    ftyp.write(&mut head);
    moov.write(&mut head);
    head.extend(1u32.to_be_bytes());
    head.extend(b"mdat");
    head.extend((position - media_start + MDAT_HEADER).to_be_bytes());

    let temp = out.with_extension("muxing");
    let result = (|| {
        let mut writer = BufWriter::with_capacity(1 << 20, File::create(&temp)?);
        writer.write_all(&head)?;
        for (track, path) in tracks.iter().zip([video, audio]) {
            let mut input = File::open(path)?;
            for &(start, len) in &track.ranges {
                input.seek(SeekFrom::Start(start))?;
                let copied = io::copy(&mut (&mut input).take(len), &mut writer)?;
                if copied != len {
                    return Err(bad("the input ended early"));
                }
            }
        }
        writer.into_inner().map_err(|e| e.into_error())?.sync_all()
    })();
    match result {
        Ok(()) => std::fs::rename(&temp, out),
        Err(e) => {
            let _ = std::fs::remove_file(&temp);
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run with TACHYON_MUX_SAMPLES set to a folder holding a YouTube picture file
    /// (video.mp4) and sound file (audio.m4a).
    #[test]
    fn merges_real_youtube_streams() {
        let Some(dir) = std::env::var_os("TACHYON_MUX_SAMPLES") else { return };
        let dir = Path::new(&dir);
        let out = dir.join("merged.mp4");
        merge(&dir.join("video.mp4"), &dir.join("audio.m4a"), &out).unwrap();
        assert!(out.metadata().unwrap().len() > 0);
    }

    #[test]
    fn boxes_survive_parse_and_write() {
        let tree = Mp4Box {
            kind: *b"moov",
            body: Body::Children(vec![Mp4Box::raw(b"mvhd", vec![0; 100]), Mp4Box {
                kind: *b"trak",
                body: Body::Children(vec![Mp4Box::raw(b"tkhd", vec![1; 84])]),
            }]),
        };
        let mut bytes = Vec::new();
        tree.write(&mut bytes);
        let back = parse(&bytes).unwrap();
        let mut again = Vec::new();
        back[0].write(&mut again);
        assert_eq!(bytes, again);
        assert_eq!(back[0].find(&[b"trak", b"tkhd"]).unwrap().payload().len(), 84);
    }
}
