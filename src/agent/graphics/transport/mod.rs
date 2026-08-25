//! Display presentation transports.

pub mod local_shm;
pub mod rfb;

use super::DisplayTransportKind;

/// Presentation transport owned by a graphics session.
///
/// A transport publishes frames to a local client protocol and provides the
/// libkrun callback vtables for display and input. RFB is the first
/// implementation; later transports such as local-shm should implement this
/// trait without changing launcher or CLI display lifecycle code.
pub trait DisplayTransport {
    /// Transport kind surfaced through non-secret status.
    fn kind(&self) -> DisplayTransportKind;

    /// Builds the libkrun display callback vtable.
    fn display_backend(&self) -> krun_display::DisplayBackend<'_>;

    /// Builds the virtual keyboard capability vtable.
    fn keyboard_config(&self) -> krun_input::InputConfigBackend<'_>;

    /// Builds the virtual keyboard event-provider vtable.
    fn keyboard_events(&self) -> krun_input::InputEventProviderBackend<'_>;

    /// Builds the absolute pointer capability vtable.
    fn pointer_config(&self) -> krun_input::InputConfigBackend<'_>;

    /// Builds the absolute pointer event-provider vtable.
    fn pointer_events(&self) -> krun_input::InputEventProviderBackend<'_>;
}
