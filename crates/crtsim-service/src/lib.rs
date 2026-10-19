//! CRTSim Renderer as a headless service: pictures, animations and videos sent over HTTP are
//! rendered through the CRT, one at a time on one GPU, a software one in a container, and sent
//! back. A queue of bounded length holds the rest, and every request, render and result has a
//! limit. See docs/SERVICE.md.

pub mod http;
pub mod jobs;
mod render;
pub mod spec;

use anyhow::{Context as _, Result};
use http::{Body, Refused, Request, Response};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How the service runs.
#[derive(Clone, Debug)]
pub struct Settings {
    pub bind: SocketAddr,
    /// Required of every request but `/health`, as `Authorization: Bearer <token>`, when set.
    pub token: Option<String>,
    pub limits: spec::Limits,
    pub jobs: jobs::Settings,
    /// Connections handled at once; more are refused.
    pub connections: usize,
    /// The longest `POST /render` waits before answering with the job to ask after instead.
    pub wait: Duration,
    /// The longest a request may take to arrive, and a response to be sent.
    pub transfer: Duration,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1:8080".parse().expect("an address"),
            token: None,
            limits: Default::default(),
            jobs: Default::default(),
            connections: 64,
            wait: Duration::from_secs(60),
            transfer: Duration::from_secs(120),
        }
    }
}

/// What every connection's handler shares.
struct Shared {
    settings: Settings,
    queue: Arc<jobs::Queue>,
    adapter: String,
    ffmpeg: bool,
    active: AtomicUsize,
}

/// A service started, listening.
pub struct Service {
    pub address: SocketAddr,
    context: Arc<Shared>,
}

impl Service {
    /// Binds `settings.bind` and starts the worker on `renderer`, with FFmpeg's encodes if
    /// `ffmpeg`, and the sweep of finished jobs. Requests are served once `serve` is called.
    pub fn start(
        settings: Settings,
        renderer: crtsim_core::Renderer,
        ffmpeg: bool,
    ) -> Result<(Self, TcpListener)> {
        let listener = TcpListener::bind(settings.bind)
            .with_context(|| format!("Cannot listen on {}", settings.bind))?;
        let address = listener.local_addr()?;
        let queue = jobs::Queue::new(settings.jobs.clone())?;
        let adapter = format!(
            "{} ({:?}, {:?})",
            renderer.adapter.name, renderer.adapter.backend, renderer.adapter.device_type
        );
        {
            let queue = queue.clone();
            std::thread::Builder::new()
                .name("renderer".into())
                .spawn(move || jobs::work(queue, renderer))?;
        }
        {
            let queue = queue.clone();
            std::thread::Builder::new()
                .name("sweep".into())
                .spawn(move || loop {
                    std::thread::sleep(Duration::from_secs(1));
                    queue.sweep();
                })?;
        }
        let context = Arc::new(Shared {
            settings,
            queue,
            adapter,
            ffmpeg,
            active: AtomicUsize::new(0),
        });
        Ok((Self { address, context }, listener))
    }

    /// Serves `listener`'s connections until the process ends, each on a thread of its own.
    pub fn serve(&self, listener: TcpListener) {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            let context = self.context.clone();
            let spawned = std::thread::Builder::new()
                .name("connection".into())
                .spawn(move || connection(&context, stream));
            if let Err(e) = spawned {
                eprintln!("Cannot start a connection's thread: {e}");
            }
        }
    }
}

fn connection(context: &Shared, mut stream: TcpStream) {
    let started = Instant::now();
    let busy = context.active.fetch_add(1, Ordering::SeqCst) >= context.settings.connections;
    let (line, response) = if busy {
        (
            "-".to_owned(),
            Response::error(503, "The service is busy; try again shortly")
                .with_header("Retry-After", "5"),
        )
    } else {
        let deadline = started + context.settings.transfer;
        // What was asked, for the log, even of a request refused before its body.
        let mut line = "-".to_owned();
        match http::read(
            &mut stream,
            deadline,
            context.settings.limits.upload,
            |request| {
                line = format!("{} {}", request.method, request.path);
                admit(context, request)
            },
        ) {
            Ok(request) => (line, route(context, request)),
            Err(refused) => (line, refusal(refused)),
        }
    };
    let status = response.status;
    if let Err(e) = http::respond(&mut stream, response, context.settings.transfer) {
        eprintln!("{line}: the response could not be sent: {e}");
    }
    eprintln!("{line} {status} {}ms", started.elapsed().as_millis());
    context.active.fetch_sub(1, Ordering::SeqCst);
}

fn refusal(refused: Refused) -> Response {
    let response = Response::error(refused.status, refused.message);
    match refused.status {
        401 => response.with_header("WWW-Authenticate", "Bearer"),
        503 => response.with_header("Retry-After", "5"),
        _ => response,
    }
}

/// Lets a request in, by its line and headers, before its body is read: every one but a
/// health check needs the token, when there is one.
fn admit(context: &Shared, request: &Request) -> Result<(), Refused> {
    let Some(token) = &context.settings.token else {
        return Ok(());
    };
    if request.method == "GET" && request.path == "/health" {
        return Ok(());
    }
    let given = request
        .header("authorization")
        .and_then(|value| value.strip_prefix("Bearer "))
        .unwrap_or_default();
    if same(given.trim().as_bytes(), token.as_bytes()) {
        Ok(())
    } else {
        Err(Refused {
            status: 401,
            message: "Send the service's token as Authorization: Bearer <token>".into(),
        })
    }
}

