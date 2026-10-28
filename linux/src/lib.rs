//! The platform-independent half of Snowhush, shared with the Windows build.
//!
//! The simulation, the snow layers and the geometry are the same code the Windows app runs;
//! only the windowing, drawing and tray code in this crate is new.

#[path = "../../windows/src/geometry.rs"]
pub mod geometry;
#[path = "../../windows/src/layer.rs"]
pub mod layer;
#[path = "../../windows/src/simulation.rs"]
pub mod simulation;
#[path = "../../windows/src/version.rs"]
pub mod version;

#[cfg(test)]
#[path = "../../windows/src/tests.rs"]
mod tests;
