//! MP4 and WebM files written without FFmpeg, as a browser page writes them: frames and sound
//! already encoded, held in memory, and put into one file with its index at the front so it
//! plays and seeks as soon as it opens.
use anyhow::{bail, ensure, Context, Result};

/// One encoded frame, or one packet of sound.
#[derive(Clone, Debug, PartialEq)]
pub struct Packet {
    pub data: Vec<u8>,
    /// When it shows or sounds, in seconds from the start.
    pub time: f64,
    pub duration: f64,
    /// Whether it decodes on its own; every packet of sound does.
    pub key: bool,
}

/// A picture track, its packets in decoding order.
#[derive(Clone, Debug)]
pub struct EncodedVideo {
    /// As WebCodecs names it: `avc1…` or `vp09…`.
    pub codec: String,
    /// The decoder configuration, such as H.264's `avcC`.
    pub description: Option<Vec<u8>>,
    pub size: (u32, u32),
    pub packets: Vec<Packet>,
}

/// A sound track.
#[derive(Clone, Debug)]
pub struct EncodedAudio {
    /// As WebCodecs names it: `mp4a.40.2`, `opus` or `vorbis`.
    pub codec: String,
    /// AAC's AudioSpecificConfig, Opus's `OpusHead` or Vorbis's headers.
    pub description: Option<Vec<u8>>,
    pub sample_rate: u32,
    pub channels: u32,
    pub packets: Vec<Packet>,
}

/// Whether a container written here holds sound in `codec` as it is.
pub fn takes_audio(container: crate::Container, codec: &str) -> bool {
    match container {
        crate::Container::Mp4 => codec.starts_with("mp4a.") || codec == "opus",
        crate::Container::Webm => codec == "opus" || codec == "vorbis",
        crate::Container::Mkv => false,
    }
}

/// The file `container` holds these tracks in.
pub fn write(
    container: crate::Container,
    video: &EncodedVideo,
    audio: Option<&EncodedAudio>,
) -> Result<Vec<u8>> {
    ensure!(!video.packets.is_empty(), "No frames to write");
    ensure!(
        video.packets[0].key,
        "The first frame must decode on its own"
    );
    if let Some(audio) = audio {
        ensure!(
            takes_audio(container, &audio.codec),
            "{} cannot hold {} sound",
            container.extension(),
            audio.codec
        );
    }
    match container {
        crate::Container::Mp4 => mp4(video, audio),
        crate::Container::Webm => webm(video, audio),
        crate::Container::Mkv => bail!("MKV is written by FFmpeg"),
    }
}

fn be16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_be_bytes());
}

fn be32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_be_bytes());
}

/// An MP4 box: its size, its name, then what `body` writes.
fn mp4_box(out: &mut Vec<u8>, name: &[u8; 4], body: impl FnOnce(&mut Vec<u8>)) {
    let start = out.len();
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(name);
    body(out);
    let size = (out.len() - start) as u32;
    out[start..start + 4].copy_from_slice(&size.to_be_bytes());
}

/// A box with a version and flags.
fn full_box(
    out: &mut Vec<u8>,
    name: &[u8; 4],
    version: u8,
    flags: u32,
    body: impl FnOnce(&mut Vec<u8>),
) {
    mp4_box(out, name, |out| {
        out.push(version);
        out.extend_from_slice(&flags.to_be_bytes()[1..]);
        body(out);
    });
}

/// The identity transform, as boxes that place a picture write it.
fn unity_matrix(out: &mut Vec<u8>) {
    for value in [0x0001_0000u32, 0, 0, 0, 0x0001_0000, 0, 0, 0, 0x4000_0000] {
        be32(out, value);
    }
}

/// One track of an MP4, laid out.
struct Mp4Track<'a> {
    id: u32,
    video: bool,
    timescale: u32,
    packets: &'a [Packet],
    /// The `stsd` entry.
    entry: Vec<u8>,
    size: (u32, u32),
    /// Decoding and showing times, in the track's timescale.
    decode: Vec<i64>,
    show: Vec<i64>,
    duration: i64,
}

