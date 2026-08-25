//! Compatibility exports for the native display bridge.
//!
//! New display work should use `agent::graphics`; this module preserves the
//! existing `agent::display::*` API for CLI and launcher call sites.

pub use super::graphics::transport::rfb::{
    endpoint_is_ready, endpoint_socket, read_endpoint, read_endpoint_json, DisplayBridge,
    DisplayEndpoint, DISPLAY_ENV, DISPLAY_HEIGHT, DISPLAY_WIDTH,
};
pub use super::graphics::GraphicsSession;
