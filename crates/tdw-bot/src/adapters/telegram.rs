//! The Telegram transport adapter (gated behind the `telegram` feature).
//!
//! A thin `teloxide` message handler: each inbound text message is handed to
//! [`crate::router::route`] and the formatted reply is sent back. No command
//! parsing, fetching, or formatting lives here — that is all in the core.

use teloxide::prelude::{Bot, Message, Requester, ResponseResult};

use crate::adapters::rest::RestDataSource;
use crate::data::DataError;
use crate::router::route;

/// Build a reply string for one message by routing it through the core.
fn render(base_url: &str, content: &str) -> String {
    match RestDataSource::new(base_url) {
        Ok(source) => route(&source, content).to_text(),
        Err(DataError { message }) => format!("data source unavailable: {message}"),
    }
}

/// Handle one Telegram message: route its text and reply.
///
/// # Errors
///
/// Returns the `teloxide` request error if sending the reply fails.
pub async fn handle(bot: Bot, message: Message, base_url: String) -> ResponseResult<()> {
    let Some(text) = message.text().map(ToString::to_string) else {
        return Ok(());
    };
    if !text.trim_start().starts_with('/') {
        return Ok(());
    }
    // The core + REST client are blocking; run them off the async executor.
    let reply = tokio::task::spawn_blocking(move || render(&base_url, &text))
        .await
        .unwrap_or_else(|error| format!("internal error: {error}"));
    bot.send_message(message.chat.id, reply).await?;
    Ok(())
}

/// Start a long-polling Telegram dispatcher wired to the bot core.
///
/// `base_url` is the warehouse REST base URL each command fetches against. The
/// `Bot` is built from the `TELOXIDE_TOKEN` environment variable by the caller.
pub async fn run(bot: Bot, base_url: String) {
    // `teloxide::repl` returns a large future; box it to keep it off the stack.
    Box::pin(teloxide::repl(bot, move |bot: Bot, message: Message| {
        let base_url = base_url.clone();
        async move { handle(bot, message, base_url).await }
    }))
    .await;
}
