//! Local shared-memory transport protocol types.
//!
//! This module owns the first versioned local-shm runtime contract. It exposes
//! a same-user endpoint socket, a same-user control socket, and a file-backed
//! frame ring that later clients can mmap or read directly.

use crossbeam_channel::{bounded, Receiver, Sender, TryRecvError};
use krun_display::{
    DisplayBackendBasicFramebuffer, DisplayBackendError, DisplayBackendNew, IntoDisplayBackend,
    Rect, ResourceFormat,
};
use krun_input::{
    InputAbsInfo, InputBackendError, InputDeviceIds, InputEvent, InputEventType, InputEventsImpl,
    InputQueryConfig, IntoInputConfig, IntoInputEvents, ObjectNew,
};
use krun_utils::pollable_channel::{
    pollable_channel, PollableChannelReciever, PollableChannelSender,
};
use serde::{Deserialize, Serialize};
use std::array;
use std::collections::HashSet;
use std::io::Write;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Instant;
use tokio::io::{AsyncBufReadExt, AsyncSeekExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;

use super::DisplayTransport;
use crate::agent::graphics::DisplayTransportKind;

/// Default native scanout width.
pub const DISPLAY_WIDTH: u32 = 1_440;
/// Default native scanout height.
pub const DISPLAY_HEIGHT: u32 = 900;

/// Protocol name returned to clients for local-shm display endpoints.
pub const PROTOCOL: &str = "local-shm";
/// Local-shm wire protocol version.
pub const PROTOCOL_VERSION: u32 = 1;
/// Control socket file name inside the display runtime directory.
pub const CONTROL_SOCKET_NAME: &str = "local-shm.sock";
/// Shared frame ring file name inside the display runtime directory.
pub const FRAME_RING_NAME: &str = "local-shm.frames";
/// Maximum accepted framebuffer dimension.
pub const MAX_DIMENSION: u32 = 16_384;
/// Maximum accepted frame payload bytes.
pub const MAX_FRAME_BYTES: u64 = 512 * 1024 * 1024;
/// Fixed frame-ring file header size.
pub const FRAME_RING_HEADER_BYTES: u64 = 256;
/// Fixed per-slot frame metadata size.
pub const FRAME_SLOT_HEADER_BYTES: u64 = 128;
/// Number of frame slots in the initial local-shm ring.
pub const FRAME_RING_SLOTS: u32 = 3;
/// Maximum payload bytes per ring slot.
pub const FRAME_PAYLOAD_CAPACITY: u64 =
    DISPLAY_WIDTH as u64 * DISPLAY_HEIGHT as u64 * PixelFormat::Rgba8888.bytes_per_pixel() as u64;
/// Full byte stride of each ring slot.
pub const FRAME_SLOT_STRIDE: u64 = FRAME_SLOT_HEADER_BYTES + FRAME_PAYLOAD_CAPACITY;
/// Total byte size of the frame-ring file.
pub const FRAME_RING_BYTES: u64 =
    FRAME_RING_HEADER_BYTES + FRAME_SLOT_STRIDE * FRAME_RING_SLOTS as u64;
const DISPLAY_BUFFERS: usize = 3;
const FRAME_QUEUE_DEPTH: usize = 2;

const FRAME_RING_MAGIC: &[u8; 8] = b"LSHMRG1\0";
const FRAME_SLOT_MAGIC: &[u8; 8] = b"LSHMSL1\0";
const PIXEL_FORMAT_RGBA8888: u32 = 1;

const EV_SYN: u16 = 0;
const EV_KEY: u16 = 1;
const EV_REL: u16 = 2;
const EV_ABS: u16 = 3;
const SYN_REPORT: u16 = 0;
const REL_X: u16 = 0;
const REL_Y: u16 = 1;
const REL_WHEEL: u16 = 8;
const ABS_X: u16 = 0;
const ABS_Y: u16 = 1;
const BTN_LEFT: u16 = 272;
const BTN_RIGHT: u16 = 273;
const BTN_MIDDLE: u16 = 274;
const BUS_VIRTUAL: u16 = 0x06;
const INPUT_PROP_POINTER: u16 = 0;

/// Endpoint metadata for a same-user local-shm client.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LocalShmEndpoint {
    /// Always [`PROTOCOL`].
    pub protocol: String,
    /// Wire protocol version.
    pub version: u32,
    /// Same-user Unix control socket.
    pub control_socket: PathBuf,
    /// Mmap-capable frame ring file.
    pub frame_ring: PathBuf,
}

impl LocalShmEndpoint {
    /// Build endpoint paths inside a display runtime directory.
    pub fn for_runtime_dir(runtime_dir: &Path) -> Self {
        Self {
            protocol: PROTOCOL.to_string(),
            version: PROTOCOL_VERSION,
            control_socket: runtime_dir.join(CONTROL_SOCKET_NAME),
            frame_ring: runtime_dir.join(FRAME_RING_NAME),
        }
    }

    /// Validate endpoint metadata without touching the filesystem.
    pub fn validate_for_runtime_dir(&self, runtime_dir: &Path) -> Result<(), String> {
        if self.protocol != PROTOCOL {
            return Err(format!(
                "unsupported local-shm protocol '{}'",
                self.protocol
            ));
        }
        if self.version != PROTOCOL_VERSION {
            return Err(format!(
                "unsupported local-shm version {}; expected {}",
                self.version, PROTOCOL_VERSION
            ));
        }
        if self.control_socket != runtime_dir.join(CONTROL_SOCKET_NAME) {
            return Err("control socket is outside the display runtime directory".to_string());
        }
        if self.frame_ring != runtime_dir.join(FRAME_RING_NAME) {
            return Err("frame ring is outside the display runtime directory".to_string());
        }
        Ok(())
    }
}

/// Pixel formats supported by the initial local-shm frame ring.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PixelFormat {
    /// Four bytes per pixel in BGRA byte order.
    Bgra8888,
    /// Four bytes per pixel in RGBA byte order.
    Rgba8888,
}

impl PixelFormat {
    /// Number of bytes per pixel.
    pub const fn bytes_per_pixel(self) -> u32 {
        match self {
            PixelFormat::Bgra8888 | PixelFormat::Rgba8888 => 4,
        }
    }
}

impl PixelFormat {
    fn ring_code(self) -> u32 {
        match self {
            PixelFormat::Rgba8888 => PIXEL_FORMAT_RGBA8888,
            PixelFormat::Bgra8888 => 2,
        }
    }
}

