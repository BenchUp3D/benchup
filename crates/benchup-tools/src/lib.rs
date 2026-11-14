//! The operation layer shared by the in-app assistant and the MCP server.
//!
//! * [`ops`] defines the tools an agent may call and executes them against a
//!   [`benchup_core::Model`]. Lengths are millimetres at the boundary and
//!   metres inside the document; conversion happens in one place. Every
//!   failure (bad arguments, unknown ids, limits, read-only or lease
//!   violations) is returned as structured data so the caller can correct
//!   itself, and a failed call never changes the model.
//! * [`scene`] describes the model compactly (summary, references, per-object
//!   detail) for an agent to read.
//! * [`files`] reads and writes `.bup` project files (gzip-compressed JSON).
//!
//! The crate has no network or UI dependencies and builds for `wasm32`.
pub mod files;
pub mod ops;
pub mod review;
pub mod scene;

/// Conventions, workflow and recipes for agents (`get_guide`, `benchup guide`).
pub const GUIDE: &str = include_str!("../../../docs/agent-guide.md");

pub use ops::{
    Call, ToolDefinition, ToolEnv, ToolOutcome, definitions, execute, is_known, is_mutating, mcp_tools,
    openai_tools,
};
