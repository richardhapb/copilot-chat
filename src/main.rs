use tracing_subscriber::prelude::__tracing_subscriber_SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
mod client;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "copilot_chat=info".into()),
        ))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let auth = client::auth::CopilotAuth::new();
    let client = client::CopilotClient::new(auth);

    client.request().await?;

    Ok(())
}
