use crate::chat::Role;

use super::Message;
use bytes::{Buf, BufMut, BytesMut};
use futures_util::{Stream, StreamExt};
use serde::Deserialize;
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc::{Receiver, Sender};
use tracing::{debug, error, trace};

/// Handle the stream and all related actions. Use channels to communicate with the
/// caller and write the content to the `writer`.
pub trait Streamer: Clone + Send {
    /// Write the stream until it finishes
    fn write_at_end(
        &self,
        writer: &mut (impl tokio::io::AsyncWrite + Unpin + Send),
        receiver: Receiver<String>,
    ) -> impl std::future::Future<Output = anyhow::Result<()>> + Send;

    /// Handle the stream data and process all the chunks; use a Finite State Machine (FSM) for
    /// capturing the chunks and ensure that incomplete chunks are not processed until the message
    /// is completely passed to the buffer.
    async fn handle_stream(
        &self,
        mut stream: (impl Stream<Item = reqwest::Result<bytes::Bytes>> + Unpin),
        sender: Sender<String>,
    ) -> anyhow::Result<Message> {
        let mut response = String::new();

        debug!("Opening stream");
        let mut buffer = BytesMut::with_capacity(8192);
        while let Some(chunk) = stream.next().await {
            trace!(?chunk, "processing");
            let chunk = chunk?;
            buffer.put_slice(&chunk);

            if let Some((chunks_str, advance)) = self.process_buffer(&buffer).await? {
                buffer.advance(advance);
                for chunk_str in chunks_str {
                    trace!(chunk_str);
                    response.push_str(&chunk_str);
                    sender.send(chunk_str).await?;
                }
            }
        }

        Ok(Message {
            role: Role::Assistant,
            content: response,
        })
    }

    /// Process the entire buffer and return the complete chunk strings.
    /// Return the chunk strings and the advancement for the buffer.
    async fn process_buffer(&self, buffer: &[u8]) -> anyhow::Result<Option<(Vec<String>, usize)>> {
        if buffer.is_empty() {
            return Ok(None);
        }

        const CHUNK_SEPARATOR: &[u8] = b"\n\n";
        const DATA_PREFIX: &[u8] = b"data: ";

        let mut chunks = Vec::new();
        let mut total_consumed = 0;
        let mut pos = 0;

        while pos < buffer.len() {
            // Find the next chunk separator
            if let Some(separator_pos) = buffer[pos..]
                .windows(CHUNK_SEPARATOR.len())
                .position(|window| window == CHUNK_SEPARATOR)
            {
                let chunk_end = pos + separator_pos;
                let chunk = &buffer[pos..chunk_end];

                // Process this chunk if it starts with "data: "
                if chunk.starts_with(DATA_PREFIX) {
                    let json_data = &chunk[DATA_PREFIX.len()..];

                    // Check for [DONE] marker
                    if json_data.starts_with(b"[DONE]") {
                        debug!("DONE detected");
                        total_consumed = pos + separator_pos + CHUNK_SEPARATOR.len();
                        break;
                    }

                    // Try to parse as JSON
                    match serde_json::from_slice::<CopilotResponse>(json_data) {
                        Ok(resp_msg) => {
                            if let Some(choice) = resp_msg.choices.first() {
                                if let Some(msg) = &choice.delta {
                                    if let Some(content) = &msg.content {
                                        chunks.push(content.to_string());
                                    }
                                }
                            }
                        }
                        Err(_) => {
                            // Try to parse as error
                            match serde_json::from_slice::<CopilotError>(json_data) {
                                Ok(err) => {
                                    error!(err.error.message, "error in stream");
                                    return Err(anyhow::anyhow!(err.error.message));
                                }
                                Err(e) => {
                                    error!("Failed to parse chunk as JSON: {}", String::from_utf8_lossy(json_data));
                                    return Err(anyhow::anyhow!("cannot parse chunk: {e}"));
                                }
                            }
                        }
                    }
                }

                // Move past this chunk and its separator
                pos = pos + separator_pos + CHUNK_SEPARATOR.len();
                total_consumed = pos;
            } else {
                // No complete chunk found, we need more data
                break;
            }
        }

        if chunks.is_empty() && total_consumed == 0 {
            Ok(None)
        } else {
            Ok(Some((chunks, total_consumed)))
        }
    }
}

#[derive(Debug, Deserialize)]
struct CopilotError {
    error: CopilotErrorDetail,
}

