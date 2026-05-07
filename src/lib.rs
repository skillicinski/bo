//! # bo — engine library
//!
//! This crate provides the core engine for `bo`, a tool that fetches web pages
//! and stores them as local markdown files.
//!
//! ## Public API
//!
//! The primary entry point for any consumer (CLI, web server, etc.) is the
//! [`pipeline`] module:
//!
//! ```ignore
//! // Full pipeline including network fetch:
//! bo::pipeline::collect_url(url, output_dir)
//!
//! // Extract-write-ledger pipeline with pre-fetched HTML (useful for testing):
//! bo::pipeline::collect_html(url, html, output_dir)
//! ```
//!
//! ## Module dependency direction
//!
//! Dependencies flow inward. The CLI layer (`main.rs`) calls into `pipeline`
//! and `compile`; those modules orchestrate the engine modules below them.
//!
//! ```text
//! main.rs  (CLI: arg parsing, output, dispatch)
//!   ├── config      (read directly by main)
//!   ├── collect     (bo collect — orchestrates fetch → extract → leaf → index)
//!   │     ├── fetch
//!   │     ├── extract
//!   │     ├── leaf
//!   │     ├── slug
//!   │     └── index
//!   └── compile     (bo compile — agent loop + tools)
//!         ├── agent
//!         ├── branch
//!         ├── leaf
//!         ├── frontmatter
//!         ├── slug
//!         └── index
//! ```

mod adapters;
pub mod agent;
pub mod branch;
pub mod collect;
pub mod compile;
pub mod config;
pub mod extract;
pub mod fetch;
pub mod frontmatter;
pub mod index;
pub mod leaf;
mod quality;
pub use quality::RejectReason;
pub mod slug;
pub mod tree;
