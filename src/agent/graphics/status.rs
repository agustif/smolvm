//! Non-secret graphics session status.

use crate::config::{GraphicsRendererIntent, GraphicsTransportIntent, RecordState, VmRecord};
use serde::{Deserialize, Serialize};
use smolvm_protocol::GraphicsProbeStatus;

/// Known graphics API intent for a machine.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphicsApi {
    /// No accelerated graphics API is requested.
    None,
    /// OpenGL is requested through the VirGL path.
    OpenGl,
    /// Vulkan is requested through the virtio-gpu/Venus path.
    Vulkan,
    /// A native-context renderer was requested but API proof is not available.
    Unknown,
}

/// Renderer capability currently known from the host-side record.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RendererKind {
    /// Software rendering is expected because no GPU path was requested.
    Software,
    /// A virtio-gpu renderer was requested, but the host/guest renderer proof is not available yet.
    VirtioGpuRequested,
    /// A VirGL renderer was requested, but host/guest OpenGL proof is not available yet.
    VirglRequested,
    /// A Venus renderer was requested, but host/guest Vulkan proof is not available yet.
    VenusRequested,
    /// A native-context renderer was requested, but host/guest proof is not available yet.
    NativeContextRequested,
}

/// Runtime qualification of the requested renderer intent.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RendererQualification {
    /// No accelerated renderer was requested.
    NotRequested,
    /// A renderer was requested, but no concrete runtime API proof is available.
    Unproven,
    /// The runtime probe proved an API compatible with the requested renderer.
    Matched,
    /// The runtime probe returned a lower capability than requested.
    Degraded,
}

impl From<GraphicsRendererIntent> for RendererKind {
    fn from(value: GraphicsRendererIntent) -> Self {
        match value {
            GraphicsRendererIntent::Auto => RendererKind::VirtioGpuRequested,
            GraphicsRendererIntent::Software => RendererKind::Software,
            GraphicsRendererIntent::Virgl => RendererKind::VirglRequested,
            GraphicsRendererIntent::Venus => RendererKind::VenusRequested,
            GraphicsRendererIntent::NativeContext => RendererKind::NativeContextRequested,
        }
    }
}

impl From<GraphicsRendererIntent> for GraphicsApi {
    fn from(value: GraphicsRendererIntent) -> Self {
        match value {
            GraphicsRendererIntent::Auto | GraphicsRendererIntent::Venus => GraphicsApi::Vulkan,
            GraphicsRendererIntent::Virgl => GraphicsApi::OpenGl,
            GraphicsRendererIntent::Software => GraphicsApi::None,
            GraphicsRendererIntent::NativeContext => GraphicsApi::Unknown,
        }
    }
}

/// Display transport exposed to local clients.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisplayTransportKind {
    /// RFB/VNC loopback transport.
    #[serde(rename = "rfb")]
    Rfb,
    /// Same-user local shared-memory transport.
    #[serde(rename = "local-shm")]
    LocalShm,
}

impl From<GraphicsTransportIntent> for DisplayTransportKind {
    fn from(value: GraphicsTransportIntent) -> Self {
        match value {
            GraphicsTransportIntent::Rfb => DisplayTransportKind::Rfb,
            GraphicsTransportIntent::LocalShm => DisplayTransportKind::LocalShm,
        }
    }
}

impl DisplayTransportKind {
    /// Whether this transport has a launchable host implementation.
    pub const fn is_supported(&self) -> bool {
        match self {
            DisplayTransportKind::Rfb => true,
            DisplayTransportKind::LocalShm => true,
        }
    }
}

/// Non-secret graphics session lifecycle state.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphicsSessionState {
    /// Machine is not currently running.
    Stopped,
    /// Machine is running without a graphics/display session request.
    Headless,
    /// Machine is running with a display endpoint ready for client attach.
    Ready,
    /// Machine is running with graphics/display intent but needs a restart to expose an endpoint.
    RestartRequired,
}

