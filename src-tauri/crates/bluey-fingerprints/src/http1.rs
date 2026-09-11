//! Just enough HTTP/1.1 to sit between an official CLI and reqwest: parse one request
//! head, read its body (`Content-Length` or chunked), and know which headers are
//! hop-by-hop. Responses are written by the proxy with chunked framing.

use std::fmt;

use tokio::io::{AsyncRead, AsyncReadExt};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestHead {
    pub method: String,
    /// Origin-form request target (`/v1/messages?beta=true`); absolute-form targets are
    /// reduced to their path and query.
    pub target: String,
    pub version: String,
    /// Lower-cased names in wire order.
    pub headers: Vec<(String, String)>,
}

impl RequestHead {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }

    pub fn content_length(&self) -> Option<usize> {
        self.header("content-length")
            .and_then(|v| v.trim().parse().ok())
    }

    pub fn is_chunked(&self) -> bool {
        self.header("transfer-encoding")
            .is_some_and(|v| v.to_ascii_lowercase().contains("chunked"))
    }

    pub fn expects_continue(&self) -> bool {
        self.header("expect")
            .is_some_and(|v| v.eq_ignore_ascii_case("100-continue"))
    }
}

#[derive(Debug)]
pub enum ReadError {
    HeadTooLarge,
    BodyTooLarge,
    Malformed(&'static str),
    Closed,
    Io(std::io::Error),
}

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReadError::HeadTooLarge => write!(f, "request head exceeds the limit"),
            ReadError::BodyTooLarge => write!(f, "request body exceeds the limit"),
            ReadError::Malformed(what) => write!(f, "malformed request: {what}"),
            ReadError::Closed => write!(f, "connection closed before a full request arrived"),
            ReadError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ReadError {}

impl From<std::io::Error> for ReadError {
    fn from(e: std::io::Error) -> Self {
        ReadError::Io(e)
    }
}

/// `http://host/path?q` → `/path?q`; origin-form targets pass through.
pub fn origin_form(target: &str) -> String {
    if target.starts_with('/') {
        return target.to_string();
    }
    if let Some(rest) = target
        .strip_prefix("http://")
        .or_else(|| target.strip_prefix("https://"))
    {
        return match rest.find('/') {
            Some(i) => rest[i..].to_string(),
            None => "/".to_string(),
        };
    }
    format!("/{}", target.trim_start_matches('/'))
}

/// Parse a request head (everything before the blank line).
pub fn parse_head(bytes: &[u8]) -> Result<RequestHead, ReadError> {
    let text = std::str::from_utf8(bytes).map_err(|_| ReadError::Malformed("head is not UTF-8"))?;
    let mut lines = text.split("\r\n").flat_map(|l| l.split('\n'));
    let request_line = lines
        .next()
        .ok_or(ReadError::Malformed("empty request line"))?;
    let mut parts = request_line.split_whitespace();
    let method = parts
        .next()
        .ok_or(ReadError::Malformed("missing method"))?
        .to_string();
    let target = parts
        .next()
        .ok_or(ReadError::Malformed("missing request target"))?;
    let version = parts.next().unwrap_or("HTTP/1.1").to_string();
    if !version.starts_with("HTTP/1.") {
        return Err(ReadError::Malformed("not HTTP/1.x"));
    }
    let mut headers = Vec::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let (name, value) = line
            .split_once(':')
            .ok_or(ReadError::Malformed("header without a colon"))?;
        headers.push((name.trim().to_ascii_lowercase(), value.trim().to_string()));
    }
    Ok(RequestHead {
        method,
        target: origin_form(target),
        version,
        headers,
    })
}

fn find_head_end(buf: &[u8]) -> Option<(usize, usize)> {
    if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
        return Some((i, i + 4));
    }
    buf.windows(2)
        .position(|w| w == b"\n\n")
        .map(|i| (i, i + 2))
}

