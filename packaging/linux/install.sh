#!/bin/sh
# rviewer をユーザー単位で Linux にインストールする（root 権限は不要）。
#   実行ファイル:       ~/.local/bin/rviewer
#   アイコン:           ~/.local/share/icons/rviewer.png
#   デスクトップエントリ: ~/.local/share/applications/rviewer.desktop
# 使い方: packaging/linux/install.sh            インストール（再実行で更新）
#         packaging/linux/install.sh --uninstall アンインストール
set -eu

PREFIX="${PREFIX:-$HOME/.local}"
DATA="${XDG_DATA_HOME:-$HOME/.local/share}"
BIN="$PREFIX/bin/rviewer"
ICON="$DATA/icons/rviewer.png"
DESKTOP="$DATA/applications/rviewer.desktop"
REPO="$(cd "$(dirname "$0")/../.." && pwd)"

refresh() {
    command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$DATA/applications" || true
}

if [ "${1:-}" = "--uninstall" ]; then
    cargo uninstall --root "$PREFIX" rview || true
    rm -f "$ICON" "$DESKTOP"
    refresh
    echo "アンインストールしました"
    exit 0
fi

cargo install --path "$REPO" --root "$PREFIX" --locked
mkdir -p "$(dirname "$ICON")" "$(dirname "$DESKTOP")"
cp "$REPO/assets/icon.png" "$ICON"
sed -e "s|@BIN@|$BIN|" -e "s|@ICON@|$ICON|" "$REPO/packaging/linux/rviewer.desktop" > "$DESKTOP"
refresh

echo "インストールしました: $BIN"
case ":$PATH:" in
    *":$PREFIX/bin:"*) ;;
    *) echo "注意: $PREFIX/bin が PATH に含まれていません。端末から rviewer で起動するには PATH に追加してください" ;;
esac
