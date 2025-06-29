use crate::chat::{Builder, Message};
use futures_util::Stream;

/// A message provider from the Copilot API
pub trait Provider {
    async fn request(
        &self,
        messages: &Vec<Message>,
    ) -> anyhow::Result<impl Stream<Item = reqwest::Result<bytes::Bytes>>>;

    fn builder(&self) -> Builder<Self>
    where
        Self: Sized,
    {
        Builder::new(self)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::{
        cell::RefCell,
        sync::atomic::{AtomicUsize, Ordering},
    };

    use bytes::{BufMut, Bytes, BytesMut};
    use futures_util::Stream;

    use crate::chat::Message;

    use super::Provider;

    #[derive(Default)]
    pub struct TestProvider<'a> {
        chunks: usize,
        content: &'a str,
        pub input_messages: RefCell<Vec<Message>>,
    }

    impl<'a> TestProvider<'a> {
        pub fn new(chunks: usize, content: &'a str) -> Self {
            Self {
                chunks,
                content,
                input_messages: RefCell::new(vec![]),
            }
        }
    }

    pub struct TestStreamProvider<'a> {
        chunks: AtomicUsize,
        content: &'a str,
    }

    impl<'a> TestStreamProvider<'a> {
        pub fn new(chunks: usize, content: &'a str) -> Self {
            Self {
                chunks: AtomicUsize::new(chunks),
                content,
            }
        }
    }

    impl<'a> Stream for TestStreamProvider<'a> {
        type Item = reqwest::Result<Bytes>;
        fn poll_next(
            self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
        ) -> std::task::Poll<Option<Self::Item>> {
            if self.chunks.fetch_sub(1, Ordering::Relaxed) == 0 {
                return std::task::Poll::Ready(None);
            }

            let mut bytes = BytesMut::with_capacity(self.content.len());
            bytes.put(&mut self.content.as_bytes());
            std::task::Poll::Ready(Some(Ok(bytes.into())))
        }
    }

    impl<'a> Provider for TestProvider<'a> {
        async fn request(
            &self,
            messages: &Vec<crate::chat::Message>,
        ) -> anyhow::Result<impl Stream<Item = reqwest::Result<bytes::Bytes>>> {
            let stream = TestStreamProvider::new(self.chunks, self.content);
            self.input_messages.replace(messages.to_owned());
            Ok(stream)
        }
    }
}
