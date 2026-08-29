# 开发笔记

面向想改代码的人。只想用的看 [README](../README.md)。

---

## 跑起来

打成 app（图标、名字、稳定的权限身份都靠它）：

```bash
./package.sh                      # → target/FireVibe.app
```

或者直接跑二进制：

```bash
cargo run --release -p firevibe-ui        # 界面
cargo run --release -p firectl -- --map     # 测绘按键 usage
cargo run --release -p firectl -- --sniff   # 看原始 HID 报文
```

### ⚠ 签名必须用真证书，别用 ad-hoc

`package.sh` 会自动挑 `security find-identity -p codesigning` 找到的第一个身份。
**不要退回 `codesign -s -`**：ad-hoc 签名的 designated requirement 是写死的 cdhash，
而 TCC 就是按它记授权的 —— cdhash 每次重建都变，于是授权静默失效，
可系统设置里那条开关**仍然显示是开着的**，根本看不出问题。

用证书签，DR 变成 `identifier + 证书`，重建不失效。验证：

```bash
codesign -d --requirements - "target/FireVibe.app" | grep designated   # 不该出现 cdhash
```

已经踩进去了：`tccutil reset ListenEvent com.tankxu.firevibe`，再去设置里勾一次。
界面上那条警示里也有个「重置授权」按钮干这件事。

**必须给「输入监控」权限**（系统设置 › 隐私与安全性 › 输入监控），
授权后要**完全退出再重开**才生效 —— 界面上打不开遥控器时会直接提示并给一个跳转按钮。

配对：遥控器一次只能配一台设备，配 Mac 前先把 Fire TV Stick 断电。
必须用 1.5V 碱性 AAA，1.2V 充电电池会让 BLE 射频 brownout。

## 想支持另一款遥控器？

先跑 `firectl --hid-list` 看它的 VID/PID —— 我们是按
`device.rs` 里的 `VID`/`PID` 两个常量打开设备的，标识对不上就完全看不到它。

然后三层依次确认，越往后越难：

1. **能被打开**：改那两个常量即可
2. **按键能识别**：`--map` 重新测绘一遍 usage。注意本机这款有几个 Amazon 私有
   usage（返回 = 键盘页 `0x00F1`、TV = `0x008D`、四个 App 键走 vendor report
   `0xEF` / 页 `0xFF00`），别指望换一款还一样
3. **麦克风能用**：最难。需要它同样支持 vendor report `0xF2` 开热麦、`0xF0` 吐音频流，
   而且编码是 Opus。这是 Amazon 私有协议 —— 「能在电视上配对使用」只说明它是标准
   BLE HID 键盘，和麦克风这条通路无关

## 协议要点

完整逆向记录在 `~/LocalDev/firetv-remote-mac/NOTES.md`。几条容易踩的：

- 语音走 **BLE HID vendor report**，不走 GATT。开麦 = `SetReport(Output, 0xF2, [01 01 00×8])`，
  麦克风是**热的**，发一次就一直吐流，退出前必须发 `[01 00 ...]` 关掉。
- 编码是 **Opus**（CELT-only / WB 16kHz / 单声道 / 20ms / 32kbit/s），不是 IMA ADPCM ——
  80 字节/帧这个尺寸跟 ADPCM 撞了容易误判，以 TOC 字节 `0xB8` 为准。
- 几个反直觉的 usage：OK = `0x58` Keypad Enter（不是 Return）、返回 = 键盘页 `0x00F1`
  （Amazon 私有，不是 Esc）、TV = `0x008D` Program Guide。
  四个 App 快捷键走 vendor report `0xEF`（页 `0xFF00`），按下发 `A1`~`A4`。
- ⚠️ **GATT 特征 `CFBFA004` 是 OTA 的 VENDOR_CMD，上面有 `WIPE=12`/`WIPE_UNPAIR=14`** ——
  盲扫 opcode 会把遥控器擦掉。本项目全程只走 HID，不碰 GATT。

## 界面

`design/mockup.html` 是**定稿**，GPUI 界面按它 1:1 实现，改界面前先改设计稿。

