//! Just enough HTTP/1.1 for the service: one request a connection, its headers parsed by
//! httparse, its body read by its Content-Length and no further, every read and write within a
//! deadline, and a response that closes the connection. Behind a reverse proxy, the proxy
//! speaks the rest of HTTP to the world.

use std::io::{Read, Write};
use std::time::{Duration, Instant};

/// The most a request's line and headers may take.
const HEAD_LIMIT: usize = 16 * 1024;
/// The most headers a request may have.
const HEADERS: usize = 64;

pub struct Request {
    pub method: String,
    pub path: String,
    pub query: Vec<(String, String)>,
    headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Request {
    /// The value of the header `name`, in any letter case.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// What a response carries.
pub enum Body {
    Bytes(Vec<u8>),
    /// A file sent as it is read, rather than read into memory first.
    File(std::path::PathBuf),
}

pub struct Response {
    pub status: u16,
    pub content_type: String,
    pub headers: Vec<(String, String)>,
    pub body: Body,
}

impl Response {
    pub fn json(status: u16, value: &serde_json::Value) -> Self {
        let mut body = serde_json::to_vec_pretty(value).expect("JSON from a value");
        body.push(b'\n');
        Self {
            status,
            content_type: "application/json".into(),
            headers: vec![],
            body: Body::Bytes(body),
        }
    }

    /// An error, saying what went wrong in a sentence.
    pub fn error(status: u16, message: impl std::fmt::Display) -> Self {
        Self::json(status, &serde_json::json!({ "error": message.to_string() }))
    }

    pub fn with_header(mut self, name: &str, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }
}

/// A request that could not be read, and the status that says why.
#[derive(Debug)]
pub struct Refused {
    pub status: u16,
    pub message: String,
}

fn refused(status: u16, message: impl Into<String>) -> Refused {
    Refused {
        status,
        message: message.into(),
    }
}

/// Reads one request from `stream` before `deadline`. Its body may be at most `limit` bytes;
/// `admit` sees its line and headers before the body is read, and may refuse it, so a request
/// that will be refused is never uploaded. A client that asked to be told to go on is told so
/// once `admit` lets it.
pub fn read(
    stream: &mut std::net::TcpStream,
    deadline: Instant,
    limit: u64,
    admit: impl FnOnce(&Request) -> Result<(), Refused>,
) -> Result<Request, Refused> {
    let mut buffer = Vec::with_capacity(4096);
    let (mut request, head) = loop {
        if let Some(parsed) = parse_head(&buffer)? {
            break parsed;
        }
        if buffer.len() >= HEAD_LIMIT {
            return Err(refused(431, "The request's headers are too large"));
        }
        let mut chunk = [0; 4096];
        let read = read_by(stream, &mut chunk, deadline)?;
        if read == 0 {
            return Err(refused(
                400,
                "The connection closed before the request was read",
            ));
        }
        buffer.extend_from_slice(&chunk[..read]);
    };
    if request.header("transfer-encoding").is_some() {
        return Err(refused(411, "Send the body with a Content-Length"));
    }
    let length = match request.header("content-length") {
        Some(text) => text
            .trim()
            .parse::<u64>()
            .map_err(|_| refused(400, "The Content-Length is not a number"))?,
        None => 0,
    };
    if length > limit {
        return Err(refused(
            413,
            format!("The upload is {length} bytes; this service takes at most {limit}",),
        ));
    }
    admit(&request)?;
    if length > 0
        && request
            .header("expect")
            .is_some_and(|e| e.eq_ignore_ascii_case("100-continue"))
    {
        write_by(stream, b"HTTP/1.1 100 Continue\r\n\r\n", deadline)?;
    }
    let length = length as usize;
    let mut body = buffer.split_off(head);
    if body.len() > length {
        return Err(refused(
            400,
            "The request carries more than its Content-Length",
        ));
    }
    body.reserve_exact(length - body.len());
    while body.len() < length {
        let mut chunk = vec![0; (length - body.len()).min(64 * 1024)];
        let read = read_by(stream, &mut chunk, deadline)?;
        if read == 0 {
            return Err(refused(
                400,
                "The connection closed before the upload was read",
            ));
        }
        body.extend_from_slice(&chunk[..read]);
    }
    request.body = body;
    Ok(request)
}

/// The request's line and headers, and where its body starts, once `buffer` holds them all.
fn parse_head(buffer: &[u8]) -> Result<Option<(Request, usize)>, Refused> {
    let mut headers = [httparse::EMPTY_HEADER; HEADERS];
    let mut parsed = httparse::Request::new(&mut headers);
    let head = match parsed.parse(buffer) {
        Ok(httparse::Status::Complete(head)) => head,
        Ok(httparse::Status::Partial) => return Ok(None),
        Err(httparse::Error::TooManyHeaders) => {
            return Err(refused(431, "The request has too many headers"))
        }
        Err(e) => return Err(refused(400, format!("Malformed request: {e}"))),
    };
    let target = parsed.path.unwrap_or("/");
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let request = Request {
        method: parsed.method.unwrap_or_default().to_owned(),
        path: path.to_owned(),
        query: form_urlencoded::parse(query.as_bytes())
            .into_owned()
            .collect(),
        headers: parsed
            .headers
            .iter()
            .map(|h| {
                (
                    h.name.to_owned(),
                    String::from_utf8_lossy(h.value).into_owned(),
                )
            })
            .collect(),
        body: vec![],
    };
    Ok(Some((request, head)))
}

fn remaining(deadline: Instant) -> Result<Duration, Refused> {
    let left = deadline.saturating_duration_since(Instant::now());
    if left.is_zero() {
        return Err(refused(408, "The request took too long to arrive"));
    }
    Ok(left)
}

fn read_by(
    stream: &mut std::net::TcpStream,
    into: &mut [u8],
    deadline: Instant,
) -> Result<usize, Refused> {
    stream
        .set_read_timeout(Some(remaining(deadline)?))
        .map_err(|e| refused(500, e.to_string()))?;
    stream.read(into).map_err(|e| match e.kind() {
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut => {
            refused(408, "The request took too long to arrive")
        }
        _ => refused(400, format!("Cannot read the request: {e}")),
    })
}

fn write_by(
    stream: &mut std::net::TcpStream,
    bytes: &[u8],
    deadline: Instant,
) -> Result<(), Refused> {
    stream
        .set_write_timeout(Some(remaining(deadline)?))
        .map_err(|e| refused(500, e.to_string()))?;
    stream
        .write_all(bytes)
        .map_err(|e| refused(500, e.to_string()))
}

fn reason(status: u16) -> &'static str {
    match status {
        100 => "Continue",
        200 => "OK",
        202 => "Accepted",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        409 => "Conflict",
        411 => "Length Required",
        413 => "Content Too Large",
        415 => "Unsupported Media Type",
        422 => "Unprocessable Content",
        431 => "Request Header Fields Too Large",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "",
    }
}

/// Writes `response` and closes the connection, within `timeout`.
pub fn respond(
    stream: &mut std::net::TcpStream,
    response: Response,
    timeout: Duration,
) -> std::io::Result<()> {
    stream.set_write_timeout(Some(timeout))?;
    let length = match &response.body {
        Body::Bytes(bytes) => bytes.len() as u64,
        Body::File(path) => std::fs::metadata(path)?.len(),
    };
    let mut head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {length}\r\nConnection: close\r\n\
         Cache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\n",
        response.status,
        reason(response.status),
        response.content_type
    );
    for (name, value) in &response.headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("\r\n");
    let mut out = std::io::BufWriter::new(&mut *stream);
    out.write_all(head.as_bytes())?;
    match response.body {
        Body::Bytes(bytes) => out.write_all(&bytes)?,
        Body::File(path) => {
            std::io::copy(&mut std::fs::File::open(path)?, &mut out)?;
        }
    }
    out.flush()?;
    drop(out);
    let _ = stream.shutdown(std::net::Shutdown::Write);
    Ok(())
}

