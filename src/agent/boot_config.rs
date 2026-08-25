//! Boot configuration for subprocess-based VM launch.
//!
//! On macOS, `fork()` in a multi-threaded process (e.g., the tokio-based API
//! server) creates unstable children because Apple frameworks like
//! Hypervisor.framework detect the forked state and abort. To avoid this,
//! the server spawns a fresh single-threaded `smolvm _boot-vm` subprocess
//! that safely runs `krun_start_enter`.
//!
//! This module defines the serializable config passed to that subprocess.

use crate::config::GraphicsTransportIntent;
use crate::data::disk::DiskFormat;
use crate::data::network::PortMapping;
use crate::data::resources::VmResources;
use crate::data::storage::HostMount;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Configuration for the `_boot-vm` subprocess.
///
/// Written to a temp file by the parent and read by the child.
#[derive(Debug, Serialize, Deserialize)]
pub struct BootConfig {
    /// Path to the agent rootfs directory.
    pub rootfs_path: PathBuf,
    /// Path to the storage disk file.
    pub storage_disk_path: PathBuf,
    /// Path to the overlay disk file.
    pub overlay_disk_path: PathBuf,
    /// Path to the vsock Unix socket.
    pub vsock_socket: PathBuf,
    /// Optional path to console log file.
    pub console_log: Option<PathBuf>,
    /// Path to write startup errors.
    pub startup_error_log: PathBuf,
    /// Storage disk size in GiB.
    pub storage_size_gb: u64,
    /// Overlay disk size in GiB.
    pub overlay_size_gb: u64,
    /// Host directory mounts.
    pub mounts: Vec<HostMount>,
    /// Port mappings.
    pub ports: Vec<PortMapping>,
    /// VM resources (CPU, memory, network, disk sizes).
    pub resources: VmResources,
    /// Path to the host-side Unix socket for SSH agent forwarding.
    /// When set, a vsock port is registered so the guest can reach the host's SSH agent.
    #[serde(default)]
    pub ssh_agent_socket: Option<PathBuf>,
    /// Hostnames for DNS filtering. When set, the host starts a DNS filter
    /// listener and the guest agent proxies DNS queries through it.
    #[serde(default)]
    pub dns_filter_hosts: Option<Vec<String>>,
    /// Pre-extracted OCI layers directory for .smolmachine-sourced machines.
    #[serde(default)]
    pub packed_layers_dir: Option<PathBuf>,
    /// Additional disk images to attach (path, read_only, format). The format
    /// lets the `pack --from-vm` exporter attach a source qcow2 disk read-only.
    #[serde(default)]
    pub extra_disks: Vec<(PathBuf, bool, DiskFormat)>,
    /// Optional mode-0600 rendezvous socket for a native display launch.
    #[serde(default)]
    pub display_socket: Option<PathBuf>,
    /// Display transport requested for this native display launch.
    #[serde(default)]
    pub display_transport: GraphicsTransportIntent,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::resources::VmResources;

    #[test]
    fn missing_display_transport_defaults_to_rfb_for_existing_boot_configs() {
        let config = BootConfig {
            rootfs_path: "/tmp/rootfs".into(),
            storage_disk_path: "/tmp/storage.raw".into(),
            overlay_disk_path: "/tmp/overlay.raw".into(),
            vsock_socket: "/tmp/vsock.sock".into(),
            console_log: None,
            startup_error_log: "/tmp/startup.log".into(),
            storage_size_gb: 10,
            overlay_size_gb: 10,
            mounts: Vec::new(),
            ports: Vec::new(),
            resources: VmResources::default(),
            ssh_agent_socket: None,
            dns_filter_hosts: None,
            packed_layers_dir: None,
            extra_disks: Vec::new(),
            display_socket: Some("/tmp/display.sock".into()),
            display_transport: GraphicsTransportIntent::LocalShm,
        };
        let mut value = serde_json::to_value(config).expect("boot config json");
        value
            .as_object_mut()
            .expect("boot config object")
            .remove("display_transport");

        let decoded: BootConfig = serde_json::from_value(value).expect("legacy boot config");
        assert_eq!(decoded.display_transport, GraphicsTransportIntent::Rfb);
    }
}
