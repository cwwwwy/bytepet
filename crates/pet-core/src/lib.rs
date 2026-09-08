//! Core logic for the desktop pet.
//!
//! This crate is deliberately free of any UI / Tauri dependency so that it can be
//! unit tested quickly and reused by the CLI (`pet`) and the desktop app.

pub mod agent;
pub mod chat;
pub mod config;
pub mod error;
pub mod llm;
pub mod memory;
pub mod persona;
pub mod pet;
pub mod secrets;

pub use error::{Error, Result};
