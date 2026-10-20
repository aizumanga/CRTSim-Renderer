//! The service as a client sees it: started on a renderer, spoken to over HTTP.
//!
//! ```text
//! cargo test -p crtsim-service --test service -- --ignored --nocapture
//! ```

use crtsim_core::{config, presets, Renderer, Sequence};
use crtsim_service::{jobs, spec::Limits, Service, Settings};
use image::GenericImageView as _;
use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

const TOKEN: &str = "a-test-token-of-some-length";

/// A response: its status, headers and body.
struct Answer {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Answer {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).expect("a JSON answer")
    }
}

fn ask(address: SocketAddr, method: &str, target: &str, token: bool, body: &[u8]) -> Answer {
    let mut stream = TcpStream::connect(address).unwrap();
    let mut request = format!(
        "{method} {target} HTTP/1.1\r\nHost: test\r\nContent-Length: {}\r\n",
        body.len()
    );
    if token {
        request.push_str(&format!("Authorization: Bearer {TOKEN}\r\n"));
    }
    request.push_str("\r\n");
    stream.write_all(request.as_bytes()).unwrap();
    let _ = stream.write_all(body);
    let mut raw = vec![];
    stream.read_to_end(&mut raw).unwrap();
    let head_end = raw.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    let head = String::from_utf8(raw[..head_end].to_vec()).unwrap();
    let mut lines = head.lines();
    let status = lines
        .next()
        .unwrap()
        .split(' ')
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    let headers = lines
        .filter_map(|l| l.split_once(": "))
        .map(|(n, v)| (n.to_owned(), v.to_owned()))
        .collect();
    Answer {
        status,
        headers,
        body: raw[head_end + 4..].to_vec(),
    }
}

fn start(settings: Settings, ffmpeg: bool) -> SocketAddr {
    let renderer = pollster::block_on(Renderer::new(wgpu::Backends::VULKAN)).unwrap();
    let (service, listener) = Service::start(settings, renderer, ffmpeg).unwrap();
    let address = service.address;
    std::thread::spawn(move || service.serve(listener));
    address
}

fn settings() -> Settings {
    Settings {
        bind: "127.0.0.1:0".parse().unwrap(),
        token: Some(TOKEN.into()),
        limits: Limits {
            upload: 4 * 1024 * 1024,
            ..Limits::default()
        },
        ..Settings::default()
    }
}

fn png(image: &image::RgbaImage) -> Vec<u8> {
    let mut bytes = std::io::Cursor::new(vec![]);
    image
        .write_to(&mut bytes, image::ImageOutputFormat::Png)
        .unwrap();
    bytes.into_inner()
}