/// Metadata for one frame in the local-shm ring.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FrameHeader {
    /// Wire protocol version.
    pub version: u32,
    /// Monotonic frame identifier.
    pub frame_id: u64,
    /// Framebuffer width in pixels.
    pub width: u32,
    /// Framebuffer height in pixels.
    pub height: u32,
    /// Bytes between adjacent rows.
    pub stride: u32,
    /// Pixel format for the payload.
    pub format: PixelFormat,
    /// Damage rectangle x offset.
    pub damage_x: u32,
    /// Damage rectangle y offset.
    pub damage_y: u32,
    /// Damage rectangle width.
    pub damage_width: u32,
    /// Damage rectangle height.
    pub damage_height: u32,
    /// Host monotonic timestamp in nanoseconds.
    pub monotonic_ns: u64,
}

impl FrameHeader {
    /// Validate bounds and return the expected full-frame payload length.
    pub fn validate(&self) -> Result<u64, String> {
        if self.version != PROTOCOL_VERSION {
            return Err(format!(
                "unsupported frame version {}; expected {}",
                self.version, PROTOCOL_VERSION
            ));
        }
        if self.width == 0 || self.height == 0 {
            return Err("frame dimensions must be nonzero".to_string());
        }
        if self.width > MAX_DIMENSION || self.height > MAX_DIMENSION {
            return Err(format!("frame dimensions exceed {} pixels", MAX_DIMENSION));
        }
        let minimum_stride = u64::from(self.width) * u64::from(self.format.bytes_per_pixel());
        if u64::from(self.stride) < minimum_stride {
            return Err("frame stride is smaller than one pixel row".to_string());
        }
        let frame_len = u64::from(self.stride)
            .checked_mul(u64::from(self.height))
            .ok_or_else(|| "frame length overflow".to_string())?;
        if frame_len > MAX_FRAME_BYTES {
            return Err(format!("frame exceeds {} bytes", MAX_FRAME_BYTES));
        }
        let damage_right = self
            .damage_x
            .checked_add(self.damage_width)
            .ok_or_else(|| "damage rectangle overflow".to_string())?;
        let damage_bottom = self
            .damage_y
            .checked_add(self.damage_height)
            .ok_or_else(|| "damage rectangle overflow".to_string())?;
        if self.damage_width == 0
            || self.damage_height == 0
            || damage_right > self.width
            || damage_bottom > self.height
        {
            return Err("damage rectangle is outside the frame".to_string());
        }
        Ok(frame_len)
    }
}

#[derive(Clone)]
struct LocalShmDisplayConfig {
    frame_tx: mpsc::Sender<FrameEvent>,
    width: u32,
    height: u32,
}

/// Owns local-shm display/input callbacks and worker state.
pub struct LocalShmBridge {
    backend: LocalShmDisplayConfig,
    keyboard_rx: PollableChannelReciever<InputEvent>,
    pointer_rx: PollableChannelReciever<InputEvent>,
    pointer_options: PointerOptions,
}

impl LocalShmBridge {
    /// Starts the local-shm worker and waits for endpoint readiness.
    pub fn start(endpoint_socket: &Path) -> Result<Box<Self>, String> {
        let runtime_dir = endpoint_socket
            .parent()
            .ok_or_else(|| "display endpoint socket has no parent directory".to_string())?
            .to_path_buf();
        let endpoint = LocalShmEndpoint::for_runtime_dir(&runtime_dir);
        endpoint.validate_for_runtime_dir(&runtime_dir)?;

        let (frame_tx, frame_rx) = mpsc::channel(FRAME_QUEUE_DEPTH);
        let (keyboard_tx, keyboard_rx) =
            pollable_channel().map_err(|error| format!("create keyboard input queue: {error}"))?;
        let (pointer_tx, pointer_rx) =
            pollable_channel().map_err(|error| format!("create pointer input queue: {error}"))?;
        let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
        let endpoint_socket = endpoint_socket.to_path_buf();

        thread::Builder::new()
            .name("smolvm-local-shm-display".into())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        let _ = ready_tx.send(Err(format!("create local-shm runtime: {error}")));
                        return;
                    }
                };
                runtime.block_on(run_bridge(
                    endpoint_socket,
                    endpoint,
                    frame_rx,
                    keyboard_tx,
                    pointer_tx,
                    ready_tx,
                ));
            })
            .map_err(|error| format!("spawn local-shm worker: {error}"))?;

        ready_rx
            .recv()
            .map_err(|error| format!("local-shm worker exited before readiness: {error}"))??;

        Ok(Box::new(Self {
            backend: LocalShmDisplayConfig {
                frame_tx,
                width: DISPLAY_WIDTH,
                height: DISPLAY_HEIGHT,
            },
            keyboard_rx,
            pointer_rx,
            pointer_options: PointerOptions {
                width: DISPLAY_WIDTH,
                height: DISPLAY_HEIGHT,
            },
        }))
    }

    /// Builds the libkrun display callback vtable.
    pub fn display_backend(&self) -> krun_display::DisplayBackend<'_> {
        LocalShmDisplayBackend::into_display_backend(Some(&self.backend))
    }

    /// Builds the virtual keyboard capability vtable.
    pub fn keyboard_config(&self) -> krun_input::InputConfigBackend<'_> {
        KeyboardConfig::into_input_config(None)
    }

    /// Builds the virtual keyboard event-provider vtable.
    pub fn keyboard_events(&self) -> krun_input::InputEventProviderBackend<'_> {
        LocalInputEventProvider::into_input_events(Some(&self.keyboard_rx))
    }

    /// Builds the absolute pointer capability vtable.
    pub fn pointer_config(&self) -> krun_input::InputConfigBackend<'_> {
        PointerConfig::into_input_config(Some(&self.pointer_options))
    }

    /// Builds the absolute pointer event-provider vtable.
    pub fn pointer_events(&self) -> krun_input::InputEventProviderBackend<'_> {
        LocalInputEventProvider::into_input_events(Some(&self.pointer_rx))
    }
}

impl DisplayTransport for LocalShmBridge {
    fn kind(&self) -> DisplayTransportKind {
        DisplayTransportKind::LocalShm
    }

