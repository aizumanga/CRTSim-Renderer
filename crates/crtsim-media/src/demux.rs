//! Video files read without FFmpeg, as a browser page reads them: the container's list of
//! frames, each with where it is in the file, when it shows and whether it can be decoded on
//! its own, for a decoder the host supplies. MP4 and MOV are read by `re_mp4`; WebM and
//! Matroska here, since their reader in Rust copies every frame out of the file.
use anyhow::{bail, ensure, Context, Result};
use std::{ops::Range, path::Path};

/// A video file's tracks, as a decoder needs them.
pub struct Demuxed {
    pub video: VideoTrack,
    pub audio: Option<AudioTrack>,
    /// Seconds, from the first frame shown.
    pub duration: f64,
    /// What the container calls itself, such as `mp4` or `webm`.
    pub container: &'static str,
}

impl std::fmt::Debug for Demuxed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Demuxed({} {} frames, {:.3}s, audio {:?})",
            self.video.codec,
            self.video.samples.len(),
            self.duration,
            self.audio.as_ref().map(|audio| &audio.codec)
        )
    }
}

pub struct VideoTrack {
    /// As WebCodecs names it, such as `avc1.64001F` or `vp09.00.10.08`.
    pub codec: String,
    /// The decoder configuration the codec needs, such as H.264's `avcC`.
    pub description: Option<Vec<u8>>,
    /// The size the frames are coded at.
    pub coded: (u32, u32),
    /// The size they show at, before any rotation.
    pub display: (u32, u32),
    /// Degrees clockwise to turn each frame to show it upright: 0, 90, 180 or 270.
    pub rotation: u32,
    /// In decoding order, which is the file's.
    pub samples: Vec<Sample>,
    /// The samples in the order they show.
    pub shown: Vec<usize>,
}

impl VideoTrack {
    /// The size frames show at, upright.
    pub fn upright(&self) -> (u32, u32) {
        let (width, height) = self.display;
        if self.rotation % 180 == 90 {
            (height, width)
        } else {
            (width, height)
        }
    }
}

pub struct AudioTrack {
    /// As WebCodecs names it, such as `mp4a.40.2` or `opus`.
    pub codec: String,
    pub description: Option<Vec<u8>>,
    pub sample_rate: u32,
    pub channels: u32,
    pub samples: Vec<Sample>,
}

/// One coded frame of a track.
#[derive(Clone, Debug, PartialEq)]
pub struct Sample {
    /// Where its bytes are in the file.
    pub range: Range<usize>,
    /// When it shows, in seconds from the first frame shown.
    pub time: f64,
    pub duration: f64,
    /// Whether it decodes without the frames before it.
    pub key: bool,
}

/// Reads the tracks of the video file `name` from its `bytes`.
pub fn demux(name: &Path, bytes: &[u8]) -> Result<Demuxed> {
    let demuxed = if bytes.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]) {
        matroska(bytes)
    } else if bytes.len() >= 8 && is_mp4(&bytes[4..8]) {
        mp4(bytes)
    } else {
        bail!("{} is not an MP4, MOV, WebM or MKV video", name.display())
    }?;
    ensure!(!demuxed.video.samples.is_empty(), "The video has no frames");
    ensure!(
        demuxed.video.samples[0].key,
        "The video's first frame cannot be decoded on its own"
    );
    for sample in demuxed
        .video
        .samples
        .iter()
        .chain(demuxed.audio.iter().flat_map(|audio| &audio.samples))
    {
        ensure!(
            sample.range.end <= bytes.len(),
            "The video file is cut short"
        );
    }
    Ok(demuxed)
}

/// The video file's comment, which an export's preset is kept in: an MP4's `©cmt` entry, as
/// FFmpeg writes it, or a WebM or Matroska file's global `COMMENT` tag. `None` without one.
pub fn comment(bytes: &[u8]) -> Result<Option<String>> {
    if bytes.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]) {
        matroska_comment(bytes)
    } else if bytes.len() >= 8 && is_mp4(&bytes[4..8]) {
        mp4_comment(bytes)
    } else {
        bail!("This is not an MP4, MOV, WebM or MKV video")
    }
}

/// The boxes directly inside `range` of an MP4: each one's name and contents.
fn boxes(bytes: &[u8], range: Range<usize>) -> Result<Vec<([u8; 4], Range<usize>)>> {
    let mut found = vec![];
    let mut at = range.start;
    while at + 8 <= range.end {
        let size = u32::from_be_bytes(bytes[at..at + 4].try_into()?) as u64;
        let name: [u8; 4] = bytes[at + 4..at + 8].try_into()?;
        let (header, size) = match size {
            0 => (8, (range.end - at) as u64),
            1 => {
                ensure!(at + 16 <= range.end, "The MP4 is cut short");
                (16, u64::from_be_bytes(bytes[at + 8..at + 16].try_into()?))
            }
            size => (8, size),
        };
        let end = at
            .checked_add(usize::try_from(size)?)
            .context("The MP4 is damaged")?;
        ensure!(end <= range.end && size >= header, "The MP4 is damaged");
        found.push((name, at + header as usize..end));
        at = end;
    }
    Ok(found)
}

