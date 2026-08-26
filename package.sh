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
# 版本号只有一个来源：根 Cargo.toml 的 [workspace.package]。
# 以前读 ui/Cargo.toml，而更新检查用的是 core 的版本（= workspace）——
# 两边能各说各话：装着的包自称 0.1.2-beta.1，更新检查却拿 0.1.3 去比。
VER=$(sed -n '/^\[workspace.package\]/,/^\[/s/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
[ -n "$VER" ] || { echo "✗ 读不到版本号"; exit 1; }

echo "▸ 构建 release"
cargo build --release -p firevibe-ui

echo "▸ 生成图标"
# icon-1024.png 是提交在 git 里的成品，默认直接用它。
# ⚠️ 不要每次都用 Chrome 从 SVG 重渲：Chrome 常在 SVG 没加载完就截图，
#    渲出个几乎空白的坏图**还会覆盖好图**（踩过：dock 图标变成占位方块）。
#    只有图**真的缺失**时才用 Chrome 兜底生成，且渲完校验大小，坏了就丢弃。
python3 design/icon/gen.py   # 只重生成 svg（无副作用），png 不动
PNG="design/icon/icon-1024.png"
if [ ! -s "$PNG" ] || [ "$(wc -c < "$PNG")" -lt 100000 ]; then
  echo "  icon-1024.png 缺失/异常，尝试用 Chrome 从 SVG 生成…"
  CHROME="/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
  if [ -x "$CHROME" ]; then
    CHROME_TMP=$(mktemp -d)
    OUT="$PNG.new"
    timeout 60 "$CHROME" --headless --disable-gpu --hide-scrollbars \
      --no-first-run --no-default-browser-check \
      --user-data-dir="$CHROME_TMP" --default-background-color=00000000 \
      --window-size=1024,1024 --screenshot="$OUT" \
      "file://$PWD/design/icon/_render.html" 2>/dev/null || true
    # 只有渲出来够大（不是空白坏图）才采用
    if [ -s "$OUT" ] && [ "$(wc -c < "$OUT")" -ge 100000 ]; then
      mv "$OUT" "$PNG"; echo "  已生成新图标"
    else
      rm -f "$OUT"; echo "  ⚠ Chrome 渲出的图不可用，保留现有 png"
    fi
    rm -rf "$CHROME_TMP"
  else
    echo "  ⚠ 没有 Chrome 也没有现成图标"
  fi
else
  echo "  用现有 icon-1024.png（$(wc -c < "$PNG") 字节）"
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
  <key>NSBluetoothAlwaysUsageDescription</key><string>读取遥控器的电池电量。</string>
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
SIGN=${IDENT:--}   # 没证书就退回 ad-hoc

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

  # 电量辅助程序：独立进程读 GATT 电池服务（进程内的 CoreBluetooth 起不来，
# 详见 core/src/battery.rs 的说明）。没 swiftc 就跳过，电量只是不显示。
if xcrun --find swiftc >/dev/null 2>&1; then
  # ⚠️ 必须是**裸二进制、由 FireVibe fork/exec 拉起**，不能打成 .app 走 LaunchServices ——
  # 前者 TCC 归责到 FireVibe，用它那一份蓝牙授权；后者会变成一个独立身份，
  # 需要用户再点一次「BattProbe 想使用蓝牙」，而且名字莫名其妙。
  xcrun swiftc -O -o "$APP/Contents/MacOS/battprobe" helper/battprobe.swift
  echo "▸ 已带上电量辅助程序"
else
  echo "  (没有 swiftc，跳过电量辅助程序)"
fi

if [ -n "$IDENT" ]; then
  echo "▸ 签名（${IDENT}）"
else
  echo "▸ ad-hoc 签名（没找到证书；注意每次重建都要重新授权输入监控）"
fi
codesign --force --deep --sign "$SIGN" "$APP" 2>&1 | sed 's/^/  /'
echo "  designated requirement:"
codesign -d --requirements - "$APP" 2>&1 | grep designated | sed 's/^/    /'
if codesign -d --requirements - "$APP" 2>&1 | grep -q 'designated => cdhash'; then
  echo "  ⚠ DR 仍按 cdhash 绑定 —— 重建后输入监控授权会静默失效"
fi

echo
# CLI 一起编出来 —— 排障和「换一款遥控器」（--adapt）都靠它。
# 不放进 bundle 内部：从 shell 跑 .app 里的可执行文件，TCC 会把权限归责到
# 父进程（shell）直接 abort，踩过。放在 zip 里和 .app 并列。
cargo build --release -p firevibe-cli  # 产物叫 firectl
cp target/release/firectl "$(dirname "$APP")/firectl"

echo "好了：$APP"
echo "拖到 /Applications，然后到 系统设置 › 隐私与安全性 › 输入监控 里勾上它。"
