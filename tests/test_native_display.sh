#!/bin/bash
#
# Native display acceptance tests for smolvm.
#
# This suite boots the persistent desktop-wayland profile and connects to the exported
# display endpoint. It is intentionally opt-in because it starts real GPU/display
# VMs, may pull Debian packages on first run, and requires a display-aware
# libkrun/rootfs bundle.
#
# Usage:
#   SMOLVM_RUN_NATIVE_DISPLAY_ACCEPTANCE=1 ./tests/test_native_display.sh
#   SMOLVM_RUN_NATIVE_DISPLAY_ACCEPTANCE=1 ./tests/run_tests.sh display

source "$(dirname "$0")/common.sh"

if [[ "${SMOLVM_RUN_NATIVE_DISPLAY_ACCEPTANCE:-0}" != "1" ]]; then
    log_skip "native display acceptance skipped (set SMOLVM_RUN_NATIVE_DISPLAY_ACCEPTANCE=1)"
    exit 0
fi

init_smolvm

"$SCRIPT_DIR/check_native_display_prereqs.sh"

log_info "Pre-flight cleanup: killing orphan processes..."
kill_orphan_smolvm_processes

DISPLAY_MACHINE_BASE="${SMOLVM_DISPLAY_ACCEPTANCE_MACHINE:-display-acceptance-$$}"
DISPLAY_MACHINE="$DISPLAY_MACHINE_BASE"
DISPLAY_REQUIRE_ACCELERATED="${SMOLVM_DISPLAY_ACCEPTANCE_REQUIRE_ACCELERATED:-0}"
if [[ -z "${SMOLVM_DISPLAY_ACCEPTANCE_SMOLFILE:-}" && "$DISPLAY_REQUIRE_ACCELERATED" == "1" ]]; then
    DISPLAY_SMOLFILE="$PROJECT_ROOT/profiles/fedora-venus/Smolfile"
    DISPLAY_RENDERER="${SMOLVM_DISPLAY_ACCEPTANCE_RENDERER:-venus}"
else
    DISPLAY_SMOLFILE="${SMOLVM_DISPLAY_ACCEPTANCE_SMOLFILE:-$PROJECT_ROOT/profiles/desktop-wayland/Smolfile}"
    DISPLAY_RENDERER="${SMOLVM_DISPLAY_ACCEPTANCE_RENDERER:-auto}"
fi
DISPLAY_EXPECT_RENDERER_QUALIFICATION="${SMOLVM_DISPLAY_ACCEPTANCE_EXPECT_RENDERER_QUALIFICATION:-}"
DISPLAY_ACCEPTANCE_MACHINES=""

cleanup_display_acceptance() {
    "$SMOLVM" machine stop --name "$DISPLAY_MACHINE" 2>/dev/null || true
    "$SMOLVM" machine delete --name "$DISPLAY_MACHINE" -f 2>/dev/null || true
}

cleanup_all_display_acceptance() {
    local machine
    for machine in $DISPLAY_ACCEPTANCE_MACHINES; do
        "$SMOLVM" machine stop --name "$machine" 2>/dev/null || true
        "$SMOLVM" machine delete --name "$machine" -f 2>/dev/null || true
    done
}
trap cleanup_all_display_acceptance EXIT

select_display_machine_for_transport() {
    local transport="$1"
    if [[ -n "${SMOLVM_DISPLAY_ACCEPTANCE_MACHINE:-}" ]]; then
        DISPLAY_MACHINE="$DISPLAY_MACHINE_BASE"
    else
        DISPLAY_MACHINE="$DISPLAY_MACHINE_BASE-${transport//[^A-Za-z0-9]/-}"
    fi
    case " $DISPLAY_ACCEPTANCE_MACHINES " in
        *" $DISPLAY_MACHINE "*) ;;
        *) DISPLAY_ACCEPTANCE_MACHINES="$DISPLAY_ACCEPTANCE_MACHINES $DISPLAY_MACHINE" ;;
    esac
}

echo ""
echo "=========================================="
echo "  smolvm Native Display Acceptance"
echo "=========================================="
echo ""

require_python3() {
    command -v python3 >/dev/null 2>&1 || {
        echo "python3 is required for JSON/RFB acceptance checks"
        return 1
    }
}