/// The box called `name` directly inside `range`, if there is one.
fn mp4_child(bytes: &[u8], range: Range<usize>, name: &[u8; 4]) -> Result<Option<Range<usize>>> {
    Ok(boxes(bytes, range)?
        .into_iter()
        .find(|(found, _)| found == name)
        .map(|(_, body)| body))
}

fn mp4_comment(bytes: &[u8]) -> Result<Option<String>> {
    let Some(moov) = mp4_child(bytes, 0..bytes.len(), b"moov")? else {
        return Ok(None);
    };
    let Some(udta) = mp4_child(bytes, moov, b"udta")? else {
        return Ok(None);
    };
    // FFmpeg writes it as iTunes metadata: meta, a full box, holding ilst.
    if let Some(meta) = mp4_child(bytes, udta.clone(), b"meta")? {
        let entries = (meta.start + 4).min(meta.end)..meta.end;
        if let Some(ilst) = mp4_child(bytes, entries, b"ilst")? {
            if let Some(comment) = mp4_child(bytes, ilst, b"\xa9cmt")? {
                if let Some(data) = mp4_child(bytes, comment, b"data")? {
                    // Its type and locale come before the text.
                    ensure!(data.len() >= 8, "The MP4's comment is damaged");
                    let text = &bytes[data.start + 8..data.end];
                    return Ok(Some(String::from_utf8_lossy(text).into_owned()));
                }
            }
        }
    }
    // Or in QuickTime's own way: the text's length and language, then the text.
    if let Some(comment) = mp4_child(bytes, udta, b"\xa9cmt")? {
        ensure!(comment.len() >= 4, "The MP4's comment is damaged");
        let length = usize::from(u16::from_be_bytes(
            bytes[comment.start..comment.start + 2].try_into()?,
        ));
        let text = comment.start + 4..(comment.start + 4 + length).min(comment.end);
        return Ok(Some(String::from_utf8_lossy(&bytes[text]).into_owned()));
    }
    Ok(None)
}

fn matroska_comment(bytes: &[u8]) -> Result<Option<String>> {
    let segment = children(bytes, 0..bytes.len())?
        .into_iter()
        .find(|element| element.id == 0x1853_8067)
        .context("The WebM has no segment")?;
    // A level of the file whose sizes were not written cannot be stepped over, so the tags
    // are looked for only up to there.
    let mut at = segment.body.start;
    while at < segment.body.end {
        let element = element(bytes, at, segment.body.end)?;
        if element.id == 0x1254_C367 {
            if let Some(comment) = global_comment(bytes, element.body.clone())? {
                return Ok(Some(comment));
            }
        }
        if element.unknown {
            break;
        }
        at = element.body.end;
    }
    Ok(None)
}

/// The `COMMENT` among the file-wide tags in `tags`: those whose targets name no track.
fn global_comment(bytes: &[u8], tags: Range<usize>) -> Result<Option<String>> {
    for tag in children(bytes, tags)?
        .into_iter()
        .filter(|e| e.id == 0x7373)
    {
        let parts = children(bytes, tag.body)?;
        let targeted = parts.iter().filter(|e| e.id == 0x63C0).try_fold(
            false,
            |targeted, targets| -> Result<bool> {
                let ids = children(bytes, targets.body.clone())?;
                Ok(targeted || ids.iter().any(|e| matches!(e.id, 0x63C5 | 0x63C9 | 0x63C4)))
            },
        )?;
        if targeted {
            continue;
        }
        for simple in parts.iter().filter(|e| e.id == 0x67C8) {
            let fields = children(bytes, simple.body.clone())?;
            let field = |id| {
                fields
                    .iter()
                    .find(|e| e.id == id)
                    .map(|e| &bytes[e.body.clone()])
            };
            let named = field(0x45A3).is_some_and(|name| name.eq_ignore_ascii_case(b"COMMENT"));
            if let (true, Some(text)) = (named, field(0x4487)) {
                return Ok(Some(String::from_utf8_lossy(text).into_owned()));
            }
        }
    }
    Ok(None)
}

fn is_mp4(kind: &[u8]) -> bool {
    matches!(
        kind,
        b"ftyp" | b"moov" | b"mdat" | b"free" | b"wide" | b"skip"
    )
}

/// The samples' order of showing, with their times moved to start from the first one shown,
/// and how far they moved, which other tracks move by too to stay in step.
fn shown(samples: &mut [Sample]) -> (Vec<usize>, f64) {
    let first = samples
        .iter()
        .map(|sample| sample.time)
        .fold(f64::INFINITY, f64::min);
    for sample in samples.iter_mut() {
        sample.time -= first;
    }
    let mut order: Vec<usize> = (0..samples.len()).collect();
    order.sort_by(|&a, &b| samples[a].time.total_cmp(&samples[b].time));
    (order, first)
}

