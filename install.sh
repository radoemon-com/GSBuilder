#!/bin/sh
set -eu

project_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
install_dir="${XDG_BIN_HOME:-$HOME/.local/bin}"
command_path="$install_dir/gsbuilder"
install_step="インストールの準備"
rustup_installer=""

finish_install() {
    result=$1
    if [ -n "$rustup_installer" ]; then
        rm -f "$rustup_installer" || :
    fi
    if [ "$result" -eq 0 ]; then
        printf '\n[完了] GSBuilder のインストールが完了しました。\n'
        printf '起動コマンド: gsbuilder\n'
    else
        printf '\n[失敗] %s でエラーが発生しました (終了コード: %s)。\n' \
            "$install_step" "$result" >&2
        printf '%s\n' '上記のエラーを確認してから ./install.sh を再実行してください。' >&2
    fi
    exit "$result"
}
trap 'finish_install $?' 0
trap 'exit 130' INT
trap 'exit 143' TERM

printf '[1/4] インストール環境を確認しています...\n'

if [ -d "$HOME/.cargo/bin" ]; then
    PATH="$HOME/.cargo/bin:$PATH"
    export PATH
fi

if [ -f "$project_dir/Cargo.toml" ] &&
    { ! command -v cargo >/dev/null 2>&1 || ! command -v rustc >/dev/null 2>&1; }; then
    install_step="Rust/Cargo インストーラーの準備"
    rustup_installer=$(mktemp "${TMPDIR:-/tmp}/gsbuilder-rustup.XXXXXX")

    if command -v curl >/dev/null 2>&1; then
        printf 'Rust/Cargo が見つからないため、公式インストーラーをダウンロードしています...\n'
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs -o "$rustup_installer"
    elif command -v wget >/dev/null 2>&1; then
        printf 'Rust/Cargo が見つからないため、公式インストーラーをダウンロードしています...\n'
        wget -q https://sh.rustup.rs -O "$rustup_installer"
    else
        printf '%s\n' 'Rust/Cargo がなく、ダウンロード用の curl または wget も見つかりません。' >&2
        exit 1
    fi

    install_step="Rust/Cargo のインストール"
    printf '%s\n' 'Rust/Cargo をインストールしています...'
    sh "$rustup_installer" -y --profile minimal
    PATH="$HOME/.cargo/bin:$PATH"
    export PATH
    if ! command -v cargo >/dev/null 2>&1 || ! command -v rustc >/dev/null 2>&1; then
        printf '%s\n' 'Rust のインストール後も ~/.cargo/bin に cargo または rustc が見つかりません。' >&2
        exit 1
    fi
fi

install_step="インストール先の準備"
printf '[2/4] インストール先を準備しています: %s\n' "$install_dir"
mkdir -p "$install_dir"

install_step="起動コマンドの設定"
printf '[3/4] 起動コマンドを設定しています...\n'
if [ -e "$command_path" ] || [ -L "$command_path" ]; then
    if [ -L "$command_path" ] && [ "$(readlink -f -- "$command_path")" = "$(readlink -f -- "$project_dir/start.sh")" ]; then
        printf '起動コマンドはすでに設定されています: %s\n' "$command_path"
    else
        printf '既存のパスを上書きできません: %s\n' "$command_path" >&2
        exit 1
    fi
else
    ln -s "$project_dir/start.sh" "$command_path"
    printf '起動コマンドを作成しました: %s\n' "$command_path"
fi

install_step="シェルの PATH 設定"
printf '[4/4] シェルの PATH を確認しています...\n'
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
    printf 'PATH に %s を追加しました: %s\n' "$install_dir" "$shell_config"
else
    printf 'PATH 設定はすでにあります: %s\n' "$shell_config"
fi

case ":$PATH:" in
    *":$install_dir:"*)
        printf 'このターミナルですぐに起動できます。\n'
        ;;
    *)
        printf 'このターミナルで使うには、次を実行してください:\n  . "%s"\n' "$shell_config"
        ;;
esac
