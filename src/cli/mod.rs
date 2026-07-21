//! Terminal interface: argument definitions and output formatting.

pub mod args;
pub mod output;

pub use args::*;
pub use output::{
    bool_text, confirm, confirmation_mode, ConfirmationMode, JsonResponse, Output, OutputFormat,
};
