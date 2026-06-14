//! The command router.
//!
//! Parse a chat string, fetch through the [`DataSource`] seam, and format a
//! [`BotResponse`]. This is the whole business core — the transport adapters do
//! nothing but pass a string in and a response out.

use crate::chart::chart_reply;
use crate::command::{Command, ParseError, parse};
use crate::data::{DataSource, NewsItem};
use crate::response::{BotResponse, Table};

/// Usage text for `/help` (and any unparseable input).
const HELP: &str = "\
The platform chat bot — quotes, news, and charts.

Commands:
  /quote <ticker>   Latest price snapshot, e.g. /quote AAPL
  /news <ticker>    Recent headlines, e.g. /news MSFT
  /chart <ticker>   Price chart of the recent series, e.g. /chart TSLA
  /help             Show this message";

/// Route one chat message to a [`BotResponse`], fetching through `source`.
///
/// Parse failures and data-fetch failures both render as a friendly text reply
/// rather than an error, because the only consumer is a human in a chat channel.
pub fn route(source: &dyn DataSource, message: &str) -> BotResponse {
    match parse(message) {
        Ok(command) => dispatch(source, &command),
        Err(error) => BotResponse::text(parse_error_text(&error)),
    }
}

/// Dispatch a parsed [`Command`] against the data seam.
fn dispatch(source: &dyn DataSource, command: &Command) -> BotResponse {
    match command {
        Command::Help => BotResponse::text(HELP),
        Command::Quote { ticker } => quote(source, ticker),
        Command::News { ticker } => news(source, ticker),
        Command::Chart { ticker } => chart(source, ticker),
    }
}

/// Render a parse failure as user-facing help text.
fn parse_error_text(error: &ParseError) -> String {
    match error {
        ParseError::NotACommand => {
            format!("Not a command. {HELP}")
        }
        ParseError::MissingTicker { verb } => {
            format!("/{verb} needs a ticker, e.g. /{verb} AAPL")
        }
        ParseError::UnknownCommand { verb } => {
            format!("Unknown command /{verb}. {HELP}")
        }
    }
}

/// `/quote` — a latest-price snapshot rendered as a key/value table.
fn quote(source: &dyn DataSource, ticker: &str) -> BotResponse {
    match source.quote(ticker) {
        Ok(quote) => {
            let mut rows = vec![(
                "price".to_string(),
                format_price(quote.price, quote.currency.as_deref()),
            )];
            if let Some(change) = quote.change {
                rows.push(("change".to_string(), format!("{change:+.2}")));
            }
            if let Some(pct) = quote.change_percent {
                rows.push(("change %".to_string(), format!("{pct:+.2}%")));
            }
            BotResponse::Table(Table::new(format!("{} quote", quote.symbol), rows))
        }
        Err(error) => BotResponse::text(format!("Could not fetch a quote for {ticker}: {error}")),
    }
}

/// `/news` — recent headlines rendered as a numbered list table.
fn news(source: &dyn DataSource, ticker: &str) -> BotResponse {
    match source.news(ticker) {
        Ok(items) if items.is_empty() => {
            BotResponse::text(format!("No recent headlines for {ticker}."))
        }
        Ok(items) => {
            let rows = items
                .iter()
                .take(5)
                .enumerate()
                .map(|(i, item)| (format!("{}.", i + 1), format_news_item(item)))
                .collect();
            BotResponse::Table(Table::new(format!("{ticker} news"), rows))
        }
        Err(error) => BotResponse::text(format!("Could not fetch news for {ticker}: {error}")),
    }
}

/// Render one headline as `Title (Source, Published)`, including only the
/// non-empty metadata fields so an empty source or date never produces a
/// dangling comma or empty parentheses.
fn format_news_item(item: &NewsItem) -> String {
    let meta: Vec<&str> = [item.source.as_str(), item.published.as_str()]
        .into_iter()
        .filter(|field| !field.is_empty())
        .collect();
    if meta.is_empty() {
        item.title.clone()
    } else {
        format!("{} ({})", item.title, meta.join(", "))
    }
}

/// `/chart` — a price chart of the recent series.
fn chart(source: &dyn DataSource, ticker: &str) -> BotResponse {
    match source.history(ticker) {
        Ok(bars) if bars.is_empty() => BotResponse::text(format!("No price history for {ticker}.")),
        Ok(bars) => BotResponse::Chart(chart_reply(ticker, &bars)),
        Err(error) => BotResponse::text(format!("Could not fetch a chart for {ticker}: {error}")),
    }
}