impl<'a> Mp4Track<'a> {
    fn new(
        id: u32,
        video: bool,
        timescale: u32,
        packets: &'a [Packet],
        entry: Vec<u8>,
        size: (u32, u32),
    ) -> Self {
        let ticks = |seconds: f64| (seconds * f64::from(timescale)).round() as i64;
        let show: Vec<i64> = packets.iter().map(|p| ticks(p.time)).collect();
        // Frames decode in the order the packets come, at the times they show in order.
        let mut decode = show.clone();
        decode.sort_unstable();
        let duration = packets
            .iter()
            .map(|p| ticks(p.time + p.duration))
            .max()
            .unwrap_or(0);
        Self {
            id,
            video,
            timescale,
            packets,
            entry,
            size,
            decode,
            show,
            duration,
        }
    }

    fn seconds(&self) -> f64 {
        self.duration as f64 / f64::from(self.timescale)
    }

    fn trak(&self, out: &mut Vec<u8>, chunks: &[(u64, usize, usize)], wide: bool) {
        mp4_box(out, b"trak", |out| {
            full_box(out, b"tkhd", 0, 3, |out| {
                be32(out, 0);
                be32(out, 0);
                be32(out, self.id);
                be32(out, 0);
                be32(out, (self.seconds() * 1000.).round() as u32);
                out.extend_from_slice(&[0; 8]);
                be16(out, 0);
                be16(out, 0);
                be16(out, if self.video { 0 } else { 0x0100 });
                be16(out, 0);
                unity_matrix(out);
                be32(out, self.size.0 << 16);
                be32(out, self.size.1 << 16);
            });
            mp4_box(out, b"mdia", |out| {
                full_box(out, b"mdhd", 0, 0, |out| {
                    be32(out, 0);
                    be32(out, 0);
                    be32(out, self.timescale);
                    be32(out, self.duration as u32);
                    // "und", packed as three five-bit letters.
                    be16(out, 0x55C4);
                    be16(out, 0);
                });
                full_box(out, b"hdlr", 0, 0, |out| {
                    be32(out, 0);
                    out.extend_from_slice(if self.video { b"vide" } else { b"soun" });
                    out.extend_from_slice(&[0; 12]);
                    out.extend_from_slice(if self.video { b"Video\0" } else { b"Sound\0" });
                });
                mp4_box(out, b"minf", |out| {
                    if self.video {
                        full_box(out, b"vmhd", 0, 1, |out| out.extend_from_slice(&[0; 8]));
                    } else {
                        full_box(out, b"smhd", 0, 0, |out| out.extend_from_slice(&[0; 4]));
                    }
                    mp4_box(out, b"dinf", |out| {
                        full_box(out, b"dref", 0, 0, |out| {
                            be32(out, 1);
                            full_box(out, b"url ", 0, 1, |_| {});
                        });
                    });
                    self.stbl(out, chunks, wide);
                });
            });
        });
    }