    fn display_backend(&self) -> krun_display::DisplayBackend<'_> {
        LocalShmBridge::display_backend(self)
    }

    fn keyboard_config(&self) -> krun_input::InputConfigBackend<'_> {
        LocalShmBridge::keyboard_config(self)
    }

    fn keyboard_events(&self) -> krun_input::InputEventProviderBackend<'_> {
        LocalShmBridge::keyboard_events(self)
    }

    fn pointer_config(&self) -> krun_input::InputConfigBackend<'_> {
        LocalShmBridge::pointer_config(self)
    }

    fn pointer_events(&self) -> krun_input::InputEventProviderBackend<'_> {
        LocalShmBridge::pointer_events(self)
    }
}

enum FrameEvent {
    Update {
        width: u32,
        height: u32,
        format: ResourceFormat,
        damage: Option<Rect>,
        buffer: Vec<u8>,
        recycle: Sender<Vec<u8>>,
    },
}

struct Scanout {
    width: u32,
    height: u32,
    format: ResourceFormat,
    available_tx: Sender<Vec<u8>>,
    available_rx: Receiver<Vec<u8>>,
    current: Option<Vec<u8>>,
}

struct LocalShmDisplayBackend {
    config: LocalShmDisplayConfig,
    scanouts: [Option<Scanout>; krun_display::MAX_DISPLAYS],
}

impl DisplayBackendNew<LocalShmDisplayConfig> for LocalShmDisplayBackend {
    fn new(userdata: Option<&LocalShmDisplayConfig>) -> Self {
        Self {
            config: userdata
                .expect("local-shm display config is required")
                .clone(),
            scanouts: array::from_fn(|_| None),
        }
    }
}

impl DisplayBackendBasicFramebuffer for LocalShmDisplayBackend {
    fn configure_scanout(
        &mut self,
        scanout_id: u32,
        _display_width: u32,
        _display_height: u32,
        width: u32,
        height: u32,
        format: ResourceFormat,
    ) -> Result<(), DisplayBackendError> {
        let index =
            usize::try_from(scanout_id).map_err(|_| DisplayBackendError::InvalidScanoutId)?;
        if index >= self.scanouts.len()
            || width == 0
            || height == 0
            || width > self.config.width
            || height > self.config.height
            || frame_len(width, height).is_none()
        {
            return Err(DisplayBackendError::InvalidParam);
        }

        let (available_tx, available_rx) = bounded(DISPLAY_BUFFERS);
        for _ in 0..DISPLAY_BUFFERS {
            available_tx
                .try_send(Vec::new())
                .map_err(|_| DisplayBackendError::InternalError)?;
        }
        self.scanouts[index] = Some(Scanout {
            width,
            height,
            format,
            available_tx,
            available_rx,
            current: None,
        });
        Ok(())
    }

    fn disable_scanout(&mut self, scanout_id: u32) -> Result<(), DisplayBackendError> {
        let index =
            usize::try_from(scanout_id).map_err(|_| DisplayBackendError::InvalidScanoutId)?;
        let slot = self
            .scanouts
            .get_mut(index)
            .ok_or(DisplayBackendError::InvalidScanoutId)?;
        *slot = None;
        Ok(())
    }

    fn alloc_frame(&mut self, scanout_id: u32) -> Result<(u32, &mut [u8]), DisplayBackendError> {
        let scanout = self.scanout_mut(scanout_id)?;
        if scanout.current.is_some() {
            return Err(DisplayBackendError::OutOfBuffers);
        }
        let mut buffer = scanout
            .available_rx
            .try_recv()
            .map_err(|error| match error {
                TryRecvError::Empty => DisplayBackendError::OutOfBuffers,
                TryRecvError::Disconnected => DisplayBackendError::InternalError,
            })?;
        let length =
            frame_len(scanout.width, scanout.height).ok_or(DisplayBackendError::InvalidParam)?;
        buffer.resize(length, 0);
        scanout.current = Some(buffer);
        Ok((
            1,
            scanout
                .current
                .as_mut()
                .expect("frame was set")
                .as_mut_slice(),
        ))
    }

    fn present_frame(
        &mut self,
        scanout_id: u32,
        frame_id: u32,
        damage: Option<&Rect>,
    ) -> Result<(), DisplayBackendError> {
        if frame_id != 1 {
            return Err(DisplayBackendError::InvalidParam);
        }
        let frame_tx = self.config.frame_tx.clone();
        let scanout = self.scanout_mut(scanout_id)?;
        let buffer = scanout
            .current
            .take()
            .ok_or(DisplayBackendError::InvalidParam)?;
        let event = FrameEvent::Update {
            width: scanout.width,
            height: scanout.height,
            format: scanout.format,
            damage: damage.copied(),
            buffer,
            recycle: scanout.available_tx.clone(),
        };

        if let Err(error) = frame_tx.try_send(event) {
            let event = match error {
                mpsc::error::TrySendError::Full(event)
                | mpsc::error::TrySendError::Closed(event) => event,
            };
            let FrameEvent::Update {
                buffer, recycle, ..
            } = event;
            let _ = recycle.try_send(buffer);
        }
        Ok(())
    }
}

impl LocalShmDisplayBackend {
    fn scanout_mut(&mut self, scanout_id: u32) -> Result<&mut Scanout, DisplayBackendError> {
        let index =
            usize::try_from(scanout_id).map_err(|_| DisplayBackendError::InvalidScanoutId)?;
        self.scanouts
            .get_mut(index)
            .and_then(Option::as_mut)
            .ok_or(DisplayBackendError::InvalidScanoutId)
    }
}

