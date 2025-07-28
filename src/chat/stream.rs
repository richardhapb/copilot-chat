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
        let mut buffer = BytesMut::new();
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
        let mut advance: usize = 0;
        let mut begin = "data: ".len();
        if buffer.is_empty() {
            return Ok(None);
        }

        const CHUNK_SEPARATOR: &[u8] = b"\n\n";
        let mut chunks = Vec::new();
        let mut i = begin;

        while i < buffer.len() {
            let end = i + 1;

            // data: {...}\n\n
            //         i-1=^ ^ = i
            // Here reachs the end of the chunk
            if buffer[i - 1..end] == *CHUNK_SEPARATOR {
                advance += end;
            } else {
                // Not enough bytes
                i += 1;
                continue;
            }
            if buffer[begin..end].starts_with(b"[DONE]") {
                // This marks the end of the stream
                debug!("DONE detected");
                break;
            }

            let text = String::from_utf8(buffer[begin..end].to_vec()).unwrap();
            println!("{}", text);

            match serde_json::from_slice::<CopilotResponse>(&buffer[begin..end]) {
                Ok(resp_msg) => {
                    if let Some(choice) = resp_msg.choices.first()
                        && let Some(msg) = &choice.delta
                    {
                        let msg = msg.content.clone();
                        if let Some(msg) = msg {
                            chunks.push(msg);
                        }
                    }
                }
                Err(_) => {
                    // Try to serialize the error if matches with the format
                    let err = serde_json::from_slice::<CopilotError>(&buffer[begin..end]);

                    match err {
                        Ok(err) => {
                            error!(err.error.message, "error in stream");
                            return Err(anyhow::anyhow!(err.error.message));
                        }
                        Err(e) => {
                            error!("error in stream, cannot capture the error message");
                            return Err(anyhow::anyhow!("cannot capture error message: {e}"));
                        }
                    }
                }
            }

            // Set the new start point in data: ...
            begin += i;
            i += 1;
        }
        Ok(Some((chunks, advance)))
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
        let chunk = "
        {\"choices\":[{\"index\":0,\"content_filter_offsets\":{\"check_offset\":175,\"start_offset\":176,\"end_offset\":280},
        \"content_filter_results\":{\"hate\":{\"filtered\":false,\"severity\":\"safe\"},\"self_harm\":{\"filtered\":false,
        \"severity\":\"safe\"},\"sexual\":{\"filtered\":false,\"severity\":\"safe\"},\"violence\":{\"filtered\":false,\"severity\":\"safe\"}},
        \"delta\":{\"content\":\" safety\"}}],\"created\":1751000792,\"id\":\"chatcmpl-BmvaCUrU0DjRli6juhycOsjF1OAZr\",
        \"model\":\"gpt-4o-2024-11-20\",\"system_fingerprint\":\"fp_b705f0c291\"}JUMP
        ";

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
        let double = "
        {\"choices\":[{\"index\":0,\"content_filter_offsets\":{\"check_offset\":175,\"start_offset\":334,\"end_offset\":435},
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
        let double_incomplete = "
        {\"choices\":[{\"index\":0,\"content_filter_offsets\":{\"check_offset\":175,\"start_offset\":334,\"end_offset\":435},
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
        let chunks = "
        {\"choices\":[{\"index\":0,\"content_filter_offsets\":{\"check_offset\":175,\"start_offset\":334,\"end_offset\":435},
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