    fn stbl(&self, out: &mut Vec<u8>, chunks: &[(u64, usize, usize)], wide: bool) {
        mp4_box(out, b"stbl", |out| {
            full_box(out, b"stsd", 0, 0, |out| {
                be32(out, 1);
                out.extend_from_slice(&self.entry);
            });
            // Decoding time deltas, run-length coded.
            let mut deltas: Vec<(u32, u32)> = vec![];
            for (i, &time) in self.decode.iter().enumerate() {
                let next = self.decode.get(i + 1).copied().unwrap_or_else(|| {
                    time + (self.packets[i].duration * f64::from(self.timescale)).round() as i64
                });
                let delta = (next - time).max(0) as u32;
                match deltas.last_mut() {
                    Some((count, last)) if *last == delta => *count += 1,
                    _ => deltas.push((1, delta)),
                }
            }
            full_box(out, b"stts", 0, 0, |out| {
                be32(out, deltas.len() as u32);
                for (count, delta) in &deltas {
                    be32(out, *count);
                    be32(out, *delta);
                }
            });
            let offsets: Vec<i64> = self
                .show
                .iter()
                .zip(&self.decode)
                .map(|(s, d)| s - d)
                .collect();
            if offsets.iter().any(|&offset| offset != 0) {
                full_box(out, b"ctts", 1, 0, |out| {
                    let mut runs: Vec<(u32, i32)> = vec![];
                    for &offset in &offsets {
                        match runs.last_mut() {
                            Some((count, last)) if i64::from(*last) == offset => *count += 1,
                            _ => runs.push((1, offset as i32)),
                        }
                    }
                    be32(out, runs.len() as u32);
                    for (count, offset) in runs {
                        be32(out, count);
                        out.extend_from_slice(&offset.to_be_bytes());
                    }
                });
            }
            if self.video && self.packets.iter().any(|p| !p.key) {
                let keys: Vec<u32> = (0..self.packets.len())
                    .filter(|&i| self.packets[i].key)
                    .map(|i| i as u32 + 1)
                    .collect();
                full_box(out, b"stss", 0, 0, |out| {
                    be32(out, keys.len() as u32);
                    for key in keys {
                        be32(out, key);
                    }
                });
            }
            let mine: Vec<&(u64, usize, usize)> = chunks.iter().collect();
            full_box(out, b"stsc", 0, 0, |out| {
                let mut runs: Vec<(u32, u32)> = vec![];
                for (index, (_, _, count)) in mine.iter().enumerate() {
                    if runs.last().is_none_or(|(_, last)| *last != *count as u32) {
                        runs.push((index as u32 + 1, *count as u32));
                    }
                }
                be32(out, runs.len() as u32);
                for (first, count) in runs {
                    be32(out, first);
                    be32(out, count);
                    be32(out, 1);
                }
            });
            full_box(out, b"stsz", 0, 0, |out| {
                be32(out, 0);
                be32(out, self.packets.len() as u32);
                for packet in self.packets {
                    be32(out, packet.data.len() as u32);
                }
            });
            if wide {
                full_box(out, b"co64", 0, 0, |out| {
                    be32(out, mine.len() as u32);
                    for (offset, _, _) in &mine {
                        out.extend_from_slice(&offset.to_be_bytes());
                    }
                });
            } else {
                full_box(out, b"stco", 0, 0, |out| {
                    be32(out, mine.len() as u32);
                    for (offset, _, _) in &mine {
                        be32(out, *offset as u32);
                    }
                });
            }
        });
    }
}

/// The `stsd` entry for a picture track.
fn visual_entry(video: &EncodedVideo) -> Result<Vec<u8>> {
    let (width, height) = video.size;
    let (name, config): (&[u8; 4], Vec<u8>) = if video.codec.starts_with("avc1") {
        let mut config = vec![];
        let avcc = video
            .description
            .as_ref()
            .context("H.264 needs its decoder configuration")?;
        mp4_box(&mut config, b"avcC", |out| out.extend_from_slice(avcc));
        (b"avc1", config)
    } else if video.codec.starts_with("vp09") {
        let fields: Vec<u8> = video
            .codec
            .split('.')
            .skip(1)
            .filter_map(|field| field.parse().ok())
            .collect();
        let (profile, level, depth) = match fields[..] {
            [profile, level, depth, ..] => (profile, level, depth),
            _ => (0, 10, 8),
        };
        let mut config = vec![];
        full_box(&mut config, b"vpcC", 1, 0, |out| {
            out.extend_from_slice(&[profile, level, depth << 4 | 1 << 1, 1, 1, 1]);
            be16(out, 0);
        });
        (b"vp09", config)
    } else {
        bail!("MP4 is written here with H.264 or VP9, not {}", video.codec);
    };
    let mut entry = vec![];
    mp4_box(&mut entry, name, |out| {
        out.extend_from_slice(&[0; 6]);
        be16(out, 1);
        out.extend_from_slice(&[0; 16]);
        be16(out, width as u16);
        be16(out, height as u16);
        be32(out, 0x0048_0000);
        be32(out, 0x0048_0000);
        be32(out, 0);
        be16(out, 1);
        out.extend_from_slice(&[0; 32]);
        be16(out, 0x0018);
        be16(out, 0xFFFF);
        out.extend_from_slice(&config);
    });
    Ok(entry)
}