async fn run_bridge(
    endpoint_socket: PathBuf,
    endpoint: LocalShmEndpoint,
    mut frame_rx: mpsc::Receiver<FrameEvent>,
    keyboard_tx: PollableChannelSender<InputEvent>,
    pointer_tx: PollableChannelSender<InputEvent>,
    ready_tx: std::sync::mpsc::SyncSender<Result<(), String>>,
) {
    if let Err(error) = prepare_runtime(&endpoint_socket, &endpoint) {
        let _ = ready_tx.send(Err(error));
        return;
    }

    let endpoint_listener = match UnixListener::bind(&endpoint_socket) {
        Ok(listener) => listener,
        Err(error) => {
            let _ = ready_tx.send(Err(format!("bind local-shm endpoint: {error}")));
            return;
        }
    };
    let control_listener = match UnixListener::bind(&endpoint.control_socket) {
        Ok(listener) => listener,
        Err(error) => {
            let _ = ready_tx.send(Err(format!("bind local-shm control socket: {error}")));
            return;
        }
    };
    let _cleanup = RuntimeCleanup {
        endpoint_socket: endpoint_socket.clone(),
        control_socket: endpoint.control_socket.clone(),
    };
    if let Err(error) =
        std::fs::set_permissions(&endpoint_socket, std::fs::Permissions::from_mode(0o600))
    {
        let _ = ready_tx.send(Err(format!("secure local-shm endpoint: {error}")));
        return;
    }
    if let Err(error) = std::fs::set_permissions(
        &endpoint.control_socket,
        std::fs::Permissions::from_mode(0o600),
    ) {
        let _ = ready_tx.send(Err(format!("secure local-shm control socket: {error}")));
        return;
    }

    let endpoint_json = match serde_json::to_vec(&endpoint) {
        Ok(json) => json,
        Err(error) => {
            let _ = ready_tx.send(Err(format!("encode local-shm endpoint: {error}")));
            return;
        }
    };
    let _ = ready_tx.send(Ok(()));

    let frames_endpoint = endpoint.clone();
    let frames = async move {
        let start = Instant::now();
        let mut frame_id = 0_u64;
        while let Some(event) = frame_rx.recv().await {
            let FrameEvent::Update {
                width,
                height,
                format,
                damage,
                buffer,
                recycle,
            } = event;
            frame_id = frame_id.wrapping_add(1);
            let write_result = write_frame_file(
                &frames_endpoint.frame_ring,
                FramePayload {
                    frame_id,
                    monotonic_ns: start.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64,
                    frame: &buffer,
                    width,
                    height,
                    format,
                    damage: damage.as_ref(),
                },
            )
            .await;
            if let Err(error) = write_result {
                tracing::warn!(%error, "failed to publish local-shm frame");
            }
            let _ = recycle.try_send(buffer);
        }
    };

    tokio::select! {
        _ = serve_endpoint(endpoint_listener, endpoint_json) => {}
        _ = serve_control(control_listener, keyboard_tx, pointer_tx) => {}
        _ = frames => {}
    }
}

fn prepare_runtime(endpoint_socket: &Path, endpoint: &LocalShmEndpoint) -> Result<(), String> {
    let runtime_dir = endpoint_socket
        .parent()
        .ok_or_else(|| "local-shm endpoint socket has no parent".to_string())?;
    std::fs::create_dir_all(runtime_dir)
        .map_err(|error| format!("create local-shm runtime dir: {error}"))?;
    std::fs::set_permissions(runtime_dir, std::fs::Permissions::from_mode(0o700))
        .map_err(|error| format!("secure local-shm runtime dir: {error}"))?;
    let _ = std::fs::remove_file(endpoint_socket);
    let _ = std::fs::remove_file(&endpoint.control_socket);
    let frame_file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&endpoint.frame_ring)
        .map_err(|error| format!("create local-shm frame ring: {error}"))?;
    frame_file
        .set_len(FRAME_RING_BYTES)
        .map_err(|error| format!("size local-shm frame ring: {error}"))?;
    write_initial_ring_header(&frame_file)
        .map_err(|error| format!("initialize local-shm frame ring: {error}"))?;
    std::fs::set_permissions(&endpoint.frame_ring, std::fs::Permissions::from_mode(0o600))
        .map_err(|error| format!("secure local-shm frame ring: {error}"))?;
    Ok(())
}

struct RuntimeCleanup {
    endpoint_socket: PathBuf,
    control_socket: PathBuf,
}

impl Drop for RuntimeCleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.endpoint_socket);
        let _ = std::fs::remove_file(&self.control_socket);
    }
}

async fn serve_endpoint(listener: UnixListener, endpoint_json: Vec<u8>) {
    loop {
        let Ok((mut stream, _)) = listener.accept().await else {
            return;
        };
        if !same_user(&stream) {
            continue;
        }
        let mut response = endpoint_json.clone();
        response.push(b'\n');
        let _ = stream.write_all(&response).await;
        let _ = stream.shutdown().await;
    }
}

async fn serve_control(
    listener: UnixListener,
    keyboard_tx: PollableChannelSender<InputEvent>,
    pointer_tx: PollableChannelSender<InputEvent>,
) {
    loop {
        let Ok((stream, _)) = listener.accept().await else {
            return;
        };
        if !same_user(&stream) {
            continue;
        }
        tokio::spawn(handle_control_client(
            stream,
            keyboard_tx.clone(),
            pointer_tx.clone(),
        ));
    }
}

async fn handle_control_client(
    stream: UnixStream,
    keyboard_tx: PollableChannelSender<InputEvent>,
    pointer_tx: PollableChannelSender<InputEvent>,
) {
    let mut input = ClientInputState::default();
    let mut lines = BufReader::new(stream).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        if line.len() > 4096 {
            continue;
        }
        let Ok(message) = serde_json::from_str::<ClientMessage>(&line) else {
            continue;
        };
        match message {
            ClientMessage::Key { keysym, down } => {
                if let Some(code) = keysym_to_linux(keysym) {
                    input.send_key(code, down, &keyboard_tx);
                }
            }
            ClientMessage::Pointer { x, y, buttons } => {
                input.pointer.send(x, y, buttons, &pointer_tx);
            }
            ClientMessage::RelativePointer { dx, dy, buttons } => {
                input.pointer.send_relative(dx, dy, buttons, &pointer_tx);
            }
            ClientMessage::ReleaseAll => {
                input.release_all(&keyboard_tx, &pointer_tx);
            }
        }
    }
    input.release_all(&keyboard_tx, &pointer_tx);
}

struct FramePayload<'a> {
    frame_id: u64,
    monotonic_ns: u64,
    frame: &'a [u8],
    width: u32,
    height: u32,
    format: ResourceFormat,
    damage: Option<&'a Rect>,
}