json_field() {
    local path="$1"
    local input
    input=$(cat)
    JSON_INPUT="$input" \
    python3 - "$path" <<'PY'
import json
import os
import sys

path = sys.argv[1].split(".")
data = json.loads(os.environ["JSON_INPUT"])
for key in path:
    data = data[key]
if isinstance(data, bool):
    print("true" if data else "false")
elif data is None:
    print("null")
else:
    print(data)
PY
}

wait_display_ready() {
    local deadline=$((SECONDS + 180))
    while [[ $SECONDS -lt $deadline ]]; do
        local status
        status=$("$SMOLVM" machine graphics status --name "$DISPLAY_MACHINE" --json 2>/dev/null) || {
            sleep 1
            continue
        }
        if [[ "$(printf '%s' "$status" | json_field display_ready)" == "true" ]]; then
            return 0
        fi
        sleep 1
    done
    echo "display_ready did not become true before timeout"
    return 1
}

assert_no_endpoint_secret_in_inventory() {
    local endpoint inventory status
    endpoint=$("$SMOLVM" machine display --name "$DISPLAY_MACHINE" --json)
    inventory=$("$SMOLVM" machine ls --json)
    status=$("$SMOLVM" machine graphics status --name "$DISPLAY_MACHINE" --json)

    ENDPOINT_JSON="$endpoint" INVENTORY_JSON="$inventory" STATUS_JSON="$status" python3 - <<'PY'
import json
import os

endpoint = json.loads(os.environ["ENDPOINT_JSON"])
inventory = os.environ["INVENTORY_JSON"]
status = os.environ["STATUS_JSON"]

sensitive = []
for key in ("password", "control_socket", "frame_ring"):
    value = endpoint.get(key)
    if isinstance(value, str) and value:
        sensitive.append((key, value))

if endpoint.get("protocol") == "vnc" and not endpoint.get("password"):
    raise SystemExit("display endpoint password was empty")
if endpoint.get("protocol") == "local-shm":
    if not endpoint.get("control_socket") or not endpoint.get("frame_ring"):
        raise SystemExit("local-shm endpoint was missing control_socket or frame_ring")

for key, value in sensitive:
    if value in inventory:
        raise SystemExit(f"machine ls --json leaked display endpoint {key}")
    if value in status:
        raise SystemExit(f"machine graphics status --json leaked display endpoint {key}")
PY
}

