//! Minimal Server-Sent Events decoder.
//!
//! Hand-rolled on purpose: the three HTTP providers all speak plain SSE and we
//! want exact control over framing, `[DONE]` handling and cancellation without
//! pulling in another dependency.

use crate::error::Result;

/// One decoded SSE event.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SseEvent {
    pub event: Option<String>,
    pub data: String,
    pub id: Option<String>,
}

impl SseEvent {
    /// `data: [DONE]` sentinel used by OpenAI-compatible streams.
    pub fn is_done(&self) -> bool {
        self.data.trim() == "[DONE]"
    }
}

/// Incremental SSE frame decoder. Feed it chunks; it yields complete events.
#[derive(Debug, Default)]
pub struct SseDecoder {
    buffer: String,
}

impl SseDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Push a chunk of text and return every complete event it contained.
    pub fn push(&mut self, chunk: &str) -> Vec<SseEvent> {
        self.buffer.push_str(chunk);
        let mut out = Vec::new();
        // Normalise CRLF/CR to LF so frame splitting is uniform.
        if self.buffer.contains('\r') {
            self.buffer = self.buffer.replace("\r\n", "\n").replace('\r', "\n");
        }
        while let Some(idx) = self.buffer.find("\n\n") {
            let frame = self.buffer[..idx].to_string();
            self.buffer.drain(..idx + 2);
            if let Some(event) = parse_frame(&frame) {
                out.push(event);
            }
        }
        out
    }

    /// Flush a trailing frame that was not terminated by a blank line.
    pub fn finish(&mut self) -> Vec<SseEvent> {
        if self.buffer.trim().is_empty() {
            self.buffer.clear();
            return Vec::new();
        }
        let frame = std::mem::take(&mut self.buffer);
        parse_frame(&frame).into_iter().collect()
    }
}

fn parse_frame(frame: &str) -> Option<SseEvent> {
    let mut event = SseEvent::default();
    let mut data_lines: Vec<&str> = Vec::new();
    let mut has_field = false;

    for line in frame.lines() {
        let line = line.trim_end();
        if line.is_empty() || line.starts_with(':') {
            continue;
        }
        let (field, value) = match line.split_once(':') {
            Some((f, v)) => (f, v.strip_prefix(' ').unwrap_or(v)),
            None => (line, ""),
        };
        match field {
            "event" => {
                event.event = Some(value.to_string());
                has_field = true;
            }
            "data" => {
                data_lines.push(value);
                has_field = true;
            }
            "id" => {
                event.id = Some(value.to_string());
                has_field = true;
            }
            _ => {}
        }
    }

    if !has_field {
        return None;
    }
    event.data = data_lines.join("\n");
    Some(event)
}

/// Parse a `data:` payload as JSON, returning a useful error with a redacted
/// snippet on failure.
pub fn parse_json<T: serde::de::DeserializeOwned>(data: &str) -> Result<T> {
    serde_json::from_str(data).map_err(|e| {
        let snippet: String = data.chars().take(200).collect();
        crate::error::Error::provider(format!(
            "cannot parse stream payload: {e} (payload: {snippet})"
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_multiple_events_across_chunks() {
        let mut dec = SseDecoder::new();
        assert!(dec.push("data: {\"a\":").is_empty());
        let events = dec.push("1}\n\ndata: {\"b\":2}\n\n");
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].data, "{\"a\":1}");
        assert_eq!(events[1].data, "{\"b\":2}");
    }

    #[test]
    fn handles_event_names_comments_and_multiline_data() {
        let mut dec = SseDecoder::new();
        let events =
            dec.push(": keepalive\n\nevent: content_block_delta\ndata: line1\ndata: line2\n\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event.as_deref(), Some("content_block_delta"));
        assert_eq!(events[0].data, "line1\nline2");
    }

    #[test]
    fn handles_crlf() {
        let mut dec = SseDecoder::new();
        let events = dec.push("data: x\r\n\r\n");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "x");
    }

    #[test]
    fn detects_done_sentinel() {
        let mut dec = SseDecoder::new();
        let events = dec.push("data: [DONE]\n\n");
        assert!(events[0].is_done());
    }

    #[test]
    fn finish_flushes_trailing_frame() {
        let mut dec = SseDecoder::new();
        dec.push("data: tail");
        let events = dec.finish();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "tail");
    }
}
