//! Framing of a streamed HTTP response into events (B05 slice 5).
//!
//! A connector operation that returns `Stream<T>` declares how its
//! response arrives: `server_sent_events` (`text/event-stream`, where an
//! event is its `data:` lines up to a blank line) or `json_lines` (one
//! value per line). This turns the bytes as they arrive into one event
//! text at a time; the interpreter decodes each into `T`. Bytes are
//! buffered only until an event is complete, so an event split across
//! network reads is whole when it is handed on.

use corvid_ast::StreamFormat;

/// What feeding bytes produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FramedEvent {
    /// One event's text, to decode as JSON.
    Data(String),
    /// The declared end marker arrived; the stream ends cleanly here.
    End,
}

/// Turns a response's bytes into events.
pub struct EventFramer {
    format: StreamFormat,
    until: Option<String>,
    pending: Vec<u8>,
    data_lines: Vec<String>,
}

impl EventFramer {
    pub fn new(format: StreamFormat, until: Option<String>) -> Self {
        Self { format, until, pending: Vec::new(), data_lines: Vec::new() }
    }

    /// Feed the next bytes; returns every event they complete, in order.
    /// Text that is not UTF-8 is an error naming the stream.
    pub fn feed(&mut self, bytes: &[u8]) -> Result<Vec<FramedEvent>, String> {
        self.pending.extend_from_slice(bytes);
        let mut events = Vec::new();
        while let Some(newline) = self.pending.iter().position(|b| *b == b'\n') {
            let mut line: Vec<u8> = self.pending.drain(..=newline).collect();
            line.pop(); // \n
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            let line = String::from_utf8(line).map_err(|_| "the stream sent text that is not UTF-8".to_string())?;
            if let Some(event) = self.line(line) {
                let end = event == FramedEvent::End;
                events.push(event);
                if end {
                    break;
                }
            }
        }
        Ok(events)
    }

    /// Bytes are finished: a last line without a newline, or a
    /// server-sent event the server closed before its blank line, is
    /// still an event.
    pub fn finish(&mut self) -> Result<Vec<FramedEvent>, String> {
        let mut events = Vec::new();
        if !self.pending.is_empty() {
            let rest = std::mem::take(&mut self.pending);
            let line = String::from_utf8(rest).map_err(|_| "the stream sent text that is not UTF-8".to_string())?;
            let line = line.trim_end_matches('\r').to_string();
            events.extend(self.line(line));
        }
        if self.format == StreamFormat::ServerSentEvents {
            events.extend(self.dispatch_sse());
        }
        Ok(events)
    }

    fn line(&mut self, line: String) -> Option<FramedEvent> {
        match self.format {
            StreamFormat::JsonLines => (!line.trim().is_empty()).then(|| FramedEvent::Data(line)),
            StreamFormat::ServerSentEvents => {
                if line.is_empty() {
                    return self.dispatch_sse();
                }
                if line.starts_with(':') {
                    return None; // a comment, often a keep-alive
                }
                let (field, value) = match line.split_once(':') {
                    Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
                    None => (line.as_str(), ""),
                };
                if field == "data" {
                    self.data_lines.push(value.to_string());
                }
                // `event`, `id` and `retry` do not change the data.
                None
            }
        }
    }

    fn dispatch_sse(&mut self) -> Option<FramedEvent> {
        if self.data_lines.is_empty() {
            return None;
        }
        let data = std::mem::take(&mut self.data_lines).join("\n");
        if self.until.as_deref() == Some(data.as_str()) {
            return Some(FramedEvent::End);
        }
        Some(FramedEvent::Data(data))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data(text: &str) -> FramedEvent {
        FramedEvent::Data(text.to_string())
    }

    #[test]
    fn server_sent_events_join_data_lines_and_skip_comments_and_fields() {
        let mut framer = EventFramer::new(StreamFormat::ServerSentEvents, None);
        let events = framer
            .feed(b": keep-alive\nevent: delta\nid: 1\ndata: {\"a\":\ndata: 1}\n\ndata: {\"a\":2}\r\n\r\n")
            .unwrap();
        assert_eq!(events, vec![data("{\"a\":\n1}"), data("{\"a\":2}")]);
    }

    #[test]
    fn an_event_split_across_reads_is_whole_when_handed_on() {
        let mut framer = EventFramer::new(StreamFormat::ServerSentEvents, None);
        assert!(framer.feed(b"data: {\"te").unwrap().is_empty());
        assert!(framer.feed(b"xt\":\"hi\"}\n").unwrap().is_empty());
        assert_eq!(framer.feed(b"\n").unwrap(), vec![data("{\"text\":\"hi\"}")]);
    }

    #[test]
    fn the_declared_end_marker_ends_the_stream() {
        let mut framer = EventFramer::new(StreamFormat::ServerSentEvents, Some("[DONE]".to_string()));
        let events = framer.feed(b"data: {\"a\":1}\n\ndata: [DONE]\n\ndata: {\"a\":2}\n\n").unwrap();
        assert_eq!(events, vec![data("{\"a\":1}"), FramedEvent::End]);
    }

    #[test]
    fn json_lines_are_one_value_per_line_and_the_last_needs_no_newline() {
        let mut framer = EventFramer::new(StreamFormat::JsonLines, None);
        assert_eq!(framer.feed(b"{\"a\":1}\n\n{\"a\":2}\n{\"a\"").unwrap(), vec![data("{\"a\":1}"), data("{\"a\":2}")]);
        assert_eq!(framer.feed(b":3}").unwrap(), vec![]);
        assert_eq!(framer.finish().unwrap(), vec![data("{\"a\":3}")]);
    }

    #[test]
    fn text_that_is_not_utf8_is_an_error() {
        let mut framer = EventFramer::new(StreamFormat::JsonLines, None);
        assert!(framer.feed(&[0xff, 0xfe, b'\n']).is_err());
    }
}
