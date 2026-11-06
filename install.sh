#!/bin/sh
set -eu

project_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
install_dir="${XDG_BIN_HOME:-$HOME/.local/bin}"
command_path="$install_dir/gsbuilder"

if [ -d "$HOME/.cargo/bin" ]; then
    PATH="$HOME/.cargo/bin:$PATH"
    export PATH
fi

if [ -f "$project_dir/Cargo.toml" ] &&
    { ! command -v cargo >/dev/null 2>&1 || ! command -v rustc >/dev/null 2>&1; }; then
    rustup_installer=$(mktemp "${TMPDIR:-/tmp}/gsbuilder-rustup.XXXXXX")
    trap 'rm -f "$rustup_installer"' 0

    if command -v curl >/dev/null 2>&1; then
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs -o "$rustup_installer"
    elif command -v wget >/dev/null 2>&1; then
        wget -q https://sh.rustup.rs -O "$rustup_installer"
    else
        printf '%s\n' 'Rust/Cargo is not installed. Install curl or wget to download Rust.' >&2
        exit 1
    fi

    printf '%s\n' 'Installing Rust/Cargo with the official rustup installer...'
    sh "$rustup_installer" -y --profile minimal
    PATH="$HOME/.cargo/bin:$PATH"
    export PATH
    if ! command -v cargo >/dev/null 2>&1 || ! command -v rustc >/dev/null 2>&1; then
        printf '%s\n' 'Rust installation finished, but cargo or rustc was not found in ~/.cargo/bin.' >&2
        exit 1
    fi
fi

mkdir -p "$install_dir"
if [ -e "$command_path" ] || [ -L "$command_path" ]; then
    if [ -L "$command_path" ] && [ "$(readlink -f -- "$command_path")" = "$(readlink -f -- "$project_dir/start.sh")" ]; then
        printf 'GSBuilder command is already installed: %s\n' "$command_path"
    else
        printf 'Refusing to overwrite existing path: %s\n' "$command_path" >&2
        exit 1
    fi
else
    ln -s "$project_dir/start.sh" "$command_path"
    printf 'Installed gsbuilder command: %s\n' "$command_path"
fi

case "${SHELL-}" in
    */bash|bash) shell_config="$HOME/.bashrc" ;;
    */zsh|zsh) shell_config="$HOME/.zshrc" ;;
    *) shell_config="$HOME/.profile" ;;
esac

if ! grep -Fq '# GSBuilder command path' "$shell_config" 2>/dev/null; then
    escaped_install_dir=$(printf '%s' "$install_dir" | sed 's/[\\"]/\\&/g')
    {
        printf '\n# GSBuilder command path\n'
        printf 'export PATH="%s:$PATH"\n' "$escaped_install_dir"
    } >> "$shell_config"
    printf 'Added %s to PATH in %s\n' "$install_dir" "$shell_config"
else
    printf 'PATH setup already exists in %s\n' "$shell_config"
fi

case ":$PATH:" in
    *":$install_dir:"*) ;;
    *)
        printf 'To use "gsbuilder" in this terminal now, run:\n  . "%s"\n' "$shell_config"
        ;;
esac
