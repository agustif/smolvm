# desktop-wayland profile

`desktop-wayland` is the reference SmolVM graphical guest profile for native
display acceptance. It is intentionally boring: Debian Bookworm, Weston on the
DRM backend, seatd/libinput environment variables, Xwayland, Mesa tools, and
Vulkan tools.

Create an RFB-backed graphical machine:

```bash
smolvm machine create --name desktop-wayland \
  --graphics --renderer auto --transport rfb \
  -s profiles/desktop-wayland/Smolfile
smolvm machine start --name desktop-wayland
```

Create a local-shm-backed graphical machine:

```bash
smolvm machine create --name desktop-wayland-shm \
  --graphics --renderer auto --transport local-shm \
  -s profiles/desktop-wayland/Smolfile
smolvm machine start --name desktop-wayland-shm
```

The first start may download packages. Later starts reuse the persistent
overlay and should go straight to the Weston session.

The profile's health contract checks:

- a DRM node under `/dev/dri`;
- at least one input event device under `/dev/input`;
- the seat socket at `/run/seatd.sock`;
- a Wayland socket from Weston;
- at least one API diagnostic tool: `vulkaninfo`, `glxinfo`, or `eglinfo`.

This profile does not prove a specific accelerated renderer by itself.
`machine graphics status --json` remains the runtime source of truth for
`renderer_detail` and `api_detail`.

Run the same health check after start:

```bash
profiles/desktop-wayland/health.sh desktop-wayland
```
