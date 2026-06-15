//! The Telegram chat-bot binary (built only under `--features telegram`).
//!
//! Builds a `teloxide` `Bot` from `TELOXIDE_TOKEN` and reads the warehouse REST
//! base URL from `TDW_BOT_REST_URL` (defaulting to the documented loopback daemon
//! bind), then runs a long-polling dispatcher wired to the bot core.

use teloxide::prelude::Bot;

#[tokio::main]
async fn main() {
    let base_url =
        std::env::var("TDW_BOT_REST_URL").unwrap_or_else(|_| "http://127.0.0.1:7878".to_string());
    // `Bot::from_env` reads TELOXIDE_TOKEN.
    let bot = Bot::from_env();
    println!("tdw-bot-telegram: starting long-poll dispatcher");
    tdw_bot::adapters::telegram::run(bot, base_url).await;
}