async fn write_frame_file(frame_ring: &Path, payload: FramePayload<'_>) -> Result<(), String> {
    let rgba = rgba_frame(payload.frame, payload.width, payload.height, payload.format)
        .ok_or_else(|| "local-shm frame conversion failed".to_string())?;
    let (damage_x, damage_y, damage_width, damage_height) =
        damage_rect(payload.width, payload.height, payload.damage);
    let header = FrameHeader {
        version: PROTOCOL_VERSION,
        frame_id: payload.frame_id,
        width: payload.width,
        height: payload.height,
        stride: payload
            .width
            .checked_mul(PixelFormat::Rgba8888.bytes_per_pixel())
            .ok_or_else(|| "local-shm stride overflow".to_string())?,
        format: PixelFormat::Rgba8888,
        damage_x,
        damage_y,
        damage_width,
        damage_height,
        monotonic_ns: payload.monotonic_ns,
    };
    header.validate()?;
    if u64::try_from(rgba.len()).map_err(|_| "local-shm payload length overflow".to_string())?
        > FRAME_PAYLOAD_CAPACITY
    {
        return Err("local-shm frame exceeds slot payload capacity".to_string());
    }

    let slot_index = ((payload.frame_id.saturating_sub(1)) % u64::from(FRAME_RING_SLOTS)) as u32;
    let slot_offset = slot_offset(slot_index);
    let mut file = tokio::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(frame_ring)
        .await
        .map_err(|error| format!("open local-shm frame ring: {error}"))?;
    file.seek(std::io::SeekFrom::Start(slot_offset))
        .await
        .map_err(|error| format!("seek local-shm frame slot: {error}"))?;
    file.write_all(&encode_slot_header(
        slot_index,
        &header,
        u64::try_from(rgba.len()).map_err(|_| "local-shm payload length overflow".to_string())?,
    ))
    .await
    .map_err(|error| format!("write local-shm slot header: {error}"))?;
    file.write_all(&rgba)
        .await
        .map_err(|error| format!("write local-shm frame payload: {error}"))?;
    file.seek(std::io::SeekFrom::Start(0))
        .await
        .map_err(|error| format!("seek local-shm ring header: {error}"))?;
    file.write_all(&encode_ring_header(
        payload.frame_id,
        slot_index,
        payload.width,
        payload.height,
    ))
    .await
    .map_err(|error| format!("write local-shm ring header: {error}"))?;
    file.flush()
        .await
        .map_err(|error| format!("flush local-shm frame ring: {error}"))
}

fn write_initial_ring_header(file: &std::fs::File) -> std::io::Result<()> {
    let mut file = file;
    file.write_all(&encode_ring_header(0, 0, 0, 0))
}

fn slot_offset(slot_index: u32) -> u64 {
    FRAME_RING_HEADER_BYTES + u64::from(slot_index) * FRAME_SLOT_STRIDE
}

fn encode_ring_header(latest_frame_id: u64, latest_slot: u32, width: u32, height: u32) -> Vec<u8> {
    let mut bytes = vec![0; FRAME_RING_HEADER_BYTES as usize];
    bytes[0..8].copy_from_slice(FRAME_RING_MAGIC);
    put_u32(&mut bytes, 8, PROTOCOL_VERSION);
    put_u32(&mut bytes, 12, FRAME_RING_SLOTS);
    put_u64(&mut bytes, 16, FRAME_RING_HEADER_BYTES);
    put_u64(&mut bytes, 24, FRAME_SLOT_HEADER_BYTES);
    put_u64(&mut bytes, 32, FRAME_SLOT_STRIDE);
    put_u64(&mut bytes, 40, FRAME_PAYLOAD_CAPACITY);
    put_u64(&mut bytes, 48, latest_frame_id);
    put_u32(&mut bytes, 56, latest_slot);
    put_u32(&mut bytes, 60, width);
    put_u32(&mut bytes, 64, height);
    bytes
}

fn encode_slot_header(slot_index: u32, header: &FrameHeader, payload_len: u64) -> Vec<u8> {
    let mut bytes = vec![0; FRAME_SLOT_HEADER_BYTES as usize];
    bytes[0..8].copy_from_slice(FRAME_SLOT_MAGIC);
    put_u32(&mut bytes, 8, PROTOCOL_VERSION);
    put_u32(&mut bytes, 12, slot_index);
    put_u64(&mut bytes, 16, header.frame_id);
    put_u64(&mut bytes, 24, header.monotonic_ns);
    put_u32(&mut bytes, 32, header.width);
    put_u32(&mut bytes, 36, header.height);
    put_u32(&mut bytes, 40, header.stride);
    put_u32(&mut bytes, 44, header.format.ring_code());
    put_u32(&mut bytes, 48, header.damage_x);
    put_u32(&mut bytes, 52, header.damage_y);
    put_u32(&mut bytes, 56, header.damage_width);
    put_u32(&mut bytes, 60, header.damage_height);
    put_u64(&mut bytes, 64, payload_len);
    bytes
}

fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn put_u64(bytes: &mut [u8], offset: usize, value: u64) {
    bytes[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
}

fn frame_len(width: u32, height: u32) -> Option<usize> {
    let length = usize::try_from(width)
        .ok()?
        .checked_mul(usize::try_from(height).ok()?)?
        .checked_mul(ResourceFormat::BYTES_PER_PIXEL)?;
    (u64::try_from(length).ok()? <= MAX_FRAME_BYTES).then_some(length)
}

fn rgba_frame(frame: &[u8], width: u32, height: u32, format: ResourceFormat) -> Option<Vec<u8>> {
    if frame.len() != frame_len(width, height)? {
        return None;
    }
    let mut output = Vec::with_capacity(frame.len());
    for pixel in frame.chunks_exact(ResourceFormat::BYTES_PER_PIXEL) {
        output.extend_from_slice(&rgba_pixel(pixel.try_into().ok()?, format));
    }
    Some(output)
}

fn rgba_pixel(pixel: [u8; 4], format: ResourceFormat) -> [u8; 4] {
    match format {
        ResourceFormat::BGRA => [pixel[2], pixel[1], pixel[0], pixel[3]],
        ResourceFormat::BGRX => [pixel[2], pixel[1], pixel[0], 255],
        ResourceFormat::ARGB => [pixel[1], pixel[2], pixel[3], pixel[0]],
        ResourceFormat::XRGB => [pixel[1], pixel[2], pixel[3], 255],
        ResourceFormat::RGBA => pixel,
        ResourceFormat::XBGR => [pixel[3], pixel[2], pixel[1], 255],
        ResourceFormat::ABGR => [pixel[3], pixel[2], pixel[1], pixel[0]],
        ResourceFormat::RGBX => [pixel[0], pixel[1], pixel[2], 255],
    }
}

fn damage_rect(width: u32, height: u32, damage: Option<&Rect>) -> (u32, u32, u32, u32) {
    match damage {
        Some(rect)
            if rect.width > 0
                && rect.height > 0
                && rect
                    .x
                    .checked_add(rect.width)
                    .is_some_and(|right| right <= width)
                && rect
                    .y
                    .checked_add(rect.height)
                    .is_some_and(|bottom| bottom <= height) =>
        {
            (rect.x, rect.y, rect.width, rect.height)
        }
        _ => (0, 0, width, height),
    }
}

#[cfg(target_os = "macos")]
fn same_user(stream: &UnixStream) -> bool {
    let mut uid = 0;
    let mut gid = 0;
    let result = unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) };
    result == 0 && uid == unsafe { libc::geteuid() }
}

