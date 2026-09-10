//! Lightweight BytePet core.
//!
//! This crate intentionally contains no UI framework and no Tauri dependency.
//! It owns the Codex pet format, the animation state machine, personas,
//! lightweight JSON memory and the single DeepSeek API client used by the
//! rebuilt desktop application.

pub mod config;
pub mod deepseek;
pub mod error;
pub mod memory;
pub mod persona;
pub mod pet;
pub mod secrets;

pub use error::{Error, Result};
