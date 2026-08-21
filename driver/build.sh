#!/bin/bash
# 编一个「自称 USB」的虚拟声卡，给 firevibe 用。
#
# 为什么要自己编：豆包、闪电说这类语音输入 app 会把传输类型为「虚拟」的设备
# 从麦克风候选里过滤掉 —— 实测它们的设备列表里没有任何 virt 设备，而且把系统
# 默认输入固定成 BlackHole、持续灌 440Hz 测试音，它们的电平也一动不动。
# 上游 BlackHole 把传输类型硬编码成 Virtual，所以只能改一行自己编。
#
# 和你已装的 BlackHole2ch 不冲突：Box/Device/Model UID 全部从 kDriver_Name 派生，
# 加上独立的 bundle id，两者互不干扰。
#
# BlackHole 是 GPL-3.0。改动就是下面 patch 的那一处，源码从上游 clone。
set -euo pipefail

DRIVER_NAME="FireVibeMic"                    # UID 前缀，别和别的驱动重名
BUNDLE_ID="com.tankxu.firevibe.audio"
DEVICE_NAME="FireVibe Mic"                   # 用户在声音设置/第三方工具里看到的名字
CHANNELS=2
UPSTREAM="https://github.com/ExistentialAudio/BlackHole.git"

HERE="$(cd "$(dirname "$0")" && pwd)"
WORK="$HERE/.build"
OUT="$HERE/out"
mkdir -p "$WORK" "$OUT"

if [ ! -d "$WORK/BlackHole/.git" ]; then
  echo "▸ 拉上游源码"
  git clone --depth 1 "$UPSTREAM" "$WORK/BlackHole"
fi

cd "$WORK/BlackHole"
git checkout -- BlackHole/BlackHole.c 2>/dev/null || true

echo "▸ 把硬编码的传输类型改成编译期常量"
python3 - <<'PY'
import pathlib
p = pathlib.Path("BlackHole/BlackHole.c")
s = p.read_text()
anchor = '''#ifndef kNumber_Of_Channels
#define                             kNumber_Of_Channels                 2
#endif'''
add = anchor + '''

// 传输类型。上游硬编码 kAudioDeviceTransportTypeVirtual，而豆包/闪电说会把
// 「虚拟」设备从麦克风候选里滤掉。做成编译期常量，好编出一个自称 USB 的实例。
#ifndef kTransportType
#define                             kTransportType                      kAudioDeviceTransportTypeVirtual
#endif'''
assert s.count(anchor) == 1, "上游改版了，锚点对不上"
s = s.replace(anchor, add)
n = s.count("*((UInt32*)outData) = kAudioDeviceTransportTypeVirtual;")
assert n == 2, f"预期 2 处硬编码取值，实际 {n}"
s = s.replace("*((UInt32*)outData) = kAudioDeviceTransportTypeVirtual;",
              "*((UInt32*)outData) = kTransportType;")
p.write_text(s)
print("  patch 应用成功")
PY

IDENT="$(security find-identity -p codesigning -v | sed -n '1s/.*"\(.*\)".*/\1/p')"
[ -n "$IDENT" ] || { echo "没有可用的签名证书"; exit 1; }
echo "▸ 签名身份：$IDENT"

DEFS="\$GCC_PREPROCESSOR_DEFINITIONS"
DEFS="$DEFS kHas_Driver_Name_Format=false"
DEFS="$DEFS kDriver_Name='\"$DRIVER_NAME\"'"
DEFS="$DEFS kPlugIn_BundleID='\"$BUNDLE_ID\"'"
# 一块设备，输入输出都有（和 BlackHole 一样的环回结构）。
#
# 曾经拆成「纯输出 + 纯输入」两块，为的是绕开 VPIO 的回声消除 —— 环回设备上
# AEC 的参考信号和输入完全一样，会被减成静音（实测 RMS 0.2295 → 0.0016）。
# 但一是没证据表明目标 app 真的用 VPIO，二是 cpal 打不开任何纯输出设备
#（macOS 扬声器、显示器都一样报 Invalid property value），代价确定、收益不确定，
# 所以合回一块。等真的证实对方用 VPIO 再说。
DEFS="$DEFS kDevice_Name='\"$DEVICE_NAME\"'"
DEFS="$DEFS kDevice2_Name='\"$DEVICE_NAME Mirror\"'"
DEFS="$DEFS kNumber_Of_Channels=$CHANNELS"
DEFS="$DEFS kTransportType=kAudioDeviceTransportTypeUSB"

echo "▸ 编译"
rm -rf "$WORK/BlackHole/build"
xcodebuild -project BlackHole.xcodeproj -configuration Release \
  CONFIGURATION_BUILD_DIR="$WORK/BlackHole/build" \
  PRODUCT_BUNDLE_IDENTIFIER="$BUNDLE_ID" \
  PRODUCT_NAME="$DRIVER_NAME" \
  CODE_SIGNING_ALLOWED=NO \
  DEVELOPMENT_TEAM="" \
  MACOSX_DEPLOYMENT_TARGET=13.0 \
  GCC_PREPROCESSOR_DEFINITIONS="$DEFS" \
  build | tail -5

SRC="$WORK/BlackHole/build/$DRIVER_NAME.driver"
[ -d "$SRC" ] || { echo "编出来的产物找不到：$SRC"; ls -la "$WORK/BlackHole/build"; exit 1; }
rm -rf "$OUT/$DRIVER_NAME.driver"
cp -R "$SRC" "$OUT/"

# xcodeproj 里写死了上游的 DEVELOPMENT_TEAM，走它的签名流程签不了 ——
# 编成不签名，再用我们自己的证书签。coreaudiod 只要求签名有效。
echo "▸ 签名"
codesign --force --sign "$IDENT" --timestamp=none "$OUT/$DRIVER_NAME.driver"
echo
echo "好了：$OUT/$DRIVER_NAME.driver  （$(du -sh "$OUT/$DRIVER_NAME.driver" | cut -f1)）"
codesign -dv "$OUT/$DRIVER_NAME.driver" 2>&1 | grep -E "Identifier|Authority" | head -3
echo
echo "装（要 sudo，会问你密码）："
echo "  sudo cp -R \"$OUT/$DRIVER_NAME.driver\" /Library/Audio/Plug-Ins/HAL/ && sudo killall -9 coreaudiod"
