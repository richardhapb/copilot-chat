use chat::ChatStreamer;
use clap::Parser;
use cli::commands::Cli;
use std::io::{self, Read};
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
        stdin.lock().read_to_string(&mut stdin_str)?;
    }
    
    // Parse the user prompt from CLI if exist - clone to avoid partial move
    let user_prompt = cli.prompt.as_ref().map(|prompt| prompt.join(" "));
    
    let mut handler = CommandHandler::new(&cli, stdin_str, user_prompt.as_deref());
    let mut attr = handler.prepare(client).await?;
    
    if attr.execution_type == ExecutionType::Exit {
        std::process::exit(0);
    }
    
    let writer = tokio::io::stdout();
    
    match attr.execution_type {
        ExecutionType::Once => {
            attr.process_request(&cli, streamer.clone(), writer).await?;
        }
        ExecutionType::Interactive | ExecutionType::Pipe => {
            attr.process_loop(&cli, &streamer, writer).await?;
            attr.chat.save_chat(None)?;
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
