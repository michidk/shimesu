//! Shimesu library: CLI surface, command implementations, and AWS helpers behind the `shimesu` binary.

pub mod cli;
pub mod commands;
pub mod config;
pub mod core;
pub mod error;

pub use config::Config;
pub use error::{ExitCode, Result, ShimesuError};