/// Moves `samples` back by `by` seconds, as the video's were.
fn moved(mut samples: Vec<Sample>, by: f64) -> Vec<Sample> {
    for sample in &mut samples {
        sample.time -= by;
    }
    samples
}

fn last_end(samples: &[Sample]) -> f64 {
    samples
        .iter()
        .map(|sample| sample.time + sample.duration)
        .fold(0., f64::max)
}

fn mp4(bytes: &[u8]) -> Result<Demuxed> {
    use re_mp4::{StsdBoxContent, TrackKind};
    let mp4 = re_mp4::Mp4::read_bytes(bytes).context("Cannot read the MP4")?;
    let samples = |track: &re_mp4::Track| -> Vec<Sample> {
        track
            .samples
            .iter()
            .map(|sample| Sample {
                range: sample.byte_range(),
                time: sample.composition_timestamp as f64 / sample.timescale as f64,
                duration: sample.duration as f64 / sample.timescale as f64,
                key: sample.is_sync,
            })
            .collect()
    };
    let track = mp4
        .tracks()
        .values()
        .find(|track| track.kind == Some(TrackKind::Video))
        .context("The file has no video track")?;
    let trak = track.trak(&mp4);
    let contents = &trak.mdia.minf.stbl.stsd.contents;
    let (description, coded) = match contents {
        StsdBoxContent::Avc1(avc1) => (
            Some(avc1.avcc.raw.clone()),
            (u32::from(avc1.width), u32::from(avc1.height)),
        ),
        StsdBoxContent::Hvc1(hevc) | StsdBoxContent::Hev1(hevc) => (
            Some(hevc.hvcc.raw.clone()),
            (u32::from(hevc.width), u32::from(hevc.height)),
        ),
        StsdBoxContent::Av01(av01) => (
            Some(av01.av1c.raw.clone()),
            (u32::from(av01.width), u32::from(av01.height)),
        ),
        StsdBoxContent::Vp08(vp08) => (None, (u32::from(vp08.width), u32::from(vp08.height))),
        StsdBoxContent::Vp09(vp09) => (None, (u32::from(vp09.width), u32::from(vp09.height))),
        _ => bail!("The video's codec cannot be opened in the browser"),
    };
    let codec = track
        .codec_string(&mp4)
        .context("The video's codec cannot be opened in the browser")?;
    let tkhd = &trak.tkhd;
    let display = (
        u32::from(tkhd.width.value()),
        u32::from(tkhd.height.value()),
    );
    let display = if display.0 == 0 || display.1 == 0 {
        coded
    } else {
        display
    };
    // The track's matrix turns frames upright: its first column is the turned x axis.
    let matrix = &tkhd.matrix;
    let rotation = match (matrix.a.signum(), matrix.b.signum()) {
        (0, 1) => 90,
        (-1, 0) => 180,
        (0, -1) => 270,
        _ => 0,
    };
    let mut video_samples = samples(track);
    let (shown_order, first) = shown(&mut video_samples);
    let audio = mp4
        .tracks()
        .values()
        // re_mp4 knows a track's kind by its codec, so it leaves Opus's unknown.
        .find(|track| {
            let opus = matches!(&track.trak(&mp4).mdia.minf.stbl.stsd.contents,
                StsdBoxContent::Unknown(kind) if kind.to_string() == "Opus");
            track.kind == Some(TrackKind::Audio) || opus
        })
        .and_then(
            |track| match &track.trak(&mp4).mdia.minf.stbl.stsd.contents {
                StsdBoxContent::Mp4a(mp4a) => {
                    let config = &mp4a.esds.as_ref()?.es_desc.dec_config.dec_specific;
                    // AudioSpecificConfig: object type, sample rate index and channels.
                    let packed = u16::from(config.profile) << 11
                        | u16::from(config.freq_index) << 7
                        | u16::from(config.chan_conf) << 3;
                    Some(AudioTrack {
                        codec: format!("mp4a.40.{}", config.profile),
                        description: Some(packed.to_be_bytes().to_vec()),
                        sample_rate: u32::from(mp4a.samplerate.value()),
                        channels: u32::from(mp4a.channelcount),
                        samples: moved(samples(track), first),
                    })
                }
                StsdBoxContent::Unknown(kind) if kind.to_string() == "Opus" => {
                    let head = opus_head(bytes)?;
                    Some(AudioTrack {
                        codec: "opus".into(),
                        channels: u32::from(head[9]),
                        description: Some(head),
                        sample_rate: 48_000,
                        samples: moved(samples(track), first),
                    })
                }
                _ => None,
            },
        );
    let duration = last_end(&video_samples);
    Ok(Demuxed {
        video: VideoTrack {
            codec,
            description,
            coded,
            display,
            rotation,
            samples: video_samples,
            shown: shown_order,
        },
        audio,
        duration,
        container: "mp4",
    })
}

