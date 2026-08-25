# game-session profile

`game-session` is the SmolVM profile for game-oriented graphics/input
qualification. It prefers `gamescope` when the guest package is available and
falls back to Weston so the profile can still expose a Wayland compositor for
display acceptance.

Create a local-shm graphical machine:

```bash
smolvm machine create --name game-session \
  --graphics --renderer auto --transport local-shm \
  -s profiles/game-session/Smolfile
smolvm machine start --name game-session
```

Create an RFB-backed fallback machine:

```bash
smolvm machine create --name game-session-rfb \
  --graphics --renderer auto --transport rfb \
  -s profiles/game-session/Smolfile
smolvm machine start --name game-session-rfb
```

The first start may download packages. Later starts reuse the persistent
overlay and should go straight to the compositor session.

The profile's health contract checks:

- a DRM node under `/dev/dri`;
- at least one input event device under `/dev/input`;
- the seat socket at `/run/seatd.sock`;
- a relative-pointer-capable input device reported in `/proc/bus/input/devices`;
- a Wayland socket from gamescope or Weston;
- a running `gamescope` or `weston` process;
- `vulkaninfo --summary` succeeds.

This profile does not prove a specific VirGL, Venus, or native-context path by
itself. `machine graphics status --json` remains the runtime source of truth
for `renderer_detail` and `api_detail`, and live acceptance must still run on a
prepared host.

Run the same health check after start:

```bash
profiles/game-session/health.sh game-session
```
