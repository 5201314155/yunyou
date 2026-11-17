#!/bin/sh
# 云悠（YunYou）一条命令全局安装
# 用法：curl -fsSL https://github.com/5201314155/yunyou/raw/main/install.sh | sh
set -e

REPO="https://github.com/5201314155/yunyou.git"
BIN_DIR="${YUNYOU_INSTALL_DIR:-$HOME/.local/bin}"

echo "☁ 云悠安装器"

# 1) 优先：下载官方预编译二进制（无需 Rust 工具链）
#    预编译包随 GitHub Release 发布（资产名：yunyou-<版本>-x86_64-linux.tar.gz）
if command -v curl >/dev/null 2>&1; then
  # Release 资产直链（github 开源发布面）
  GH="https://github.com/5201314155/yunyou/releases/latest/download/yunyou-v0.1.0-x86_64-linux.tar.gz"
  ASSET=""
  curl -fsSLI -o /dev/null "$GH" && ASSET="$GH"

  if [ -n "$ASSET" ]; then
    echo "→ 下载预编译二进制：$ASSET"
    TMP=$(mktemp -d)
    curl -fsSL "$ASSET" -o "$TMP/yunyou.tar.gz"
    tar -xzf "$TMP/yunyou.tar.gz" -C "$TMP"
    mkdir -p "$BIN_DIR"
    install -m 755 "$TMP/yunyou" "$BIN_DIR/yunyou"
    rm -rf "$TMP"
    echo "✓ 已安装到 $BIN_DIR/yunyou（预编译）"
    exit 0
  fi
fi

# 2) 回退：cargo install --git（需要 Rust 1.75+；源码直装，约 1-2 分钟）
if ! command -v cargo >/dev/null 2>&1; then
  echo "✗ 未找到预编译包，且本机无 cargo。"
  echo "  安装 Rust：curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
  echo "  或到 https://github.com/5201314155/yunyou/releases 手动下载二进制。"
  exit 1
fi
echo "→ 未见预编译包，走源码安装（cargo install --git，1-2 分钟）"
cargo install --git "$REPO" yunyou-cli

# 3) PATH 检查
case ":$PATH:" in
  *":$BIN_DIR:"*) ;;
  *)
    echo "⚠ $BIN_DIR 不在 PATH。已自动追加到 ~/.zshrc / ~/.bashrc"
    for RC in "$HOME/.zshrc" "$HOME/.bashrc"; do
      [ -f "$RC" ] && ! grep -q "$BIN_DIR" "$RC" && echo "export PATH=\"$BIN_DIR:\$PATH\"" >> "$RC"
    done
    echo "  新开终端或 source 对应 rc 生效。"
    ;;
esac

echo "✓ 完成。运行：yunyou"