/// Opus's `OpusHead`, from the `dOps` box an MP4's index holds, which `re_mp4` leaves unread.
fn opus_head(bytes: &[u8]) -> Option<Vec<u8>> {
    // The index box, found among the top-level boxes so a frame's bytes are never searched.
    let mut at = 0usize;
    let moov = loop {
        let size = u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?) as usize;
        let kind = bytes.get(at + 4..at + 8)?;
        let size = match size {
            1 => usize::try_from(u64::from_be_bytes(
                bytes.get(at + 8..at + 16)?.try_into().ok()?,
            ))
            .ok()?,
            0 => bytes.len() - at,
            size => size,
        };
        if kind == b"moov" {
            break bytes.get(at..at.checked_add(size)?)?;
        }
        at = at.checked_add(size.max(8))?;
    };
    let found = moov.windows(4).position(|w| w == b"dOps")?;
    let size = u32::from_be_bytes(moov.get(found - 4..found)?.try_into().ok()?) as usize;
    let dops = moov.get(found + 4..found - 4 + size)?;
    // dOps is OpusHead's fields after its magic, big-endian where OpusHead is little-endian.
    let [_, channels, skip0, skip1, rate0, rate1, rate2, rate3, gain0, gain1, rest @ ..] = dops
    else {
        return None;
    };
    let mut head = b"OpusHead".to_vec();
    head.extend_from_slice(&[
        1, *channels, *skip1, *skip0, *rate3, *rate2, *rate1, *rate0, *gain1, *gain0,
    ]);
    head.extend_from_slice(rest);
    Some(head)
}

/// Matroska's element ids, as they are written.
mod id {
    pub const SEGMENT: u32 = 0x1853_8067;
    pub const INFO: u32 = 0x1549_A966;
    pub const TIMESTAMP_SCALE: u32 = 0x2A_D7B1;
    pub const TRACKS: u32 = 0x1654_AE6B;
    pub const TRACK_ENTRY: u32 = 0xAE;
    pub const TRACK_NUMBER: u32 = 0xD7;
    pub const TRACK_TYPE: u32 = 0x83;
    pub const CODEC_ID: u32 = 0x86;
    pub const CODEC_PRIVATE: u32 = 0x63A2;
    pub const DEFAULT_DURATION: u32 = 0x23_E383;
    pub const CONTENT_ENCODINGS: u32 = 0x6D80;
    pub const VIDEO: u32 = 0xE0;
    pub const PIXEL_WIDTH: u32 = 0xB0;
    pub const PIXEL_HEIGHT: u32 = 0xBA;
    pub const DISPLAY_WIDTH: u32 = 0x54B0;
    pub const DISPLAY_HEIGHT: u32 = 0x54BA;
    pub const AUDIO: u32 = 0xE1;
    pub const SAMPLING_FREQUENCY: u32 = 0xB5;
    pub const CHANNELS: u32 = 0x9F;
    pub const CLUSTER: u32 = 0x1F43_B675;
    pub const TIMESTAMP: u32 = 0xE7;
    pub const SIMPLE_BLOCK: u32 = 0xA3;
    pub const BLOCK_GROUP: u32 = 0xA0;
    pub const BLOCK: u32 = 0xA1;
    pub const REFERENCE_BLOCK: u32 = 0xFB;
    pub const BLOCK_DURATION: u32 = 0x9B;
    /// The segment's other children, which also end a cluster of unknown size.
    pub const SEGMENT_CHILDREN: [u32; 8] = [
        CLUSTER,
        INFO,
        TRACKS,
        0x114D_9B74,
        0x1C53_BB6B,
        0x1254_C367,
        0x1043_A770,
        0x1941_A469,
    ];
}

/// One element: its id, and where its contents are.
struct Element {
    id: u32,
    body: Range<usize>,
    /// Its size was not written, as in a file still being recorded when it was written.
    unknown: bool,
}

/// A variable-length number: its value, with the length marker kept for ids, and its length.
fn vint(bytes: &[u8], at: usize, keep_marker: bool) -> Result<(u64, usize)> {
    let first = *bytes.get(at).context("The WebM is cut short")?;
    ensure!(first != 0, "The WebM is damaged");
    let length = first.leading_zeros() as usize + 1;
    ensure!(at + length <= bytes.len(), "The WebM is cut short");
    let mut value = u64::from(if keep_marker {
        first
    } else {
        first & (0xFFu16 >> length) as u8
    });
    for &byte in &bytes[at + 1..at + length] {
        value = value << 8 | u64::from(byte);
    }
    Ok((value, length))
}

fn element(bytes: &[u8], at: usize, end: usize) -> Result<Element> {
    let (id, id_length) = vint(bytes, at, true)?;
    let (size, size_length) = vint(bytes, at + id_length, false)?;
    let start = at + id_length + size_length;
    let unknown = size == (1 << (7 * size_length)) - 1;
    let body_end = if unknown {
        end
    } else {
        start
            .checked_add(usize::try_from(size)?)
            .context("The WebM is damaged")?
    };
    ensure!(body_end <= end, "The WebM is cut short");
    Ok(Element {
        id: id as u32,
        body: start..body_end,
        unknown,
    })
}

