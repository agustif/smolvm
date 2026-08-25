//! GPU/display session support.
//!
//! The current production transport is RFB loopback. Keeping it under the
//! graphics namespace makes the transport boundary explicit without changing
//! the existing `--display` behavior.

mod session;
mod status;
pub mod transport;

pub use session::GraphicsSession;
pub use status::{
    DisplayTransportKind, GraphicsApi, GraphicsStatus, RendererKind, RendererQualification,
};
