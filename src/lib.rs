//! # bo — engine library
//!
//! Layered architecture:
//!
//! ```text
//! main.rs → cli/ → engine/ → adapters/
//!                      ↓
//!                 domain/ (pure types, depends on nothing)
//! ```

pub mod adapters;
pub mod cli;
pub mod domain;
pub mod engine;