/// The `stsd` entry for a sound track.
fn audio_entry(audio: &EncodedAudio) -> Result<Vec<u8>> {
    let description = audio.description.as_deref().unwrap_or_default();
    let mut entry = vec![];
    let (name, config): (&[u8; 4], Vec<u8>) = if audio.codec.starts_with("mp4a.") {
        ensure!(description.len() >= 2, "AAC needs its AudioSpecificConfig");
        let mut config = vec![];
        full_box(&mut config, b"esds", 0, 0, |out| {
            // An elementary stream descriptor holding the decoder's configuration.
            let specific = [&[0x05, description.len() as u8][..], description].concat();
            let decoder = [
                &[0x04, (13 + specific.len()) as u8, 0x40, 0x15, 0, 0, 0][..],
                &[0; 8],
                &specific,
            ]
            .concat();
            let stream = [
                &[0x03, (3 + decoder.len() + 3) as u8, 0, 1, 0][..],
                &decoder,
                &[0x06, 1, 2],
            ]
            .concat();
            out.extend_from_slice(&stream);
        });
        (b"mp4a", config)
    } else if audio.codec == "opus" {
        ensure!(
            description.len() >= 19 && description.starts_with(b"OpusHead"),
            "Opus needs its OpusHead"
        );
        let mut config = vec![];
        // OpusHead's fields, little-endian there and big-endian here.
        mp4_box(&mut config, b"dOps", |out| {
            out.push(0);
            out.push(description[9]);
            out.extend_from_slice(&[description[11], description[10]]);
            out.extend_from_slice(&[
                description[15],
                description[14],
                description[13],
                description[12],
            ]);
            out.extend_from_slice(&[description[17], description[16]]);
            out.extend_from_slice(&description[18..]);
        });
        (b"Opus", config)
    } else {
        bail!(
            "MP4 is written here with AAC or Opus sound, not {}",
            audio.codec
        );
    };
    mp4_box(&mut entry, name, |out| {
        out.extend_from_slice(&[0; 6]);
        be16(out, 1);
        out.extend_from_slice(&[0; 8]);
        be16(out, audio.channels as u16);
        be16(out, 16);
        be32(out, 0);
        be32(out, audio.sample_rate.min(65535) << 16);
        out.extend_from_slice(&config);
    });
    Ok(entry)
}