/// Read a request head from `reader`; returns the head and any bytes read past it.
pub async fn read_head<R: AsyncRead + Unpin>(
    reader: &mut R,
    max_head: usize,
) -> Result<(RequestHead, Vec<u8>), ReadError> {
    let mut buf: Vec<u8> = Vec::with_capacity(4096);
    let mut chunk = [0u8; 16 * 1024];
    loop {
        if let Some((end, after)) = find_head_end(&buf) {
            let head = parse_head(&buf[..end])?;
            return Ok((head, buf[after..].to_vec()));
        }
        if buf.len() > max_head {
            return Err(ReadError::HeadTooLarge);
        }
        let n = reader.read(&mut chunk).await?;
        if n == 0 {
            return Err(if buf.is_empty() {
                ReadError::Closed
            } else {
                ReadError::Malformed("connection closed inside the head")
            });
        }
        buf.extend_from_slice(&chunk[..n]);
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Chunked {
    /// Decoded body and the number of input bytes consumed (including the trailer).
    Complete(Vec<u8>, usize),
    Incomplete,
}

/// Decode a chunked body held entirely in `buf` (trailers are skipped).
pub fn decode_chunked(buf: &[u8]) -> Result<Chunked, ReadError> {
    let mut pos = 0;
    let mut out = Vec::new();
    loop {
        let Some(line_end) = buf[pos..].windows(2).position(|w| w == b"\r\n") else {
            return Ok(Chunked::Incomplete);
        };
        let size_line = std::str::from_utf8(&buf[pos..pos + line_end])
            .map_err(|_| ReadError::Malformed("chunk size is not UTF-8"))?;
        let size_hex = size_line.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size_hex, 16)
            .map_err(|_| ReadError::Malformed("chunk size is not hex"))?;
        pos += line_end + 2;
        if size == 0 {
            // Trailers until a blank line.
            loop {
                let Some(t) = buf[pos..].windows(2).position(|w| w == b"\r\n") else {
                    return Ok(Chunked::Incomplete);
                };
                pos += t + 2;
                if t == 0 {
                    return Ok(Chunked::Complete(out, pos));
                }
            }
        }
        if buf.len() < pos + size + 2 {
            return Ok(Chunked::Incomplete);
        }
        out.extend_from_slice(&buf[pos..pos + size]);
        pos += size + 2;
    }
}

/// Read the body that follows `head`, starting from `leftover` bytes already read.
pub async fn read_body<R: AsyncRead + Unpin>(
    reader: &mut R,
    head: &RequestHead,
    mut leftover: Vec<u8>,
    max_body: usize,
) -> Result<Vec<u8>, ReadError> {
    let mut chunk = [0u8; 16 * 1024];
    if head.is_chunked() {
        loop {
            match decode_chunked(&leftover)? {
                Chunked::Complete(body, _) => return Ok(body),
                Chunked::Incomplete => {}
            }
            if leftover.len() > max_body {
                return Err(ReadError::BodyTooLarge);
            }
            let n = reader.read(&mut chunk).await?;
            if n == 0 {
                return Err(ReadError::Malformed(
                    "connection closed inside a chunked body",
                ));
            }
            leftover.extend_from_slice(&chunk[..n]);
        }
    }
    let Some(length) = head.content_length() else {
        return Ok(Vec::new());
    };
    if length > max_body {
        return Err(ReadError::BodyTooLarge);
    }
    while leftover.len() < length {
        let n = reader.read(&mut chunk).await?;
        if n == 0 {
            return Err(ReadError::Malformed("connection closed inside the body"));
        }
        leftover.extend_from_slice(&chunk[..n]);
    }
    leftover.truncate(length);
    Ok(leftover)
}

/// Read one whole request.
pub async fn read_request<R: AsyncRead + Unpin>(
    reader: &mut R,
    max_head: usize,
    max_body: usize,
) -> Result<(RequestHead, Vec<u8>), ReadError> {
    let (head, leftover) = read_head(reader, max_head).await?;
    let body = read_body(reader, &head, leftover, max_body).await?;
    Ok((head, body))
}

/// Headers that describe this hop, never forwarded in either direction.
pub fn is_hop_by_hop(name: &str) -> bool {
    matches!(
        name,
        "connection"
            | "keep-alive"
            | "proxy-connection"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
    )
}

/// Encode one chunk of a chunked response body.
pub fn chunk_frame(data: &[u8]) -> Vec<u8> {
    let mut out = format!("{:x}\r\n", data.len()).into_bytes();
    out.extend_from_slice(data);
    out.extend_from_slice(b"\r\n");
    out
}

pub const LAST_CHUNK: &[u8] = b"0\r\n\r\n";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_heads_in_origin_and_absolute_form() {
        let head = parse_head(
            b"POST http://127.0.0.1:1456/v1/messages?beta=true HTTP/1.1\r\nHost: 127.0.0.1:1456\r\nContent-Length: 12\r\nExpect: 100-continue",
        )
        .unwrap();
        assert_eq!(head.method, "POST");
        assert_eq!(head.target, "/v1/messages?beta=true");
        assert_eq!(head.content_length(), Some(12));
        assert!(head.expects_continue());
        assert_eq!(head.header("host"), Some("127.0.0.1:1456"));
        assert_eq!(origin_form("https://chatgpt.com"), "/");
        assert!(parse_head(b"GET / SPDY/3").is_err());
        assert!(parse_head(b"GET / HTTP/1.1\r\nbroken header").is_err());
    }

    #[test]
    fn decodes_chunked_bodies_and_reports_incomplete_input() {
        let buf = b"4\r\nWiki\r\n5\r\npedia\r\n0\r\n\r\n";
        assert_eq!(
            decode_chunked(buf).unwrap(),
            Chunked::Complete(b"Wikipedia".to_vec(), buf.len())
        );
        assert_eq!(decode_chunked(b"4\r\nWik").unwrap(), Chunked::Incomplete);
        let with_trailer = b"3;ext=1\r\nabc\r\n0\r\nTrailer: x\r\n\r\n";
        assert_eq!(
            decode_chunked(with_trailer).unwrap(),
            Chunked::Complete(b"abc".to_vec(), with_trailer.len())
        );
        assert!(decode_chunked(b"zz\r\n").is_err());
    }

    #[tokio::test]
    async fn reads_a_request_with_a_content_length_body_split_across_reads() {
        let (mut client, mut server) = tokio::io::duplex(64);
        let writer = tokio::spawn(async move {
            use tokio::io::AsyncWriteExt;
            client
                .write_all(
                    b"POST /v1/messages HTTP/1.1\r\nHost: x\r\nContent-Length: 11\r\n\r\nhello",
                )
                .await
                .unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            client.write_all(b" world").await.unwrap();
        });
        let (head, body) = read_request(&mut server, 8192, 1 << 20).await.unwrap();
        assert_eq!(head.target, "/v1/messages");
        assert_eq!(body, b"hello world");
        writer.await.unwrap();
    }

    #[tokio::test]
    async fn reads_a_chunked_request_body() {
        let (mut client, mut server) = tokio::io::duplex(1024);
        tokio::spawn(async move {
            use tokio::io::AsyncWriteExt;
            client
                .write_all(
                    b"POST / HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n5\r\nhello\r\n0\r\n\r\n",
                )
                .await
                .unwrap();
        });
        let (_, body) = read_request(&mut server, 8192, 1 << 20).await.unwrap();
        assert_eq!(body, b"hello");
    }

    #[tokio::test]
    async fn enforces_the_body_cap() {
        let (mut client, mut server) = tokio::io::duplex(1024);
        tokio::spawn(async move {
            use tokio::io::AsyncWriteExt;
            let _ = client
                .write_all(b"POST / HTTP/1.1\r\nContent-Length: 999999\r\n\r\nxx")
                .await;
        });
        assert!(matches!(
            read_request(&mut server, 8192, 1024).await,
            Err(ReadError::BodyTooLarge)
        ));
    }
}
