#!/usr/bin/env bash
# 打 FireVibe.app。
#
# 为什么要打 bundle 而不是直接跑二进制：
#   1) 图标 / 名字只有 bundle 才显示（Dock、⌘Tab、启动台）
#   2)「输入监控」权限是挂在应用身份上的，裸二进制每次重建路径不变但
#      cdhash 变，容易反复要权限；bundle + ad-hoc 签名稳定得多
#   3) 默认配置里 hulu 键的动作是 `open -b com.tankxu.firevibe`，
#      有了 bundle 这条才真能打开自己
set -euo pipefail
cd "$(dirname "$0")"

APP="target/FireVibe.app"
BIN="target/release/firevibe-ui"
VER=$(sed -n 's/^version = "\(.*\)"/\1/p' ui/Cargo.toml | head -1)

echo "▸ 构建 release"
cargo build --release -p firevibe-ui

echo "▸ 生成图标"
python3 design/icon/gen.py
CHROME="/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
if [ -x "$CHROME" ]; then
  "$CHROME" --headless --disable-gpu --hide-scrollbars --default-background-color=00000000 \
    --window-size=1024,1024 --screenshot="design/icon/icon-1024.png" \
    "file://$PWD/design/icon/_render.html" 2>/dev/null
else
  echo "  (没有 Chrome，沿用现有 icon-1024.png)"
fi

ICONSET=$(mktemp -d)/icon.iconset; mkdir -p "$ICONSET"
for spec in "16 16x16" "32 16x16@2x" "32 32x32" "64 32x32@2x" \
            "128 128x128" "256 128x128@2x" "256 256x256" "512 256x256@2x" \
            "512 512x512" "1024 512x512@2x"; do
  set -- $spec
  sips -Z "$1" design/icon/icon-1024.png --out "$ICONSET/icon_$2.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o design/icon/FireVibe.icns

echo "▸ 组装 $APP"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/firevibe"
cp design/icon/FireVibe.icns "$APP/Contents/Resources/icon.icns"
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>FireVibe</string>
  <key>CFBundleDisplayName</key><string>FireVibe</string>
  <key>CFBundleIdentifier</key><string>com.tankxu.firevibe</string>
  <key>CFBundleExecutable</key><string>firevibe</string>
  <key>CFBundleIconFile</key><string>icon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>${VER}</string>
  <key>CFBundleVersion</key><string>${VER}</string>
  <key>LSMinimumSystemVersion</key><string>12.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSSupportsAutomaticGraphicsSwitching</key><true/>
  <!-- 麦克风走 HID，不用 AVFoundation，所以不需要 NSMicrophoneUsageDescription；
       需要的是「输入监控」(kTCCServiceListenEvent)，那个没有 plist 键，只能用户去勾 -->
  <key>NSSpeechRecognitionUsageDescription</key>
  <string>用来把遥控器麦克风说的话转成文字，打进当前输入框。识别在本机离线完成。</string>
  <key>NSMicrophoneUsageDescription</key>
  <string>遥控器的麦克风音频经蓝牙 HID 进来，不使用系统麦克风。</string>
  <key>NSAppleEventsUsageDescription</key>
  <string>用来执行你为遥控器按键配置的 AppleScript 动作。</string>
</dict>
</plist>
PLIST

# 签名。**必须优先用真证书，别用 ad-hoc** ——
# ad-hoc 签名的 designated requirement 是一个写死的 cdhash，而 TCC
# （输入监控 / 辅助功能）就是按这个记授权的。cdhash 每次重新构建都变，
# 于是「系统设置里开关还是开着，但应用实际被拒」，极难自查。
# 用证书签，DR 变成 identifier + 证书，重建也不失效。
IDENT=$(security find-identity -v -p codesigning 2>/dev/null         | sed -n 's/.*) [0-9A-F]* "\(.*\)"/\1/p' | head -1)
if [ -n "$IDENT" ]; then
# 带上我们自己那块虚拟声卡（driver/build.sh 编的）。第三方语音输入工具会把
# 传输类型为「虚拟」的设备滤掉，所以必须用自建的这块（自称 USB）。
# 没编过就跳过 —— 界面上会提示去编。
if [ -d "driver/out/FireVibeMic.driver" ]; then
  mkdir -p "$APP/Contents/Resources"
  rm -rf "$APP/Contents/Resources/FireVibeMic.driver"
  cp -R "driver/out/FireVibeMic.driver" "$APP/Contents/Resources/"
  echo "▸ 已带上虚拟声卡驱动"
else
  echo "▸ 没找到 driver/out/FireVibeMic.driver，跳过（先跑 ./driver/build.sh）"
fi

  echo "▸ 签名（${IDENT}）"
  codesign --force --deep --sign "$IDENT" "$APP" 2>&1 | sed 's/^/  /'
else
  echo "▸ ad-hoc 签名（没找到证书；注意每次重建都要重新授权输入监控）"
  codesign --force --deep --sign - "$APP" 2>&1 | sed 's/^/  /'
fi
echo "  designated requirement:"
codesign -d --requirements - "$APP" 2>&1 | grep designated | sed 's/^/    /'
if codesign -d --requirements - "$APP" 2>&1 | grep -q 'designated => cdhash'; then
  echo "  ⚠ DR 仍按 cdhash 绑定 —— 重建后输入监控授权会静默失效"
fi

echo
echo "好了：$APP"
echo "拖到 /Applications，然后到 系统设置 › 隐私与安全性 › 输入监控 里勾上它。"
