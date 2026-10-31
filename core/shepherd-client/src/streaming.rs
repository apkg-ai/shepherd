use serde::Deserialize;

use crate::error::Error;
use crate::generated::types::Event;
use crate::transport::Transport;

/// One item from the project event stream.
#[derive(Debug)]
pub enum EventStreamItem {
    Change(Box<Event>),
    /// The server cannot replay from the requested cursor; it closes the
    /// stream after this frame and the caller must resynchronize.
    ResyncRequired {
        cursor: Option<String>,
        reason: String,
    },
}

#[derive(Deserialize)]
struct ResyncData {
    #[serde(default)]
    cursor: Option<String>,
    #[serde(default)]
    reason: String,
}

#[derive(Default)]
struct Frame {
    event: Option<String>,
    id: Option<String>,
    data: Vec<String>,
}

/// Resumable consumer for getEvents (SSE). Heartbeat comments are consumed
/// silently; the last seen event id is retained for reconnection.
pub struct EventStream {
    transport: Transport,
    path: String,
    initial_after: Option<String>,
    last_event_id: Option<String>,
    response: reqwest::Response,
    buffer: String,
    ended: bool,
}

impl std::fmt::Debug for EventStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventStream")
            .field("path", &self.path)
            .field("last_event_id", &self.last_event_id)
            .field("ended", &self.ended)
            .finish_non_exhaustive()
    }
}

impl EventStream {
    pub(crate) async fn connect(
        transport: Transport,
        path: String,
        after: Option<String>,
    ) -> Result<Self, Error> {
        let response = transport
            .start_events(&path, after.as_deref(), None)
            .await?;
        Ok(Self {
            transport,
            path,
            initial_after: after,
            last_event_id: None,
            response,
            buffer: String::new(),
            ended: false,
        })
    }

    /// Cursor of the last received change event, used by [`EventStream::resume`].
    pub fn last_event_id(&self) -> Option<&str> {
        self.last_event_id.as_deref()
    }

    /// Reconnects after the stream closed or failed. Sends `Last-Event-ID`
    /// when an event was already received (it takes precedence over `after`);
    /// otherwise falls back to the original `after` cursor.
    pub async fn resume(&mut self) -> Result<(), Error> {
        self.response = self
            .transport
            .start_events(
                &self.path,
                self.initial_after.as_deref(),
                self.last_event_id.as_deref(),
            )
            .await?;
        self.buffer.clear();
        self.ended = false;
        Ok(())
    }

    /// Next change event; `None` when the server closed the stream.
    pub async fn next(&mut self) -> Option<Result<EventStreamItem, Error>> {
        if self.ended {
            return None;
        }
        loop {
            while let Some(end) = self.buffer.find("\n\n") {
                let raw: String = self.buffer.drain(..end + 2).collect();
                match self.handle_frame(&raw) {
                    Ok(Some(item)) => return Some(Ok(item)),
                    Ok(None) => {}
                    Err(err) => {
                        self.ended = true;
                        return Some(Err(err));
                    }
                }
            }
            match self.response.chunk().await {
                Ok(Some(chunk)) => match std::str::from_utf8(&chunk) {
                    Ok(text) => self.buffer.push_str(text),
                    Err(err) => {
                        self.ended = true;
                        return Some(Err(Error::Decode {
                            status: 200,
                            message: format!("event stream is not valid UTF-8: {err}"),
                        }));
                    }
                },
                Ok(None) => {
                    self.ended = true;
                    return None;
                }
                Err(err) => {
                    self.ended = true;
                    return Some(Err(Error::Transport {
                        message: err.to_string(),
                        attempts: 1,
                        source: Some(err),
                    }));
                }
            }
        }
    }

    fn handle_frame(&mut self, raw: &str) -> Result<Option<EventStreamItem>, Error> {
        let frame = parse_frame(raw);
        match frame.event.as_deref() {
            Some("change") => {
                let data = frame.data.join("\n");
                let event: Event = serde_json::from_str(&data).map_err(|err| Error::Decode {
                    status: 200,
                    message: format!("change event body: {err}"),
                })?;
                if let Some(id) = frame.id {
                    self.last_event_id = Some(id);
                }
                Ok(Some(EventStreamItem::Change(Box::new(event))))
            }
            Some("resync_required") => {
                let data = frame.data.join("\n");
                let resync: ResyncData =
                    serde_json::from_str(&data).map_err(|err| Error::Decode {
                        status: 200,
                        message: format!("resync_required body: {err}"),
                    })?;
                self.ended = true;
                Ok(Some(EventStreamItem::ResyncRequired {
                    cursor: resync.cursor,
                    reason: resync.reason,
                }))
            }
            // Heartbeat comments and unknown event kinds are skipped.
            _ => Ok(None),
        }
    }
}

fn parse_frame(raw: &str) -> Frame {
    let mut frame = Frame::default();
    for line in raw.lines() {
        if line.starts_with(':') {
            continue;
        }
        let (field, value) = match line.split_once(':') {
            Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
            None => (line, ""),
        };
        match field {
            "event" => frame.event = Some(value.to_owned()),
            "id" => frame.id = Some(value.to_owned()),
            "data" => frame.data.push(value.to_owned()),
            _ => {}
        }
    }
    frame
}