/// Format a price with an optional currency suffix.
fn format_price(price: f64, currency: Option<&str>) -> String {
    currency.map_or_else(
        || format!("{price:.2}"),
        |code| format!("{price:.2} {code}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::fake::FakeDataSource;
    use crate::data::{NewsItem, Quote};
    use crate::response::BotResponse;
    use tdw_domain::{MarketDataBar, TimeGranularity};

    fn bar(ts: &str, close: f64) -> MarketDataBar {
        MarketDataBar {
            symbol: "AAPL".to_string(),
            venue: "XNAS".to_string(),
            granularity: TimeGranularity::Day,
            ts: ts.to_string(),
            open: close,
            high: close,
            low: close,
            close,
            volume: 1.0,
            source: "fake".to_string(),
        }
    }

    fn seeded() -> FakeDataSource {
        FakeDataSource::default()
            .with_quote(
                "AAPL",
                Quote {
                    symbol: "AAPL".to_string(),
                    price: 192.32,
                    change: Some(1.25),
                    change_percent: Some(0.65),
                    currency: Some("USD".to_string()),
                },
            )
            .with_news(
                "AAPL",
                vec![NewsItem {
                    title: "Apple ships new chip".to_string(),
                    source: "Wire".to_string(),
                    published: "2026-06-14".to_string(),
                }],
            )
            .with_history(
                "AAPL",
                vec![
                    bar("2026-06-12", 190.0),
                    bar("2026-06-13", 191.0),
                    bar("2026-06-14", 192.3),
                ],
            )
    }

    #[test]
    fn quote_dispatches_to_a_table_with_price() {
        let response = route(&seeded(), "/quote aapl");
        let BotResponse::Table(table) = response else {
            panic!("expected a table");
        };
        assert_eq!(table.title, "AAPL quote");
        assert!(
            table
                .rows
                .iter()
                .any(|(k, v)| k == "price" && v.contains("192.32"))
        );
        assert!(table.rows.iter().any(|(k, _)| k == "change %"));
    }

    #[test]
    fn news_dispatches_to_a_list() {
        let response = route(&seeded(), "/news AAPL");
        let text = response.to_text();
        assert!(text.contains("AAPL news"));
        assert!(text.contains("Apple ships new chip"));
    }

    #[test]
    fn chart_dispatches_to_a_chart_reply_with_a_figure() {
        let response = route(&seeded(), "/chart AAPL");
        let BotResponse::Chart(chart) = response else {
            panic!("expected a chart");
        };
        assert!(chart.caption.contains("AAPL"));
        assert!(chart.caption.contains("3 points"));
        // The figure spec is always present (a Plotly figure object).
        assert!(chart.figure_json.get("data").is_some());
    }

    #[test]
    fn news_with_empty_metadata_has_no_dangling_punctuation() {
        // A headline whose source and date are both empty must render as the
        // bare title — no `(, )` and no empty parentheses.
        let item = NewsItem {
            title: "Headline only".to_string(),
            source: String::new(),
            published: String::new(),
        };
        assert_eq!(format_news_item(&item), "Headline only");

        // Only the source present → `Title (Source)`, no trailing comma.
        let with_source = NewsItem {
            title: "With source".to_string(),
            source: "Wire".to_string(),
            published: String::new(),
        };
        assert_eq!(format_news_item(&with_source), "With source (Wire)");

        // Only the date present → `Title (Date)`, no leading comma.
        let with_date = NewsItem {
            title: "With date".to_string(),
            source: String::new(),
            published: "2026-06-14".to_string(),
        };
        assert_eq!(format_news_item(&with_date), "With date (2026-06-14)");

        // Both present → `Title (Source, Date)`.
        let both = NewsItem {
            title: "Both".to_string(),
            source: "Wire".to_string(),
            published: "2026-06-14".to_string(),
        };
        assert_eq!(format_news_item(&both), "Both (Wire, 2026-06-14)");
    }

    #[test]
    fn help_renders_usage() {
        let response = route(&seeded(), "/help");
        assert!(response.to_text().contains("/quote <ticker>"));
    }

    #[test]
    fn unknown_command_falls_back_to_help() {
        let response = route(&seeded(), "/frobnicate AAPL");
        let text = response.to_text();
        assert!(text.contains("Unknown command /frobnicate"));
        assert!(text.contains("/quote <ticker>"));
    }

    #[test]
    fn missing_ticker_explains_usage() {
        let response = route(&seeded(), "/quote");
        assert!(response.to_text().contains("/quote needs a ticker"));
    }

    #[test]
    fn fetch_error_renders_as_text_not_a_panic() {
        // UNKNOWN is not seeded, so the fake yields a DataError.
        let response = route(&seeded(), "/quote UNKNOWN");
        let text = response.to_text();
        assert!(text.contains("Could not fetch a quote for UNKNOWN"));
    }

    #[cfg(feature = "charts")]
    #[test]
    fn chart_feature_rasterizes_a_png() {
        let response = route(&seeded(), "/chart AAPL");
        let BotResponse::Chart(chart) = response else {
            panic!("expected a chart");
        };
        let png = chart.png.expect("charts feature must attach a PNG");
        // PNG magic number.
        assert_eq!(
            &png[..8],
            &[0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n']
        );
    }
}
