use crate::client::provider::Provider;
use serde::{Deserialize, Serialize};
use tokio::{io::AsyncWrite, sync::mpsc::channel};
use tracing::{debug, error, info};

use super::stream::Streamer;

/// Main Chat structure, contains all chat-related attributes and methods
pub struct Chat<P: Provider> {
    messages: Vec<Message>,
    provider: P,
}

impl<P: Provider> Chat<P> {
    /// Create new chat
    pub fn new(provider: P) -> Self {
        Self {
            // All the messages of the chat
            messages: vec![],

            // Provider wich connect with the API
            provider,
        }
    }

    /// Send a message to Copilot and write the response to `Stdout` using the streamed data
    /// also returns the `System` message when it is ready.
    pub async fn send_message_with_stream(
        &self,
        message: Message,
        streamer: impl Streamer + Send + 'static,
        mut writer: impl AsyncWrite + Send + Unpin + 'static,
    ) -> anyhow::Result<Message> {
        let mut stream = self.provider.request(message).await?;
        debug!("Creating channels");
        let (sender, receiver) = channel(32);

        let streamer_clone = streamer.clone();

        // Write the stream to stdout
        let job = tokio::spawn(async move {
            streamer_clone
                .write_at_end(&mut writer, receiver)
                .await
                .unwrap_or_else(|e| {
                    error!(%e, "Error processing stream");
                });
        });

        info!("Collecting message");

        // Collect the message
        let message = streamer
            .handle_stream(std::pin::pin!(stream), sender)
            .await?;

        job.await?;

        info!("Message collected");
        Ok(message)
    }
}

/// A chat message
#[derive(Serialize, Deserialize, Debug)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

/// The sender of the message
#[derive(Serialize, Deserialize, Debug)]
pub enum Role {
    #[serde(rename = "system")]
    System,
    #[serde(rename = "user")]
    User,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::stream::tests::TestStreamer;
    use crate::client::provider::tests::TestProvider;

    /// Simulate the > /dev/null
    struct TestWriter;
    impl AsyncWrite for TestWriter {
        fn poll_write(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
            _buf: &[u8],
        ) -> std::task::Poll<Result<usize, std::io::Error>> {
            std::task::Poll::Ready(Ok(10))
        }

        fn poll_flush(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), std::io::Error>> {
            std::task::Poll::Ready(Ok(()))
        }

        fn poll_shutdown(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Result<(), std::io::Error>> {
            std::task::Poll::Ready(Ok(()))
        }
    }

    #[tokio::test]
    async fn test_send_message_with_stream() {
        let chunk = "
        {\"choices\":[{\"index\":0,\"content_filter_offsets\":{\"check_offset\":175,\"start_offset\":176,\"end_offset\":280},
        \"content_filter_results\":{\"hate\":{\"filtered\":false,\"severity\":\"safe\"},\"self_harm\":{\"filtered\":false,
        \"severity\":\"safe\"},\"sexual\":{\"filtered\":false,\"severity\":\"safe\"},\"violence\":{\"filtered\":false,\"severity\":\"safe\"}},
        \"delta\":{\"content\":\"Rust \"}}],\"created\":1751000792,\"id\":\"chatcmpl-BmvaCUrU0DjRli6juhycOsjF1OAZr\",
        \"model\":\"gpt-4o-2024-11-20\",\"system_fingerprint\":\"fp_b705f0c291\"}
        ";

        let provider = TestProvider::new(10, chunk);
        let chat = Chat::new(provider);
        let streamer = TestStreamer;
        let writer = TestWriter;

        let message = Message {
            role: Role::User,
            content: "hello".to_string(),
        };

        let response = chat
            .send_message_with_stream(message, streamer, writer)
            .await
            .expect("process the stream");

        assert_eq!(response.content, "Rust ".repeat(10));
    }
}
