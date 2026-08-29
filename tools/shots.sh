#!/bin/zsh
# 重拍 README 用的截图。
#
# ⚠️ 跑之前先确认：
#   · 跑它的终端要有「屏幕录制」权限（系统设置 › 隐私与安全性 › 屏幕录制），
#     否则 screencapture 只会说 "could not create image from rect"
#   · 截的是**当前配置**，所以先把配置摆成好看的样子，或用 FIREVIBE_CONFIG 指一份样例
#   · 窗口必须在前台 —— gpui 不在前台时暂停绘制，截出来是**旧帧**（踩过）
set -e
OUT="$(cd "$(dirname "$0")/.." && pwd)/design/shots"
shot() {   # shot <文件名> <FIREVIBE_BOOT 值>
  osascript -e 'quit app "FireVibe"' 2>/dev/null || true
  sleep 2
  if [[ -n "$2" ]]; then open -b com.tankxu.firevibe --env "FIREVIBE_BOOT=$2"
  else open -b com.tankxu.firevibe; fi
  sleep 6
  osascript -e 'tell application "FireVibe" to activate'
  sleep 2
  local pos size x y w h
  pos=$(osascript -e 'tell application "System Events" to tell process "FireVibe" to get position of window 1')
  size=$(osascript -e 'tell application "System Events" to tell process "FireVibe" to get size of window 1')
  x=${${pos%%,*}// /}; y=${${pos##*,}// /}
  w=${${size%%,*}// /}; h=${${size##*,}// /}
  screencapture -o -x -R "${x},${y},${w},${h}" "$OUT/$1"
  # 截图默认带显示器色彩描述文件，不转 sRGB 的话在 GitHub 上偏色
  sips -m /System/Library/ColorSync/Profiles/sRGB.icc "$OUT/$1" --out "$OUT/$1" >/dev/null
  echo "✓ $1"
}
shot screen-main.png     ""
shot screen-settings.png "settings"
shot edit-dialog.png     "dialog:mute:short"
shot screen-stats.png    "stats"
echo "⚠️ GitHub 的图片代理按 URL 缓存 —— 换了内容不换文件名，读者看到的还是旧图"
