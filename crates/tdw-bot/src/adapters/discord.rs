//! The Discord transport adapter (gated behind the `discord` feature).
//!
//! A thin `serenity` [`EventHandler`]: on each inbound message it hands the text
//! to [`crate::router::route`] and sends the formatted reply back. No command
//! parsing, fetching, or formatting lives here — that is all in the core.

use serenity::Client;
use serenity::all::{Context, EventHandler, GatewayIntents, Message, Ready};
use serenity::async_trait;

use crate::adapters::rest::RestDataSource;
use crate::data::DataError;
use crate::router::route;

/// A `serenity` event handler that routes each message through the bot core.
pub struct DiscordHandler {
    base_url: String,
}

impl DiscordHandler {
    /// Build a handler that fetches against the warehouse REST base URL.
    #[must_use]
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
        }
    }
}

#[async_trait]
impl EventHandler for DiscordHandler {
    async fn message(&self, ctx: Context, message: Message) {
        // Ignore non-command chatter and the bot's own messages.
        if message.author.bot || !message.content.trim_start().starts_with('/') {
            return;
        }
        let base_url = self.base_url.clone();
        let content = message.content.clone();
        // The core + REST client are blocking; run them off the async executor.
        let reply = tokio::task::spawn_blocking(move || render(&base_url, &content))
            .await
            .unwrap_or_else(|error| format!("internal error: {error}"));
        if let Err(error) = message.channel_id.say(&ctx.http, reply).await {
            eprintln!("tdw-bot-discord: failed to send reply: {error}");
        }
    }

    async fn ready(&self, _ctx: Context, ready: Ready) {
        println!("tdw-bot-discord: connected as {}", ready.user.name);
    }
}

/// Build a reply string for one message by routing it through the core.
fn render(base_url: &str, content: &str) -> String {
    match RestDataSource::new(base_url) {
        Ok(source) => route(&source, content).to_text(),
        Err(DataError { message }) => format!("data source unavailable: {message}"),
    }
}

/// Connect a Discord client wired to the bot core and run until disconnected.
///
/// # Errors
///
/// Returns the `serenity` client error if the gateway connection fails.
pub async fn run(token: &str, base_url: &str) -> Result<(), serenity::Error> {
    let intents = GatewayIntents::GUILD_MESSAGES
        | GatewayIntents::DIRECT_MESSAGES
        | GatewayIntents::MESSAGE_CONTENT;
    let mut client = Client::builder(token, intents)
        .event_handler(DiscordHandler::new(base_url))
        .await?;
    client.start().await
}
