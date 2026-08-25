//! Agent VM management.
//!
//! This module manages the agent VM lifecycle and provides a client
//! for communicating with the smolvm-agent via vsock.

pub mod boot_config;
mod client;
pub mod display;
pub mod graphics;
mod krun;
mod launcher;
pub mod launcher_dynamic;
mod manager;
pub mod state_probe;
pub mod terminal;

pub use crate::data::network::PortMapping;
pub use crate::data::resources::VmResources;
pub use crate::data::storage::HostMount;
pub use client::{
    AgentClient, ExecEvent, InteractiveInput, InteractiveOutput, PullOptions, RunConfig,
};
pub use krun::KrunFunctions;
pub use launcher::{
    create_disk_overlays, find_lib_dir, launch_agent_vm, DiskOverlaySpec, LaunchConfig,
    LaunchFeatures, VmDisks,
};
pub use manager::{
    disk_used_mb, docker_config_dir, docker_config_mount, ensure_vm_dir, machine_layers_cache_dir,
    read_egress_telemetry, resolve_disk_image, vm_cache_root, vm_data_dir, vm_dir_hash,
    AgentManager, AgentState,
};

/// Agent VM name.
pub const AGENT_VM_NAME: &str = "smolvm-agent";

/// Compute the `virgl_flags` bitmask for `krun_set_gpu_options2`.
///
/// Shared by both the static (`launcher.rs`) and dynamic (`launcher_dynamic.rs`)
/// launchers so they can never silently diverge.
///
/// Flag values from `libkrun/include/libkrun.h` virglrenderer bindings:
///   bit 0  — VIRGLRENDERER_USE_EGL         (Linux): EGL context for GPU rendering
///   bit 3  — VIRGLRENDERER_USE_SURFACELESS  (Linux): no display server required
///   bit 6  — VIRGLRENDERER_VENUS           (both): Vulkan-over-virtio-gpu (Venus ICD)
///   bit 7  — VIRGLRENDERER_NO_VIRGL        (macOS): skip OpenGL (vrend) init — without
///             EGL, vrend_renderer_init crashes on null platform function pointers
///   bit 9  — VIRGLRENDERER_RENDER_SERVER   (Linux): REQUIRED for render-server mode.
///             Enables virglrenderer to call the get_server_fd callback and use an
///             external render server.  Without this bit, virglrenderer attempts
///             in-process Venus which fails (version stays 0).  With get_server_fd
///             provided in the callbacks struct, virglrenderer uses the externally
///             spawned virgl_render_server instead of fork/exec-ing its own process.
///   bit 10 — VIRGLRENDERER_DRM             (both): DRM native context support,
///             required for guests to expose render nodes for accelerated clients.
fn gpu_virgl_flags(software_display: bool) -> u32 {
    #[cfg(target_os = "linux")]
    {
        (1 << 0)
            | (1 << 3)
            | (1 << 6)
            | (1 << 9)
            | (1 << 10)
            | if software_display { 1 << 31 } else { 0 }
    }
    #[cfg(not(target_os = "linux"))]
    {
        (1 << 6) | (1 << 7) | (1 << 10) | if software_display { 1 << 31 } else { 0 }
    }
}

#[cfg(test)]
mod tests {
    use super::gpu_virgl_flags;

    #[cfg(target_os = "linux")]
    const VIRGLRENDERER_USE_EGL: u32 = 1 << 0;
    #[cfg(target_os = "linux")]
    const VIRGLRENDERER_USE_SURFACELESS: u32 = 1 << 3;
    const VIRGLRENDERER_VENUS: u32 = 1 << 6;
    const VIRGLRENDERER_NO_VIRGL: u32 = 1 << 7;
    #[cfg(target_os = "linux")]
    const VIRGLRENDERER_RENDER_SERVER: u32 = 1 << 9;
    const VIRGLRENDERER_DRM: u32 = 1 << 10;
    const KRUN_DISPLAY_SOFTWARE_ONLY: u32 = 1 << 31;

    #[test]
    #[cfg(target_os = "linux")]
    fn gpu_flags_enable_linux_venus_render_server_and_drm() {
        let flags = gpu_virgl_flags(false);
        assert_ne!(flags & VIRGLRENDERER_USE_EGL, 0);
        assert_ne!(flags & VIRGLRENDERER_USE_SURFACELESS, 0);
        assert_ne!(flags & VIRGLRENDERER_VENUS, 0);
        assert_ne!(flags & VIRGLRENDERER_RENDER_SERVER, 0);
        assert_ne!(flags & VIRGLRENDERER_DRM, 0);
        assert_eq!(flags & KRUN_DISPLAY_SOFTWARE_ONLY, 0);
    }

    #[test]
    #[cfg(not(target_os = "linux"))]
    fn gpu_flags_enable_macos_venus_no_virgl_and_drm() {
        let flags = gpu_virgl_flags(false);
        assert_ne!(flags & VIRGLRENDERER_VENUS, 0);
        assert_ne!(flags & VIRGLRENDERER_NO_VIRGL, 0);
        assert_ne!(flags & VIRGLRENDERER_DRM, 0);
        assert_eq!(flags & KRUN_DISPLAY_SOFTWARE_ONLY, 0);
    }

    #[test]
    fn gpu_flags_include_software_display_marker_when_requested() {
        let flags = gpu_virgl_flags(true);
        assert_ne!(flags & KRUN_DISPLAY_SOFTWARE_ONLY, 0);
        assert_ne!(flags & VIRGLRENDERER_DRM, 0);
    }
}