assert_graphics_status_ready() {
    local expected_transport="$1"
    local deadline=$((SECONDS + 60))
    local status
    local last_error="graphics status was not checked"
    local last_status=""

    while [[ $SECONDS -lt $deadline ]]; do
        status=$("$SMOLVM" machine graphics status --name "$DISPLAY_MACHINE" --json 2>/dev/null) || {
            last_error="machine graphics status --json failed"
            sleep 1
            continue
        }
        last_status="$status"
        if last_error=$(
            STATUS_JSON="$status" \
            EXPECTED_MACHINE="$DISPLAY_MACHINE" \
            EXPECTED_TRANSPORT="$expected_transport" \
            EXPECTED_RENDERER="$DISPLAY_RENDERER" \
            EXPECTED_RENDERER_QUALIFICATION="$DISPLAY_EXPECT_RENDERER_QUALIFICATION" \
            REQUIRE_ACCELERATED="$DISPLAY_REQUIRE_ACCELERATED" \
            python3 - 2>&1 >/dev/null <<'PY'
import json
import os
import sys

status = json.loads(os.environ["STATUS_JSON"])
expected_machine = os.environ["EXPECTED_MACHINE"]
expected_transport = os.environ["EXPECTED_TRANSPORT"]
expected_renderer = os.environ["EXPECTED_RENDERER"]
expected_renderer_qualification = os.environ["EXPECTED_RENDERER_QUALIFICATION"]
require_accelerated = os.environ["REQUIRE_ACCELERATED"] == "1"


def expect(field, expected):
    actual = status.get(field)
    if actual != expected:
        raise SystemExit(f"{field} expected {expected!r}, got {actual!r}")


def expect_true(field):
    expect(field, True)


def expect_false(field):
    expect(field, False)


expect("schema", "smolvm-graphics-status-v1")
expect("machine", expected_machine)
expect("state", "running")
expect("session_state", "ready")
expect_true("display_requested")
expect_true("display_ready")
expect_true("client_attach_supported")
expect_false("hot_attach_supported")
expect_false("restart_required_for_display")
expect_true("gpu_requested")
expect_true("gpu_ready")
expect_true("guest_dri_ready")
guest_render_node_ready = status.get("guest_render_node_ready")
if guest_render_node_ready not in (True, False, None):
    raise SystemExit(
        f"unexpected guest_render_node_ready {guest_render_node_ready!r}"
    )
expect_true("input_ready")
expect_true("seat_ready")
expect_true("compositor_ready")
expect("transport", expected_transport)
expect_true("transport_supported")
expect_true("transport_ready")

renderer_requested = status.get("renderer_requested")
if renderer_requested not in {"auto", "software", "virgl", "venus", "native-context"}:
    raise SystemExit(f"unexpected renderer_requested {renderer_requested!r}")
if expected_renderer and renderer_requested != expected_renderer:
    raise SystemExit(
        f"renderer_requested expected {expected_renderer!r}, got {renderer_requested!r}"
    )
renderer = status.get("renderer")
if renderer not in {
    "software",
    "virtio_gpu_requested",
    "virgl_requested",
    "venus_requested",
    "native_context_requested",
}:
    raise SystemExit(f"unexpected renderer {renderer!r}")
renderer_detail = status.get("renderer_detail")
if not isinstance(renderer_detail, str) or not renderer_detail.strip():
    raise SystemExit(f"renderer_detail was not populated: {renderer_detail!r}")

renderer_qualification = status.get("renderer_qualification")
if renderer_qualification not in {"not_requested", "unproven", "matched", "degraded"}:
    raise SystemExit(f"unexpected renderer_qualification {renderer_qualification!r}")
if expected_renderer_qualification and renderer_qualification != expected_renderer_qualification:
    raise SystemExit(
        "renderer_qualification expected "
        f"{expected_renderer_qualification!r}, got {renderer_qualification!r}"
    )

api_requested = status.get("api_requested")
api = status.get("api")
if api_requested not in {"none", "open_gl", "vulkan", "unknown"}:
    raise SystemExit(f"unexpected api_requested {api_requested!r}")
if api not in {"none", "open_gl", "vulkan", "unknown"}:
    raise SystemExit(f"unexpected api {api!r}")
api_detail = status.get("api_detail")
if api_detail is not None and (not isinstance(api_detail, str) or not api_detail.strip()):
    raise SystemExit(f"api_detail was empty when present: {api_detail!r}")
api_probe_failure_detail = status.get("api_probe_failure_detail")
if api_probe_failure_detail is not None and (
    not isinstance(api_probe_failure_detail, str) or not api_probe_failure_detail.strip()
):
    raise SystemExit(
        f"api_probe_failure_detail was empty when present: {api_probe_failure_detail!r}"
    )

if require_accelerated:
    if guest_render_node_ready is not True:
        raise SystemExit(
            "accelerated renderer required, but guest_render_node_ready was "
            f"{guest_render_node_ready!r}"
        )
    if renderer_qualification != "matched":
        raise SystemExit(
            f"accelerated renderer required, got qualification {renderer_qualification!r}: "
            f"{status.get('renderer_qualification_detail') or 'no detail'}"
        )
    if api_detail is None:
        raise SystemExit("accelerated renderer required, but api_detail was unavailable")
    lower_api_detail = api_detail.lower()
    software_markers = (
        "llvmpipe",
        "lavapipe",
        "softpipe",
        "swrast",
        "software rasterizer",
        "software renderer",
    )
    if any(marker in lower_api_detail for marker in software_markers):
        raise SystemExit(f"accelerated renderer required, got software api_detail {api_detail!r}")
    if expected_renderer == "venus" and api != "vulkan":
        raise SystemExit(f"venus renderer required Vulkan proof, got api {api!r}")
    if expected_renderer == "virgl" and api != "open_gl":
        raise SystemExit(f"virgl renderer required OpenGL proof, got api {api!r}")

print(
    f"renderer_qualification={renderer_qualification} "
    f"api_detail={api_detail or 'unknown'} "
    f"api_probe_failure_detail={api_probe_failure_detail or 'unknown'}",
    file=sys.stderr,
)

expect_false("pointer_capture_supported")
expect_false("gamepad_supported")
expect_false("gamepad_ready")
expect_false("audio_supported")
expect_false("audio_ready")
if expected_transport == "local-shm":
    expect_true("relative_pointer_supported")
else:
    expect_false("relative_pointer_supported")
PY
        ); then
            echo "graphics status proved GPU/display readiness for transport=$expected_transport ($last_error)"
            return 0
        fi
        sleep 1
    done

    echo "graphics status did not prove GPU/display readiness before timeout"
    echo "$last_error"
    if [[ -n "$last_status" ]]; then
        printf '%s\n' "$last_status"
    fi
    return 1
}

