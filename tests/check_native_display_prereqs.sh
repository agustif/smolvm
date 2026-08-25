#!/bin/bash
#
# Host prerequisite check for the opt-in native display acceptance suite.
#
# This does not boot a VM. It verifies the local host has the cheap, static
# pieces needed before test_native_display.sh spends time creating a graphics
# machine and pulling guest packages.

source "$(dirname "$0")/common.sh"

if [[ -z "${SMOLVM_DISPLAY_ACCEPTANCE_SMOLFILE:-}" && "${SMOLVM_DISPLAY_ACCEPTANCE_REQUIRE_ACCELERATED:-0}" == "1" ]]; then
    DISPLAY_SMOLFILE="$PROJECT_ROOT/profiles/fedora-venus/Smolfile"
    MIN_FREE_GIB="${SMOLVM_DISPLAY_ACCEPTANCE_MIN_FREE_GIB:-28}"
else
    DISPLAY_SMOLFILE="${SMOLVM_DISPLAY_ACCEPTANCE_SMOLFILE:-$PROJECT_ROOT/profiles/desktop-wayland/Smolfile}"
    MIN_FREE_GIB="${SMOLVM_DISPLAY_ACCEPTANCE_MIN_FREE_GIB:-20}"
fi

failures=()
warnings=()

add_failure() {
    failures+=("$1")
}

add_warning() {
    warnings+=("$1")
}

check_executable() {
    local name="$1"
    command -v "$name" >/dev/null 2>&1 || add_failure "missing executable: $name"
}

check_openssl_des() {
    command -v openssl >/dev/null 2>&1 || {
        add_failure "missing executable: openssl"
        return
    }

    if printf '12345678' | openssl enc -des-ecb -nopad \
        -K 0000000000000000 >/dev/null 2>&1; then
        return
    fi
    if printf '12345678' | openssl enc -des-ecb -nopad \
        -K 0000000000000000 \
        -provider legacy -provider default >/dev/null 2>&1; then
        return
    fi
    add_failure "openssl is present but DES-ECB support is unavailable for RFB authentication"
}

check_smolvm_binary() {
    local candidate
    candidate="$(find_smolvm)"
    if [[ -z "$candidate" || ! -x "$candidate" ]]; then
        add_failure "missing executable smolvm binary; build it or set SMOLVM=/path/to/smolvm"
        return
    fi
    echo "smolvm: $candidate"
}

check_display_bundle_files() {
    if [[ "$(uname -s)" == "Darwin" ]]; then
        [[ -f "$PROJECT_ROOT/lib/libkrun.dylib" ]] \
            || add_failure "missing display lib: $PROJECT_ROOT/lib/libkrun.dylib"
        [[ -f "$PROJECT_ROOT/lib/libkrunfw.5.dylib" ]] \
            || add_failure "missing display firmware: $PROJECT_ROOT/lib/libkrunfw.5.dylib"
    else
        local arch
        arch="$(uname -m)"
        [[ -d "$PROJECT_ROOT/lib/linux-$arch" ]] \
            || add_warning "missing platform lib directory: $PROJECT_ROOT/lib/linux-$arch"
    fi

    if [[ -n "${SMOLVM_AGENT_ROOTFS:-}" ]]; then
        [[ -d "$SMOLVM_AGENT_ROOTFS" ]] \
            || add_failure "SMOLVM_AGENT_ROOTFS is set but not a directory: $SMOLVM_AGENT_ROOTFS"
    elif [[ -d "$PROJECT_ROOT/target/agent-rootfs" ]]; then
        :
    elif [[ -d "$PROJECT_ROOT/agent-rootfs" ]]; then
        :
    else
        add_failure "missing agent rootfs; set SMOLVM_AGENT_ROOTFS or build $PROJECT_ROOT/target/agent-rootfs"
    fi
}

check_free_space() {
    local available_kib
    available_kib="$(df -Pk "$PROJECT_ROOT" | awk 'NR == 2 { print $4 }')"
    if [[ -z "$available_kib" || ! "$available_kib" =~ ^[0-9]+$ ]]; then
        add_warning "could not determine free disk space for $PROJECT_ROOT"
        return
    fi

    local min_kib=$((MIN_FREE_GIB * 1024 * 1024))
    if (( available_kib < min_kib )); then
        add_failure "free disk space below ${MIN_FREE_GIB}GiB for graphics acceptance; available KiB: $available_kib"
    fi
}

echo ""
echo "=========================================="
echo "  smolvm Native Display Prerequisites"
echo "=========================================="
echo ""

check_executable python3
check_openssl_des
check_smolvm_binary
[[ -f "$DISPLAY_SMOLFILE" ]] \
    || add_failure "missing display acceptance Smolfile: $DISPLAY_SMOLFILE"
check_display_bundle_files
check_free_space

if [[ ${#warnings[@]} -gt 0 ]]; then
    echo "Warnings:"
    for warning in "${warnings[@]}"; do
        echo "  - $warning"
    done
    echo ""
fi

if [[ ${#failures[@]} -gt 0 ]]; then
    echo "Native display acceptance prerequisites failed:"
    for failure in "${failures[@]}"; do
        echo "  - $failure"
    done
    echo ""
    echo "The live display suite is opt-in because it boots graphics VMs and may pull packages."
    echo "Fix the prerequisites, then run:"
    echo "  SMOLVM_RUN_NATIVE_DISPLAY_ACCEPTANCE=1 ./tests/run_tests.sh display"
    exit 1
fi

echo "Native display acceptance prerequisites look usable."
