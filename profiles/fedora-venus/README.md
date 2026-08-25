# fedora-venus profile

`fedora-venus` is the Venus/Vulkan qualification guest. Use it when the
question is "does this machine have a real virtio-gpu Vulkan ICD?", not
just "does Weston paint pixels?"

`desktop-wayland` stays Debian Bookworm for RFB/local-shm smoke. Bookworm
Mesa cannot initialize Venus on Apple Silicon because host pages are 16KB.
This profile uses Fedora 42 plus `slp/mesa-libkrun-vulkan` on aarch64, the
same stack as `tests/test_gpu.sh`.

```bash
smolvm machine create --name fedora-venus \
  --graphics --renderer venus --transport rfb \
  -s profiles/fedora-venus/Smolfile
smolvm machine start --name fedora-venus
smolvm machine graphics status --name fedora-venus --json
```

Strict native-display acceptance selects this profile automatically when
`SMOLVM_DISPLAY_ACCEPTANCE_REQUIRE_ACCELERATED=1` and no Smolfile override
is set.

Health requires `vulkaninfo --summary` to succeed, not merely that the
binary exists.

Fedora Weston 14 does not accept Debian's `--tty=1`. The profile also exports
a PATH that includes `/usr/sbin` because `vulkaninfo` lands there.
