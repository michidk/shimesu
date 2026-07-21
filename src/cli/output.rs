//! Output handling: JSON/human mode detection, rendering, and confirmation helpers.

use owo_colors::OwoColorize;
use serde::Serialize;
use std::io::{self, IsTerminal};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Human,
    Json,
}

impl OutputFormat {
    #[must_use]
    pub fn detect(json_flag: bool) -> Self {
        Self::from_stdout_terminal(json_flag, io::stdout().is_terminal())
    }

    #[must_use]
    fn from_stdout_terminal(json_flag: bool, stdout_is_terminal: bool) -> Self {
        if json_flag || !stdout_is_terminal {
            Self::Json
        } else {
            Self::Human
        }
    }

    pub fn allows_prompts(self) -> bool {
        matches!(self, Self::Human) && io::stdin().is_terminal()
    }
}

#[derive(Serialize)]
pub struct JsonResponse<T: Serialize> {
    pub schema_version: &'static str,
    #[serde(flatten)]
    pub data: T,
}

impl<T: Serialize> JsonResponse<T> {
    #[must_use]
    pub fn new(data: T) -> Self {
        Self {
            schema_version: "1",
            data,
        }
    }
}

pub struct Output {
    format: OutputFormat,
}

impl Output {
    #[must_use]
    pub fn new(format: OutputFormat) -> Self {
        Self { format }
    }

    pub fn success<T: Serialize>(&self, data: T) -> crate::error::Result<()> {
        match self.format {
            OutputFormat::Json => {
                let response = JsonResponse::new(data);
                let json = serde_json::to_string_pretty(&response)?;
                println!("{json}");
            }
            OutputFormat::Human => {
                unreachable!(
                    "Output::success must not be called in human mode; \
                     add an explicit human-readable rendering branch and call \
                     output.success only under OutputFormat::Json"
                );
            }
        }

        Ok(())
    }

    pub fn kv(&self, key: &str, value: &str) {
        if self.format == OutputFormat::Human {
            println!("{}: {}", key.bold(), value);
        }
    }

    pub fn header(&self, text: &str) {
        if self.format == OutputFormat::Human {
            println!("\n{}", text.bold().underline());
        }
    }

    pub fn ok(&self, message: &str) {
        if self.format == OutputFormat::Human {
            println!("{} {}", "✓".green().bold(), message);
        }
    }

    pub fn warn(&self, message: &str) {
        if self.format == OutputFormat::Human {
            eprintln!("{} {}", "⚠".yellow().bold(), message);
        }
    }

    pub fn error(&self, message: &str) {
        if self.format == OutputFormat::Human {
            eprintln!("{} {}", "✗".red().bold(), message);
        }
    }

    pub fn progress(&self, message: &str) {
        if self.format == OutputFormat::Human {
            println!("{} {}", "→".cyan(), message);
        }
    }

    #[must_use]
    pub fn format(&self) -> OutputFormat {
        self.format
    }

    pub fn allows_prompts(&self) -> bool {
        self.format.allows_prompts()
    }

    /// Render a command result: JSON mode serializes `data` to stdout,
    /// human mode runs the provided rendering closure. This is the single
    /// place where command handlers branch on the output format.
    pub fn render<T: Serialize>(
        &self,
        data: &T,
        human: impl FnOnce(&Self, &T),
    ) -> crate::error::Result<()> {
        match self.format {
            OutputFormat::Json => self.success(data),
            OutputFormat::Human => {
                human(self, data);
                Ok(())
            }
        }
    }
}

/// How a destructive or overwriting command should confirm with the operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmationMode {
    Skip,
    Prompt,
}

/// Resolve the confirmation mode for a destructive or overwriting operation.
/// `--yes` skips the prompt; otherwise an interactive terminal prompts and a
/// non-interactive invocation fails with `noninteractive_error`.
pub fn confirmation_mode(
    yes: bool,
    allows_prompts: bool,
    noninteractive_error: &str,
) -> crate::error::Result<ConfirmationMode> {
    if yes {
        Ok(ConfirmationMode::Skip)
    } else if allows_prompts {
        Ok(ConfirmationMode::Prompt)
    } else {
        Err(crate::error::ShimesuError::Usage(
            noninteractive_error.into(),
        ))
    }
}

/// Render a boolean as `"true"` / `"false"` for human-readable key-value rows.
#[must_use]
pub const fn bool_text(value: bool) -> &'static str {
    if value {
        "true"
    } else {
        "false"
    }
}

pub fn confirm(prompt: &str, default: bool) -> crate::error::Result<bool> {
    use dialoguer::Confirm;

    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err(crate::error::ShimesuError::Usage(
            "Cannot prompt for confirmation in non-interactive mode. Use --yes to skip prompts."
                .into(),
        ));
    }

    Confirm::new()
        .with_prompt(prompt)
        .default(default)
        .interact()
        .map_err(|error| crate::error::ShimesuError::Generic(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_detection_json_flag() {
        assert_eq!(OutputFormat::detect(true), OutputFormat::Json);
    }

    #[test]
    fn test_format_detection_non_tty_stdout() {
        assert_eq!(
            OutputFormat::from_stdout_terminal(false, false),
            OutputFormat::Json
        );
    }

    #[test]
    fn test_format_detection_interactive_stdout() {
        assert_eq!(
            OutputFormat::from_stdout_terminal(false, true),
            OutputFormat::Human
        );
    }

    #[test]
    fn confirmation_is_skipped_when_yes_is_set() {
        assert_eq!(
            confirmation_mode(true, false, "unused").expect("--yes should skip confirmation"),
            ConfirmationMode::Skip
        );
    }

    #[test]
    fn confirmation_prompts_in_interactive_mode() {
        assert_eq!(
            confirmation_mode(false, true, "unused").expect("interactive mode should prompt"),
            ConfirmationMode::Prompt
        );
    }

    #[test]
    fn confirmation_rejects_noninteractive_without_yes() {
        let result = confirmation_mode(false, false, "Use --yes to continue.");

        assert!(
            matches!(result, Err(crate::error::ShimesuError::Usage(message)) if message.contains("--yes"))
        );
    }

    #[test]
    fn bool_text_renders_true_and_false() {
        assert_eq!(bool_text(true), "true");
        assert_eq!(bool_text(false), "false");
    }

    #[test]
    fn test_json_response_has_schema_version() {
        #[derive(Serialize)]
        struct TestData {
            value: i32,
        }

        let response = JsonResponse::new(TestData { value: 42 });
        let json = serde_json::to_string(&response).expect("JSON serialization should succeed");

        assert!(json.contains("\"schema_version\":\"1\""));
        assert!(json.contains("\"value\":42"));
    }
}
