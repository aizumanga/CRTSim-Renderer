//! Subtitles drawn into a video's frames, and the subtitle tracks an export keeps, read back
//! from files FFmpeg makes.
use crtsim_core::config::Config;
use crtsim_media::{Options, Subtitles, Video};
use image::RgbaImage;
use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::{atomic::AtomicBool, Arc},
};

fn ffmpeg(args: &[&str]) {
    let output = Command::new("ffmpeg")
        .args(["-v", "error", "-y"])
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn text(path: &Path) -> &str {
    path.to_str().unwrap()
}

/// A DVD subtitle: a white rectangle 120 by 24 at 100,180 on a 320 by 240 canvas, appearing
/// at each of `shows` (a time and how long it stays, in seconds), as the `.idx` file that
/// FFmpeg reads with its `.sub` beside it.
fn vobsub(dir: &Path, shows: &[(f64, f64)]) -> PathBuf {
    const WIDTH: usize = 120;
    const HEIGHT: usize = 24;
    let spu = |seconds: f64| {
        // One run to the end of each line, in colour 1, for the lines of each field.
        let field: Vec<u8> = (0..HEIGHT / 2).flat_map(|_| [0x00, 0x01]).collect();
        let control = 4 + 2 * field.len();
        let first = 4 + 3 + 3 + 7 + 5 + 1 + 1;
        let second = control + first;
        let (x1, x2, y1, y2) = (100usize, 100 + WIDTH - 1, 180usize, 180 + HEIGHT - 1);
        let mut bytes = vec![];
        let word = |bytes: &mut Vec<u8>, value: usize| bytes.extend((value as u16).to_be_bytes());
        word(&mut bytes, 0); // The size, once known.
        word(&mut bytes, control);
        bytes.extend(&field);
        bytes.extend(&field);
        word(&mut bytes, 0);
        word(&mut bytes, second);
        // The palette and alpha: colour 1 is the second palette entry, opaque.
        bytes.extend([0x03, 0x00, 0x10, 0x04, 0x00, 0xF0]);
        bytes.extend([
            0x05,
            (x1 >> 4) as u8,
            (((x1 & 15) << 4) | (x2 >> 8)) as u8,
            x2 as u8,
            (y1 >> 4) as u8,
            (((y1 & 15) << 4) | (y2 >> 8)) as u8,
            y2 as u8,
        ]);
        bytes.push(0x06);
        word(&mut bytes, 4);
        word(&mut bytes, 4 + field.len());
        bytes.extend([0x01, 0xFF]);
        // Then it stops, after a delay counted in 1024ths of 90 kHz ticks.
        word(&mut bytes, (seconds * 90000. / 1024.) as usize);
        word(&mut bytes, second);
        bytes.extend([0x02, 0xFF]);
        let size = bytes.len();
        bytes[..2].copy_from_slice(&(size as u16).to_be_bytes());
        bytes
    };
    let pts = |seconds: f64| {
        let pts = (seconds * 90000.) as u64;
        [
            0x21 | (((pts >> 30) & 7) << 1) as u8,
            (pts >> 22) as u8,
            ((((pts >> 15) & 0x7F) << 1) | 1) as u8,
            (pts >> 7) as u8,
            (((pts & 0x7F) << 1) | 1) as u8,
        ]
    };
    let (mut sub, mut index) = (vec![], String::new());
    for &(at, lasts) in shows {
        let spu = spu(lasts);
        index += &format!(
            "timestamp: 00:{:02}:{:02}:{:03}, filepos: {:09x}\n",
            at as u64 / 60,
            at as u64 % 60,
            (at.fract() * 1000.).round() as u64,
            sub.len()
        );
        // An MPEG program stream's pack, holding one private packet.
        sub.extend([
            0x00, 0x00, 0x01, 0xBA, 0x44, 0x00, 0x04, 0x00, 0x04, 0x01, 0x01, 0x89, 0xC3, 0xF8,
        ]);
        sub.extend([0x00, 0x00, 0x01, 0xBD]);
        sub.extend(((3 + 5 + 1 + spu.len()) as u16).to_be_bytes());
        sub.extend([0x81, 0x80, 5]);
        sub.extend(pts(at));
        sub.push(0x20);
        sub.extend(spu);
    }
    std::fs::write(dir.join("shows.sub"), sub).unwrap();
    let palette = ["000000", "ffffff"]
        .into_iter()
        .chain(["808080"; 14])
        .collect::<Vec<_>>()
        .join(", ");
    let idx = dir.join("shows.idx");
    std::fs::write(
        &idx,
        format!(
            "# VobSub index file, v7 (do not modify this line!)\nsize: 320x240\npalette: {palette}\n\n\
             id: en, index: 0\n{index}"
        ),
    )
    .unwrap();
    idx
}

fn open(path: &Path) -> Video {
    crtsim_media::probe(path, &Arc::new(AtomicBool::new(false))).unwrap()
}

fn frame(video: &Video, time: f64) -> RgbaImage {
    crtsim_media::preview(video, time, &Arc::new(AtomicBool::new(false))).unwrap()
}

/// How many pixels differ visibly.
fn differing(a: &RgbaImage, b: &RgbaImage) -> usize {
    assert_eq!(a.dimensions(), b.dimensions());
    a.pixels()
        .zip(b.pixels())
        .filter(|(a, b)| a.0.iter().zip(b.0).any(|(&a, b)| a.abs_diff(b) > 40))
        .count()
}

/// Two text subtitle tracks over a five second video, in a file whose name has characters
/// that mean something to a filter graph.
fn text_source(dir: &Path) -> Video {
    std::fs::write(
        dir.join("a.srt"),
        "1\n00:00:01,000 --> 00:00:02,500\nHELLO ONE\n\n2\n00:00:03,000 --> 00:00:04,000\nSECOND LINE\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("b.srt"),
        "1\n00:00:00,500 --> 00:00:04,500\nOTHER TRACK\n",
    )
    .unwrap();
    let path = dir.join("we ird [1080p], 'x'; y.mkv");
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "testsrc2=size=320x240:rate=10",
        "-i",
        text(&dir.join("a.srt")),
        "-i",
        text(&dir.join("b.srt")),
        "-map",
        "0:v",
        "-map",
        "1",
        "-map",
        "2",
        "-t",
        "5",
        "-c:v",
        "libx264",
        "-pix_fmt",
        "yuv420p",
        "-c:s",
        "srt",
        "-metadata:s:s:0",
        "language=eng",
        "-metadata:s:s:1",
        "language=jpn",
        "-metadata:s:s:1",
        "title=Signs",
        text(&path),
    ]);
    open(&path)
}