/// Non-secret status suitable for inventory, diagnostics, and UI state.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GraphicsStatus {
    /// Wire schema identifier for compatibility checks.
    pub schema: String,
    /// Machine name.
    pub machine: String,
    /// Resolved lifecycle state.
    pub state: String,
    /// Derived graphics-session state for clients that should not infer from booleans.
    pub session_state: GraphicsSessionState,
    /// Whether display startup was requested and is known from current state.
    pub display_requested: bool,
    /// Whether the display endpoint is currently ready.
    pub display_ready: bool,
    /// Whether a local client can attach/reconnect without restarting the VM.
    pub client_attach_supported: bool,
    /// Whether a headless running VM can gain display/input without restart.
    pub hot_attach_supported: bool,
    /// Whether the current state requires restart to create a display endpoint.
    pub restart_required_for_display: bool,
    /// Whether GPU acceleration is configured for this machine.
    pub gpu_requested: bool,
    /// Whether GPU readiness is proven; `None` means not probed yet.
    pub gpu_ready: Option<bool>,
    /// Renderer policy requested in persistent machine configuration.
    pub renderer_requested: GraphicsRendererIntent,
    /// Renderer status known without guest probing.
    pub renderer: RendererKind,
    /// Best-effort renderer detail from a guest probe.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub renderer_detail: Option<String>,
    /// Whether runtime proof satisfies the requested renderer intent.
    pub renderer_qualification: RendererQualification,
    /// Non-secret explanation for renderer qualification decisions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub renderer_qualification_detail: Option<String>,
    /// Graphics API intent known from configuration.
    pub api_requested: GraphicsApi,
    /// Graphics API status known without guest probing.
    pub api: GraphicsApi,
    /// Best-effort graphics API detail from a guest probe.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_detail: Option<String>,
    /// Best-effort failure detail from the preferred graphics API probe.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_probe_failure_detail: Option<String>,
    /// Whether guest `/dev/dri` readiness is proven; `None` means not probed yet.
    pub guest_dri_ready: Option<bool>,
    /// Whether a guest DRM render node is present for accelerated API clients.
    pub guest_render_node_ready: Option<bool>,
    /// Whether input readiness is proven.
    pub input_ready: Option<bool>,
    /// Whether relative pointer transport is available for game-style mouse input.
    pub relative_pointer_supported: bool,
    /// Whether product-level pointer capture/release UX is available.
    pub pointer_capture_supported: bool,
    /// Whether gamepad transport is implemented.
    pub gamepad_supported: bool,
    /// Whether gamepad readiness is proven.
    pub gamepad_ready: Option<bool>,
    /// Whether guest audio transport is implemented.
    pub audio_supported: bool,
    /// Whether guest audio readiness is proven.
    pub audio_ready: Option<bool>,
    /// Whether seat readiness is proven; `None` means not probed yet.
    pub seat_ready: Option<bool>,
    /// Whether compositor readiness is proven; `None` means not probed yet.
    pub compositor_ready: Option<bool>,
    /// Current presentation transport.
    pub transport: DisplayTransportKind,
    /// Whether the requested presentation transport is implemented by this helper.
    pub transport_supported: bool,
    /// Whether the presentation transport is ready.
    pub transport_ready: Option<bool>,
}

impl GraphicsStatus {
    /// Build status from the persisted record and known local endpoint state.
    ///
    /// This intentionally avoids guest probing and endpoint secrets. Call
    /// [`Self::with_guest_probe`] to merge authenticated guest facts when the
    /// running agent is reachable.
    pub fn from_record(
        machine: &str,
        record: &VmRecord,
        actual_state: RecordState,
        display_ready: bool,
    ) -> Self {
        let renderer_requested = record.graphics.renderer.clone();
        let renderer_requests_gpu = renderer_requested.requests_gpu();
        let gpu_requested = record.gpu.unwrap_or(false) || renderer_requests_gpu;
        let running = actual_state == RecordState::Running;
        let display_requested = record.graphics.enabled || display_ready;
        let requested_transport = DisplayTransportKind::from(record.graphics.transport.clone());
        let transport_supported = requested_transport.is_supported();
        let session_state = match (running, display_requested, display_ready) {
            (false, _, _) => GraphicsSessionState::Stopped,
            (true, false, _) => GraphicsSessionState::Headless,
            (true, true, true) => GraphicsSessionState::Ready,
            (true, true, false) => GraphicsSessionState::RestartRequired,
        };
        let relative_pointer_supported =
            requested_transport == DisplayTransportKind::LocalShm && transport_supported;
        let gpu_ready = match (gpu_requested, running) {
            (false, _) => Some(false),
            (true, false) => Some(false),
            // The current host-side status cannot prove guest /dev/dri readiness.
            (true, true) => None,
        };
        let mut status = Self {
            schema: "smolvm-graphics-status-v1".to_string(),
            machine: machine.to_string(),
            state: actual_state.to_string(),
            session_state,
            display_requested,
            display_ready,
            client_attach_supported: display_ready,
            hot_attach_supported: false,
            restart_required_for_display: running && display_requested && !display_ready,
            gpu_requested,
            gpu_ready,
            renderer_requested: renderer_requested.clone(),
            renderer: if gpu_requested {
                RendererKind::from(renderer_requested.clone())
            } else {
                RendererKind::Software
            },
            renderer_detail: None,
            renderer_qualification: RendererQualification::Unproven,
            renderer_qualification_detail: None,
            api_requested: if gpu_requested {
                GraphicsApi::from(renderer_requested.clone())
            } else {
                GraphicsApi::None
            },
            api: if gpu_requested {
                GraphicsApi::from(renderer_requested)
            } else {
                GraphicsApi::None
            },
            api_detail: None,
            api_probe_failure_detail: None,
            guest_dri_ready: None,
            guest_render_node_ready: None,
            input_ready: Some(display_ready),
            relative_pointer_supported,
            pointer_capture_supported: false,
            gamepad_supported: false,
            gamepad_ready: Some(false),
            audio_supported: false,
            audio_ready: Some(false),
            seat_ready: None,
            compositor_ready: None,
            transport: requested_transport,
            transport_supported,
            transport_ready: Some(display_ready && transport_supported),
        };
        status.refresh_renderer_qualification();
        status
    }