#[cfg(target_os = "linux")]
fn same_user(stream: &UnixStream) -> bool {
    let mut credential = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let result = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut credential as *mut libc::ucred).cast(),
            &mut length,
        )
    };
    result == 0 && credential.uid == unsafe { libc::geteuid() }
}

struct LocalInputEventProvider {
    rx: PollableChannelReciever<InputEvent>,
}

impl ObjectNew<PollableChannelReciever<InputEvent>> for LocalInputEventProvider {
    fn new(userdata: Option<&PollableChannelReciever<InputEvent>>) -> Self {
        Self {
            rx: userdata.expect("input event receiver is required").clone(),
        }
    }
}

impl InputEventsImpl for LocalInputEventProvider {
    fn get_read_notify_fd(&self) -> Result<BorrowedFd<'_>, InputBackendError> {
        Ok(self.rx.as_fd())
    }

    fn next_event(&mut self) -> Result<Option<InputEvent>, InputBackendError> {
        self.rx
            .try_recv()
            .map_err(|_| InputBackendError::InternalError)
    }
}

#[derive(Clone, Copy)]
struct KeyboardConfig;

impl ObjectNew<()> for KeyboardConfig {
    fn new(_userdata: Option<&()>) -> Self {
        Self
    }
}

impl InputQueryConfig for KeyboardConfig {
    fn query_device_name(&self, buffer: &mut [u8]) -> Result<u8, InputBackendError> {
        copy_name(buffer, b"SmolVM LocalShm Keyboard")
    }

    fn query_serial_name(&self, buffer: &mut [u8]) -> Result<u8, InputBackendError> {
        copy_name(buffer, b"SMOLVM-LSHM-KBD")
    }

    fn query_device_ids(&self, ids: &mut InputDeviceIds) -> Result<(), InputBackendError> {
        *ids = InputDeviceIds {
            bustype: BUS_VIRTUAL,
            vendor: 0x534d,
            product: 2,
            version: 1,
        };
        Ok(())
    }

    fn query_event_capabilities(
        &self,
        event_type: u8,
        buffer: &mut [u8],
    ) -> Result<u8, InputBackendError> {
        match InputEventType::try_from(event_type as u16) {
            Ok(InputEventType::Syn) => set_bits(buffer, &[SYN_REPORT]),
            Ok(InputEventType::Key) => {
                let keys: Vec<u16> = (1..=248).collect();
                set_bits(buffer, &keys)
            }
            _ => Ok(0),
        }
    }

    fn query_abs_info(&self, _axis: u8, _info: &mut InputAbsInfo) -> Result<(), InputBackendError> {
        Ok(())
    }

    fn query_properties(&self, _buffer: &mut [u8]) -> Result<u8, InputBackendError> {
        Ok(0)
    }
}

#[derive(Clone, Copy)]
struct PointerOptions {
    width: u32,
    height: u32,
}

#[derive(Clone, Copy)]
struct PointerConfig(PointerOptions);

impl ObjectNew<PointerOptions> for PointerConfig {
    fn new(userdata: Option<&PointerOptions>) -> Self {
        Self(*userdata.expect("pointer options are required"))
    }
}

impl InputQueryConfig for PointerConfig {
    fn query_device_name(&self, buffer: &mut [u8]) -> Result<u8, InputBackendError> {
        copy_name(buffer, b"SmolVM LocalShm Pointer")
    }

    fn query_serial_name(&self, buffer: &mut [u8]) -> Result<u8, InputBackendError> {
        copy_name(buffer, b"SMOLVM-LSHM-PTR")
    }

    fn query_device_ids(&self, ids: &mut InputDeviceIds) -> Result<(), InputBackendError> {
        *ids = InputDeviceIds {
            bustype: BUS_VIRTUAL,
            vendor: 0x534d,
            product: 3,
            version: 1,
        };
        Ok(())
    }

    fn query_event_capabilities(
        &self,
        event_type: u8,
        buffer: &mut [u8],
    ) -> Result<u8, InputBackendError> {
        match InputEventType::try_from(event_type as u16) {
            Ok(InputEventType::Syn) => set_bits(buffer, &[SYN_REPORT]),
            Ok(InputEventType::Key) => set_bits(buffer, &[BTN_LEFT, BTN_RIGHT, BTN_MIDDLE]),
            Ok(InputEventType::Rel) => set_bits(buffer, &[REL_X, REL_Y, REL_WHEEL]),
            Ok(InputEventType::Abs) => set_bits(buffer, &[ABS_X, ABS_Y]),
            _ => Ok(0),
        }
    }

    fn query_abs_info(&self, axis: u8, info: &mut InputAbsInfo) -> Result<(), InputBackendError> {
        let max = match u16::from(axis) {
            ABS_X => self.0.width.saturating_sub(1),
            ABS_Y => self.0.height.saturating_sub(1),
            _ => 0,
        };
        *info = InputAbsInfo {
            min: 0,
            max,
            fuzz: 0,
            flat: 0,
            res: 0,
        };
        Ok(())
    }

    fn query_properties(&self, buffer: &mut [u8]) -> Result<u8, InputBackendError> {
        set_bits(buffer, &[INPUT_PROP_POINTER])
    }
}

fn copy_name(buffer: &mut [u8], name: &[u8]) -> Result<u8, InputBackendError> {
    let length = buffer.len().min(name.len()).min(u8::MAX as usize);
    buffer[..length].copy_from_slice(&name[..length]);
    Ok(length as u8)
}

fn set_bits(buffer: &mut [u8], bits: &[u16]) -> Result<u8, InputBackendError> {
    let mut last = None;
    for bit in bits {
        let byte = usize::from(*bit / 8);
        if byte >= buffer.len() {
            return Err(InputBackendError::InvalidParam);
        }
        buffer[byte] |= 1 << (*bit % 8);
        last = Some(last.map_or(byte, |current: usize| current.max(byte)));
    }
    Ok(last.map_or(0, |byte| (byte + 1).min(u8::MAX as usize) as u8))
}

#[derive(Default)]
struct PointerState {
    buttons: u8,
}

