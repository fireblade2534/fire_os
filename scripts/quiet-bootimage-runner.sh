#!/usr/bin/env bash

set -euo pipefail

args=()

for arg in "$@"; do
    case "$arg" in
        --quiet|-q)
            # Cargo adds this to normal harnessed tests.
            # Do not let it reach QEMU.
            ;;
        *)
            args+=("$arg")
            ;;
    esac
done

exec bootimage runner --quiet "${args[@]}"