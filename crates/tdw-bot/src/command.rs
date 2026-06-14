//! The slash-command parser: a chat string becomes a typed [`Command`].
//!
//! The grammar is intentionally tiny and offline: a leading `/`, a verb, and a
//! single ticker argument for the data commands. Parsing never touches the
//! network — it is pure string work, so every branch is unit-testable.

/// A parsed chat command the router can dispatch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    /// `/quote <ticker>` — a latest-price snapshot.
    Quote { ticker: String },
    /// `/news <ticker>` — recent headlines for a ticker.
    News { ticker: String },
    /// `/chart <ticker>` — a price chart of the recent series.
    Chart { ticker: String },
    /// `/help` (or `/start`) — usage text.
    Help,
}

/// Why a chat string did not parse into a [`Command`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParseError {
    /// The string was empty or not a `/`-prefixed command.
    NotACommand,
    /// A known verb was missing its required ticker argument.
    MissingTicker {
        /// The verb that needed an argument (e.g. `"quote"`).
        verb: String,
    },
    /// The verb was not one the bot understands.
    UnknownCommand {
        /// The unrecognized verb (without the leading `/`).
        verb: String,
    },
}

/// The set of upper-cased ticker characters the bot accepts (letters, digits, a
/// dot and a dash — enough for `BRK.B`, `9988.HK`, dashed class tickers).
fn normalize_ticker(raw: &str) -> Option<String> {
    let ticker: String = raw.trim().to_ascii_uppercase();
    if !ticker.is_empty()
        && ticker
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
    {
        Some(ticker)
    } else {
        None
    }
}

/// Parse a chat message into a [`Command`].
///
/// # Errors
///
/// Returns [`ParseError::NotACommand`] for non-`/` input, [`ParseError::
/// MissingTicker`] when a data verb has no argument, and
/// [`ParseError::UnknownCommand`] for an unknown verb.
pub fn parse(input: &str) -> Result<Command, ParseError> {
    let trimmed = input.trim();
    let body = trimmed.strip_prefix('/').ok_or(ParseError::NotACommand)?;

    let mut parts = body.split_whitespace();
    let verb = parts
        .next()
        .ok_or(ParseError::NotACommand)?
        .to_ascii_lowercase();
    let arg = parts.next();

    let require_ticker = |arg: Option<&str>| {
        arg.and_then(normalize_ticker)
            .ok_or_else(|| ParseError::MissingTicker { verb: verb.clone() })
    };

    match verb.as_str() {
        "help" | "start" => Ok(Command::Help),
        "quote" => Ok(Command::Quote {
            ticker: require_ticker(arg)?,
        }),
        "news" => Ok(Command::News {
            ticker: require_ticker(arg)?,
        }),
        "chart" => Ok(Command::Chart {
            ticker: require_ticker(arg)?,
        }),
        other => Err(ParseError::UnknownCommand {
            verb: other.to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_quote_and_uppercases_ticker() {
        assert_eq!(
            parse("/quote aapl"),
            Ok(Command::Quote {
                ticker: "AAPL".to_string()
            })
        );
    }

    #[test]
    fn parses_news_and_chart() {
        assert_eq!(
            parse("/news MSFT"),
            Ok(Command::News {
                ticker: "MSFT".to_string()
            })
        );
        assert_eq!(
            parse("/chart brk.b"),
            Ok(Command::Chart {
                ticker: "BRK.B".to_string()
            })
        );
    }

    #[test]
    fn help_and_start_alias() {
        assert_eq!(parse("/help"), Ok(Command::Help));
        assert_eq!(parse("/start"), Ok(Command::Help));
    }

    #[test]
    fn leading_and_trailing_whitespace_is_tolerated() {
        assert_eq!(
            parse("   /quote   tsla   "),
            Ok(Command::Quote {
                ticker: "TSLA".to_string()
            })
        );
    }

    #[test]
    fn missing_ticker_is_an_error() {
        assert_eq!(
            parse("/quote"),
            Err(ParseError::MissingTicker {
                verb: "quote".to_string()
            })
        );
    }

    #[test]
    fn malformed_ticker_is_rejected() {
        assert_eq!(
            parse("/quote a b c"),
            Ok(Command::Quote {
                ticker: "A".to_string()
            })
        );
        // A ticker with illegal characters is treated as missing.
        assert_eq!(
            parse("/quote $$$"),
            Err(ParseError::MissingTicker {
                verb: "quote".to_string()
            })
        );
    }

    #[test]
    fn non_command_text_is_not_a_command() {
        assert_eq!(parse("hello there"), Err(ParseError::NotACommand));
        assert_eq!(parse(""), Err(ParseError::NotACommand));
    }

    #[test]
    fn unknown_verb_is_reported() {
        assert_eq!(
            parse("/frobnicate AAPL"),
            Err(ParseError::UnknownCommand {
                verb: "frobnicate".to_string()
            })
        );
    }
}
