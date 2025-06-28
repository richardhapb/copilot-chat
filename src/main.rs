use chat::{Chat, ChatStreamer, Message, MessageType};
use clap::Parser;
use cli::commands::{Cli, Commands};
use std::io::{self, Read};
use tools::cli::CliExecutor;
use tracing_subscriber::prelude::__tracing_subscriber_SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

mod chat;
mod cli;
mod client;
mod tools;

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

    // Dependencies
    let auth = client::auth::CopilotAuth::new();
    let client = client::CopilotClient::new(auth);
    let chat = Chat::new(client);
    let streamer = ChatStreamer;
    let writer = tokio::io::stdout();

    let mut stdin_str = String::new();

    // Read only from piped stdin
    if !atty::is(atty::Stream::Stdin) {
        // STDIN
        let stdin = io::stdin();
        stdin.lock().read_to_string(&mut stdin_str)?;
    }

    let mut message_type = MessageType::Code;
    match cli.command {
        Some(Commands::Commit) => {
            message_type = MessageType::Commit;

            // If there is not a stdin, try to get the git diff
            // in the current directory
            if stdin_str.is_empty() {
                stdin_str = CliExecutor::new()
                    .execute("git", &["diff", "--staged"])
                    .await?;

                if stdin_str.is_empty() {
                    eprintln!(
                        "Git diff is empty. Ensure you are in a repository and that the changes are staged."
                    );
                    std::process::exit(1);
                }
            }
        }
        None => {}
    }

    // First message
    let message = Message {
        role: chat::Role::User,
        content: stdin_str,
    };

    // Send with stream by default, maybe in the future a buffered
    // response can be returned if it is configured
    chat.send_message_with_stream(message, message_type, streamer, writer)
        .await?;

    Ok(())
}