impl PointerState {
    fn send(&mut self, x: u32, y: u32, buttons: u8, sender: &PollableChannelSender<InputEvent>) {
        let mut events = vec![
            input_event(EV_ABS, ABS_X, x.min(DISPLAY_WIDTH.saturating_sub(1))),
            input_event(EV_ABS, ABS_Y, y.min(DISPLAY_HEIGHT.saturating_sub(1))),
        ];
        for (mask, code) in [(1, BTN_LEFT), (2, BTN_MIDDLE), (4, BTN_RIGHT)] {
            if self.buttons & mask != buttons & mask {
                events.push(input_event(EV_KEY, code, u32::from(buttons & mask != 0)));
            }
        }
        if buttons & 8 != 0 {
            events.push(input_event(EV_REL, REL_WHEEL, 1));
        }
        if buttons & 16 != 0 {
            events.push(input_event(EV_REL, REL_WHEEL, u32::MAX));
        }
        events.push(input_event(EV_SYN, SYN_REPORT, 0));
        self.buttons = buttons & 0b0000_0111;
        let _ = sender.send_many(events);
    }

    fn send_relative(
        &mut self,
        dx: i32,
        dy: i32,
        buttons: u8,
        sender: &PollableChannelSender<InputEvent>,
    ) {
        let mut events = Vec::with_capacity(6);
        if dx != 0 {
            events.push(input_event(EV_REL, REL_X, dx as u32));
        }
        if dy != 0 {
            events.push(input_event(EV_REL, REL_Y, dy as u32));
        }
        for (mask, code) in [(1, BTN_LEFT), (2, BTN_MIDDLE), (4, BTN_RIGHT)] {
            if self.buttons & mask != buttons & mask {
                events.push(input_event(EV_KEY, code, u32::from(buttons & mask != 0)));
            }
        }
        if events.is_empty() {
            return;
        }
        events.push(input_event(EV_SYN, SYN_REPORT, 0));
        self.buttons = buttons & 0b0000_0111;
        let _ = sender.send_many(events);
    }

    fn release_all(&mut self, sender: &PollableChannelSender<InputEvent>) {
        if self.buttons == 0 {
            return;
        }
        let mut events = Vec::with_capacity(4);
        for (mask, code) in [(1, BTN_LEFT), (2, BTN_MIDDLE), (4, BTN_RIGHT)] {
            if self.buttons & mask != 0 {
                events.push(input_event(EV_KEY, code, 0));
            }
        }
        events.push(input_event(EV_SYN, SYN_REPORT, 0));
        self.buttons = 0;
        let _ = sender.send_many(events);
    }
}

#[derive(Default)]
struct ClientInputState {
    keys: HashSet<u16>,
    pointer: PointerState,
}

impl ClientInputState {
    fn send_key(&mut self, code: u16, down: bool, sender: &PollableChannelSender<InputEvent>) {
        let changed = if down {
            self.keys.insert(code)
        } else {
            self.keys.remove(&code)
        };
        if changed {
            let _ = sender.send_many([
                input_event(EV_KEY, code, u32::from(down)),
                input_event(EV_SYN, SYN_REPORT, 0),
            ]);
        }
    }

    fn release_all(
        &mut self,
        keyboard: &PollableChannelSender<InputEvent>,
        pointer: &PollableChannelSender<InputEvent>,
    ) {
        if !self.keys.is_empty() {
            let mut events: Vec<_> = self
                .keys
                .drain()
                .map(|code| input_event(EV_KEY, code, 0))
                .collect();
            events.push(input_event(EV_SYN, SYN_REPORT, 0));
            let _ = keyboard.send_many(events);
        }
        self.pointer.release_all(pointer);
    }
}

fn input_event(event_type: u16, code: u16, value: u32) -> InputEvent {
    InputEvent {
        type_: event_type,
        code,
        value,
    }
}

fn keysym_to_linux(keysym: u32) -> Option<u16> {
    let lower = if (b'A' as u32..=b'Z' as u32).contains(&keysym) {
        keysym + 32
    } else {
        keysym
    };
    Some(match lower {
        0x0061 => 30,
        0x0062 => 48,
        0x0063 => 46,
        0x0064 => 32,
        0x0065 => 18,
        0x0066 => 33,
        0x0067 => 34,
        0x0068 => 35,
        0x0069 => 23,
        0x006a => 36,
        0x006b => 37,
        0x006c => 38,
        0x006d => 50,
        0x006e => 49,
        0x006f => 24,
        0x0070 => 25,
        0x0071 => 16,
        0x0072 => 19,
        0x0073 => 31,
        0x0074 => 20,
        0x0075 => 22,
        0x0076 => 47,
        0x0077 => 17,
        0x0078 => 45,
        0x0079 => 21,
        0x007a => 44,
        0x0031 | 0x0021 => 2,
        0x0032 | 0x0040 => 3,
        0x0033 | 0x0023 => 4,
        0x0034 | 0x0024 => 5,
        0x0035 | 0x0025 => 6,
        0x0036 | 0x005e => 7,
        0x0037 | 0x0026 => 8,
        0x0038 | 0x002a => 9,
        0x0039 | 0x0028 => 10,
        0x0030 | 0x0029 => 11,
        0x0020 => 57,
        0xff0d => 28,
        0xff1b => 1,
        0xff08 => 14,
        _ => return None,
    })
}

/// Client-to-server local-shm control messages.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    /// Keyboard event using an RFB/X11-style keysym.
    Key {
        /// RFB/X11-style keysym.
        keysym: u32,
        /// `true` for key press, `false` for key release.
        down: bool,
    },
    /// Absolute pointer event in framebuffer coordinates.
    Pointer {
        /// Pointer x coordinate in framebuffer pixels.
        x: u32,
        /// Pointer y coordinate in framebuffer pixels.
        y: u32,
        /// Bitmask of pressed pointer buttons.
        buttons: u8,
    },
    /// Relative pointer event in device counts for game-style pointer capture.
    RelativePointer {
        /// Signed x-axis movement delta.
        dx: i32,
        /// Signed y-axis movement delta.
        dy: i32,
        /// Bitmask of pressed pointer buttons.
        buttons: u8,
    },
    /// Release all tracked keyboard and pointer state for disconnect cleanup.
    ReleaseAll,
}

