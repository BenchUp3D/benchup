//! Geometry import/export shared by the desktop and browser applications.
//!
//! This crate depends on the model core, never on the application or renderer,
//! so UI edits can reuse parsers, exporters, and the embedded browser SKP reader.

pub mod interchange;
pub mod mesh_exchange;
mod skp_recovery;