#[test]
#[ignore = "requires FFmpeg and ffprobe with libass"]
fn text_subtitles_are_drawn_where_they_show_whichever_frame_is_asked_for() {
    let dir = tempfile::tempdir().unwrap();
    let plain = text_source(dir.path());
    let labels: Vec<_> = plain.subtitles().map(|track| track.label()).collect();
    assert_eq!(labels, ["eng (subrip)", "jpn · Signs (subrip)"]);
    let first = plain.with_subtitle(Some(0));
    let second = plain.with_subtitle(Some(1));
    assert_eq!((first.subtitle, second.subtitle), (Some(0), Some(1)));

    // Before either shows, and while only the second does.
    assert!(differing(&frame(&plain, 0.2), &frame(&first, 0.2)) < 20);
    assert!(differing(&frame(&plain, 0.2), &frame(&second, 0.2)) < 20);
    assert!(differing(&frame(&plain, 2.7), &frame(&first, 2.7)) < 20);
    assert!(differing(&frame(&plain, 2.7), &frame(&second, 2.7)) > 100);
    // While both do, each in its own words.
    let (one, other) = (frame(&first, 1.5), frame(&second, 1.5));
    assert!(differing(&frame(&plain, 1.5), &one) > 100);
    assert!(differing(&one, &other) > 100);
    // The same frame, asked for by its number, is the same frame.
    let numbered = crtsim_media::preview_frame(&first, 15, &Arc::new(AtomicBool::new(false)));
    assert!(differing(&numbered.unwrap(), &one) < 20);
    // A track that is not there leaves the picture alone.
    assert_eq!(plain.with_subtitle(Some(2)).subtitle, None);
}

