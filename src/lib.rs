//! <!-- This crate's documentation is its README, so the two cannot disagree.
//! The examples below are compiled and run by `cargo test` wherever they
//! render: on docs.rs, on crates.io, and in the repository. -->
#![doc = include_str!("../README.md")]
//
// Author: David M. Anderson
// Built with AI assistance (Claude, Anthropic)
#![forbid(unsafe_code)]
#![warn(missing_docs, clippy::pedantic)]
#![allow(clippy::missing_errors_doc, clippy::module_name_repetitions)]

pub mod ast;
mod error;
mod eval;
mod exec;
mod lexer;
mod notice;
mod parser;
pub mod render;
mod scan;
mod value;

pub use error::{Error, Result};
pub use exec::{execute, execute_with, Cell, Options, Results, Row};
pub use notice::{Skipped, Tally};
pub use parser::{parse, parse_with};
pub use value::{Kind, Value};

/// The container library this crate reads through, re-exported.
///
/// [`Limits`](slpc::Limits) appears in [`Options`], and the TOML types under
/// [`slpc::toml_edit`] appear in [`Value`], so a caller needs the same
/// version of both.
pub use slpc;
