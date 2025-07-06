use chat::ChatStreamer;
use clap::Parser;
use cli::commands::Cli;
use std::io::{self, BufRead};
use tracing::debug;
use tracing_subscriber::prelude::__tracing_subscriber_SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use crate::cli::handlers::{CommandHandler, ExecutionType};

mod chat;
mod cli;
mod client;
mod tools;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_logging()?;
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
        stdin.lock().read_line(&mut stdin_str)?;
    }

    debug! {%stdin_str, "Received"};

    // Parse the user prompt from CLI if exist - clone to avoid partial move
    let user_prompt = cli.prompt.as_ref().map(|prompt| prompt.join(" "));

    debug!(?user_prompt);

    let mut handler = CommandHandler::new(&cli, user_prompt.as_deref());
    let mut attr = handler.prepare(client, &mut stdin_str).await?;

    if attr.execution_type == ExecutionType::Exit {
        std::process::exit(0);
    }

    let writer = tokio::io::stdout();

    match attr.execution_type {
        ExecutionType::Once => {
            if let Err(e) = attr.process_request(&cli, streamer.clone(), writer, stdin_str).await {
                eprintln!("Error: {}", e);
            }
        }
        ExecutionType::Interactive | ExecutionType::Pipe => {
            if let Err(e) = attr.process_loop(&cli, &streamer, writer, stdin_str).await {
                eprintln!("Error: {}", e);
            }
            if let Err(e) = attr.chat.save_chat(None) {
                eprintln!("Error saving chat: {}", e);
            }
        }
        ExecutionType::Exit => {
            std::process::exit(0);
        }
    }

    Ok(())
}

fn init_logging() -> anyhow::Result<()> {
    let file = std::fs::File::create("/tmp/copilot-chat.log")?;
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "copilot_chat=info".into()),
        ))
        .with(tracing_subscriber::fmt::layer().with_writer(file))
        .init();
    Ok(())
}