    /// Merge authenticated guest probe facts into the non-secret status.
    pub fn with_guest_probe(mut self, probe: Option<&GraphicsProbeStatus>) -> Self {
        let Some(probe) = probe else {
            return self;
        };

        self.guest_dri_ready = Some(probe.dri_ready);
        self.guest_render_node_ready = Some(
            probe
                .dri_nodes
                .iter()
                .any(|node| node.starts_with("renderD")),
        );
        self.input_ready = Some(probe.input_ready);
        self.seat_ready = Some(probe.seat_ready);
        self.compositor_ready = Some(probe.compositor_ready);
        self.renderer_detail = probe.renderer_hint.clone();
        self.api_detail = probe.api_hint.clone();
        self.api_probe_failure_detail = probe.api_probe_failure_detail.clone();
        if let Some(api_hint) = probe.api_hint.as_deref() {
            if api_hint.starts_with("vulkan:") {
                self.api = GraphicsApi::Vulkan;
            } else if api_hint.starts_with("opengl:") {
                self.api = GraphicsApi::OpenGl;
            }
        }
        if self.gpu_requested {
            self.gpu_ready = Some(probe.dri_ready);
        }
        self.refresh_renderer_qualification();
        self
    }

    fn refresh_renderer_qualification(&mut self) {
        let software_renderer = self
            .api_detail
            .as_deref()
            .map(is_software_renderer_detail)
            .unwrap_or(false);

        let (qualification, detail) = if !self.gpu_requested {
            (
                RendererQualification::NotRequested,
                Some("accelerated renderer was not requested".to_string()),
            )
        } else if self.api_detail.is_none() {
            (
                RendererQualification::Unproven,
                Some("no runtime graphics API proof is available".to_string()),
            )
        } else if software_renderer {
            (
                RendererQualification::Degraded,
                Some("runtime probe observed a software renderer".to_string()),
            )
        } else {
            match self.renderer_requested {
                GraphicsRendererIntent::Software => (
                    RendererQualification::Matched,
                    Some("software renderer was requested".to_string()),
                ),
                GraphicsRendererIntent::Auto => (
                    RendererQualification::Matched,
                    Some("runtime probe observed a non-software graphics API".to_string()),
                ),
                GraphicsRendererIntent::Virgl => {
                    if self.api == GraphicsApi::OpenGl {
                        (
                            RendererQualification::Matched,
                            Some("runtime probe observed OpenGL for VirGL intent".to_string()),
                        )
                    } else {
                        (
                            RendererQualification::Degraded,
                            Some(
                                "runtime probe did not observe OpenGL for VirGL intent".to_string(),
                            ),
                        )
                    }
                }
                GraphicsRendererIntent::Venus => {
                    if self.api == GraphicsApi::Vulkan {
                        (
                            RendererQualification::Matched,
                            Some("runtime probe observed Vulkan for Venus intent".to_string()),
                        )
                    } else {
                        (
                            RendererQualification::Degraded,
                            Some(
                                "runtime probe did not observe Vulkan for Venus intent".to_string(),
                            ),
                        )
                    }
                }
                GraphicsRendererIntent::NativeContext => {
                    let native_context_observed = self
                        .renderer_detail
                        .as_deref()
                        .map(mentions_native_context)
                        .unwrap_or(false)
                        || self
                            .api_detail
                            .as_deref()
                            .map(mentions_native_context)
                            .unwrap_or(false);
                    if native_context_observed {
                        (
                            RendererQualification::Matched,
                            Some(
                                "runtime probe observed native-context renderer detail".to_string(),
                            ),
                        )
                    } else {
                        (
                            RendererQualification::Unproven,
                            Some(
                                "runtime probe did not prove native-context renderer detail"
                                    .to_string(),
                            ),
                        )
                    }
                }
            }
        };

        self.renderer_qualification = qualification;
        self.renderer_qualification_detail = detail;
    }
}

