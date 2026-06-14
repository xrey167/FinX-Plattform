//! The Discord chat-bot binary (built only under `--features discord`).
//!
//! Reads the bot token from `DISCORD_TOKEN` and the warehouse REST base URL from
//! `TDW_BOT_REST_URL` (defaulting to the documented loopback daemon bind), then
//! connects a `serenity` client wired to the platform-agnostic bot core.

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let Ok(token) = std::env::var("DISCORD_TOKEN") else {
        eprintln!("tdw-bot-discord: set DISCORD_TOKEN to the bot token");
        return std::process::ExitCode::FAILURE;
    };
    let base_url =
        std::env::var("TDW_BOT_REST_URL").unwrap_or_else(|_| "http://127.0.0.1:7878".to_string());

    if let Err(error) = tdw_bot::adapters::discord::run(&token, &base_url).await {
        eprintln!("tdw-bot-discord: client error: {error}");
        return std::process::ExitCode::FAILURE;
    }
    std::process::ExitCode::SUCCESS
}