fn mp4(video: &EncodedVideo, audio: Option<&EncodedAudio>) -> Result<Vec<u8>> {
    let mut tracks = vec![Mp4Track::new(
        1,
        true,
        90_000,
        &video.packets,
        visual_entry(video)?,
        video.size,
    )];
    if let Some(audio) = audio.filter(|audio| !audio.packets.is_empty()) {
        tracks.push(Mp4Track::new(
            2,
            false,
            audio.sample_rate.max(1),
            &audio.packets,
            audio_entry(audio)?,
            (0, 0),
        ));
    }
    // Chunks of up to half a second from each track in turn, so a player reading the file
    // in order finds sound and picture together.
    let mut chunks: Vec<(usize, usize, usize)> = vec![];
    for (index, track) in tracks.iter().enumerate() {
        let mut first = 0;
        while first < track.packets.len() {
            let start = track.packets[first].time;
            let count = track.packets[first..]
                .iter()
                .take_while(|p| p.time < start + 0.5)
                .count()
                .max(1);
            chunks.push((index, first, count));
            first += count;
        }
    }
    chunks.sort_by(|a, b| {
        tracks[a.0].packets[a.1]
            .time
            .total_cmp(&tracks[b.0].packets[b.1].time)
            .then(a.0.cmp(&b.0))
    });
    let data: u64 = chunks
        .iter()
        .flat_map(|&(t, first, count)| &tracks[t].packets[first..first + count])
        .map(|p| p.data.len() as u64)
        .sum();
    let wide = data > u64::from(u32::MAX) - 1_000_000;
    let mut ftyp = vec![];
    mp4_box(&mut ftyp, b"ftyp", |out| {
        out.extend_from_slice(b"isom");
        be32(out, 0x200);
        for brand in [b"isom", b"iso2", b"mp41"] {
            out.extend_from_slice(brand);
        }
        out.extend_from_slice(if video.codec.starts_with("avc1") {
            b"avc1"
        } else {
            b"iso6"
        });
    });
    let moov = |base: u64| -> Vec<u8> {
        let mut at = base;
        let mut placed: Vec<Vec<(u64, usize, usize)>> = vec![vec![]; tracks.len()];
        for &(t, first, count) in &chunks {
            placed[t].push((at, first, count));
            at += tracks[t].packets[first..first + count]
                .iter()
                .map(|p| p.data.len() as u64)
                .sum::<u64>();
        }
        let mut out = vec![];
        mp4_box(&mut out, b"moov", |out| {
            full_box(out, b"mvhd", 0, 0, |out| {
                be32(out, 0);
                be32(out, 0);
                be32(out, 1000);
                let seconds = tracks.iter().map(Mp4Track::seconds).fold(0., f64::max);
                be32(out, (seconds * 1000.).round() as u32);
                be32(out, 0x0001_0000);
                be16(out, 0x0100);
                out.extend_from_slice(&[0; 10]);
                unity_matrix(out);
                out.extend_from_slice(&[0; 24]);
                be32(out, tracks.len() as u32 + 1);
            });
            for (track, chunks) in tracks.iter().zip(&placed) {
                track.trak(out, chunks, wide);
            }
        });
        out
    };
    let header = if wide { 16 } else { 8 };
    let size = moov(0).len() as u64;
    let moov = moov(ftyp.len() as u64 + size + header);
    ensure!(moov.len() as u64 == size, "The MP4 index changed size");
    let mut file = Vec::with_capacity(ftyp.len() + moov.len() + header as usize + data as usize);
    file.extend_from_slice(&ftyp);
    file.extend_from_slice(&moov);
    if wide {
        be32(&mut file, 1);
        file.extend_from_slice(b"mdat");
        file.extend_from_slice(&(data + 16).to_be_bytes());
    } else {
        be32(&mut file, (data + 8) as u32);
        file.extend_from_slice(b"mdat");
    }
    for &(t, first, count) in &chunks {
        for packet in &tracks[t].packets[first..first + count] {
            file.extend_from_slice(&packet.data);
        }
    }
    Ok(file)
}

/// A Matroska element: its id, as written, its size and its contents.
fn element(out: &mut Vec<u8>, id: u32, body: &[u8]) {
    let bytes = id.to_be_bytes();
    let skip = bytes.iter().take_while(|&&b| b == 0).count();
    out.extend_from_slice(&bytes[skip..]);
    let size = body.len() as u64;
    let length = (1..=8).find(|&l| size < (1 << (7 * l)) - 1).unwrap_or(8);
    let marked = size | 1 << (7 * length);
    out.extend_from_slice(&marked.to_be_bytes()[8 - length..]);
    out.extend_from_slice(body);
}

fn uint(value: u64) -> Vec<u8> {
    let bytes = value.to_be_bytes();
    let skip = bytes.iter().take_while(|&&b| b == 0).count().min(7);
    bytes[skip..].to_vec()
}

fn nested(parts: &[(u32, Vec<u8>)]) -> Vec<u8> {
    let mut out = vec![];
    for (id, body) in parts {
        element(&mut out, *id, body);
    }
    out
}

