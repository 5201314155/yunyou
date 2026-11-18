#!/bin/sh
# 云悠（YunYou）一条命令全局安装
# 用法：curl -fsSL https://github.com/5201314155/yunyou/raw/main/install.sh | sh
set -e

REPO="https://github.com/5201314155/yunyou.git"
BIN_DIR="${YUNYOU_INSTALL_DIR:-$HOME/.local/bin}"
HOME_DIR="${YUNYOU_HOME:-$HOME/.yunyou}"

echo "☁ 云悠安装器"

# 1) 主路径：release-bin 分支直拉预编译二进制（raw，免 Release 网页操作）
ARCH=$(uname -m)
case "$ARCH" in
  aarch64|arm64) BIN_NAME="yunyou-arm64.gz" ;;
  *) BIN_NAME="yunyou.gz" ;;
esac
GH="https://raw.githubusercontent.com/5201314155/yunyou/release-bin/$BIN_NAME"
ASSET=""
curl -fsSLI -o /dev/null "$GH" && ASSET="$GH"

install_binary() {
    TMP=$(mktemp -d)
    curl -fsSL "$GH" -o "$TMP/yunyou.gz"
    gunzip -f "$TMP/yunyou.gz"
    mkdir -p "$BIN_DIR"
    install -m 755 "$TMP/yunyou" "$BIN_DIR/yunyou"
    rm -rf "$TMP"
    echo "✓ 已安装到 $BIN_DIR/yunyou（预编译）"
}

if [ -n "$ASSET" ]; then
    echo "→ 下载预编译二进制：$ASSET"
    install_binary
elif command -v cargo >/dev/null 2>&1; then
    echo "→ 未见预编译包，走源码安装（cargo install --git，1-2 分钟）"
    cargo install --git "$REPO" yunyou-cli
else
    echo "✗ 未找到预编译包，且本机无 cargo。"
    echo "  安装 Rust：curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
    echo "  或到 https://github.com/5201314155/yunyou/releases 手动下载二进制。"
    exit 1
fi

# 2) PATH 配置（两安装路径都过这里）
case ":$PATH:" in
  *":$BIN_DIR:"*) ;;
  *)
    echo "⚠ $BIN_DIR 不在 PATH。已自动追加到 ~/.zshrc / ~/.bashrc"
    for RC in "$HOME/.zshrc" "$HOME/.bashrc"; do
      [ -f "$RC" ] && ! grep -q "$BIN_DIR" "$RC" && echo "export PATH=\"$BIN_DIR:\$PATH\"" >> "$RC"
    done
    echo "  新开终端或 source 对应 rc 生效（当前会话可执行：export PATH=\"$BIN_DIR:\$PATH\"）。"
    ;;
esac

# 3) 模型配置向导（交互终端才启用；已配置则跳过）
if [ -t 0 ] && [ ! -f "$HOME_DIR/yunyou.toml" ]; then
  mkdir -p "$HOME_DIR"
  chmod 700 "$HOME_DIR" 2>/dev/null || true
  echo ""
  echo "── 模型配置向导（回车=跳过，稍后手动配置 $HOME_DIR/yunyou.toml）"
  printf "模型端点（OpenAI 兼容 /v1 地址）: "
  read -r BASE_URL
  if [ -n "$BASE_URL" ]; then
    printf "API 密钥: "
    stty -echo 2>/dev/null || true
    read -r API_KEY
    stty echo 2>/dev/null || true
    printf "模型名（如 z-ai/glm-5.3-flash）: "
    read -r MODEL
    API_KEY="${API_KEY:-$MODEL}"
    MODEL="${MODEL:-z-ai/glm-5.3-flash}"
    cat > "$HOME_DIR/credentials.toml" << CREDS
YUNYOU_LLM_API_KEY = "$API_KEY"
CREDS
    chmod 600 "$HOME_DIR/credentials.toml"
    cat > "$HOME_DIR/yunyou.toml" << CONF
[llm]
base_url = "$BASE_URL"
api_key = "\$YUNYOU_LLM_API_KEY"
model = "$MODEL"
CONF
    echo "✓ 已写入 $HOME_DIR/yunyou.toml（密钥在 credentials.toml，600 权限，配置文件不落明文）"
  fi
fi

echo ""
echo "✓ 完成。运行：yunyou"