/// Server-to-client local-shm control messages.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    /// A new frame is available in the shared frame ring.
    FrameAvailable {
        /// Metadata describing the frame now available in the shared ring.
        header: FrameHeader,
    },
    /// Transport-level counters for diagnostics.
    Stats {
        /// Latest frame identifier observed by the transport.
        frame_id: u64,
        /// Number of frames dropped before clients could consume them.
        dropped_frames: u64,
        /// Number of frames currently queued for delivery.
        queued_frames: u32,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_header() -> FrameHeader {
        FrameHeader {
            version: PROTOCOL_VERSION,
            frame_id: 42,
            width: 800,
            height: 600,
            stride: 800 * 4,
            format: PixelFormat::Bgra8888,
            damage_x: 0,
            damage_y: 0,
            damage_width: 800,
            damage_height: 600,
            monotonic_ns: 123,
        }
    }

    #[test]
    fn endpoint_paths_stay_inside_runtime_dir() {
        let runtime_dir = Path::new("/tmp/smolvm-display-runtime");
        let endpoint = LocalShmEndpoint::for_runtime_dir(runtime_dir);
        assert_eq!(endpoint.protocol, PROTOCOL);
        assert_eq!(endpoint.version, PROTOCOL_VERSION);
        assert_eq!(
            endpoint.control_socket,
            runtime_dir.join(CONTROL_SOCKET_NAME)
        );
        assert_eq!(endpoint.frame_ring, runtime_dir.join(FRAME_RING_NAME));
        endpoint
            .validate_for_runtime_dir(runtime_dir)
            .expect("endpoint should validate");

        let mut escaped = endpoint.clone();
        escaped.frame_ring = PathBuf::from("/tmp/other.frames");
        assert!(escaped.validate_for_runtime_dir(runtime_dir).is_err());
    }

    #[test]
    fn frame_header_validates_dimensions_stride_damage_and_size() {
        assert_eq!(valid_header().validate(), Ok(1_920_000));

        let mut zero = valid_header();
        zero.width = 0;
        assert!(zero.validate().is_err());

        let mut short_stride = valid_header();
        short_stride.stride = short_stride.width * 4 - 1;
        assert!(short_stride.validate().is_err());

        let mut bad_damage = valid_header();
        bad_damage.damage_x = 799;
        bad_damage.damage_width = 2;
        assert!(bad_damage.validate().is_err());

        let mut huge = valid_header();
        huge.width = MAX_DIMENSION;
        huge.height = MAX_DIMENSION;
        huge.stride = MAX_DIMENSION * 4;
        huge.damage_width = MAX_DIMENSION;
        huge.damage_height = MAX_DIMENSION;
        assert!(huge.validate().is_err());
    }

    #[test]
    fn control_messages_are_tagged_json() {
        let key = ClientMessage::Key {
            keysym: 0x61,
            down: true,
        };
        let json = serde_json::to_value(&key).expect("client message json");
        assert_eq!(json["type"], "key");
        assert_eq!(json["keysym"], 0x61);

        let relative = ClientMessage::RelativePointer {
            dx: -3,
            dy: 4,
            buttons: 1,
        };
        let json = serde_json::to_value(&relative).expect("relative pointer json");
        assert_eq!(json["type"], "relative_pointer");
        assert_eq!(json["dx"], -3);
        assert_eq!(json["dy"], 4);
        assert_eq!(json["buttons"], 1);

        let frame = ServerMessage::FrameAvailable {
            header: valid_header(),
        };
        let json = serde_json::to_value(&frame).expect("server message json");
        assert_eq!(json["type"], "frame_available");
        assert_eq!(json["header"]["format"], "bgra8888");
    }

    #[tokio::test]
    async fn frame_ring_file_has_fixed_mmap_friendly_layout() {
        let tempdir = tempfile::tempdir().expect("tempdir");
        let frame_ring = tempdir.path().join(FRAME_RING_NAME);
        let file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&frame_ring)
            .expect("create ring");
        file.set_len(FRAME_RING_BYTES).expect("size ring");
        write_initial_ring_header(&file).expect("initial ring header");

        let first = [1, 2, 3, 4, 5, 6, 7, 8];
        write_frame_file(
            &frame_ring,
            FramePayload {
                frame_id: 1,
                monotonic_ns: 10,
                frame: &first,
                width: 2,
                height: 1,
                format: ResourceFormat::RGBA,
                damage: None,
            },
        )
        .await
        .expect("write first frame");

        let second = [30, 20, 10, 40, 70, 60, 50, 80];
        write_frame_file(
            &frame_ring,
            FramePayload {
                frame_id: 2,
                monotonic_ns: 20,
                frame: &second,
                width: 2,
                height: 1,
                format: ResourceFormat::BGRA,
                damage: Some(&Rect {
                    x: 1,
                    y: 0,
                    width: 1,
                    height: 1,
                }),
            },
        )
        .await
        .expect("write second frame");

        let bytes = std::fs::read(&frame_ring).expect("read ring");
        assert_eq!(bytes.len(), FRAME_RING_BYTES as usize);
        assert_eq!(&bytes[0..8], FRAME_RING_MAGIC);
        assert_eq!(read_u64(&bytes, 48), 2);
        assert_eq!(read_u32(&bytes, 56), 1);
        assert_eq!(read_u32(&bytes, 60), 2);
        assert_eq!(read_u32(&bytes, 64), 1);

        let slot = slot_offset(1) as usize;
        assert_eq!(&bytes[slot..slot + 8], FRAME_SLOT_MAGIC);
        assert_eq!(read_u32(&bytes, slot + 12), 1);
        assert_eq!(read_u64(&bytes, slot + 16), 2);
        assert_eq!(read_u64(&bytes, slot + 24), 20);
        assert_eq!(read_u32(&bytes, slot + 32), 2);
        assert_eq!(read_u32(&bytes, slot + 36), 1);
        assert_eq!(read_u32(&bytes, slot + 40), 8);
        assert_eq!(read_u32(&bytes, slot + 44), PIXEL_FORMAT_RGBA8888);
        assert_eq!(read_u32(&bytes, slot + 48), 1);
        assert_eq!(read_u32(&bytes, slot + 56), 1);
        assert_eq!(read_u64(&bytes, slot + 64), 8);
        let payload = slot + FRAME_SLOT_HEADER_BYTES as usize;
        assert_eq!(
            &bytes[payload..payload + 8],
            &[10, 20, 30, 40, 50, 60, 70, 80]
        );
    }

    fn read_u32(bytes: &[u8], offset: usize) -> u32 {
        u32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("u32 bytes"))
    }

    fn read_u64(bytes: &[u8], offset: usize) -> u64 {
        u64::from_le_bytes(bytes[offset..offset + 8].try_into().expect("u64 bytes"))
    }
}