rfb_probe_endpoint() {
    local endpoint
    endpoint=$("$SMOLVM" machine display --name "$DISPLAY_MACHINE" --json)
    ENDPOINT_JSON="$endpoint" python3 - <<'PY'
import json
import os
import socket
import struct
import subprocess
import sys


def read_exact(sock, length):
    data = bytearray()
    while len(data) < length:
        chunk = sock.recv(length - len(data))
        if not chunk:
            raise RuntimeError(f"connection closed while reading {length} bytes")
        data.extend(chunk)
    return bytes(data)


def reverse_bits(byte):
    return int(f"{byte:08b}"[::-1], 2)


def vnc_auth_response(password, challenge):
    key = (password.encode("latin1")[:8]).ljust(8, b"\0")
    key = bytes(reverse_bits(byte) for byte in key)
    key_hex = key.hex()
    candidates = [
        ["openssl", "enc", "-des-ecb", "-nopad", "-K", key_hex],
        ["openssl", "enc", "-des-ecb", "-nopad", "-K", key_hex, "-provider", "legacy", "-provider", "default"],
    ]
    for command in candidates:
        try:
            result = subprocess.run(
                command,
                input=challenge,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                check=True,
            )
            return result.stdout
        except (FileNotFoundError, subprocess.CalledProcessError):
            continue
    raise RuntimeError("openssl DES-ECB support is required for VNC authentication")


def parse_pixel_format(data):
    bits_per_pixel, depth, big_endian, true_color = struct.unpack(">BBBB", data[:4])
    red_max, green_max, blue_max, red_shift, green_shift, blue_shift = struct.unpack(">HHHBBB", data[4:13])
    return {
        "bits_per_pixel": bits_per_pixel,
        "bytes_per_pixel": bits_per_pixel // 8,
        "big_endian": bool(big_endian),
        "true_color": bool(true_color),
        "red_max": red_max,
        "green_max": green_max,
        "blue_max": blue_max,
        "red_shift": red_shift,
        "green_shift": green_shift,
        "blue_shift": blue_shift,
    }


def channel(value, shift, maximum):
    if maximum == 0:
        return 0
    return ((value >> shift) & maximum) * 255 // maximum


def has_non_black_pixel(rectangle, pixel_format):
    bytes_per_pixel = pixel_format["bytes_per_pixel"]
    if bytes_per_pixel not in (2, 4) or not pixel_format["true_color"]:
        return any(byte != 0 for byte in rectangle)
    endian = "big" if pixel_format["big_endian"] else "little"
    limit = min(len(rectangle), 4096 * bytes_per_pixel)
    for offset in range(0, limit - bytes_per_pixel + 1, bytes_per_pixel):
        value = int.from_bytes(rectangle[offset:offset + bytes_per_pixel], endian)
        red = channel(value, pixel_format["red_shift"], pixel_format["red_max"])
        green = channel(value, pixel_format["green_shift"], pixel_format["green_max"])
        blue = channel(value, pixel_format["blue_shift"], pixel_format["blue_max"])
        if red or green or blue:
            return True
    return False


endpoint = json.loads(os.environ["ENDPOINT_JSON"])
if endpoint.get("protocol") != "vnc":
    raise RuntimeError(f"unsupported protocol: {endpoint.get('protocol')}")
if endpoint.get("host") != "127.0.0.1":
    raise RuntimeError(f"endpoint is not loopback: {endpoint.get('host')}")

with socket.create_connection((endpoint["host"], int(endpoint["port"])), timeout=10) as sock:
    sock.settimeout(20)
    version = read_exact(sock, 12)
    sock.sendall(version)

    security_type_count = read_exact(sock, 1)[0]
    if security_type_count == 0:
        reason_len = struct.unpack(">I", read_exact(sock, 4))[0]
        reason = read_exact(sock, reason_len).decode("utf-8", "replace")
        raise RuntimeError(f"server rejected security negotiation: {reason}")
    security_types = read_exact(sock, security_type_count)
    if 2 in security_types:
        sock.sendall(b"\x02")
        challenge = read_exact(sock, 16)
        sock.sendall(vnc_auth_response(endpoint["password"], challenge))
    elif 1 in security_types:
        sock.sendall(b"\x01")
    else:
        raise RuntimeError(f"server offered unsupported security types: {list(security_types)}")

    security_result = struct.unpack(">I", read_exact(sock, 4))[0]
    if security_result != 0:
        raise RuntimeError(f"RFB authentication failed with status {security_result}")

    sock.sendall(b"\x01")
    width, height = struct.unpack(">HH", read_exact(sock, 4))
    pixel_format = parse_pixel_format(read_exact(sock, 16))
    name_len = struct.unpack(">I", read_exact(sock, 4))[0]
    _name = read_exact(sock, name_len)

    if width == 0 or height == 0:
        raise RuntimeError(f"invalid framebuffer size {width}x{height}")

    sock.sendall(struct.pack(">BBHi", 2, 0, 1, 0))  # SetEncodings: Raw only.
    sock.sendall(struct.pack(">BBHHHH", 3, 0, 0, 0, width, height))

    saw_non_black = False
    for _ in range(8):
        message_type = read_exact(sock, 1)[0]
        if message_type != 0:
            continue
        _padding = read_exact(sock, 1)
        rectangle_count = struct.unpack(">H", read_exact(sock, 2))[0]
        for _ in range(rectangle_count):
            x, y, rect_width, rect_height, encoding = struct.unpack(">HHHHi", read_exact(sock, 12))
            if encoding != 0:
                raise RuntimeError(f"unexpected non-raw encoding {encoding} at {x},{y}")
            length = rect_width * rect_height * pixel_format["bytes_per_pixel"]
            rectangle = read_exact(sock, length)
            saw_non_black = saw_non_black or has_non_black_pixel(rectangle, pixel_format)
        if saw_non_black:
            break
        sock.sendall(struct.pack(">BBHHHH", 3, 1, 0, 0, width, height))

    if not saw_non_black:
        raise RuntimeError("RFB framebuffer stayed black across update requests")

    key = 0x61
    center_x = max(0, width // 2)
    center_y = max(0, height // 2)
    sock.sendall(struct.pack(">BBHI", 4, 1, 0, key))
    sock.sendall(struct.pack(">BBHI", 4, 0, 0, key))
    sock.sendall(struct.pack(">BBHH", 5, 1, center_x, center_y))
    sock.sendall(struct.pack(">BBHH", 5, 0, center_x, center_y))

print("RFB endpoint accepted auth, produced a non-black frame, and accepted key/pointer events")
PY
}

local_shm_probe_endpoint() {
    local endpoint
    endpoint=$("$SMOLVM" machine display --name "$DISPLAY_MACHINE" --json)
    ENDPOINT_JSON="$endpoint" python3 - <<'PY'
import json
import os
import socket
import struct
import sys
import time

RING_MAGIC = b"LSHMRG1\0"
SLOT_MAGIC = b"LSHMSL1\0"
PROTOCOL_VERSION = 1
PIXEL_FORMAT_RGBA8888 = 1


def read_u32(data, offset):
    return struct.unpack_from("<I", data, offset)[0]


def read_u64(data, offset):
    return struct.unpack_from("<Q", data, offset)[0]


def load_non_black_frame(path, deadline):
    last_error = None
    while time.monotonic() < deadline:
        try:
            with open(path, "rb") as handle:
                data = handle.read()
            if len(data) < 256:
                raise RuntimeError("frame ring shorter than header")
            if data[0:8] != RING_MAGIC:
                raise RuntimeError("invalid frame ring magic")
            version = read_u32(data, 8)
            slots = read_u32(data, 12)
            header_bytes = read_u64(data, 16)
            slot_header_bytes = read_u64(data, 24)
            slot_stride = read_u64(data, 32)
            payload_capacity = read_u64(data, 40)
            latest_frame_id = read_u64(data, 48)
            latest_slot = read_u32(data, 56)
            if version != PROTOCOL_VERSION:
                raise RuntimeError(f"unsupported local-shm version {version}")
            if slots != 3 or header_bytes != 256 or slot_header_bytes != 128:
                raise RuntimeError("unexpected local-shm frame ring layout")
            if payload_capacity != slot_stride - slot_header_bytes:
                raise RuntimeError("local-shm payload capacity did not match slot stride")
            if latest_frame_id == 0:
                time.sleep(0.25)
                continue
            if latest_slot >= slots:
                raise RuntimeError(f"invalid latest slot {latest_slot}")
            slot = header_bytes + latest_slot * slot_stride
            if slot + slot_header_bytes > len(data):
                raise RuntimeError("published local-shm slot exceeds file length")
            if data[slot:slot + 8] != SLOT_MAGIC:
                raise RuntimeError("invalid local-shm slot magic")
            if read_u32(data, slot + 8) != PROTOCOL_VERSION:
                raise RuntimeError("invalid local-shm slot version")
            if read_u32(data, slot + 12) != latest_slot:
                raise RuntimeError("local-shm slot index mismatch")
            if read_u64(data, slot + 16) != latest_frame_id:
                raise RuntimeError("local-shm frame id mismatch")
            width = read_u32(data, slot + 32)
            height = read_u32(data, slot + 36)
            stride = read_u32(data, slot + 40)
            pixel_format = read_u32(data, slot + 44)
            payload_len = read_u64(data, slot + 64)
            if not width or not height or stride < width * 4:
                raise RuntimeError(f"invalid local-shm dimensions {width}x{height} stride {stride}")
            if pixel_format != PIXEL_FORMAT_RGBA8888:
                raise RuntimeError(f"unsupported local-shm pixel format {pixel_format}")
            minimum_payload = (height - 1) * stride + width * 4
            if payload_len < minimum_payload or payload_len > payload_capacity:
                raise RuntimeError("invalid local-shm payload length")
            payload_start = slot + slot_header_bytes
            payload_end = payload_start + payload_len
            if payload_end > len(data):
                raise RuntimeError("local-shm payload exceeds file length")
            payload = data[payload_start:payload_end]
            scan_limit = min(len(payload), 4096 * 4)
            for offset in range(0, scan_limit - 3, 4):
                if payload[offset] or payload[offset + 1] or payload[offset + 2]:
                    return width, height
            last_error = "local-shm frame was black"
        except OSError as error:
            last_error = str(error)
        time.sleep(0.25)
    raise RuntimeError(last_error or "local-shm frame ring did not publish a non-black frame")


endpoint = json.loads(os.environ["ENDPOINT_JSON"])
if endpoint.get("protocol") != "local-shm":
    raise RuntimeError(f"unsupported protocol: {endpoint.get('protocol')}")
if endpoint.get("version") != PROTOCOL_VERSION:
    raise RuntimeError(f"unsupported endpoint version: {endpoint.get('version')}")

control_socket = endpoint.get("control_socket")
frame_ring = endpoint.get("frame_ring")
if not control_socket or not frame_ring:
    raise RuntimeError("local-shm endpoint omitted control_socket or frame_ring")
if os.path.basename(control_socket) != "local-shm.sock":
    raise RuntimeError(f"unexpected control socket path: {control_socket}")
if os.path.basename(frame_ring) != "local-shm.frames":
    raise RuntimeError(f"unexpected frame ring path: {frame_ring}")
if os.path.dirname(control_socket) != os.path.dirname(frame_ring):
    raise RuntimeError("control socket and frame ring are not in the same runtime directory")

width, height = load_non_black_frame(frame_ring, time.monotonic() + 30)
center_x = max(0, width // 2)
center_y = max(0, height // 2)
messages = [
    {"type": "key", "keysym": 0x61, "down": True},
    {"type": "key", "keysym": 0x61, "down": False},
    {"type": "pointer", "x": center_x, "y": center_y, "buttons": 1},
    {"type": "pointer", "x": center_x, "y": center_y, "buttons": 0},
    {"type": "release_all"},
]
with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as sock:
    sock.settimeout(10)
    sock.connect(control_socket)
    for message in messages:
        sock.sendall(json.dumps(message, separators=(",", ":")).encode("utf-8") + b"\n")

print("local-shm endpoint produced a non-black frame and accepted key/pointer control messages")
PY
}

create_display_fixture() {
    local transport="$1"
    local create_args=(
        machine create
        --name "$DISPLAY_MACHINE"
        --graphics
        --renderer "$DISPLAY_RENDERER"
        -s "$DISPLAY_SMOLFILE"
    )
    if [[ "$transport" != "rfb" ]]; then
        create_args+=(--transport "$transport")
    fi

    run_with_timeout 300 "$SMOLVM" "${create_args[@]}" 2>&1 || return 1
}

start_and_assert_graphics_fixture() {
    run_with_timeout 600 "$SMOLVM" machine start \
        --name "$DISPLAY_MACHINE" 2>&1 || return 1

    wait_vm_ready --name "$DISPLAY_MACHINE" 180 || {
        echo "machine agent did not become ready"
        return 1
    }
    wait_display_ready || return 1
    assert_graphics_status_ready "$1" || return 1

    "$SMOLVM" machine exec --name "$DISPLAY_MACHINE" -- \
        sh -lc 'test -e /dev/dri/card0 || test -e /dev/dri/renderD128' 2>&1 || {
        echo "guest did not expose /dev/dri"
        return 1
    }
    "$SMOLVM" machine exec --name "$DISPLAY_MACHINE" -- \
        sh -lc 'test -d /dev/input && find /dev/input -name "event*" | grep -q .' 2>&1 || {
        echo "guest did not expose input event devices"
        return 1
    }
    "$SMOLVM" machine exec --name "$DISPLAY_MACHINE" -- \
        sh -lc 'test -S /run/seatd.sock' 2>&1 || {
        echo "guest did not expose seatd socket"
        return 1
    }
}

assert_endpoint_removed_after_stop() {
    "$SMOLVM" machine stop --name "$DISPLAY_MACHINE" 2>&1 || return 1
    if "$SMOLVM" machine display --name "$DISPLAY_MACHINE" --json >/tmp/smolvm-display-after-stop.$$ 2>&1; then
        cat /tmp/smolvm-display-after-stop.$$
        rm -f /tmp/smolvm-display-after-stop.$$
        echo "display endpoint remained readable after stop"
        return 1
    fi
    rm -f /tmp/smolvm-display-after-stop.$$
}

test_native_display_weston_rfb_fixture() {
    require_python3 || return 1
    [[ -f "$DISPLAY_SMOLFILE" ]] || {
        echo "missing Weston Smolfile: $DISPLAY_SMOLFILE"
        return 1
    }

    select_display_machine_for_transport rfb
    cleanup_display_acceptance

    create_display_fixture rfb || return 1
    start_and_assert_graphics_fixture rfb || return 1
    assert_no_endpoint_secret_in_inventory || return 1
    rfb_probe_endpoint || return 1
    assert_endpoint_removed_after_stop || return 1
}

test_native_display_weston_local_shm_fixture() {
    require_python3 || return 1
    [[ -f "$DISPLAY_SMOLFILE" ]] || {
        echo "missing Weston Smolfile: $DISPLAY_SMOLFILE"
        return 1
    }

    select_display_machine_for_transport local-shm
    cleanup_display_acceptance

    create_display_fixture local-shm || return 1
    start_and_assert_graphics_fixture local-shm || return 1
    assert_no_endpoint_secret_in_inventory || return 1
    local_shm_probe_endpoint || return 1
    assert_endpoint_removed_after_stop || return 1
}

run_test "Native display: Weston fixture exposes graphics, RFB, input, and cleanup" \
    test_native_display_weston_rfb_fixture || true

run_test "Native display: Weston fixture exposes graphics, local-shm frames, input, and cleanup" \
    test_native_display_weston_local_shm_fixture || true

print_summary "Native Display Acceptance"
