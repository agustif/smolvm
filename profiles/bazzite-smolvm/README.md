# Bazzite on SmolVM (aarch64)

Official Bazzite (`ghcr.io/ublue-os/bazzite:stable`) is an **x86_64 ostree/bootc
OS image**. SmolVM on Apple Silicon is an **aarch64 OCI microVM** (libkrun +
Venus). That is why `smolvm machine create --image ghcr.io/ublue-os/bazzite:stable`
cannot boot the upstream image: the registry blob is `linux/amd64`, the guest
CPU is arm64, and SmolVM does not replace systemd/ostree as PID 1.

This directory is the port that *does* boot on that stack:

- Fedora 42 aarch64 OCI workload (same class as `profiles/fedora-venus`)
- `slp/mesa-libkrun-vulkan` so `vulkaninfo` sees **Virtio-GPU Venus**
- Weston DRM + pixman so frames actually reach RFB/local-shm
- Optional `gamescope` and `fex-emu` when Fedora has aarch64 packages
- Bazzite wallpaper from `press_kit/art/Convergence_Wallpaper.png`

Steam/Proton remain a follow-up: they are x86_64 and need a working FEX rootfs
plus audio/gamepad. The gate this profile proves is **Bazzite-branded desktop +
matched Venus on SmolVM**.

## Run

From a SmolVM tree that has GPU-enabled libkrun (Venus scanout):

```bash
export SMOLVM_AGENT_ROOTFS=./target/agent-rootfs
export DYLD_LIBRARY_PATH=./lib   # macOS

smolvm machine create --name bazzite \
  --graphics --renderer venus --transport rfb \
  -s /path/to/bazzite/smolvm/Smolfile

smolvm machine start --name bazzite
smolvm machine graphics status --name bazzite --json
smolvm machine display --name bazzite --json
```

First boot installs COPR Mesa + Weston (several minutes). Later starts reuse
the overlay.

Connect the RFB endpoint from `machine display --json` (loopback + password).
On macOS, Screen Sharing can open `vnc://127.0.0.1:<port>`.

## Warzone / Vulkan present on MoltenVK

Warzone 2100 (`lib/ivis_opengl/gfx_api_vk.cpp`) is ordinary Vulkan:

1. `setupSwapchainImages()` submits MSAA/depth layout barriers, then
   `graphicsQueue.waitIdle()`.
2. The game loop presents with `presentKHR` waiting on
   `renderFinishedSemaphore` (`vk/screen_frame_coordinator.cpp`).

Mesa Venus 25.2.3 then calls `vn_queue_wsi_present`. With
`venus_implicit_fencing=false` (default) that is `vn_QueueWaitIdle` →
`vn_WaitForFences` polling a **local fence feedback slot**. MoltenVK has
no `SYNC_FD`, so the slot never becomes `VK_SUCCESS` and the window
stays black. Guest ICD-internal waits cannot be intercepted by a Vulkan
layer.

Workarounds installed into the overlay:

- `/usr/share/drirc.d/50-smolvm-venus.conf` — `venus_implicit_fencing=true`
  so present attaches the WSI BO instead of waiting.
- `VN_PERF=no_fence_feedback,no_semaphore_feedback` — `GetFenceStatus`
  talks to the host; virglrenderer returns `VK_SUCCESS` when the host
  fence is `NOT_READY` and SYNC_FD is missing.
- `VK_LAYER_SMOLVM_wsi_sync` — acquire without a sync-fd, skip app
  `QueueWaitIdle`, strip present wait semaphores.

Relaunch:

```bash
smolvm machine exec --name bazzite -- /usr/bin/bash -lc \
  'source /etc/profile.d/smolvm-venus.sh
   export XDG_RUNTIME_DIR=/tmp/weston-runtime WAYLAND_DISPLAY=wayland-0 DISPLAY=:0 HOME=/root SDL_VIDEODRIVER=x11
   warzone2100 --gfxbackend=vulkan --window --resolution=1280x720 --nosound'
```

## What this is not

- Not a rebase of Fedora Atomic onto Bazzite.
- Not Steam Gaming Mode / gamescope-as-session yet (needs audio, relative
  pointer capture, and FEX Steam).
- Not an x86_64 ISO. Use QEMU TCG for that; it will not use SmolVM Venus.
