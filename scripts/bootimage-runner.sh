#!/usr/bin/env bash

set -euo pipefail

quiet=false
args=()

for arg in "$@"; do
    case "$arg" in
        --quiet|-q)
            quiet=true
            ;;
        *)
            args+=("$arg")
            ;;
    esac
done

if $quiet; then
    exec bootimage runner --quiet "${args[@]}"
else
    exec bootimage runner "${args[@]}"
fi