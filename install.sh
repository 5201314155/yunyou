#!/bin/sh
# 云悠（YunYou）一条命令全局安装（v4：官方插件集循环落位——业主定「官方插件
# 随主程序同通道」规矩，2026-11-19；v3=仅 scheduler 单件）
# 用法：curl -fsSL https://github.com/5201314155/yunyou/raw/main/install.sh | sh
set -e

REPO="https://github.com/5201314155/yunyou.git"
BIN_DIR="${YUNYOU_INSTALL_DIR:-$HOME/.local/bin}"
HOME_DIR="${YUNYOU_HOME:-$HOME/.yunyou}"

echo "☁ 云悠安装器"

# 1) 主路径：release-bin 分支直拉预编译二进制（raw，免 Release 网页操作）
#    v4：主程序 + 官方插件集（scheduler=定时三工具；web-search=联网搜索单工具
#    四后端）逐个落位 plugins/<category>/<id>/——装完重启即被发现装载。
#    hello-sdk 不随装（示例归示例——示例是仓库内布线，作者参考 main.rs 写法）。
#    memory 插件候入库后加行（设计已批 20261118，代码未落）。
ARCH=$(uname -m)
case "$ARCH" in
  aarch64|arm64)
    BIN_NAME="yunyou-arm64.gz"
    ARCH_SUFFIX="-arm64" ;;
  *)
    BIN_NAME="yunyou.gz"
    ARCH_SUFFIX="" ;;
esac
GH_BASE="${YUNYOU_RELEASE_BASE:-https://raw.githubusercontent.com/5201314155/yunyou/release-bin}"
GH_BIN="$GH_BASE/$BIN_NAME"
ASSET=""
curl -fsSLI -o /dev/null "$GH_BIN" && ASSET="$GH_BIN"

# 通用插件安装（v4 循环体）：plugin_id 落位目录名 / plugin_bin release-bin 产物名
# （不带 .gz 与架构后缀）/ plugin_category 目录分类（与 manifest.category 一致）。
# 单件失败=跳过不阻塞主程序（安装器纪律：主程序成功优先）。
install_plugin() {
    plugin_id="$1"; plugin_bin="$2"; plugin_category="$3"
    asset="$GH_BASE/$plugin_bin$ARCH_SUFFIX.gz"
    manifest_url="$GH_BASE/$plugin_bin-manifest.toml"
    dest="$HOME_DIR/plugins/$plugin_category/$plugin_id"
    echo "→ 下载插件 $plugin_id：$asset"
    TMP=$(mktemp -d)
    curl -fsSL "$asset" -o "$TMP/plugin.gz" || {
        echo "⚠ $plugin_id 下载失败（跳过，主程序不受影响）"
        rm -rf "$TMP"; return 0
    }
    curl -fsSL "$manifest_url" -o "$TMP/manifest.toml" || {
        echo "⚠ $plugin_id manifest 下载失败（跳过）"
        rm -rf "$TMP"; return 0
    }
    gunzip -f "$TMP/plugin.gz"
    mkdir -p "$dest"
    install -m 755 "$TMP/plugin" "$dest/$plugin_bin"
    install -m 644 "$TMP/manifest.toml" "$dest/manifest.toml"
    rm -rf "$TMP"
    echo "✓ $plugin_id 已装到 $dest（重启 yunyou 后模型面即有该插件工具）"
}

# 官方插件集清单（加行即扩——产物推 release-bin 后在此登记）
PLUGINS=""
PLUGINS="$PLUGINS scheduler scheduler-sample workflows"
PLUGINS="$PLUGINS web-search web-search tools"

install_binary() {
    TMP=$(mktemp -d)
    curl -fsSL "$GH_BIN" -o "$TMP/yunyou.gz"
    gunzip -f "$TMP/yunyou.gz"
    mkdir -p "$BIN_DIR"
    install -m 755 "$TMP/yunyou" "$BIN_DIR/yunyou"
    rm -rf "$TMP"
    echo "✓ 已安装到 $BIN_DIR/yunyou（预编译）"
}

if [ -n "$ASSET" ]; then
    echo "→ 下载预编译二进制：$ASSET"
    install_binary
    # 官方插件集循环落位（v4；GH_BASE 可被 YUNYOU_RELEASE_BASE 覆盖=离线模拟安装）
    set -- $PLUGINS
    while [ $# -ge 3 ]; do
        install_plugin "$1" "$2" "$3"
        shift 3
    done
elif command -v cargo >/dev/null 2>&1; then
    echo "→ 未见预编译包，走源码安装（cargo install --git，1-2 分钟）"
    cargo install --git "$REPO" yunyou-cli
    # 源码路径同步装官方插件集（cargo install 第二批 bin + raw manifest）
    set -- $PLUGINS
    while [ $# -ge 3 ]; do
        plugin_id="$1"; plugin_bin="$2"; plugin_category="$3"
        dest="$HOME_DIR/plugins/$plugin_category/$plugin_id"
        if cargo install --git "$REPO" "$plugin_bin"; then
            SRC_BIN_DIR=$(command -v "$plugin_bin" >/dev/null 2>&1 && dirname "$(command -v "$plugin_bin")" || echo "$HOME/.cargo/bin")
            if [ -f "$SRC_BIN_DIR/$plugin_bin" ]; then
                mkdir -p "$dest"
                install -m 755 "$SRC_BIN_DIR/$plugin_bin" "$dest/$plugin_bin"
                curl -fsSL "https://raw.githubusercontent.com/5201314155/yunyou/main/sdk/examples/$plugin_id/manifest.toml" -o "$dest/manifest.toml" \
                    && echo "✓ $plugin_id 已装到 $dest（源码路径）" \
                    || echo "⚠ $plugin_id manifest 下载失败（跳过）"
            fi
        else
            echo "⚠ $plugin_id 源码安装失败（跳过，主程序不受影响）"
        fi
        shift 3
    done
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