窗口用系统的透明标题栏（`appears_transparent`）：整条系统标题栏隐掉，
顶部留 40px 可拖拽条给红绿灯浮着，页内没有品牌标题，设置按钮在状态行最右端。
内容整体最大宽度 1280px 居中，遥控器那一栏 300px、遥控器在栏内居中。
顶栏和左侧遥控栏都固定不滚，只有右侧那列滚（遥控栏在窗口太矮时自己滚一点）。
卡片 hover 是按帧插值的 140ms 缓出过渡；图上按键**鼠标按住会凹下去**（压暗+去投影+收 1px），
**实体遥控器按下则是亮起来** —— 前者模拟手感，后者用来指示「刚按的是哪个键」。

图标：`design/icon/gen.py` 生成 `icon.svg`，改图标改脚本、别手改 svg；
`package.sh` 会自动重新生成、转 `.icns`、装进 bundle。
`design/demo-config.json` 是设计稿里那套演示状态（含禁用键、AppleScript、shell 命令），
想复现截图就拷到 `~/Library/Application Support/firevibe/config.json`。

自检用的启动开关（只影响首帧，方便截图核对）：

```bash
FIREVIBE_WIN=1060x1360 \
FIREVIBE_BOOT=settings|add|profile|menu:app2|hover:app1|dialog:app1:long \
  cargo run -p firevibe-ui
```

## 自带语音转文字（不依赖任何第三方）

`ActionType::VoiceDictate` —— 按住遥控器说话，松手把识别出的文字打进当前焦点。
用系统的 `SFSpeechRecognizer`，**离线**（`requiresOnDeviceRecognition`）、中文原生、不用下模型。

**走「文件识别」而不是实时 buffer**：按住期间把解码后的 PCM 攒在内存，松手写一个临时
WAV 再识别。省掉构造 `AVAudioPCMBuffer` 那一大坨 unsafe，而且能用 `say` 合成语音自测。
代价是没有实时中间结果 —— 对「按住说话、松手出字」这个用法无所谓。

**它完全不碰虚拟声卡和系统输入设备** —— 直接吃解码后的 PCM。这是它相对
「喂第三方工具」最大的好处：不用把系统默认输入切走，你的真麦克风一直可用。

需要一次「语音识别」权限（Info.plist 的 `NSSpeechRecognitionUsageDescription`）。
**裸二进制拿不到这个权限** —— 没有 Info.plist，系统不会弹授权框。只能从
打好包的 app 里申请：设置 › 语音转文字 › 请求授权。

设置里还有识别语言（zh-CN / en-US）和「识别后自动回车」——
后者在 agent 里就是说完直接发出去。

## 语音输入怎么落到 agent 里

Claude Code、Claude 桌面版这类 agent **没有「音频输入」的概念**，只认「文字进焦点框」。
所以音频和 agent 之间必须有个东西把语音变成击键。三条路：

| 路线 | 谁做识别 | 要改系统输入设备 | 状态 |
|---|---|---|---|
| 灌 BlackHole，靠第三方输入法 | 豆包输入法等 | **要**（全局副作用）| 可用，见下 |
| 发快捷键唤起外部语音 app | 闪电说 / VoiceInput | 不要 | **已做** |
| firevibe 自己识别再打字 | 系统 SFSpeechRecognizer（离线） | 不要 | **已做** |