#[derive(Debug, Deserialize)]
struct CopilotErrorDetail {
    message: String,
}

/// Copilot response data
#[derive(Debug, Deserialize)]
struct CopilotResponse {
    choices: Vec<Choice>,
}

/// Content 'delta' of the message: a partial chunk of the complete message
#[derive(Deserialize, Debug)]
struct Delta {
    content: Option<String>,
}

/// All options and content related to the response
#[allow(dead_code)]
#[derive(Debug, Deserialize)]
struct Choice {
    delta: Option<Delta>,
    index: i32,
    finish_reason: Option<String>,
}

/// Handle the stream of the chat
#[derive(Clone)]
pub struct ChatStreamer;

impl Streamer for ChatStreamer {
    async fn write_at_end(
        &self,
        writer: &mut (impl tokio::io::AsyncWrite + Unpin),
        mut receiver: Receiver<String>,
    ) -> anyhow::Result<()> {
        loop {
            match receiver.recv().await {
                Some(content) => {
                    writer.write_all(content.as_bytes()).await?;
                    writer.flush().await?
                }
                None => {
                    debug!("End of streaming");
                    break;
                }
            };
        }

        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use tokio::sync::mpsc::channel;

    #[derive(Clone)]
    pub(crate) struct TestStreamer;

    impl Streamer for TestStreamer {
        async fn write_at_end(
            &self,
            writer: &mut (impl tokio::io::AsyncWrite + Unpin),
            mut receiver: Receiver<String>,
        ) -> anyhow::Result<()> {
            loop {
                match receiver.recv().await {
                    Some(chunk) => {
                        writer.write(chunk.as_bytes()).await?;
                        writer.flush().await?;
                    }
                    None => break,
                };
            }
            Ok(())
        }
    }

    async fn count_chunks(mut receiver: Receiver<String>) -> usize {
        let mut result = 0;
        loop {
            match receiver.recv().await {
                Some(_) => result = result + 1,
                None => break,
            }
        }

        result
    }

    #[tokio::test]
    async fn chunk_parsing() {
        let chunk = "data: {\"choices\":[{\"index\":0,\"content_filter_offsets\":{\"check_offset\":175,\"start_offset\":176,\"end_offset\":280},
        \"content_filter_results\":{\"hate\":{\"filtered\":false,\"severity\":\"safe\"},\"self_harm\":{\"filtered\":false,
        \"severity\":\"safe\"},\"sexual\":{\"filtered\":false,\"severity\":\"safe\"},\"violence\":{\"filtered\":false,\"severity\":\"safe\"}},
        \"delta\":{\"content\":\" safety\"}}],\"created\":1751000792,\"id\":\"chatcmpl-BmvaCUrU0DjRli6juhycOsjF1OAZr\",
        \"model\":\"gpt-4o-2024-11-20\",\"system_fingerprint\":\"fp_b705f0c291\"}JUMP";

        // Normalize string
        let chunk = chunk.replace("\n", "").replace("JUMP", "\n\n");

        let streamer = TestStreamer;
        let (sender, receiver) = channel(1);
        let resp = streamer.process_buffer(&chunk.as_bytes()).await;

        assert!(resp.is_ok());

        let (msgs, _) = resp.unwrap().unwrap();

        for m in msgs {
            sender.send(m).await.unwrap();
        }
        drop(sender);

        let count = count_chunks(receiver).await;

        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn double_chunk_parsing() {
        let double = "data: {\"choices\":[{\"index\":0,\"content_filter_offsets\":{\"check_offset\":175,\"start_offset\":334,\"end_offset\":435},
        \"content_filter_results\":{\"hate\":{\"filtered\":false,\"severity\":\"safe\"},\"self_harm\":{\"filtered\":false,
        \"severity\":\"safe\"},\"sexual\":{\"filtered\":false,\"severity\":\"safe\"},
        \"violence\":{\"filtered\":false,\"severity\":\"safe\"}},\"delta\":{\"content\":\" the\"}}],
        \"created\":1751000792,\"id\":\"chatcmpl-BmvaCUrU0DjRli6juhycOsjF1OAZr\",\"model\":\"gpt-4o-2024-11-20\",
        \"system_fingerprint\":\"fp_b705f0c291\"}JUMPdata: {\"choices\":[{\"index\":0,\"content_filter_offsets\":{\"check_offset\":175,\"start_offset\":334,
        \"end_offset\":435},\"content_filter_results\":{\"hate\":{\"filtered\":false,\"severity\":\"safe\"},\"self_harm\":{\"filtered\":false,\"severity\":\"safe\"},
        \"sexual\":{\"filtered\":false,\"severity\":\"safe\"},\"violence\":{\"filtered\":false,\"severity\":\"safe\"}},
        \"delta\":{\"content\":\" most\"}}],\"created\":1751000792,\"id\":\"chatcmpl-BmvaCUrU0DjRli6juhycOsjF1OAZr\",
        \"model\":\"gpt-4o-2024-11-20\",\"system_fingerprint\":\"fp_b705f0c291\"}JUMP
        ";

        // Normalize string
        let double = double.replace("\n", "").replace("JUMP", "\n\n");

        let streamer = TestStreamer;
        let (sender, receiver) = channel(2);
        let resp = streamer.process_buffer(&double.as_bytes()).await;

        assert!(resp.is_ok());

        let (msgs, _) = resp.unwrap().unwrap();

        for m in msgs {
            sender.send(m).await.unwrap();
        }
        drop(sender);

        let count = count_chunks(receiver).await;

        assert_eq!(count, 2);
    }

    #[tokio::test]
    async fn incomplete_chunk() {
        let double_incomplete = "data: {\"choices\":[{\"index\":0,\"content_filter_offsets\":{\"check_offset\":175,\"start_offset\":334,\"end_offset\":435},
        \"content_filter_results\":{\"hate\":{\"filtered\":false,\"severity\":\"safe\"},\"self_harm\":{\"filtered\":false,
        \"severity\":\"safe\"},\"sexual\":{\"filtered\":false,\"severity\":\"safe\"},
        \"violence\":{\"filtered\":false,\"severity\":\"safe\"}},\"delta\":{\"content\":\" the\"}}],
        \"created\":1751000792,\"id\":\"chatcmpl-BmvaCUrU0DjRli6juhycOsjF1OAZr\",\"model\":\"gpt-4o-2024-11-20\",
        \"system_fingerprint\":\"fp_b705f0c291\"}JUMPdata: {\"choices\":[{\"index\":0,\"content_filter_offsets\":{\"check_offset\":175,\"start_offset\":334,
        \"end_offset\":435},\"content_filter_results\":{\"hate\":{\"filtered\":false,\"severity\":\"safe\"},\"self_harm\":{\"filtered\":false,\"severity\":\"safe";

        // Normalize string
        let double_incomplete = double_incomplete.replace("\n", "").replace("JUMP", "\n\n");

        let streamer = TestStreamer;
        let (sender, receiver) = channel(2);
        let resp = streamer.process_buffer(&double_incomplete.as_bytes()).await;

        assert!(resp.is_ok());

        let (msgs, _) = resp.unwrap().unwrap();

        for m in msgs {
            sender.send(m).await.unwrap();
        }
        drop(sender);

        let count = count_chunks(receiver).await;

        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn done_chunk() {
        let chunks = "data: {\"choices\":[{\"index\":0,\"content_filter_offsets\":{\"check_offset\":175,\"start_offset\":334,\"end_offset\":435},
        \"content_filter_results\":{\"hate\":{\"filtered\":false,\"severity\":\"safe\"},\"self_harm\":{\"filtered\":false,
        \"severity\":\"safe\"},\"sexual\":{\"filtered\":false,\"severity\":\"safe\"},
        \"violence\":{\"filtered\":false,\"severity\":\"safe\"}},\"delta\":{\"content\":\" the\"}}],
        \"created\":1751000792,\"id\":\"chatcmpl-BmvaCUrU0DjRli6juhycOsjF1OAZr\",\"model\":\"gpt-4o-2024-11-20\",
        \"system_fingerprint\":\"fp_b705f0c291\"}JUMPdata: [DONE]";

        // Normalize string
        let chunks = chunks.replace("\n", "").replace("JUMP", "\n\n");

        let streamer = TestStreamer;
        let (sender, receiver) = channel(2);
        let resp = streamer.process_buffer(&chunks.as_bytes()).await;

        assert!(resp.is_ok());

        let (msgs, _) = resp.unwrap().unwrap();

        for m in msgs {
            sender.send(m).await.unwrap();
        }
        drop(sender);

        let count = count_chunks(receiver).await;

        assert_eq!(count, 1);
    }
}