#[test]
#[ignore = "requires a Vulkan adapter"]
fn the_service_renders_what_the_renderer_renders_and_keeps_to_its_limits() {
    let ffmpeg = crtsim_media::ToolCheck::run(&Default::default()).ready();
    let address = start(settings(), ffmpeg);
    let health = ask(address, "GET", "/health", false, b"");
    assert_eq!(health.status, 200);
    assert_eq!(health.json()["status"], "ok");
    assert_eq!(ask(address, "GET", "/presets", false, b"").status, 401);
    let presets = ask(address, "GET", "/presets", true, b"").json();
    assert_eq!(presets.as_array().unwrap().len(), presets::included().len());

    // A still, rendered as the renderer renders it.
    let card = config::test_card();
    let answer = ask(
        address,
        "POST",
        "/render?preset=NTSC%20240p&size=320x240",
        true,
        &png(&card),
    );
    assert_eq!(
        answer.status,
        200,
        "{}",
        String::from_utf8_lossy(&answer.body)
    );
    assert_eq!(answer.header("content-type"), Some("image/png"));
    let rendered = image::load_from_memory(&answer.body).unwrap().to_rgba8();
    let mut look = presets::named("NTSC 240p").unwrap().config;
    look.output = "320x240".into();
    let renderer = pollster::block_on(Renderer::new(wgpu::Backends::VULKAN)).unwrap();
    let expected = pollster::block_on(async {
        let mut sequence = Sequence::still();
        renderer
            .frame(&mut sequence, &card, &look, None, |_| {})
            .await?;
        renderer.read(&mut sequence).await
    })
    .unwrap();
    assert_eq!(rendered, expected);

    // A job queued, asked after, fetched and forgotten.
    let queued = ask(address, "POST", "/jobs?size=160x120", true, &png(&card));
    assert_eq!(queued.status, 202);
    let id = queued.json()["id"].as_str().unwrap().to_owned();
    assert_eq!(
        queued.header("location"),
        Some(format!("/jobs/{id}").as_str())
    );
    let mut status = String::new();
    for _ in 0..600 {
        status = ask(address, "GET", &format!("/jobs/{id}"), true, b"").json()["status"]
            .as_str()
            .unwrap()
            .to_owned();
        if status != "queued" && status != "running" {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(status, "done");
    let result = ask(address, "GET", &format!("/jobs/{id}/result"), true, b"");
    assert_eq!(result.status, 200);
    assert_eq!(
        image::load_from_memory(&result.body).unwrap().dimensions(),
        (160, 120)
    );
    assert_eq!(
        ask(address, "DELETE", &format!("/jobs/{id}"), true, b"").status,
        200
    );
    assert_eq!(
        ask(address, "GET", &format!("/jobs/{id}"), true, b"").status,
        404
    );

    // Refused, and why: a wrong token, too large an upload, an unknown option, nothing to draw.
    let mut wrong = TcpStream::connect(address).unwrap();
    wrong
        .write_all(
            b"POST /render HTTP/1.1\r\nAuthorization: Bearer nope\r\nContent-Length: 3\r\n\r\nabc",
        )
        .unwrap();
    let mut answer = String::new();
    wrong.read_to_string(&mut answer).unwrap();
    assert!(answer.starts_with("HTTP/1.1 401"), "{answer}");
    let mut large = TcpStream::connect(address).unwrap();
    large
        .write_all(
            format!(
                "POST /render HTTP/1.1\r\nAuthorization: Bearer {TOKEN}\r\nContent-Length: 99999999\r\n\r\n"
            )
            .as_bytes(),
        )
        .unwrap();
    let mut answer = String::new();
    large.read_to_string(&mut answer).unwrap();
    assert!(
        answer.starts_with("HTTP/1.1 413"),
        "refused before the upload: {answer}"
    );
    let unknown = ask(address, "POST", "/render?glow=lots", true, &png(&card));
    assert_eq!(unknown.status, 400);
    assert!(unknown.json()["error"].as_str().unwrap().contains("glow"));
    assert_eq!(ask(address, "POST", "/render", true, b"").status, 400);
    assert_eq!(ask(address, "GET", "/nowhere", true, b"").status, 404);

    // An animated GIF through FFmpeg, when there is one.
    if ffmpeg {
        let frames: Vec<_> = (0..4u8)
            .map(|i| {
                let image =
                    image::RgbaImage::from_pixel(32, 24, image::Rgba([i * 60, 0, 255, 255]));
                image::Frame::from_parts(image, 0, 0, image::Delay::from_numer_denom_ms(100, 1))
            })
            .collect();
        let mut gif = vec![];
        {
            let mut encoder = image::codecs::gif::GifEncoder::new(&mut gif);
            encoder.encode_frames(frames).unwrap();
        }
        let answer = ask(
            address,
            "POST",
            "/render?max_side=160&seconds=1&fps=10",
            true,
            &gif,
        );
        assert_eq!(
            answer.status,
            200,
            "{}",
            String::from_utf8_lossy(&answer.body)
        );
        assert_eq!(answer.header("content-type"), Some("image/gif"));
        assert!(answer.body.starts_with(b"GIF8"));
    }
}

#[test]
#[ignore = "requires a Vulkan adapter"]
fn a_full_queue_refuses_more_until_jobs_finish() {
    let address = start(
        Settings {
            jobs: jobs::Settings {
                waiting: 0,
                ..Default::default()
            },
            ..settings()
        },
        false,
    );
    let answer = ask(address, "POST", "/jobs", true, &png(&config::test_card()));
    assert_eq!(answer.status, 503);
    assert_eq!(answer.header("retry-after"), Some("5"));
}
