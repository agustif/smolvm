#!/bin/bash
set -euo pipefail

if [[ $# -lt 1 || $# -gt 2 ]]; then
    echo "usage: $0 MACHINE [SMOLVM_BIN]" >&2
    exit 64
fi

machine="$1"
smolvm_bin="${2:-${SMOLVM_BIN:-smolvm}}"

"$smolvm_bin" machine exec --name "$machine" -- sh -lc '
set -eu

if ! { test -e /dev/dri/card0 || test -e /dev/dri/renderD128; }; then
  echo "GPU requested but /dev/dri/card0 or /dev/dri/renderD128 is missing" >&2
  exit 1
fi

if ! test -e /dev/dri/renderD128; then
  echo "Venus qualification needs /dev/dri/renderD128" >&2
  exit 1
fi

if ! { test -d /dev/input && find /dev/input -name "event*" | grep -q .; }; then
  echo "input requested but /dev/input has no event devices" >&2
  exit 1
fi

if ! test -S /run/seatd.sock; then
  echo "seatd socket /run/seatd.sock is missing" >&2
  exit 1
fi

if ! { test -S /tmp/weston-runtime/wayland-0 || test -S /run/user/0/wayland-0 || test -S /run/wayland-0; }; then
  echo "Weston did not expose a Wayland socket" >&2
  exit 1
fi

if [[ -z "${VK_ICD_FILENAMES:-}" && -z "${VK_DRIVER_FILES:-}" ]]; then
  arch="$(uname -m)"
  if [[ "$arch" == "arm64" ]]; then
    arch="aarch64"
  fi
  for icd in \
    "/usr/share/vulkan/icd.d/virtio_icd.${arch}.json" \
    /usr/share/vulkan/icd.d/virtio_icd.aarch64.json \
    /usr/share/vulkan/icd.d/virtio_icd.x86_64.json
  do
    if [[ -r "$icd" ]]; then
      export VK_ICD_FILENAMES="$icd"
      break
    fi
  done
fi

if [[ -z "${VK_ICD_FILENAMES:-}" ]]; then
  echo "virtio Venus ICD json was not found under /usr/share/vulkan/icd.d" >&2
  exit 1
fi

if ! command -v vulkaninfo >/dev/null 2>&1; then
  echo "vulkaninfo is required for fedora-venus qualification" >&2
  exit 1
fi

vulkaninfo --summary >/tmp/smolvm-vulkaninfo.summary 2>&1 || {
  echo "vulkaninfo failed; see /tmp/smolvm-vulkaninfo.summary" >&2
  exit 1
}

if ! grep -qiE "Venus|Virtio-GPU|virtio" /tmp/smolvm-vulkaninfo.summary; then
  echo "vulkaninfo did not report a Venus/virtio-gpu device" >&2
  cat /tmp/smolvm-vulkaninfo.summary >&2
  exit 1
fi
'
