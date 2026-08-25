//! Graphics session lifecycle.

use std::path::Path;

use crate::config::GraphicsTransportIntent;

use super::transport::{local_shm::LocalShmBridge, rfb::DisplayBridge, DisplayTransport};
use super::DisplayTransportKind;

/// Owns one VM graphics session and its presentation transport.
///
/// The session is intentionally transport-agnostic: launcher code configures
/// libkrun from this type, while RFB/local-shm details live below the transport
/// boundary.
pub struct GraphicsSession {
    transport: Box<dyn DisplayTransport>,
}

impl GraphicsSession {
    /// Starts the requested presentation transport.
    pub fn start(
        transport: GraphicsTransportIntent,
        endpoint_socket: &Path,
    ) -> Result<Box<Self>, String> {
        match transport {
            GraphicsTransportIntent::Rfb => Self::start_rfb(endpoint_socket),
            GraphicsTransportIntent::LocalShm => Self::start_local_shm(endpoint_socket),
        }
    }

    /// Starts the current production RFB transport.
    pub fn start_rfb(endpoint_socket: &Path) -> Result<Box<Self>, String> {
        Ok(Box::new(Self {
            transport: DisplayBridge::start(endpoint_socket)?,
        }))
    }

    /// Starts the local shared-memory transport.
    pub fn start_local_shm(endpoint_socket: &Path) -> Result<Box<Self>, String> {
        Ok(Box::new(Self {
            transport: LocalShmBridge::start(endpoint_socket)?,
        }))
    }

    /// Returns the active presentation transport kind.
    pub fn transport_kind(&self) -> DisplayTransportKind {
        self.transport.kind()
    }

    /// Builds the libkrun display callback vtable.
    pub fn display_backend(&self) -> krun_display::DisplayBackend<'_> {
        self.transport.display_backend()
    }

    /// Builds the virtual keyboard capability vtable.
    pub fn keyboard_config(&self) -> krun_input::InputConfigBackend<'_> {
        self.transport.keyboard_config()
    }

    /// Builds the virtual keyboard event-provider vtable.
    pub fn keyboard_events(&self) -> krun_input::InputEventProviderBackend<'_> {
        self.transport.keyboard_events()
    }

    /// Builds the absolute pointer capability vtable.
    pub fn pointer_config(&self) -> krun_input::InputConfigBackend<'_> {
        self.transport.pointer_config()
    }

    /// Builds the absolute pointer event-provider vtable.
    pub fn pointer_events(&self) -> krun_input::InputEventProviderBackend<'_> {
        self.transport.pointer_events()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MarkerTransport;

    impl DisplayTransport for MarkerTransport {
        fn kind(&self) -> DisplayTransportKind {
            DisplayTransportKind::Rfb
        }

        fn display_backend(&self) -> krun_display::DisplayBackend<'_> {
            panic!("not used by this session routing test")
        }

        fn keyboard_config(&self) -> krun_input::InputConfigBackend<'_> {
            panic!("not used by this session routing test")
        }

        fn keyboard_events(&self) -> krun_input::InputEventProviderBackend<'_> {
            panic!("not used by this session routing test")
        }

        fn pointer_config(&self) -> krun_input::InputConfigBackend<'_> {
            panic!("not used by this session routing test")
        }

        fn pointer_events(&self) -> krun_input::InputEventProviderBackend<'_> {
            panic!("not used by this session routing test")
        }
    }

    #[test]
    fn graphics_session_reports_transport_kind_through_trait_boundary() {
        let session = GraphicsSession {
            transport: Box::new(MarkerTransport),
        };
        assert_eq!(session.transport_kind(), DisplayTransportKind::Rfb);
    }
}