/// The elements directly inside `range`.
fn children(bytes: &[u8], range: Range<usize>) -> Result<Vec<Element>> {
    let mut found = vec![];
    let mut at = range.start;
    while at < range.end {
        let child = element(bytes, at, range.end)?;
        at = child.body.end;
        found.push(child);
    }
    Ok(found)
}

fn unsigned(bytes: &[u8]) -> u64 {
    bytes
        .iter()
        .fold(0, |value, &byte| value << 8 | u64::from(byte))
}

fn float(bytes: &[u8]) -> f64 {
    match bytes.len() {
        4 => f64::from(f32::from_be_bytes(bytes.try_into().unwrap_or_default())),
        8 => f64::from_be_bytes(bytes.try_into().unwrap_or_default()),
        _ => 0.,
    }
}

#[derive(Default)]
struct Entry {
    number: u64,
    kind: u64,
    codec: String,
    private: Option<Vec<u8>>,
    default_duration: Option<u64>,
    pixels: (u32, u32),
    display: (u32, u32),
    sample_rate: f64,
    channels: u32,
    encoded: bool,
    samples: Vec<Sample>,
}

fn track_entry(bytes: &[u8], body: Range<usize>) -> Result<Entry> {
    let mut entry = Entry {
        channels: 1,
        sample_rate: 8000.,
        ..Entry::default()
    };
    for child in children(bytes, body)? {
        let data = &bytes[child.body.clone()];
        match child.id {
            id::TRACK_NUMBER => entry.number = unsigned(data),
            id::TRACK_TYPE => entry.kind = unsigned(data),
            id::CODEC_ID => entry.codec = String::from_utf8_lossy(data).into_owned(),
            id::CODEC_PRIVATE => entry.private = Some(data.to_vec()),
            id::DEFAULT_DURATION => entry.default_duration = Some(unsigned(data)),
            id::CONTENT_ENCODINGS => entry.encoded = true,
            id::VIDEO => {
                for field in children(bytes, child.body)? {
                    let value = unsigned(&bytes[field.body]) as u32;
                    match field.id {
                        id::PIXEL_WIDTH => entry.pixels.0 = value,
                        id::PIXEL_HEIGHT => entry.pixels.1 = value,
                        id::DISPLAY_WIDTH => entry.display.0 = value,
                        id::DISPLAY_HEIGHT => entry.display.1 = value,
                        _ => {}
                    }
                }
            }
            id::AUDIO => {
                for field in children(bytes, child.body)? {
                    let data = &bytes[field.body];
                    match field.id {
                        id::SAMPLING_FREQUENCY => entry.sample_rate = float(data),
                        id::CHANNELS => entry.channels = unsigned(data) as u32,
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    Ok(entry)
}

/// A block's frames: its track, its time against the cluster's, whether its flags mark it a
/// keyframe, and where each laced frame is.
fn block(bytes: &[u8], body: Range<usize>) -> Result<(u64, i16, bool, Vec<Range<usize>>)> {
    let (track, length) = vint(bytes, body.start, false)?;
    let at = body.start + length;
    ensure!(at + 3 <= body.end, "The WebM is damaged");
    let time = i16::from_be_bytes([bytes[at], bytes[at + 1]]);
    let flags = bytes[at + 2];
    let mut at = at + 3;
    let lacing = (flags >> 1) & 3;
    if lacing == 0 {
        return Ok((
            track,
            time,
            flags & 0x80 != 0,
            std::iter::once(at..body.end).collect(),
        ));
    }
    let count = usize::from(*bytes.get(at).context("The WebM is damaged")?) + 1;
    at += 1;
    let mut sizes = Vec::with_capacity(count);
    match lacing {
        // Xiph: each size as bytes of 255 and a last smaller one.
        1 => {
            for _ in 1..count {
                let mut size = 0;
                loop {
                    let byte = *bytes.get(at).context("The WebM is damaged")?;
                    at += 1;
                    size += usize::from(byte);
                    if byte < 255 {
                        break;
                    }
                }
                sizes.push(size);
            }
        }
        // EBML: the first size, then each as a signed difference from the one before.
        3 => {
            let (first, length) = vint(bytes, at, false)?;
            at += length;
            sizes.push(usize::try_from(first)?);
            for _ in 2..count {
                let (raw, length) = vint(bytes, at, false)?;
                at += length;
                let bias = (1i64 << (7 * length - 1)) - 1;
                let previous = *sizes.last().unwrap_or(&0) as i64;
                sizes.push(usize::try_from(previous + raw as i64 - bias)?);
            }
        }
        // Fixed: all the same size.
        _ => {
            let total = body.end.checked_sub(at).context("The WebM is damaged")?;
            sizes = vec![total / count; count - 1];
        }
    }
    let mut frames = Vec::with_capacity(count);
    for size in sizes {
        ensure!(at + size <= body.end, "The WebM is damaged");
        frames.push(at..at + size);
        at += size;
    }
    frames.push(at..body.end);
    Ok((track, time, flags & 0x80 != 0, frames))
}

fn matroska(bytes: &[u8]) -> Result<Demuxed> {
    let header = element(bytes, 0, bytes.len())?;
    let segment = element(bytes, header.body.end, bytes.len())?;
    ensure!(segment.id == id::SEGMENT, "The WebM has no segment");
    let mut scale = 1_000_000u64;
    let mut entries: Vec<Entry> = vec![];
    // Clusters of unknown size end where the next of the segment's children starts, so the
    // segment is walked element by element rather than child by child.
    let mut at = segment.body.start;
    let end = segment.body.end;
    let mut cluster_time = 0u64;
    let mut cluster_end = at;
    while at < end {
        let element = element(bytes, at, end)?;
        let in_cluster = at < cluster_end && !id::SEGMENT_CHILDREN.contains(&element.id);
        if !in_cluster {
            cluster_end = at;
        }
        match element.id {
            id::INFO => {
                for child in children(bytes, element.body.clone())? {
                    if child.id == id::TIMESTAMP_SCALE {
                        scale = unsigned(&bytes[child.body]).max(1);
                    }
                }
            }
            id::TRACKS => {
                for child in children(bytes, element.body.clone())? {
                    if child.id == id::TRACK_ENTRY {
                        entries.push(track_entry(bytes, child.body)?);
                    }
                }
            }
            id::CLUSTER => {
                cluster_time = 0;
                cluster_end = element.body.end;
                // Step into the cluster rather than over it.
                at = element.body.start;
                continue;
            }
            id::TIMESTAMP if in_cluster => cluster_time = unsigned(&bytes[element.body.clone()]),
            id::SIMPLE_BLOCK | id::BLOCK_GROUP if in_cluster => {
                let (block_body, key, duration) = if element.id == id::SIMPLE_BLOCK {
                    (element.body.clone(), None, None)
                } else {
                    let mut found = None;
                    let mut referenced = false;
                    let mut duration = None;
                    for child in children(bytes, element.body.clone())? {
                        match child.id {
                            id::BLOCK => found = Some(child.body),
                            id::REFERENCE_BLOCK => referenced = true,
                            id::BLOCK_DURATION => duration = Some(unsigned(&bytes[child.body])),
                            _ => {}
                        }
                    }
                    (
                        found.context("The WebM has an empty block group")?,
                        Some(!referenced),
                        duration,
                    )
                };
                let (track, relative, flagged, frames) = block(bytes, block_body)?;
                if let Some(entry) = entries.iter_mut().find(|entry| entry.number == track) {
                    let ticks = cluster_time as i64 + i64::from(relative);
                    let start = ticks as f64 * scale as f64 / 1e9;
                    let each = entry
                        .default_duration
                        .map(|ns| ns as f64 / 1e9)
                        .or(duration.map(|d| d as f64 * scale as f64 / 1e9 / frames.len() as f64))
                        .unwrap_or(0.);
                    for (index, range) in frames.into_iter().enumerate() {
                        entry.samples.push(Sample {
                            range,
                            time: start + each * index as f64,
                            duration: each,
                            key: key.unwrap_or(flagged) && index == 0,
                        });
                    }
                }
            }
            _ => {}
        }
        if element.unknown && element.id != id::CLUSTER {
            // Only clusters are expected without a size.
            break;
        }
        at = element.body.end;
    }
    let video = entries
        .iter()
        .position(|entry| entry.kind == 1)
        .context("The file has no video track")?;
    let mut entry = entries.swap_remove(video);
    ensure!(
        !entry.encoded,
        "The video's frames are compressed inside the file, which the browser cannot open"
    );
    // Frames without their own duration last until the next one starts.
    fill_durations(&mut entry.samples);
    let codec = matroska_codec(&entry, bytes)?;
    let description = match entry.codec.as_str() {
        "V_VP8" | "V_VP9" => None,
        _ => entry.private.clone(),
    };
    let display = if entry.display.0 > 0 && entry.display.1 > 0 {
        entry.display
    } else {
        entry.pixels
    };
    let mut samples = entry.samples;
    let (shown_order, first) = shown(&mut samples);
    let audio = entries
        .iter_mut()
        .find(|entry| entry.kind == 2 && !entry.encoded)
        .and_then(|audio| {
            let codec = match audio.codec.as_str() {
                "A_OPUS" => "opus".to_owned(),
                "A_VORBIS" => "vorbis".to_owned(),
                "A_AAC" => "mp4a.40.2".to_owned(),
                _ => return None,
            };
            fill_durations(&mut audio.samples);
            Some(AudioTrack {
                codec,
                description: audio.private.clone(),
                sample_rate: audio.sample_rate.round() as u32,
                channels: audio.channels,
                samples: moved(std::mem::take(&mut audio.samples), first),
            })
        });
    let duration = last_end(&samples);
    Ok(Demuxed {
        video: VideoTrack {
            codec,
            description,
            coded: entry.pixels,
            display,
            rotation: 0,
            samples,
            shown: shown_order,
        },
        audio,
        duration,
        container: if bytes.windows(4).take(64).any(|w| w == b"webm") {
            "webm"
        } else {
            "mkv"
        },
    })
}

fn fill_durations(samples: &mut [Sample]) {
    let mut starts: Vec<f64> = samples.iter().map(|sample| sample.time).collect();
    starts.sort_by(f64::total_cmp);
    let typical = starts
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .filter(|gap| *gap > 0.)
        .fold(f64::INFINITY, f64::min);
    for sample in samples.iter_mut().filter(|sample| sample.duration <= 0.) {
        let next = starts.partition_point(|&start| start <= sample.time);
        sample.duration = match starts.get(next) {
            Some(start) => start - sample.time,
            None if typical.is_finite() => typical,
            None => 1. / 30.,
        };
    }
}

/// The WebCodecs name of a Matroska track's codec.
fn matroska_codec(entry: &Entry, bytes: &[u8]) -> Result<String> {
    let private = entry.private.as_deref().unwrap_or_default();
    Ok(match entry.codec.as_str() {
        "V_VP8" => "vp8".into(),
        "V_VP9" => {
            // The profile and bit depth, from the first keyframe's header.
            let (profile, depth) = entry
                .samples
                .iter()
                .find(|sample| sample.key)
                .and_then(|sample| vp9_profile(&bytes[sample.range.clone()]))
                .unwrap_or((0, 8));
            format!("vp09.{profile:02}.10.{depth:02}")
        }
        "V_AV1" => {
            ensure!(private.len() >= 4, "The AV1 video has no configuration");
            let profile = private[1] >> 5;
            let level = private[1] & 0x1f;
            let tier = if private[2] & 0x80 != 0 { 'H' } else { 'M' };
            let depth = if private[2] & 0x40 == 0 {
                8
            } else if private[2] & 0x20 != 0 {
                12
            } else {
                10
            };
            format!("av01.{profile}.{level:02}{tier}.{depth:02}")
        }
        "V_MPEG4/ISO/AVC" => {
            ensure!(private.len() >= 4, "The H.264 video has no configuration");
            format!(
                "avc1.{:02X}{:02X}{:02X}",
                private[1], private[2], private[3]
            )
        }
        other => bail!("The video's codec ({other}) cannot be opened in the browser"),
    })
}

/// A VP9 keyframe's profile and bit depth, from its uncompressed header.
fn vp9_profile(frame: &[u8]) -> Option<(u8, u8)> {
    let mut bit = 0usize;
    let mut read = || -> Option<u8> {
        let value = (frame.get(bit / 8)? >> (7 - bit % 8)) & 1;
        bit += 1;
        Some(value)
    };
    // The frame marker, 0b10.
    if (read()?, read()?) != (1, 0) {
        return None;
    }
    let low = read()?;
    let profile = read()? << 1 | low;
    if profile == 3 {
        read()?;
    }
    // Not a shown copy of an earlier frame, and a keyframe.
    if read()? == 1 || read()? == 1 {
        return None;
    }
    // Shown, error resilient, then the sync code.
    read()?;
    read()?;
    let mut sync = 0u32;
    for _ in 0..24 {
        sync = sync << 1 | u32::from(read()?);
    }
    if sync != 0x49_8342 {
        return None;
    }
    let depth = if profile >= 2 {
        if read()? == 1 {
            12
        } else {
            10
        }
    } else {
        8
    };
    Some((profile, depth))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Demuxed {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name);
        demux(&path, &std::fs::read(&path).unwrap()).unwrap()
    }

    /// Frame times in the order they show, to the millisecond.
    fn shown_ms(demuxed: &Demuxed) -> Vec<u32> {
        let video = &demuxed.video;
        video
            .shown
            .iter()
            .map(|&i| (video.samples[i].time * 1000.).round() as u32)
            .collect()
    }

    // The fixtures are one second of FFmpeg's testsrc2 at 64x48 and 10 frames per second,
    // with a keyframe every 5; `tests/fixtures/README.md` has the commands.

    #[test]
    fn h264_in_mp4_with_reordered_frames_and_aac() {
        let mp4 = fixture("h264-aac.mp4");
        let video = &mp4.video;
        assert_eq!(mp4.container, "mp4");
        assert!(video.codec.starts_with("avc1.64"), "{}", video.codec);
        assert_eq!(
            video.description.as_ref().map(|d| d[0]),
            Some(1),
            "avcC version"
        );
        assert_eq!(
            (video.coded, video.display, video.rotation),
            ((64, 48), (64, 48), 0)
        );
        assert_eq!(video.samples.len(), 10);
        // B-frames: decoding order is not showing order, but every frame shows once.
        let decoding: Vec<f64> = video.samples.iter().map(|s| s.time).collect();
        assert!(decoding.windows(2).any(|pair| pair[1] < pair[0]));
        assert_eq!(shown_ms(&mp4), (0..10).map(|i| i * 100).collect::<Vec<_>>());
        let keys: Vec<usize> = (0..10).filter(|&i| video.samples[i].key).collect();
        assert_eq!(keys, vec![0, 5]);
        assert!((mp4.duration - 1.).abs() < 1e-6);
        let audio = mp4.audio.as_ref().unwrap();
        assert_eq!(
            (audio.codec.as_str(), audio.sample_rate, audio.channels),
            ("mp4a.40.2", 48000, 1)
        );
        // AAC-LC, 48 kHz (index 3), one channel.
        assert_eq!(audio.description.as_deref(), Some(&[0x11, 0x88][..]));
        assert!(audio.samples.len() > 40);
    }

    #[test]
    fn an_mp4_turned_by_its_track_matrix_shows_upright() {
        let mp4 = fixture("rotated.mp4");
        // FFmpeg's 90 degrees counterclockwise is 270 clockwise.
        assert_eq!(mp4.video.rotation, 270);
        assert_eq!(mp4.video.upright(), (48, 64));
        assert!(mp4.audio.is_none());
    }

    #[test]
    fn vp9_in_webm_with_opus() {
        let webm = fixture("vp9-opus.webm");
        let video = &webm.video;
        assert_eq!(
            (webm.container, video.codec.as_str()),
            ("webm", "vp09.00.10.08")
        );
        assert_eq!(
            (video.description.as_ref(), video.display),
            (None, (64, 48))
        );
        assert_eq!(
            shown_ms(&webm),
            (0..10).map(|i| i * 100).collect::<Vec<_>>()
        );
        let keys: Vec<usize> = (0..10).filter(|&i| video.samples[i].key).collect();
        assert_eq!(keys, vec![0, 5]);
        assert!(video
            .samples
            .iter()
            .all(|s| (s.duration - 0.1).abs() < 1e-6));
        let audio = webm.audio.as_ref().unwrap();
        assert_eq!((audio.codec.as_str(), audio.channels), ("opus", 1));
        assert!(audio.description.as_ref().unwrap().starts_with(b"OpusHead"));
        let bytes = std::fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/vp9-opus.webm"),
        )
        .unwrap();
        // Each frame's bytes are where the file has them: a keyframe starts with VP9's marker.
        assert_eq!(bytes[video.samples[5].range.start] >> 6, 2);
    }

    #[test]
    fn ten_bit_vp9_is_named_by_its_profile_and_depth() {
        assert_eq!(fixture("vp9-10bit.webm").video.codec, "vp09.02.10.10");
    }

    #[test]
    fn a_webm_written_live_with_unsized_clusters_reads_whole() {
        let webm = fixture("live-vp8.webm");
        assert_eq!(webm.video.codec, "vp8");
        assert_eq!(webm.video.samples.len(), 10);
        assert_eq!(
            shown_ms(&webm),
            (0..10).map(|i| i * 100).collect::<Vec<_>>()
        );
        assert!((webm.duration - 1.).abs() < 1e-6, "{}", webm.duration);
        // A cluster without its size, as a recording browser writes it: its 2-byte size
        // field set to all ones.
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/vp9-opus.webm");
        let mut bytes = std::fs::read(&path).unwrap();
        let cluster = bytes
            .windows(4)
            .position(|w| w == [0x1f, 0x43, 0xb6, 0x75])
            .unwrap();
        assert_eq!(bytes[cluster + 4] >> 6, 1, "a 2-byte size");
        bytes[cluster + 4..cluster + 6].copy_from_slice(&[0x7f, 0xff]);
        let unsized_cluster = demux(&path, &bytes).unwrap();
        let sized = fixture("vp9-opus.webm");
        assert_eq!(unsized_cluster.video.samples, sized.video.samples);
        assert_eq!(
            unsized_cluster.audio.unwrap().samples,
            sized.audio.unwrap().samples
        );
    }

    #[test]
    fn a_demuxed_video_is_probed_for_the_app() {
        let read = |name: &str| {
            let path = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures")
                .join(name);
            let contents = crate::Contents(std::fs::read(&path).unwrap().into());
            crate::probe_demuxed(name.into(), contents).unwrap()
        };
        let video = read("h264-aac.mp4");
        assert_eq!(
            (video.size, video.frames, video.audio),
            ((64, 48), Some(10), true)
        );
        assert_eq!((video.fps, video.rate.as_str()), (10., "10/1"));
        assert!((video.frame_time(3) - 0.3).abs() < 1e-9);
        assert_eq!(read("rotated.mp4").size, (48, 64));
        assert!(!read("vp9-10bit.webm").audio);
        assert_eq!(
            crate::probe::usual_rate(29.97),
            (30000. / 1001., "30000/1001".into())
        );
    }

    #[test]
    fn other_files_are_refused_by_name() {
        let error = demux(Path::new("notes.txt"), b"hello, this is not a video").unwrap_err();
        assert!(error.to_string().contains("notes.txt"), "{error}");
        assert!(demux(Path::new("cut.webm"), &[0x1a, 0x45, 0xdf, 0xa3, 0x84]).is_err());
    }
}
