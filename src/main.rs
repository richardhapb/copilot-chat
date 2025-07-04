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
    let streamer = ChatStreamer;
    let writer = tokio::io::stdout();

    let mut stdin_str = String::new();

    // Read only from piped stdin
    if !atty::is(atty::Stream::Stdin) {
        // STDIN
        let stdin = io::stdin();
        stdin.lock().read_to_string(&mut stdin_str)?;
    }

    // Parse the user prompt from CLI if exist.
    let user_prompt = match cli.prompt {
        Some(prompt) => Some(prompt.join(" ")),
        None => None,
    };

    // Get the message and chat; if the command is `commit`, a new chat is always used.
    let (message_type, mut chat) = match cli.command {
        Some(Commands::Commit) => {
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
            (MessageType::Commit(user_prompt), Chat::new(client))
        }
        // Default
        None => (
            MessageType::Code {
                user_prompt,
                files: cli.files,
            },
            match Chat::try_load_chat(None)? {
                Some(chat) => chat.with_provider(client),
                None => Chat::new(client),
            },
        ),
    };

    // First message
    let message = Message {
        role: chat::Role::User,
        content: stdin_str,
    };

    // Send with stream by default, maybe in the future a buffered
    // response can be returned if it is configured
    let response_message = chat
        .send_message_with_stream(message, message_type, streamer, writer)
        .await?;

    chat.add_message(response_message);
    chat.save_chat(None)?;
    Ok(())
}
