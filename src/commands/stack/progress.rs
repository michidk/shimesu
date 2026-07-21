//! Spinner progress reporting for long-running stack operations.

use std::time::Duration;

use indicatif::ProgressBar;

use crate::cli::{Output, OutputFormat};

const SPINNER_TICK: Duration = Duration::from_millis(120);

pub(super) struct StackProgress {
    progress_bar: Option<ProgressBar>,
}

impl StackProgress {
    pub(super) fn new(output: &Output, message: &str) -> Self {
        let progress_bar = match output.format() {
            OutputFormat::Human => {
                let progress_bar = ProgressBar::new_spinner();
                progress_bar.set_message(message.to_string());
                progress_bar.enable_steady_tick(SPINNER_TICK);
                Some(progress_bar)
            }
            OutputFormat::Json => None,
        };

        Self { progress_bar }
    }

    pub(super) fn set_message(&self, message: &str) {
        if let Some(progress_bar) = &self.progress_bar {
            progress_bar.set_message(message.to_string());
        }
    }

    pub(super) fn clear(&mut self) {
        if let Some(progress_bar) = self.progress_bar.take() {
            progress_bar.finish_and_clear();
        }
    }
}

impl Drop for StackProgress {
    fn drop(&mut self) {
        self.clear();
    }
}
