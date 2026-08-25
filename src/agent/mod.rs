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

use crate::config::GraphicsRendererIntent;

/// Agent VM name.
pub const AGENT_VM_NAME: &str = "smolvm-agent";

/// libkrun private flag: 2D scanout only, no Venus/VirGL capsets.
const SMOLVM_GPU_2D_DISPLAY: u32 = 1 << 31;

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
///   bit 31 — SMOLVM_GPU_2D_DISPLAY         (both): software scanout only. libkrun
///             then advertises zero 3D capsets. Native display used to set this
///             whenever a display socket existed, which made Venus impossible.
fn gpu_virgl_flags(software_only: bool) -> u32 {
    if software_only {
        return SMOLVM_GPU_2D_DISPLAY;
    }
    #[cfg(target_os = "linux")]
    {
        (1 << 0) | (1 << 3) | (1 << 6) | (1 << 9) | (1 << 10)
    }
    #[cfg(not(target_os = "linux"))]
    {
        (1 << 6) | (1 << 7) | (1 << 10)
    }
}

fn gpu_virgl_flags_for_renderer(renderer: GraphicsRendererIntent) -> u32 {
    gpu_virgl_flags(renderer.software_scanout_only())
}

#[cfg(test)]
mod tests {
    use super::{gpu_virgl_flags, gpu_virgl_flags_for_renderer, SMOLVM_GPU_2D_DISPLAY};
    use crate::config::GraphicsRendererIntent;

    #[cfg(target_os = "linux")]
    const VIRGLRENDERER_USE_EGL: u32 = 1 << 0;
    #[cfg(target_os = "linux")]
    const VIRGLRENDERER_USE_SURFACELESS: u32 = 1 << 3;
    const VIRGLRENDERER_VENUS: u32 = 1 << 6;
    const VIRGLRENDERER_NO_VIRGL: u32 = 1 << 7;
    #[cfg(target_os = "linux")]
    const VIRGLRENDERER_RENDER_SERVER: u32 = 1 << 9;
    const VIRGLRENDERER_DRM: u32 = 1 << 10;

    #[test]
    #[cfg(target_os = "linux")]
    fn gpu_flags_enable_linux_venus_render_server_and_drm() {
        let flags = gpu_virgl_flags(false);
        assert_ne!(flags & VIRGLRENDERER_USE_EGL, 0);
        assert_ne!(flags & VIRGLRENDERER_USE_SURFACELESS, 0);
        assert_ne!(flags & VIRGLRENDERER_VENUS, 0);
        assert_ne!(flags & VIRGLRENDERER_RENDER_SERVER, 0);
        assert_ne!(flags & VIRGLRENDERER_DRM, 0);
        assert_eq!(flags & SMOLVM_GPU_2D_DISPLAY, 0);
    }

    #[test]
    #[cfg(not(target_os = "linux"))]
    fn gpu_flags_enable_macos_venus_no_virgl_and_drm() {
        let flags = gpu_virgl_flags(false);
        assert_ne!(flags & VIRGLRENDERER_VENUS, 0);
        assert_ne!(flags & VIRGLRENDERER_NO_VIRGL, 0);
        assert_ne!(flags & VIRGLRENDERER_DRM, 0);
        assert_eq!(flags & SMOLVM_GPU_2D_DISPLAY, 0);
    }

    #[test]
    fn gpu_flags_software_scanout_does_not_advertise_3d_capsets() {
        let flags = gpu_virgl_flags(true);
        assert_eq!(flags, SMOLVM_GPU_2D_DISPLAY);
        assert_eq!(flags & VIRGLRENDERER_VENUS, 0);
        assert_eq!(flags & VIRGLRENDERER_DRM, 0);
    }

    #[test]
    fn native_display_with_auto_or_venus_keeps_3d_capsets() {
        for renderer in [
            GraphicsRendererIntent::Auto,
            GraphicsRendererIntent::Venus,
            GraphicsRendererIntent::Virgl,
            GraphicsRendererIntent::NativeContext,
        ] {
            let flags = gpu_virgl_flags_for_renderer(renderer);
            assert_eq!(
                flags & SMOLVM_GPU_2D_DISPLAY,
                0,
                "{renderer} must not force software 2D scanout"
            );
            assert_ne!(flags & VIRGLRENDERER_VENUS, 0);
            assert_ne!(flags & VIRGLRENDERER_DRM, 0);
        }
        assert_eq!(
            gpu_virgl_flags_for_renderer(GraphicsRendererIntent::Software),
            SMOLVM_GPU_2D_DISPLAY
        );
    }
}
