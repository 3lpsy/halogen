use futures_util::TryStreamExt;
use quick_xml::{
    Reader, Writer,
    events::{BytesEnd, Event},
};
use tokio::io::{AsyncBufRead, AsyncReadExt};
use tokio_util::io::StreamReader;

pub(super) const MAX_EPISODES: usize = 200;
const MAX_PREFIX_BYTES: u64 = 16 * 1024 * 1024;
const MAX_DEPTH: usize = 64;

pub(super) async fn read_response(response: reqwest::Response) -> Result<Vec<u8>, String> {
    let stream = response.bytes_stream().map_err(std::io::Error::other);
    read_prefix(StreamReader::new(stream)).await
}

/// Stop at an item boundary, preserving XML namespaces and channel metadata.
/// Bound decoded input even when a feed contains huge text nodes or no items.
async fn read_prefix(input: impl AsyncBufRead + Unpin) -> Result<Vec<u8>, String> {
    let mut reader = Reader::from_reader(input.take(MAX_PREFIX_BYTES + 1));
    let mut writer = Writer::new(Vec::new());
    let mut buffer = Vec::new();
    let mut stack = Vec::new();
    let mut items = 0;
    let mut channel_seen = false;
    loop {
        let event = reader.read_event_into_async(&mut buffer).await;
        if reader.buffer_position() > MAX_PREFIX_BYTES {
            return Err("Feed preview exceeds the 16 MiB safety limit".into());
        }
        let event = event.map_err(|_| "Invalid RSS feed")?;
        match &event {
            Event::Start(tag) => {
                let name = tag.name().as_ref().to_vec();
                if stack.is_empty() && name != b"rss" {
                    return Err("Invalid RSS feed".into());
                }
                if stack.len() == 1 && name == b"channel" {
                    channel_seen = true;
                }
                stack.push(name);
                if stack.len() > MAX_DEPTH {
                    return Err("Feed nesting exceeds safety limit".into());
                }
            }
            Event::End(tag) => {
                if stack.len() == 3 && stack[1] == b"channel" && tag.name().as_ref() == b"item" {
                    items += 1;
                }
                stack.pop();
            }
            Event::DocType(_) => return Err("RSS document types are not supported".into()),
            Event::Eof => {
                return if channel_seen && stack.is_empty() {
                    Ok(writer.into_inner())
                } else {
                    Err("Invalid RSS feed".into())
                };
            }
            _ => {}
        }
        writer.write_event(event).map_err(|_| "Invalid RSS feed")?;
        if items == MAX_EPISODES {
            writer
                .write_event(Event::End(BytesEnd::new("channel")))
                .map_err(|_| "Invalid RSS feed")?;
            writer
                .write_event(Event::End(BytesEnd::new("rss")))
                .map_err(|_| "Invalid RSS feed")?;
            return Ok(writer.into_inner());
        }
        buffer.clear();
    }
}

#[cfg(test)]
mod tests;