第一条是 [mi_remote_control](https://github.com/godarrenw/mi_remote_control) 的路子，
省事但要把 系统设置 › 声音 › 输入 切成 BlackHole 2ch —— 一切过去，你的真麦克风
对所有 app 就失效了（会议、录屏全在听 BlackHole），用完得手动切回来。

第二条不动任何系统设置：给麦克风键配「外部语音 app」动作，填上那个 app 的快捷键。
**注意 Fn 和右 Command 合成无效** —— macOS 里合成修饰键码不更新全局修饰位，
这是系统行为改不了。去那个 app 里把快捷键改成普通组合键（F13 或 ⌥⌘Space）。

## 麦克风键为什么会弹 Spotlight

麦克风键的 HID usage 是 **Consumer `0x0221` = AC Search**，macOS 自己就把这个
usage 当「搜索键」→ 弹 Spotlight。而我们**打不开独占 HID**（独占要 root），
所以系统和 firevibe 同时收到这个键。

注意这跟 ⌘Space 那个符号热键无关 —— 实测本机 `AppleSymbolicHotKeys` 里
Spotlight（id 64）是 `enabled = 0`，Spotlight 照样弹，说明走的是更底层的路径，
改设置压不住。

唯一的解法是在事件层拦掉。先用诊断工具看清系统生成了什么事件：

```bash
firectl --tap          # 然后按遥控器上的键
```

它**只打印非字符键**（功能/媒体键区、修饰键、systemDefined），你打的字一个都不记录。
需要「辅助功能」权限。排障时可以 `FIREVIBE_TAP_ALL=1` 关掉过滤（会看到所有按键）。

拿到确切事件后，用 `core/src/tap.rs` 的 `spawn(..., listen_only=false, ...)`
把它吞掉 —— tap 跑在自己的线程和 CFRunLoop 上，不碰 gpui 的主循环。

### 其它平台的语音助手

- **macOS**：Siri，`open -b com.apple.siri.launcher`（已作为麦克风键短按的默认动作）。
  想要「语音转文字进焦点框」其实**系统听写**更对路 —— 它直接打字，而 Siri 是命令助手。
- **Windows 11**：`Win+H` 是语音输入（直接打字进焦点框，对应听写）；
  `Win+Ctrl+S` 是 Voice Access。Cortana 已经废弃了，别指望。
- **Linux**：没有系统级语音助手，只有第三方（nerd-dictation 之类）。

后两个目前是纸面对应 —— 按键注入只做了 macOS（见已知边界）。

## 说话时自动切系统输入设备

靠输入法/外部 app 做识别时，它们听的是**系统默认输入**。所以默认开启：
按下说话键把系统默认输入切到虚拟声卡，说完 400ms 后切回原来的设备
（延迟一点是为了不把尾音截断）。设置里可关。

实测切换耗时 **3~13ms**（`AudioObjectSetPropertyData` 是异步的，
返回后要轮询才知道生效；见 `core/src/audio.rs` 的 `switch_timing` 测试）。
注意消费方 app 未必立刻跟着换流 —— 那部分延迟不在我们手里。
另外某些虚拟驱动（本机的 "Virtual Audio"）压根不接受当默认输入，切了 2 秒也不生效。

⚠️ 副作用：切走期间你的真麦克风对所有用默认输入的 app 都失效。
`Runtime::stop()` 和关掉这个开关时都会还原。

## 已知边界

- 界面只做了 macOS。**按键注入也只有 macOS**：`inject/linux.rs`、`inject/windows.rs`
  从来没写过（mod.rs 里那两个 cfg 分支指向不存在的文件，非 macOS 压根编译不过），
  现已统一落到 fallback —— 别的功能都在，只有注入报「这个平台没有按键注入」。
- 「开机启动」写 `~/Library/LaunchAgents/com.tankxu.firevibe.plist`，未实机验证。
- 按键注入（含媒体键）还需要**「辅助功能」权限**，和读 HID 的「输入监控」是两回事。
- 「检查更新」需要在设置里配一个 JSON 清单地址（`settings.update_endpoint`），
  格式 `{"version":"0.2.0","url":"...","notes":"..."}`；没配就显示「未配置更新源」。

## 附：Fire TV 遥控器的 PID 名单（从固件里挖的）

亚马逊自家的 BLE 遥控器不止一款。Fire TV 固件里能查到 **16 个** PID，VID 统一
`0x0171`（其中 `0x419` 是手柄）：

`0x411 0x412 0x413 0x414 0x415 0x418 0x419 0x41c 0x41e 0x420 0x421 0x423 0x424 0x425 0x42f 0x431`

来自两份名单 —— `ConnectivityControllerService` 的型号档案 `remote_config.json`（13 款）
和 `BluetoothKeyMapLib` 的按键映射白名单（11 款）。**两份互相不是子集**，
所以「支持」是分层的。

⚠️ **PID 不代表机器。** 名单里每个 PID 各带一张按键表：`0x0421` 那张正好 21 项、
与官方 3rd Gen 的 21 颗实体键一一对应；`0x0425` 那张有 45 项（多出数字键盘、
红绿黄蓝、频道键），对应的是一支带数字键盘的电视遥控 —— 但实测手上这台报
`0x0425` 的只有 21 颗键。**仿品会直接借用一个合法 PID。**

所以别拿 PID 猜键位，也别拿它猜麦克风行为（那张表里 11 款都只有一个 `Voice` 条目，
压根不区分两种开麦方式）。按键靠配对时实地测绘，麦克风靠连上后实测判定。

不在名单里的遥控器，Fire TV 只是不给它下发动态按键映射（影响 App 快捷键、
红外那类定制行为）；方向键、音量、语音这些走标准 BLE HID，照样能用。