#[test]
#[ignore = "requires FFmpeg and ffprobe"]
fn bitmap_subtitles_are_drawn_even_when_the_frame_is_inside_one() {
    let dir = tempfile::tempdir().unwrap();
    let idx = vobsub(dir.path(), &[(20., 3.), (26., 1.)]);
    let path = dir.path().join("long.mkv");
    ffmpeg(&[
        "-f",
        "lavfi",
        "-i",
        "testsrc2=size=320x240:rate=10",
        "-i",
        text(&idx),
        "-map",
        "0:v",
        "-map",
        "1",
        "-t",
        "30",
        "-c:v",
        "libx264",
        "-pix_fmt",
        "yuv420p",
        "-c:s",
        "copy",
        text(&path),
    ]);
    let plain = open(&path);
    assert_eq!(plain.subtitles().count(), 1);
    let drawn = plain.with_subtitle(Some(0));
    assert_eq!(drawn.subtitle, Some(0));
    let seen = |time| differing(&frame(&plain, time), &frame(&drawn, time));
    // The rectangle is 2880 pixels. Seeking to the middle of the first one keeps it.
    assert!(seen(21.5) > 2000, "{}", seen(21.5));
    assert!(seen(20.2) > 2000);
    assert!(seen(26.5) > 2000);
    assert!(seen(18.) < 50);
    assert!(seen(24.) < 50);
    // The first seconds have nothing before them to look back over.
    assert!(seen(0.5) < 50);
}

#[test]
#[ignore = "requires FFmpeg and ffprobe with libass and libx264"]
fn an_export_draws_the_chosen_subtitle_and_keeps_only_the_tracks_asked_for() {
    let dir = tempfile::tempdir().unwrap();
    let source = text_source(dir.path());
    let cancel = Arc::new(AtomicBool::new(false));
    let config = Config {
        output: "320x240".into(),
        ..Config::default()
    };
    let export = |name: &str, video: &Video, options: &Options| {
        let path = dir.path().join(name);
        crtsim_media::export_with(
            video,
            &path,
            &config,
            options,
            &cancel,
            |_, image, _| Ok(image.clone()),
            |_| {},
        )
        .unwrap();
        path
    };
    let subtitles = |path: &Path| -> Vec<String> {
        let out = Command::new("ffprobe")
            .args(["-v", "error", "-select_streams", "s"])
            .args(["-show_entries", "stream_tags=language", "-of", "csv=p=0"])
            .arg(path)
            .output()
            .unwrap();
        String::from_utf8(out.stdout)
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect()
    };

    let kept = export("all.mkv", &source, &Options::default());
    assert_eq!(subtitles(&kept), ["eng", "jpn"]);
    let second_only = Options {
        keep_subtitles: Subtitles::Only(vec![1]),
        ..Options::default()
    };
    let only = export("only.mkv", &source, &second_only);
    assert_eq!(subtitles(&only), ["jpn"]);
    let none = Options {
        keep_subtitles: Subtitles::none(),
        ..Options::default()
    };
    let drawn = export("drawn.mkv", &source.with_subtitle(Some(0)), &none);
    assert!(subtitles(&drawn).is_empty());

    // The picture of the drawn one has the words, and only where they show.
    let (without, with) = (open(&kept), open(&drawn));
    assert!(differing(&frame(&without, 1.5), &frame(&with, 1.5)) > 100);
    assert!(differing(&frame(&without, 0.2), &frame(&with, 0.2)) < 20);
    assert!(differing(&frame(&without, 2.7), &frame(&with, 2.7)) < 20);
}