fn webm(video: &EncodedVideo, audio: Option<&EncodedAudio>) -> Result<Vec<u8>> {
    let codec = if video.codec.starts_with("vp09") {
        "V_VP9"
    } else if video.codec == "vp8" {
        "V_VP8"
    } else {
        bail!("WebM is written here with VP8 or VP9, not {}", video.codec);
    };
    let audio = audio.filter(|audio| !audio.packets.is_empty());
    let mut entries = vec![(
        0xAE,
        nested(&[
            (0xD7, uint(1)),
            (0x73C5, uint(1)),
            (0x83, uint(1)),
            (0x86, codec.as_bytes().to_vec()),
            (
                0xE0,
                nested(&[
                    (0xB0, uint(video.size.0.into())),
                    (0xBA, uint(video.size.1.into())),
                ]),
            ),
        ]),
    )];
    if let Some(audio) = audio {
        let mut fields = vec![
            (0xD7, uint(2)),
            (0x73C5, uint(2)),
            (0x83, uint(2)),
            (
                0x86,
                if audio.codec == "opus" {
                    b"A_OPUS".to_vec()
                } else {
                    b"A_VORBIS".to_vec()
                },
            ),
        ];
        if let Some(description) = &audio.description {
            fields.push((0x63A2, description.clone()));
        }
        if audio.codec == "opus" {
            // Opus needs 80 ms before a seek point to settle.
            fields.push((0x56BB, uint(80_000_000)));
        }
        fields.push((
            0xE1,
            nested(&[
                (0xB5, f64::from(audio.sample_rate).to_be_bytes().to_vec()),
                (0x9F, uint(audio.channels.into())),
            ]),
        ));
        entries.push((0xAE, nested(&fields)));
    }
    let tracks = nested(&[(0x1654_AE6B, nested(&entries))]);
    // Every packet in order of time, picture before sound at the same time.
    let ms = |seconds: f64| (seconds * 1000.).round() as i64;
    let mut packets: Vec<(u64, &Packet)> = video.packets.iter().map(|p| (1, p)).collect();
    if let Some(audio) = audio {
        packets.extend(audio.packets.iter().map(|p| (2, p)));
    }
    packets.sort_by(|a, b| ms(a.1.time).cmp(&ms(b.1.time)).then(a.0.cmp(&b.0)));
    let duration = packets
        .iter()
        .map(|(_, p)| p.time + p.duration)
        .fold(0., f64::max);
    let mut clusters = vec![];
    let mut cues = vec![];
    let mut cluster: Option<(i64, Vec<u8>)> = None;
    let flush = |cluster: &mut Option<(i64, Vec<u8>)>, clusters: &mut Vec<u8>| {
        if let Some((time, blocks)) = cluster.take() {
            let body = [nested(&[(0xE7, uint(time as u64))]), blocks].concat();
            element(clusters, 0x1F43_B675, &body);
        }
    };
    for (track, packet) in packets {
        let time = ms(packet.time).max(0);
        let starts_group = track == 1 && packet.key;
        let too_far = cluster
            .as_ref()
            .is_some_and(|(start, _)| time - start > 30_000);
        if cluster.is_none() || starts_group || too_far {
            flush(&mut cluster, &mut clusters);
            if starts_group {
                cues.push((time, clusters.len()));
            }
            cluster = Some((time, vec![]));
        }
        let (start, blocks) = cluster.as_mut().context("No cluster to write into")?;
        let mut block = vec![0x80 | track as u8];
        block.extend_from_slice(&((time - *start) as i16).to_be_bytes());
        block.push(if packet.key { 0x80 } else { 0 });
        block.extend_from_slice(&packet.data);
        element(blocks, 0xA3, &block);
    }
    flush(&mut cluster, &mut clusters);
    let info = nested(&[(
        0x1549_A966,
        nested(&[
            (0x2A_D7B1, uint(1_000_000)),
            (0x4D80, b"CRTSim Renderer".to_vec()),
            (0x5741, b"CRTSim Renderer".to_vec()),
            (0x4489, (duration * 1000.).to_be_bytes().to_vec()),
        ]),
    )]);
    // The seek head's positions are written at full width, so its size is known before them.
    let seek = |id: u32, position: u64| -> (u32, Vec<u8>) {
        (
            0x4DBB,
            nested(&[
                (0x53AB, uint(id.into())),
                (0x53AC, position.to_be_bytes().to_vec()),
            ]),
        )
    };
    let seek_head = |cues_at: u64| {
        nested(&[(
            0x114D_9B74,
            nested(&[
                seek(0x1549_A966, 0),
                seek(0x1654_AE6B, 0),
                seek(0x1C53_BB6B, cues_at),
            ]),
        )])
    };
    let head = seek_head(0).len() as u64;
    let info_at = head;
    let tracks_at = info_at + info.len() as u64;
    let clusters_at = tracks_at + tracks.len() as u64;
    let cues_at = clusters_at + clusters.len() as u64;
    let cue_points: Vec<(u32, Vec<u8>)> = cues
        .iter()
        .map(|&(time, offset)| {
            (
                0xBB,
                nested(&[
                    (0xB3, uint(time as u64)),
                    (
                        0xB7,
                        nested(&[(0xF7, uint(1)), (0xF1, uint(clusters_at + offset as u64))]),
                    ),
                ]),
            )
        })
        .collect();
    let cues = nested(&[(0x1C53_BB6B, nested(&cue_points))]);
    let seek_head = nested(&[(
        0x114D_9B74,
        nested(&[
            seek(0x1549_A966, info_at),
            seek(0x1654_AE6B, tracks_at),
            seek(0x1C53_BB6B, cues_at),
        ]),
    )]);
    ensure!(
        seek_head.len() as u64 == head,
        "The WebM seek head changed size"
    );
    let segment = [seek_head, info, tracks, clusters, cues].concat();
    let mut file = nested(&[(
        0x1A45_DFA3,
        nested(&[
            (0x4286, uint(1)),
            (0x42F7, uint(1)),
            (0x42F2, uint(4)),
            (0x42F3, uint(8)),
            (0x4282, b"webm".to_vec()),
            (0x4287, uint(4)),
            (0x4285, uint(2)),
        ]),
    )]);
    element(&mut file, 0x1853_8067, &segment);
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{demux, Container};
    use std::path::Path;

    fn fixture(name: &str) -> (Vec<u8>, demux::Demuxed) {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name);
        let bytes = std::fs::read(&path).unwrap();
        let demuxed = demux::demux(&path, &bytes).unwrap();
        (bytes, demuxed)
    }

    fn packets(bytes: &[u8], samples: &[demux::Sample]) -> Vec<Packet> {
        samples
            .iter()
            .map(|s| Packet {
                data: bytes[s.range.clone()].to_vec(),
                time: s.time,
                duration: s.duration,
                key: s.key,
            })
            .collect()
    }

    /// The tracks of a fixture, as an encoder would hand them over.
    fn tracks(name: &str) -> (EncodedVideo, Option<EncodedAudio>) {
        let (bytes, demuxed) = fixture(name);
        let video = EncodedVideo {
            codec: demuxed.video.codec.clone(),
            description: demuxed.video.description.clone(),
            size: demuxed.video.coded,
            packets: packets(&bytes, &demuxed.video.samples),
        };
        let audio = demuxed.audio.as_ref().map(|audio| EncodedAudio {
            codec: audio.codec.clone(),
            description: audio.description.clone(),
            sample_rate: audio.sample_rate,
            channels: audio.channels,
            packets: packets(&bytes, &audio.samples),
        });
        (video, audio)
    }

    /// Writes the tracks, reads them back, and checks every packet came back as it went in.
    fn round_trip(
        container: Container,
        video: &EncodedVideo,
        audio: Option<&EncodedAudio>,
    ) -> Vec<u8> {
        let file = write(container, video, audio).unwrap();
        let name = format!("out.{}", container.extension());
        let back = demux::demux(Path::new(&name), &file).unwrap();
        assert_eq!(back.video.codec, video.codec);
        assert_eq!(back.video.samples.len(), video.packets.len());
        for (sample, packet) in back.video.samples.iter().zip(&video.packets) {
            assert_eq!(&file[sample.range.clone()], &packet.data[..]);
            assert_eq!(sample.key, packet.key);
            assert!(
                (sample.time - packet.time).abs() < 0.001,
                "{} {}",
                sample.time,
                packet.time
            );
        }
        let (Some(audio), Some(back)) = (audio, back.audio.as_ref()) else {
            assert!(audio.is_none(), "the sound was lost");
            return file;
        };
        assert_eq!(
            (back.codec.as_str(), back.sample_rate, back.channels),
            (audio.codec.as_str(), audio.sample_rate, audio.channels)
        );
        assert_eq!(back.samples.len(), audio.packets.len());
        for (sample, packet) in back.samples.iter().zip(&audio.packets) {
            assert_eq!(&file[sample.range.clone()], &packet.data[..]);
            assert!((sample.time - packet.time).abs() < 0.001);
        }
        file
    }

    #[test]
    fn h264_and_aac_written_as_mp4_read_back_whole() {
        let (video, audio) = tracks("h264-aac.mp4");
        let file = round_trip(Container::Mp4, &video, audio.as_ref());
        // The index comes before the frames, so a browser can play it as it loads.
        let moov = file.windows(4).position(|w| w == b"moov").unwrap();
        let mdat = file.windows(4).position(|w| w == b"mdat").unwrap();
        assert!(moov < mdat);
    }

    #[test]
    fn vp9_and_opus_written_as_webm_and_as_mp4_read_back_whole() {
        let (video, audio) = tracks("vp9-opus.webm");
        round_trip(Container::Webm, &video, audio.as_ref());
        round_trip(Container::Mp4, &video, audio.as_ref());
        round_trip(Container::Webm, &video, None);
    }

    #[test]
    fn sound_a_container_cannot_hold_is_refused() {
        let (video, audio) = tracks("h264-aac.mp4");
        assert!(!takes_audio(Container::Webm, "mp4a.40.2"));
        assert!(write(Container::Webm, &video, audio.as_ref()).is_err());
        assert!(takes_audio(Container::Mp4, "opus"));
    }

    /// FFmpeg, which shares no code with the reader here, decodes every frame and packet.
    #[test]
    #[ignore = "requires FFmpeg"]
    fn ffmpeg_reads_what_is_written() {
        let dir = tempfile::tempdir().unwrap();
        for (fixture_name, container) in [
            ("h264-aac.mp4", Container::Mp4),
            ("vp9-opus.webm", Container::Webm),
            ("vp9-opus.webm", Container::Mp4),
        ] {
            let (video, audio) = tracks(fixture_name);
            let path = dir
                .path()
                .join(format!("{fixture_name}.{}", container.extension()));
            std::fs::write(&path, write(container, &video, audio.as_ref()).unwrap()).unwrap();
            let output = std::process::Command::new("ffmpeg")
                .args(["-v", "error", "-i"])
                .arg(&path)
                .args(["-f", "null", "-"])
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{fixture_name}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(
                output.stderr.is_empty(),
                "{fixture_name}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let probe = std::process::Command::new("ffprobe")
                .args([
                    "-v",
                    "error",
                    "-count_frames",
                    "-show_entries",
                    "stream=codec_type,nb_read_frames",
                    "-of",
                    "csv=p=0",
                ])
                .arg(&path)
                .output()
                .unwrap();
            let text = String::from_utf8_lossy(&probe.stdout);
            assert!(text.contains("video,10"), "{fixture_name}: {text}");
            assert!(text.contains("audio,"), "{fixture_name}: {text}");
        }
    }
}
