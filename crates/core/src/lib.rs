//! Shared types and configuration for the Keeprun agent, CLI and desktop UI.

pub mod config;
pub mod detect;
pub mod model;
pub mod token;

pub use config::{Env, Settings};