/// Requests made from bytes, for tests that need no connection.
#[cfg(test)]
pub(crate) mod tests_support {
    use super::*;

    /// The request in `raw`, all of it.
    pub fn parse(raw: &[u8]) -> Request {
        let (mut request, head) = parse_head(raw).ok().flatten().expect("a whole request");
        request.body = raw[head..].to_vec();
        request
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{TcpListener, TcpStream};

    /// `raw` sent to `read`, as a client would send it, and what `read` made of it.
    fn exchange(raw: &[u8], limit: u64) -> Result<Request, Refused> {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let raw = raw.to_vec();
        let client = std::thread::spawn(move || {
            let mut stream = TcpStream::connect(address).unwrap();
            let _ = stream.write_all(&raw);
            // Keep the connection open until the server has read what it wants.
            std::thread::sleep(Duration::from_millis(300));
        });
        let (mut stream, _) = listener.accept().unwrap();
        let deadline = Instant::now() + Duration::from_millis(200);
        let read = read(&mut stream, deadline, limit, |_| Ok(()));
        client.join().unwrap();
        read
    }

    #[test]
    fn a_request_is_read_with_its_query_headers_and_body() {
        let request = exchange(
            b"POST /render?preset=NTSC%20240p&size=720p HTTP/1.1\r\nHost: x\r\n\
              Content-Length: 5\r\nAuthorization: Bearer abc\r\n\r\nhello",
            1024,
        )
        .unwrap();
        assert_eq!(
            (request.method.as_str(), request.path.as_str()),
            ("POST", "/render")
        );
        assert_eq!(
            request.query,
            vec![
                ("preset".into(), "NTSC 240p".into()),
                ("size".into(), "720p".into())
            ]
        );
        assert_eq!(request.header("authorization"), Some("Bearer abc"));
        assert_eq!(request.body, b"hello");
    }

    #[test]
    fn a_request_past_its_limits_is_refused_with_why() {
        let status = |raw: &[u8], limit| exchange(raw, limit).err().map(|r| r.status);
        assert_eq!(
            status(b"POST / HTTP/1.1\r\nContent-Length: 2000\r\n\r\n", 1000),
            Some(413)
        );
        assert_eq!(
            status(
                b"POST / HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n",
                1000
            ),
            Some(411)
        );
        // A body that never arrives in full, and headers that never end.
        assert_eq!(
            status(b"POST / HTTP/1.1\r\nContent-Length: 10\r\n\r\nabc", 1000),
            Some(408)
        );
        assert_eq!(status(b"GET / HTTP/1.1\r\nHost: x\r\n", 1000), Some(408));
        let mut many = b"GET / HTTP/1.1\r\n".to_vec();
        for i in 0..70 {
            many.extend(format!("X-{i}: y\r\n").as_bytes());
        }
        many.extend(b"\r\n");
        assert_eq!(status(&many, 1000), Some(431));
        assert_eq!(status(b"NOT HTTP AT ALL\r\n\r\n", 1000), Some(400));
    }
}
