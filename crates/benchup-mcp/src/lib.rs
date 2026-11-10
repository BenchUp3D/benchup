//! MCP server for BenchUp: stdio JSON-RPC to the agent, backed either by a
//! project file (`file` mode) or by the running app over a loopback WebSocket
//! (`live` mode).
pub mod file_mode;
pub mod live;
pub mod protocol;

pub use protocol::{Backend, serve};
