//! Backmatic — centralized backup orchestrator.

pub mod app;
pub mod cli;
pub mod config;
pub mod error;
pub mod healthcheck;
pub mod inject;
pub mod mount;
pub mod retention;
pub mod runners;
pub mod scheduler;
pub mod shutdown;

pub use app::Backmatic;
pub use cli::CliArgs;
pub use error::{BackmaticError, Result};