fn is_software_renderer_detail(detail: &str) -> bool {
    let lower = detail.to_ascii_lowercase();
    lower.contains("llvmpipe")
        || lower.contains("lavapipe")
        || lower.contains("softpipe")
        || lower.contains("swrast")
        || lower.contains("software rasterizer")
        || lower.contains("software renderer")
}

fn mentions_native_context(detail: &str) -> bool {
    let lower = detail.to_ascii_lowercase();
    lower.contains("native-context") || lower.contains("native context")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record_with_gpu(gpu: bool) -> VmRecord {
        let mut record = VmRecord::new("gui".to_string(), 2, 512, vec![], vec![], false);
        record.gpu = Some(gpu);
        record
    }

    fn record_with_graphics() -> VmRecord {
        let mut record = record_with_gpu(false);
        record.graphics.enabled = true;
        record
    }

    fn probe_with_api(api_hint: &str) -> GraphicsProbeStatus {
        GraphicsProbeStatus {
            dri_ready: true,
            dri_nodes: vec!["renderD128".to_string()],
            input_ready: true,
            input_nodes: vec!["event0".to_string()],
            seat_ready: true,
            seat_socket: Some("/run/seatd.sock".to_string()),
            compositor_ready: true,
            compositor_sockets: vec!["/run/user/0/wayland-0".to_string()],
            renderer_hint: Some("virtio-gpu".to_string()),
            api_hint: Some(api_hint.to_string()),
            api_probe_failure_detail: None,
        }
    }

    #[test]
    fn graphics_status_does_not_claim_guest_probe_results() {
        let status =
            GraphicsStatus::from_record("gui", &record_with_gpu(true), RecordState::Running, true);
        assert!(status.gpu_requested);
        assert_eq!(status.gpu_ready, None);
        assert_eq!(status.renderer_requested, GraphicsRendererIntent::Auto);
        assert_eq!(status.renderer, RendererKind::VirtioGpuRequested);
        assert_eq!(status.renderer_detail, None);
        assert_eq!(
            status.renderer_qualification,
            RendererQualification::Unproven
        );
        assert_eq!(status.api_requested, GraphicsApi::Vulkan);
        assert_eq!(status.api, GraphicsApi::Vulkan);
        assert_eq!(status.api_detail, None);
        assert_eq!(status.api_probe_failure_detail, None);
        assert_eq!(status.guest_dri_ready, None);
        assert_eq!(status.guest_render_node_ready, None);
        assert_eq!(status.seat_ready, None);
        assert_eq!(status.compositor_ready, None);
        assert_eq!(status.transport, DisplayTransportKind::Rfb);
        assert!(status.transport_supported);
        assert_eq!(status.transport_ready, Some(true));
        assert!(status.client_attach_supported);
        assert!(!status.hot_attach_supported);
        assert!(!status.restart_required_for_display);
    }

    #[test]
    fn graphics_status_reports_software_when_gpu_not_requested() {
        let status = GraphicsStatus::from_record(
            "plain",
            &record_with_gpu(false),
            RecordState::Stopped,
            false,
        );
        assert!(!status.gpu_requested);
        assert_eq!(status.gpu_ready, Some(false));
        assert_eq!(status.renderer_requested, GraphicsRendererIntent::Auto);
        assert_eq!(status.renderer, RendererKind::Software);
        assert_eq!(
            status.renderer_qualification,
            RendererQualification::NotRequested
        );
        assert_eq!(status.api_requested, GraphicsApi::None);
        assert_eq!(status.api, GraphicsApi::None);
        assert!(!status.display_ready);
        assert!(!status.client_attach_supported);
        assert!(!status.hot_attach_supported);
        assert!(!status.restart_required_for_display);
        assert_eq!(status.input_ready, Some(false));
    }

    #[test]
    fn graphics_status_serializes_transport_as_rfb() {
        let status =
            GraphicsStatus::from_record("gui", &record_with_gpu(true), RecordState::Running, true);
        let json = serde_json::to_value(status).expect("graphics status json");
        assert_eq!(json["schema"], "smolvm-graphics-status-v1");
        assert_eq!(json["session_state"], "ready");
        assert_eq!(json["renderer_requested"], "auto");
        assert_eq!(json["renderer"], "virtio_gpu_requested");
        assert_eq!(json["renderer_qualification"], "unproven");
        assert_eq!(json["api_requested"], "vulkan");
        assert_eq!(json["transport"], "rfb");
        assert_eq!(json["transport_supported"], true);
        assert!(json["gpu_ready"].is_null());
        assert_eq!(json["relative_pointer_supported"], false);
        assert_eq!(json["pointer_capture_supported"], false);
        assert_eq!(json["gamepad_supported"], false);
        assert_eq!(json["gamepad_ready"], false);
        assert_eq!(json["audio_supported"], false);
        assert_eq!(json["audio_ready"], false);
    }

    #[test]
    fn graphics_status_reports_requested_renderer_without_proven_api() {
        let mut record = record_with_graphics();
        record.graphics.renderer = GraphicsRendererIntent::Venus;
        let status = GraphicsStatus::from_record("gui", &record, RecordState::Running, true);
        let json = serde_json::to_value(status).expect("graphics status json");
        assert_eq!(json["gpu_requested"], true);
        assert!(json["gpu_ready"].is_null());
        assert_eq!(json["renderer_requested"], "venus");
        assert_eq!(json["renderer"], "venus_requested");
        assert_eq!(json["api_requested"], "vulkan");
        assert_eq!(json["api"], "vulkan");
        assert_eq!(json["renderer_qualification"], "unproven");
        assert!(json.get("renderer_detail").is_none());
        assert!(json.get("api_detail").is_none());
    }

    #[test]
    fn graphics_status_reports_virgl_and_native_context_requests() {
        let mut virgl = record_with_graphics();
        virgl.graphics.renderer = GraphicsRendererIntent::Virgl;
        let status = GraphicsStatus::from_record("gui", &virgl, RecordState::Running, true);
        assert!(status.gpu_requested);
        assert_eq!(status.renderer, RendererKind::VirglRequested);
        assert_eq!(status.api, GraphicsApi::OpenGl);

        let mut native = record_with_graphics();
        native.graphics.renderer = GraphicsRendererIntent::NativeContext;
        let status = GraphicsStatus::from_record("gui", &native, RecordState::Running, true);
        assert!(status.gpu_requested);
        assert_eq!(status.renderer, RendererKind::NativeContextRequested);
        assert_eq!(status.api, GraphicsApi::Unknown);
    }

    #[test]
    fn graphics_status_serializes_requested_local_shm_transport() {
        let mut record = record_with_graphics();
        record.graphics.transport = crate::config::GraphicsTransportIntent::LocalShm;
        let status = GraphicsStatus::from_record("gui", &record, RecordState::Stopped, false);
        let json = serde_json::to_value(status).expect("graphics status json");
        assert_eq!(json["transport"], "local-shm");
        assert_eq!(json["transport_supported"], true);
        assert_eq!(json["transport_ready"], false);
        assert_eq!(json["session_state"], "stopped");
        assert_eq!(json["relative_pointer_supported"], true);
    }

    #[test]
    fn graphics_status_does_not_require_restart_for_plain_running_headless_machine() {
        let status =
            GraphicsStatus::from_record("gui", &record_with_gpu(true), RecordState::Running, false);
        assert!(!status.display_ready);
        assert_eq!(status.session_state, GraphicsSessionState::Headless);
        assert!(!status.client_attach_supported);
        assert!(!status.hot_attach_supported);
        assert!(!status.restart_required_for_display);
    }

    #[test]
    fn graphics_status_reports_restart_required_for_graphics_intent_without_endpoint() {
        let status = GraphicsStatus::from_record(
            "gui",
            &record_with_graphics(),
            RecordState::Running,
            false,
        );
        assert!(status.display_requested);
        assert!(!status.display_ready);
        assert_eq!(status.session_state, GraphicsSessionState::RestartRequired);
        assert!(!status.client_attach_supported);
        assert!(!status.hot_attach_supported);
        assert!(status.restart_required_for_display);
    }

    #[test]
    fn graphics_status_merges_guest_probe_facts() {
        let probe = probe_with_api("opengl: renderer: llvmpipe");
        let status =
            GraphicsStatus::from_record("gui", &record_with_gpu(true), RecordState::Running, true)
                .with_guest_probe(Some(&probe));

        assert_eq!(status.gpu_ready, Some(true));
        assert_eq!(status.guest_dri_ready, Some(true));
        assert_eq!(status.guest_render_node_ready, Some(true));
        assert_eq!(status.input_ready, Some(true));
        assert_eq!(status.seat_ready, Some(true));
        assert_eq!(status.compositor_ready, Some(true));
        assert_eq!(status.renderer_detail.as_deref(), Some("virtio-gpu"));
        assert_eq!(status.api, GraphicsApi::OpenGl);
        assert_eq!(
            status.renderer_qualification,
            RendererQualification::Degraded
        );
        assert_eq!(
            status.api_detail.as_deref(),
            Some("opengl: renderer: llvmpipe")
        );
    }

    #[test]
    fn graphics_status_carries_api_probe_failure_detail() {
        let mut probe = probe_with_api("opengl: renderer: llvmpipe");
        probe.api_probe_failure_detail =
            Some("vulkan: vulkaninfo failed: vkCreateInstance failed".to_string());
        let status =
            GraphicsStatus::from_record("gui", &record_with_gpu(true), RecordState::Running, true)
                .with_guest_probe(Some(&probe));

        assert_eq!(
            status.api_probe_failure_detail.as_deref(),
            Some("vulkan: vulkaninfo failed: vkCreateInstance failed")
        );
    }

    #[test]
    fn graphics_status_degrades_venus_when_probe_observes_software_opengl() {
        let mut record = record_with_graphics();
        record.graphics.renderer = GraphicsRendererIntent::Venus;
        let mut probe = probe_with_api("opengl: Device: llvmpipe (LLVM 15.0.6, 128 bits)");
        probe.dri_nodes = vec!["card0".to_string()];
        let status = GraphicsStatus::from_record("gui", &record, RecordState::Running, true)
            .with_guest_probe(Some(&probe));

        assert_eq!(status.api_requested, GraphicsApi::Vulkan);
        assert_eq!(status.api, GraphicsApi::OpenGl);
        assert_eq!(status.guest_dri_ready, Some(true));
        assert_eq!(status.guest_render_node_ready, Some(false));
        assert_eq!(
            status.renderer_qualification,
            RendererQualification::Degraded
        );
        assert_eq!(
            status.renderer_qualification_detail.as_deref(),
            Some("runtime probe observed a software renderer")
        );
    }

    #[test]
    fn graphics_status_matches_venus_only_with_non_software_vulkan() {
        let mut record = record_with_graphics();
        record.graphics.renderer = GraphicsRendererIntent::Venus;
        let probe = probe_with_api("vulkan: GPU id = 0 (virtio-gpu Venus)");
        let status = GraphicsStatus::from_record("gui", &record, RecordState::Running, true)
            .with_guest_probe(Some(&probe));

        assert_eq!(status.api, GraphicsApi::Vulkan);
        assert_eq!(
            status.renderer_qualification,
            RendererQualification::Matched
        );
    }

    #[test]
    fn graphics_status_matches_virgl_only_with_non_software_opengl() {
        let mut record = record_with_graphics();
        record.graphics.renderer = GraphicsRendererIntent::Virgl;
        let probe = probe_with_api("opengl: OpenGL renderer string: virgl");
        let status = GraphicsStatus::from_record("gui", &record, RecordState::Running, true)
            .with_guest_probe(Some(&probe));

        assert_eq!(status.api, GraphicsApi::OpenGl);
        assert_eq!(
            status.renderer_qualification,
            RendererQualification::Matched
        );
    }
}