/// Whether `a` and `b` are equal, taking as long whatever they hold, so how long a guess takes
/// to be refused says nothing of the token.
fn same(a: &[u8], b: &[u8]) -> bool {
    let mut difference = a.len() ^ b.len();
    for (i, byte) in b.iter().enumerate() {
        difference |= usize::from(a.get(i).copied().unwrap_or(0) ^ byte);
    }
    difference == 0
}

fn route(context: &Shared, request: Request) -> Response {
    let segments: Vec<&str> = request.path.trim_matches('/').split('/').collect();
    match (request.method.as_str(), segments.as_slice()) {
        ("GET", [""]) => index(),
        ("GET", ["health"]) => health(context),
        ("GET", ["presets"]) => presets(),
        ("POST", ["render"]) => submit(context, &request, true),
        ("POST", ["jobs"]) => submit(context, &request, false),
        ("GET", ["jobs", id]) => match context.queue.describe(id) {
            Some(description) => Response::json(200, &description),
            None => Response::error(404, "No such job; results are kept only for a while"),
        },
        ("GET", ["jobs", id, "result"]) => result(context, id),
        ("DELETE", ["jobs", id]) => {
            if context.queue.cancel(id) {
                Response::json(200, &serde_json::json!({ "id": id, "status": "cancelled" }))
            } else {
                Response::error(404, "No such job")
            }
        }
        (_, [""] | ["health"] | ["presets"] | ["render"] | ["jobs"] | ["jobs", _])
        | (_, ["jobs", _, "result"]) => Response::error(405, "Not with that method"),
        _ => Response::error(404, "No such endpoint; GET / lists them"),
    }
}

fn index() -> Response {
    let options: serde_json::Map<_, _> = spec::OPTIONS
        .iter()
        .map(|(name, what)| ((*name).to_owned(), (*what).into()))
        .collect();
    Response::json(
        200,
        &serde_json::json!({
            "service": "CRTSim Renderer",
            "version": env!("CARGO_PKG_VERSION"),
            "endpoints": {
                "GET /health": "whether it runs, on what, and how busy it is",
                "GET /presets": "the included looks, for the preset option",
                "POST /render": "render the body and answer with the result, or with the job \
                                 if it takes longer than the service waits",
                "POST /jobs": "queue the body and answer with the job at once",
                "GET /jobs/{id}": "how a job is getting on",
                "GET /jobs/{id}/result": "a finished job's result",
                "DELETE /jobs/{id}": "stop a job, or forget its result",
            },
            "options": options,
        }),
    )
}

fn health(context: &Shared) -> Response {
    let (waiting, running) = context.queue.load();
    Response::json(
        200,
        &serde_json::json!({
            "status": "ok",
            "adapter": context.adapter,
            "moving": context.ffmpeg,
            "queue": {
                "waiting": waiting,
                "running": running,
                "limit": context.settings.jobs.waiting,
            },
        }),
    )
}

fn presets() -> Response {
    let looks: Vec<_> = crtsim_core::presets::included()
        .into_iter()
        .map(|p| serde_json::json!({ "name": p.name, "description": p.description }))
        .collect();
    Response::json(200, &serde_json::Value::Array(looks))
}

/// Queues the request's job; `waiting` answers with its result if it finishes in time.
fn submit(context: &Shared, request: &Request, waiting: bool) -> Response {
    let spec = match spec::read(request, &context.settings.limits, context.ffmpeg) {
        Ok(spec) => spec,
        Err(refused) => return refusal(refused),
    };
    let id = match context.queue.submit(spec) {
        Ok(id) => id,
        Err(refused) => return refusal(refused),
    };
    let accepted = || {
        let description = context
            .queue
            .describe(&id)
            .unwrap_or_else(|| serde_json::json!({ "id": id }));
        Response::json(202, &description).with_header("Location", format!("/jobs/{id}"))
    };
    if !waiting {
        return accepted();
    }
    match context
        .queue
        .wait(&id, Instant::now() + context.settings.wait)
    {
        Some(status) if status.finished() => result(context, &id),
        _ => accepted(),
    }
}

fn result(context: &Shared, id: &str) -> Response {
    match context.queue.result(id) {
        Ok((path, format)) => Response {
            status: 200,
            content_type: format.content_type().into(),
            headers: vec![
                (
                    "Content-Disposition".into(),
                    format!("inline; filename=\"crt.{}\"", format.extension()),
                ),
                ("X-Job".into(), id.into()),
            ],
            body: Body::File(path),
        },
        Err(failure) => {
            Response::error(failure.status, failure.message).with_header("X-Job", id.to_owned())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_compare_whole() {
        assert!(same(b"secret", b"secret"));
        assert!(!same(b"secret", b"secreT"));
        assert!(!same(b"secre", b"secret"));
        assert!(!same(b"secrets", b"secret"));
        assert!(!same(b"", b"secret"));
    }
}
