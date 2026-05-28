#![forbid(unsafe_code)]

pub const CRATE_NAME: &str = "tdw-rewrite";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RewriteRule {
    pub rule_id: String,
    pub find: String,
    pub replace: String,
    pub enabled: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RewritePlan {
    pub rules: Vec<RewriteRule>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RewriteError {
    InvalidRuleId,
    EmptyFind,
    UnsafePattern,
}

/// # Errors
///
/// Returns an error variant if the underlying operation fails.
pub fn apply_rewrites(input: &str, plan: &RewritePlan) -> Result<String, RewriteError> {
    validate_plan(plan)?;
    let mut output = input.to_string();
    for rule in plan.rules.iter().filter(|rule| rule.enabled) {
        output = output.replace(&rule.find, &rule.replace);
    }
    Ok(output)
}

/// # Errors
///
/// Returns an error variant if the underlying operation fails.
pub fn validate_plan(plan: &RewritePlan) -> Result<(), RewriteError> {
    for rule in &plan.rules {
        if !is_identifier(&rule.rule_id) {
            return Err(RewriteError::InvalidRuleId);
        }
        if rule.find.is_empty() {
            return Err(RewriteError::EmptyFind);
        }
        if contains_control_or_shell(&rule.find) || contains_control_or_shell(&rule.replace) {
            return Err(RewriteError::UnsafePattern);
        }
    }
    Ok(())
}

fn is_identifier(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
}

fn contains_control_or_shell(value: &str) -> bool {
    value
        .chars()
        .any(|character| character.is_control() || matches!(character, ';' | '|' | '`'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applies_enabled_rewrite_rules_in_order() {
        let plan = RewritePlan {
            rules: vec![
                RewriteRule {
                    rule_id: "normalize-symbol".to_string(),
                    find: "aapl".to_string(),
                    replace: "AAPL".to_string(),
                    enabled: true,
                },
                RewriteRule {
                    rule_id: "disabled".to_string(),
                    find: "AAPL".to_string(),
                    replace: "MSFT".to_string(),
                    enabled: false,
                },
            ],
        };

        assert_eq!(
            apply_rewrites("research aapl", &plan),
            Ok("research AAPL".to_string())
        );
    }

    #[test]
    fn rejects_empty_and_unsafe_rewrite_patterns() {
        assert_eq!(
            validate_plan(&RewritePlan {
                rules: vec![RewriteRule {
                    rule_id: CRATE_NAME.to_string(),
                    find: String::new(),
                    replace: "safe".to_string(),
                    enabled: true,
                }],
            }),
            Err(RewriteError::EmptyFind)
        );
        assert_eq!(
            validate_plan(&RewritePlan {
                rules: vec![RewriteRule {
                    rule_id: "unsafe".to_string(),
                    find: "AAPL".to_string(),
                    replace: "AAPL;DROP".to_string(),
                    enabled: true,
                }],
            }),
            Err(RewriteError::UnsafePattern)
        );
    }
}
