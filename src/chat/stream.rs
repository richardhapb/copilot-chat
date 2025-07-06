use crate::chat::Role;

use super::Message;
use futures_util::{Stream, StreamExt};
use serde::Deserialize;
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc::{Receiver, Sender};
use tracing::{debug, trace};

/// Handle the stream and all related actions. Use channels to communicate with the
/// caller and write the content to the `writer`.
pub trait Streamer: Clone + Send {
    /// Write the stream until it finishes
    fn write_at_end(
        &self,
        writer: &mut (impl tokio::io::AsyncWrite + Unpin + Send),
        receiver: Receiver<String>,
    ) -> impl std::future::Future<Output = anyhow::Result<()>> + Send;

    /// Handle the stream data and process all the chunks
    async fn handle_stream(
        &self,
        mut stream: (impl Stream<Item = reqwest::Result<bytes::Bytes>> + Unpin),
        sender: Sender<String>,
    ) -> anyhow::Result<Message> {
        let mut response = String::new();

        debug!("Opening stream");
        let mut partial_chunk = None;
        while let Some(chunk) = stream.next().await {
            trace!(?chunk, "processing");
            let chunk = chunk?;

            let mut chunk_str = String::from_utf8_lossy(&chunk);
            if partial_chunk.is_some() {
                chunk_str = format!("{}{}", partial_chunk.unwrap(), chunk_str).into();
            }
            partial_chunk = self
                .process_chunk(&chunk_str, &mut response, &sender)
                .await
                .unwrap_or(None)
        }

        Ok(Message {
            role: Role::System,
            content: response,
        })
    }

    /// Process an individual chunk and return a partial chunk if it exists.
    async fn process_chunk(
        &self,
        chunk: &str,
        destination: &mut String,
        sender: &Sender<String>,
    ) -> anyhow::Result<Option<String>> {
        let chunks = chunk.split("\n\n");

        for (i, chunk) in chunks.clone().enumerate() {
            if chunk.is_empty() {
                continue;
            }
            let begin = "data: ".len();
            match serde_json::from_str::<CopilotResponse>(&chunk[begin..]) {
                Ok(resp_msg) => {
                    if let Some(choice) = resp_msg.choices.first()
                        && let Some(msg) = &choice.delta
                    {
                        let msg = msg.content.clone();
                        destination.push_str(&msg);
                        sender.send(msg).await?;
                    }
                }
                Err(e) => {
                    // Is the last, should be a cutted chunk
                    if chunks.count() == i + 1 {
                        return Ok(Some(chunk.to_string()));
                    }
                    return Err(e.into());
                }
            }
        }
        Ok(None)
    }
}

/// Copilot response data
#[derive(Debug, Deserialize)]
struct CopilotResponse {
    choices: Vec<Choice>,
}

/// Content 'delta' of the message: a partial chunk of the complete message
#[derive(Deserialize, Debug)]
struct Delta {
    content: String,
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
                    writer.write(content.as_bytes()).await?;
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
                    Some(chunk) => writer.write(chunk.as_bytes()).await?,
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
    async fn test_chunk_parsing() {
        let chunk = "
        {\"choices\":[{\"index\":0,\"content_filter_offsets\":{\"check_offset\":175,\"start_offset\":176,\"end_offset\":280},
        \"content_filter_results\":{\"hate\":{\"filtered\":false,\"severity\":\"safe\"},\"self_harm\":{\"filtered\":false,
        \"severity\":\"safe\"},\"sexual\":{\"filtered\":false,\"severity\":\"safe\"},\"violence\":{\"filtered\":false,\"severity\":\"safe\"}},
        \"delta\":{\"content\":\" safety\"}}],\"created\":1751000792,\"id\":\"chatcmpl-BmvaCUrU0DjRli6juhycOsjF1OAZr\",
        \"model\":\"gpt-4o-2024-11-20\",\"system_fingerprint\":\"fp_b705f0c291\"}
        ";

        let streamer = TestStreamer;
        let (sender, receiver) = channel(1);
        let mut dest = String::new();
        let resp = streamer.process_chunk(chunk, &mut dest, &sender).await;

        drop(sender);
        let count = count_chunks(receiver).await;

        assert!(resp.is_ok());
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn test_double_chunk_parsing() {
        let double = "
        {\"choices\":[{\"index\":0,\"content_filter_offsets\":{\"check_offset\":175,\"start_offset\":334,\"end_offset\":435},
        \"content_filter_results\":{\"hate\":{\"filtered\":false,\"severity\":\"safe\"},\"self_harm\":{\"filtered\":false,
        \"severity\":\"safe\"},\"sexual\":{\"filtered\":false,\"severity\":\"safe\"},
        \"violence\":{\"filtered\":false,\"severity\":\"safe\"}},\"delta\":{\"content\":\" the\"}}],
        \"created\":1751000792,\"id\":\"chatcmpl-BmvaCUrU0DjRli6juhycOsjF1OAZr\",\"model\":\"gpt-4o-2024-11-20\",
        \"system_fingerprint\":\"fp_b705f0c291\"}\n\ndata: {\"choices\":[{\"index\":0,\"content_filter_offsets\":{\"check_offset\":175,\"start_offset\":334,
        \"end_offset\":435},\"content_filter_results\":{\"hate\":{\"filtered\":false,\"severity\":\"safe\"},\"self_harm\":{\"filtered\":false,\"severity\":\"safe\"},
        \"sexual\":{\"filtered\":false,\"severity\":\"safe\"},\"violence\":{\"filtered\":false,\"severity\":\"safe\"}},
        \"delta\":{\"content\":\" most\"}}],\"created\":1751000792,\"id\":\"chatcmpl-BmvaCUrU0DjRli6juhycOsjF1OAZr\",
        \"model\":\"gpt-4o-2024-11-20\",\"system_fingerprint\":\"fp_b705f0c291\"}
        ";

        let streamer = TestStreamer;
        let (sender, receiver) = channel(2);
        let mut dest = String::new();
        let resp = streamer.process_chunk(double, &mut dest, &sender).await;

        drop(sender);
        let count = count_chunks(receiver).await;

        assert!(resp.is_ok());
        assert_eq!(count, 2);
    }
}
