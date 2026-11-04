#!/bin/sh
set -eu

script_path=$(readlink -f -- "$0")
project_dir=$(CDPATH= cd -- "$(dirname -- "$script_path")" && pwd)
if [ -x "$project_dir/GSBuilder" ]; then
    binary="$project_dir/GSBuilder"
else
    binary="$project_dir/target/release/GSBuilder"
fi

if [ ! -x "$binary" ]; then
    if ! command -v cargo >/dev/null 2>&1; then
        printf '%s\n' "GSBuilder is not built yet. Install Rust/Cargo, or download a release build." >&2
        exit 1
    fi
    (cd "$project_dir" && cargo build --release)
fi

if [ -z "${GSBUILDER_CONFIG:-}" ]; then
    export GSBUILDER_CONFIG="$project_dir/config.toml"
fi

exec "$binary" "$@"
