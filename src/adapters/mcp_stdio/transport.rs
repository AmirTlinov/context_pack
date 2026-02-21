use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader, BufWriter};

use crate::adapters::mcp_stdio::rpc::RpcEnvelope;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TransportMode {
    Framed,
    JsonLine,
}

fn parse_content_length(headers: &[String], max_frame_bytes: usize) -> anyhow::Result<usize> {
    let mut content_length: Option<usize> = None;
    for line in headers {
        let (name, raw_value) = line
            .split_once(':')
            .ok_or_else(|| anyhow::anyhow!("invalid frame header '{}'", line))?;
        if name.trim().eq_ignore_ascii_case("content-length") {
            let length = raw_value
                .trim()
                .parse::<usize>()
                .map_err(|_| anyhow::anyhow!("invalid Content-Length value"))?;
            if length > max_frame_bytes {
                return Err(anyhow::anyhow!(
                    "frame too large: {} bytes (max {})",
                    length,
                    max_frame_bytes
                ));
            }
            content_length = Some(length);
        }
    }

    content_length.ok_or_else(|| anyhow::anyhow!("missing Content-Length header"))
}

pub(super) async fn read_next_message<R>(
    reader: &mut BufReader<R>,
    max_frame_bytes: usize,
) -> anyhow::Result<Option<(String, TransportMode)>>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let first = loop {
        let mut one = [0u8; 1];
        let n = reader.read(&mut one).await?;
        if n == 0 {
            return Ok(None);
        }
        if one[0].is_ascii_whitespace() {
            continue;
        }
        break one[0];
    };

    if first == b'{' || first == b'[' {
        let msg = read_json_message_from_first_byte(reader, first, max_frame_bytes).await?;
        return Ok(msg.map(|m| (m, TransportMode::JsonLine)));
    }

    let first_line = read_line_from_first_byte(reader, first).await?;
    let trimmed = first_line.trim_end_matches(['\r', '\n']);
    if trimmed.trim().is_empty() {
        return Ok(Some((String::new(), TransportMode::Framed)));
    }

    let mut headers = Vec::new();
    headers.push(trimmed.to_string());

    // consume remaining headers until empty line
    loop {
        let line = read_line(reader).await?;
        if line.is_empty() {
            return Err(anyhow::anyhow!(
                "unexpected EOF while reading frame headers"
            ));
        }
        let trimmed_header = line.trim_end_matches(['\r', '\n']);
        if trimmed_header.trim().is_empty() {
            break;
        }
        if !trimmed_header.contains(':') {
            return Err(anyhow::anyhow!("invalid frame header '{}'", trimmed_header));
        }
        headers.push(trimmed_header.to_string());
    }

    let length = parse_content_length(&headers, max_frame_bytes)?;
    let mut body = vec![0u8; length];
    reader.read_exact(&mut body).await?;
    Ok(Some((
        String::from_utf8_lossy(&body).into_owned(),
        TransportMode::Framed,
    )))
}

async fn read_line_from_first_byte<R>(
    reader: &mut BufReader<R>,
    first: u8,
) -> anyhow::Result<String>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut buf = vec![first];
    let mut tail = Vec::new();
    let n = reader.read_until(b'\n', &mut tail).await?;
    if n == 0 {
        return Ok(String::from_utf8_lossy(&buf).into_owned());
    }
    buf.extend_from_slice(&tail);
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

async fn read_line<R>(reader: &mut BufReader<R>) -> anyhow::Result<String>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut buf = Vec::new();
    let n = reader.read_until(b'\n', &mut buf).await?;
    if n == 0 {
        return Ok(String::new());
    }
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

fn parse_complete_json(buf: &[u8]) -> anyhow::Result<Option<usize>> {
    match serde_json::from_slice::<Value>(buf) {
        Ok(_) => Ok(Some(buf.len())),
        Err(e) if e.is_eof() => Ok(None),
        Err(e) => Err(anyhow::anyhow!("invalid JSON message: {}", e)),
    }
}

async fn read_json_message_from_first_byte<R>(
    reader: &mut BufReader<R>,
    first: u8,
    max_frame_bytes: usize,
) -> anyhow::Result<Option<String>>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut payload = vec![first];
    if payload.len() > max_frame_bytes {
        return Err(anyhow::anyhow!(
            "message too large: {} bytes (max {})",
            payload.len(),
            max_frame_bytes
        ));
    }

    loop {
        if parse_complete_json(&payload)?.is_some() {
            return Ok(Some(String::from_utf8_lossy(&payload).into_owned()));
        }

        let mut chunk = [0u8; 4096];
        let n = reader.read(&mut chunk).await?;
        if n == 0 {
            return Err(anyhow::anyhow!("invalid JSON message (unexpected EOF)"));
        }
        payload.extend_from_slice(&chunk[..n]);
        if payload.len() > max_frame_bytes {
            return Err(anyhow::anyhow!(
                "message too large: {} bytes (max {})",
                payload.len(),
                max_frame_bytes
            ));
        }
    }
}

pub(super) async fn write_response<W: tokio::io::AsyncWrite + Unpin>(
    writer: &mut BufWriter<W>,
    envelope: &RpcEnvelope,
    mode: TransportMode,
) -> anyhow::Result<()> {
    let body = serde_json::to_vec(envelope)?;
    match mode {
        TransportMode::Framed => {
            let header = format!("Content-Length: {}\r\n\r\n", body.len());
            writer.write_all(header.as_bytes()).await?;
            writer.write_all(&body).await?;
        }
        TransportMode::JsonLine => {
            writer.write_all(&body).await?;
            writer.write_all(b"\n").await?;
        }
    }
    writer.flush().await?;
    Ok(())
}
