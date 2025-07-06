use chat::{Chat, ChatStreamer, Message, MessageType};
use clap::Parser;
use cli::commands::{Cli, Commands};
use std::io::{self, Read, Write};
use tools::cli::CliExecutor;
use tracing::debug;
use tracing_subscriber::prelude::__tracing_subscriber_SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use crate::client::CopilotClient;
use crate::client::provider::Provider;

mod chat;
mod cli;
mod client;
mod tools;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_logging();
    let cli = Cli::parse();

    // Dependencies
    let auth = client::auth::CopilotAuth::new();
    let client = client::CopilotClient::new(auth);
    let streamer = ChatStreamer;

    let mut stdin_str = String::new();

    // Read only from piped stdin
    if !atty::is(atty::Stream::Stdin) {
        debug!("Reading from stdin");
        // STDIN
        let stdin = io::stdin();
        stdin.lock().read_to_string(&mut stdin_str)?;
    }

    // Parse the user prompt from CLI if exist.
    let user_prompt = match cli.prompt {
        Some(prompt) => Some(prompt.join(" ")),
        None => None,
    };

    let mut should_save_chat = true;
    let mut once = false;

    // Get the message and chat; if the command is `commit`, a new chat is always used.
    let (message_type, mut chat) = match cli.command {
        Some(Commands::Commit) => {
            // If there is not a stdin, try to get the git diff
            // in the current directory
            if stdin_str.is_empty() {
                stdin_str = CliExecutor::new().execute("git", &["diff", "--staged"]).await?;

                if stdin_str.is_empty() {
                    eprintln!("Git diff is empty. Ensure you are in a repository and that the changes are staged.");
                    std::process::exit(1);
                }
            }

            should_save_chat = false;
            once = true;
            (MessageType::Commit(user_prompt), Chat::new(client))
        }
        Some(Commands::Models) => {
            client.get_models().await?;
            std::process::exit(0);
        }
        Some(Commands::Clear) => match Chat::<CopilotClient>::try_load_chat(None)? {
            Some(chat) => {
                chat.remove_chat(None)?;
                println!("Chat cleared successfully");
                std::process::exit(0)
            }
            None => {
                println!("Chat not found; skipping clearing.");
                std::process::exit(0);
            }
        },
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

    let mut first = true;

    loop {
        let writer = tokio::io::stdout();
        let mut stdout = io::stdout();

        let inner_str = if !first {
            println!();
            println!();
            print!("> ");
            debug!("Capturing new message");
            stdout.flush()?;

            let stdin = io::stdin();
            let mut read_str = String::new();

            if !atty::is(atty::Stream::Stdin) {
                debug!("Reading from stdin piped");
                // STDIN
                stdin.lock().read_to_string(&mut read_str)?;
            } else {
                debug!("Reading from interactive mode");
                stdin.read_line(&mut read_str)?;
            }

            debug!(%read_str, "New user message");
            if read_str == "exit" {
                break;
            }
            read_str
        } else {
            debug!("Processing first message");
            debug!(%stdin_str, "Processing first message");
            first = false;

            stdin_str.clone()
        };

        // First message
        let message = Message {
            role: chat::Role::User,
            content: inner_str,
        };

        debug!(?message_type, "User message");

        // Send with stream by default, maybe in the future a buffered
        // response can be returned if it is configured
        let response_message = chat
            .send_message_with_stream(
                cli.model.as_deref(),
                message,
                message_type.clone(),
                streamer.clone(),
                writer,
            )
            .await?;
        chat.add_message(response_message);

        // If it is a one-time execution, breaks
        if once {
            break;
        }
    }

    if should_save_chat {
        chat.save_chat(None)?;
    }
    Ok(())
}

fn init_logging() {
    let file = std::fs::File::create("/tmp/copilot-chat.log").unwrap();
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "copilot_chat=info".into()),
        ))
        .with(tracing_subscriber::fmt::layer().with_writer(file))
        .init();
}
