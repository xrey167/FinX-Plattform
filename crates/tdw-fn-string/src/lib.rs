#![forbid(unsafe_code)]
#![deny(clippy::pedantic, clippy::nursery)]
pub const CRATE_NAME: &str = "tdw-fn-string";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StringFn {
    Trim,
    Uppercase,
    Lowercase,
    Replace { from: String, to: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StringPipeline {
    pub name: String,
    pub steps: Vec<StringFn>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StringFnError {
    InvalidPipelineName,
    EmptyPipeline,
    EmptyPattern,
    UnsafePattern,
}

/// # Errors
///
/// Returns an error variant if the underlying operation fails.
pub fn apply_pipeline(input: &str, pipeline: &StringPipeline) -> Result<String, StringFnError> {
    validate_pipeline(pipeline)?;
    let mut value = input.to_string();
    for step in &pipeline.steps {
        match step {
            StringFn::Trim => value = value.trim().to_string(),
            StringFn::Uppercase => value = value.to_ascii_uppercase(),
            StringFn::Lowercase => value = value.to_ascii_lowercase(),
            StringFn::Replace { from, to } => value = value.replace(from, to),
        }
    }
    Ok(value)
}

/// # Errors
///
/// Returns an error variant if the underlying operation fails.
pub fn validate_pipeline(pipeline: &StringPipeline) -> Result<(), StringFnError> {
    if !is_identifier(&pipeline.name) {
        return Err(StringFnError::InvalidPipelineName);
    }
    if pipeline.steps.is_empty() {
        return Err(StringFnError::EmptyPipeline);
    }
    for step in &pipeline.steps {
        if let StringFn::Replace { from, to } = step {
            if from.is_empty() {
                return Err(StringFnError::EmptyPattern);
            }
            if contains_control_or_shell(from) || contains_control_or_shell(to) {
                return Err(StringFnError::UnsafePattern);
            }
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
    fn applies_deterministic_string_pipeline() {
        let pipeline = StringPipeline {
            name: CRATE_NAME.to_string(),
            steps: vec![
                StringFn::Trim,
                StringFn::Replace {
                    from: " ".to_string(),
                    to: "_".to_string(),
                },
                StringFn::Uppercase,
            ],
        };

        assert_eq!(
            apply_pipeline(" aapl equity ", &pipeline),
            Ok("AAPL_EQUITY".to_string())
        );
    }

    #[test]
    fn rejects_empty_and_unsafe_replace_patterns() {
        let pipeline = StringPipeline {
            name: "normalize".to_string(),
            steps: vec![StringFn::Replace {
                from: String::new(),
                to: "safe".to_string(),
            }],
        };

        assert_eq!(
            validate_pipeline(&pipeline),
            Err(StringFnError::EmptyPattern)
        );
        assert_eq!(
            validate_pipeline(&StringPipeline {
                steps: vec![StringFn::Replace {
                    from: "AAPL".to_string(),
                    to: "AAPL;DROP".to_string(),
                }],
                ..pipeline
            }),
            Err(StringFnError::UnsafePattern)
        );
    }

    #[test]
    fn rejects_invalid_names_and_empty_pipelines() {
        // Non-identifier names are rejected before steps are inspected.
        for bad in ["", "has space", "semi;colon"] {
            assert_eq!(
                validate_pipeline(&StringPipeline {
                    name: bad.to_string(),
                    steps: vec![StringFn::Trim],
                }),
                Err(StringFnError::InvalidPipelineName)
            );
        }

        // A valid name with no steps is an empty pipeline.
        assert_eq!(
            validate_pipeline(&StringPipeline {
                name: "normalize".to_string(),
                steps: Vec::new(),
            }),
            Err(StringFnError::EmptyPipeline)
        );
    }

    #[test]
    fn applies_lowercase_step() {
        let pipeline = StringPipeline {
            name: "normalize".to_string(),
            steps: vec![StringFn::Lowercase],
        };
        assert_eq!(apply_pipeline("AApL", &pipeline), Ok("aapl".to_string()));
    }
}
