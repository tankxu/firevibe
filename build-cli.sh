#!/usr/bin/env bash
# 构建 firectl 并**用真证书签名**。别直接 `cargo build`：裸二进制 ad-hoc 签名，
# 「输入监控」授权会失配、静默失效（见 CLAUDE.md）。
# 关键：用证书 HASH（纯十六进制）签，不靠证书名里的引号/括号解析；
# 签不成就报错退出，**绝不悄悄退回 ad-hoc**（那会毁掉已有授权）。
set -eo pipefail
cd "$(dirname "$0")"
export PATH="$HOME/.cargo/bin:$PATH"

cargo build --release -p firevibe-cli
BIN="target/release/firectl"

HASH=$(security find-identity -v -p codesigning 2>/dev/null \
  | sed -n 's/^ *[0-9]*) \([0-9A-F]\{40\}\) .*/\1/p' | head -1)

if [ -z "$HASH" ]; then
  echo "✗ 没找到签名证书（security find-identity 为空）。" >&2
  echo "  不签就是 ad-hoc，会毁掉「输入监控」授权 —— 中止。" >&2
  exit 1
fi

codesign --force --sign "$HASH" --identifier com.tankxu.firectl "$BIN"

# 验签：必须是带 TeamIdentifier 的真签名，不能是 ad-hoc
if codesign -dv "$BIN" 2>&1 | grep -q "TeamIdentifier=not set"; then
  echo "✗ 签出来还是 ad-hoc（TeamIdentifier=not set）—— 授权会失效，中止。" >&2
  exit 1
fi
echo "▸ 已用证书签名："
codesign -dv "$BIN" 2>&1 | grep -iE "Identifier|TeamId" | sed 's/^/    /'
echo "好了：$BIN"
