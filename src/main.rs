use chat::prompts::DEFAULT_PROMPT;
use chat::{Chat, ChatStreamer, Message};
use clap::Parser;
use cli::commands::{Cli, Commands};
use std::io::{self, Read};
use tracing_subscriber::prelude::__tracing_subscriber_SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

mod chat;
mod cli;
mod client;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let file = std::fs::File::create("/tmp/copilot-chat.log").unwrap();
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "copilot_chat=info".into()),
        ))
        .with(tracing_subscriber::fmt::layer().with_writer(file))
        .init();

    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Commit) => {}
        None => {}
    }

    // Dependencies
    let auth = client::auth::CopilotAuth::new();
    let client = client::CopilotClient::new(auth);
    let chat = Chat::new(client);
    let streamer = ChatStreamer;
    let writer = tokio::io::stdout();

    // STDIN
    let stdin = io::stdin();
    let mut stdin_str = String::new();
    let n = stdin.lock().read_to_string(&mut stdin_str)?;

    // If there is not a stdin, pass the default prompt
    if n == 0 {
        stdin_str = DEFAULT_PROMPT.to_string();
    }

    // First message
    let message = Message {
        role: chat::Role::User,
        content: stdin_str,
    };

    // Send with stream by default, maybe in the future a buffered
    // response can be returned if it is configured
    chat.send_message_with_stream(message, streamer, writer)
        .await?;

    Ok(())
}
