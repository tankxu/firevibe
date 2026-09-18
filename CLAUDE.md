# 交接给下一个 agent

只写**代码里看不出来、但会让你白花几小时**的东西。工程说明看
[README](README.md) 和 [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md)。

---

## 一、构建经济学（最容易浪费时间的地方）

`[profile.release]` 是 `lto = "thin"` + `codegen-units = 1`。后果：

- **改 `Cargo.toml` 里的版本号会让所有 crate 全量重编。** 平时增量 12～16 秒，
  一改版本号退回全量 + LTO（好几分钟）。要发版就一次改到位，别边改边验。
- **改 `core` 会连带重做 LTO**，所以「只编 CLI」也可能要 2 分钟。
- **验证界面用 debug**（`cargo build -p firevibe-ui`，2 分钟，有缓存），
  **只在打包时用 release**。
- ⚠️ **别跑 `cargo test --workspace`（dev profile）**：它和 release 是两套独立产物，
  等于把 gpui 再编一遍。要测就 `cargo test --release`。
- ⚠️ **别为了「快点」加新 profile**（我加过 `[profile.quick]`）——
  新 profile 不共用缓存，等于全量重编，比原路更慢。

**构建时间忽快忽慢先查机器负载**，不是你的代码问题。踩过一次：`load average 94`，
真凶是 ChatGPT 的 Codex 进程常驻 60% + iCloud 的 `bird`/`fileproviderd` 39%
（很可能在同步 15G 的 `target/`）。rustc 抢不到核。

**`package.sh` 里的无头 Chrome 曾让整条打包卡死**：共用默认 profile 时，
上一次没退干净的实例持着锁，新实例挂着不返回，还攒僵尸进程。已加
`timeout 60` + 独立 `--user-data-dir`，渲不出来就沿用现有图标。

**release 产物的增量判断失灵过一次**：16 秒就「Finished」但改动没进去，
表现是 release 行为和 debug 不一致。`touch` 一下源文件重编即恢复。根因没查到，
**记住症状：release 和 debug 表现不同时，先 touch 重编，别急着怀疑代码。**

## 二、验证纪律（我在这上面栽过）

- **别拿半边数据下结论。** 我加了个 `--synth-only` 捷径只跑合成侧，
  得出「合成的修饰键系统不认」，用户跑完整对照直接推翻 —— 事件层和状态层
  其实完全一致。**跑完整对照，或者别下机制性结论。**
- **验证「装的是不是刚构建的」**。`open -a` 遇到已在运行的实例只会激活它，
  不会重启 —— 我据此误判过。用 `ps -o lstart` 看进程启动时间。
- **`strings` 找不到中文字面量**（macOS 的 `strings` 对这个二进制不管用），
  用 `grep -a` 做字节级搜索。
- **窗口不在前台时 gpui 暂停绘制**，`CGWindowListCreateImage` 会返回**旧帧**。
  截图前先激活窗口，否则会误判「改动没生效」。
- **合成点击对这个 app 不稳**，别在上面耗时间。项目里有 `FIREVIBE_BOOT=`
  启动开关，直接把界面摆到目标状态再截图。

## 三、TCC / 权限（错一次就白折腾半天）

- **绝不从 shell 直接跑 `.app/Contents/MacOS/` 里的可执行文件。**
  TCC 把权限归责到**父进程**（shell），进程会被 `__TCC_CRASHING_DUE_TO_PRIVACY_VIOLATION__`
  当场打死，报的错还是「Info.plist 缺 NSSpeechRecognitionUsageDescription」——
  而那个键明明就在。要测就 `open -b com.tankxu.firevibe`（配 `--env` 传开关、
  `--stderr` 收日志）。
- **`firectl` 自持「输入监控」授权，从任何终端跑都行**（靠 disclaim 自负 TCC 责任，
  见 `become_self_responsible()`）。**不要再说「用 Warp / 授权终端」** —— 那是旧结论。
  把 firectl 本体加进「输入监控」一次即可，Fleet 里直接跑。见第四节 ★ 那两条。
- **签名必须用真证书。** ad-hoc 签名的 DR 是写死的 cdhash，重建即失效，
  而系统设置里的开关**看着还是开的**。证书签名的 DR 是 identifier + 证书，
  跨版本稳定（所以从 Releases 下载的包升级不掉权限）。
- **CoreBluetooth 毫无反应 ≠ 代码有问题，先截屏看有没有挂着的授权框。**
  授权框没答复时，`CBCentralManager` 的状态永远停在 `Unknown(0)`、
  `centralManagerDidUpdateState` 一次都不来 —— 不报错、不超时，
  `tccd` 和 `bluetoothd` 的日志里也一行都查不到（`tccutil reset` 还会说
  「Failed to reset」，因为库里压根没记录）。而且它挂着的时候，
  **本机别的进程申请蓝牙也一样卡住**（拿全新 bundle id 的 app 实测过）。
  我为此依次错怪了 objc2、进程内环境、签名、嵌套 bundle，全不是。
- **辅助小程序要用 fork/exec 拉起，不要打成 .app 走 LaunchServices。**
  裸二进制的 TCC 归责到父进程，用 FireVibe 那一份授权；打成 .app 它就是
  独立身份，得让用户再点一次名字莫名其妙的第二个授权框。
- **`codesign --deep` 管不到 `Contents/Resources/` 里的 .app** ——
  实测它留在 adhoc + linker-signed + `Info.plist=not bound`，得单独签。
- **不要替用户点系统隐私弹框。** 我为了关崩溃框做盲点击，很可能替用户点了
  「允许」语音识别 —— 已如实告知并给了 `tccutil reset` 的撤销命令。

## 四、已查清的硬事实（别重复验证）

**合成修饰键能和真键盘逐位一致，但有两位去不掉。** 完整对照：

|  | 事件 flags（按下 / 松开） | 全局状态三项 |
|---|---|---|
| 真键盘 | `0x80140` / `0x100` | 都置位 |
| 合成 | `0x20080140` / `0x20000100` | **也都置位** |

要对齐低位必须补：**IOKit 左右设备位**（右⌥ `0x40`、左⌥ `0x20`、左⌃ `0x01`、
右⌃ `0x2000`、左⇧ `0x02`、右⇧ `0x04`、左⌘ `0x08`、右⌘ `0x10`）+ **`0x100`
NonCoalesced**。去不掉的是 `0x20000000`「进程合成标记」和 `pid != 0`。
**有的第三方语音工具只认硬件来源**，对它们合成这条路是死的 ——
唯一出路是 `hidutil` **设备层重映射**（只匹配这台遥控器，不碰用户键盘）。
那是**进程外系统状态**：启动按配置重下、`on_app_quit` 清、关开关立刻清。

**`type_text` 用 `HIDSystemState` 建事件源会静默丢字**：post 返回成功、
前台也没变，字就是不出来。改用 `Private` 源 + 显式 `set_flags(CGEventFlagNull)`
+ 按 ≤16 个 UTF-16 单元切段。⚠️ 三处一起改的，**没隔离出哪个是关键**；
`CGEventSourceFlagsState` 当时读出来是 0x0，所以「残留修饰位」这个解释
**未被证实**。诡异之处：同样代码从 CLI 打字一直好，只有从 app 进程里会丢。

⚠️ **`preferred_voice_device()` 永远只认 FireVibe Mic，绝不回退真 BlackHole。**
真 BlackHole 传输类型 Virtual，会被豆包/闪电说滤掉、喂不进 —— 回退到它会让「没装
FireVibe Mic」时界面显示成「BlackHole 就绪」，误导用户以为能用（实则豆包里选不到）。
老配置停在 "BlackHole" 的一律迁成 FireVibe Mic（load 里无条件兜底）；loopback 的 HAL
检测也只认 `firevibemic` 驱动文件夹，别把机器上已有的真 BlackHole 当成我们的装了。

**语音输入工具会把传输类型为「虚拟」的声卡从麦克风列表里滤掉**（BlackHole 就是
这么被漏掉的）。自建的 `FireVibe Mic` 只改了一行：自称 USB。
⚠️ 曾把它拆成「纯输出 + 纯输入」两块设备去绕 VPIO 的回声消除
（环回设备上 AEC 参考和输入相同，实测 RMS 0.2295 → 0.0016），
**但已撤回** —— cpal 打不开任何纯输出设备（macOS 扬声器、显示器都报
`Invalid property value`），代价确定、收益不确定。

**★ 菜单项别用系统标准 selector（macOS 26 会自动加图标）。** `terminate:`/`unhide:`
这类标准动作，macOS 26 在菜单显示时会自动塞个 SF Symbol 图标（Quit 上那个 ⊠），
还占左侧图标列缩进；创建时 `setImage(None)` 会在显示时被加回来，塞透明图又占缩进。
解法：`define_class!` 建个自定义 target 类（`FVTrayTarget`，MainThreadOnly），
菜单项用**自定义 selector**（`fvShow:`/`fvQuit:`）+ `setTarget(自定义对象)`，AppKit
不认得就不加图标 —— 纯文本、无缩进。`fvQuit:` 里调 `NSApp.terminate(None)`（照样触发
on_app_quit 清 hidremap），`fvShow:` 里 `unhide + activateIgnoringOtherApps`。

**★ 窗口拖拽用 performWindowDragWithEvent，不要用 movableByWindowBackground。**
后者把整个窗口背景变成可拖 —— 但 gpui 的输入框不是独立 NSView，整块内容都算背景，
于是**在输入框里拖拽变成拖窗、没法选文本**。正解：header 的 `on_mouse_down` 里调
`tray::start_window_drag()` → 取 `NSApp.currentEvent` + `keyWindow`，
`performWindowDragWithEvent`。只有 header 触发，输入框/按钮不受影响。
（gpui 0.2.2 的 `window_control_area(Drag)` 和 `start_window_move()` 在 mac 上都是空实现，
那条 40px 顶栏能拖纯粹因为压在 AppKit 透明标题栏上。）

**★ firectl 靠 disclaim 自持授权，不再依赖终端。** CLI 从 shell 跑，TCC 默认把
权限归责到父进程（终端）—— 所以哪怕 firectl 用和 FireVibe 同一张证书正确签名，
用的还是终端那份授权，终端若 ad-hoc 签名还会静默失效。解法在
`cli/src/main.rs::become_self_responsible()`：进程一启动就用带
`responsibility_spawnattrs_setdisclaim` 的 `posix_spawn(SETEXEC)` 原地重执行自己，
重执行出的这份对 TCC **自负其责**，成为独立授权主体（和 FireVibe.app 一样）。
- 裸二进制**不会弹交互授权框**，也不写 TCC 日志 —— 要用户手动把 firectl 可执行文件
  拖进 系统设置 › 输入监控 并打开开关，加一次之后**任何终端都认**（实测 Fleet 里直接过）。
- ⚠️ **必须每次构建都重新签名**，否则 `cargo build` 出的裸二进制 DR 变成 ad-hoc、
  授权失配。用 **`./build-cli.sh`**（cargo build + 用稳定 identifier/证书签名一步到位），
  别直接 `cargo build`。
  ⚠️ **`package.sh` 跑 workspace 构建也会把 `target/release/firectl` 覆盖成未签名版**
  （它只签 app bundle，不签独立 firectl）。所以每次 package 之后，要单独用 firectl 前
  必须再 `./build-cli.sh` 重签一次，否则授权失配报 not permitted。（我在这忘过。）
- 别用 sudo：root 不在图形登录会话里，反而连设备都打不开。

**★ 终端签名决定授权是否生效，不只是「勾没勾输入监控」。** 实测：Fleet
（`land.levelup.fleet`，`TeamIdentifier=not set` + `get-task-allow`，属 ad-hoc 自签）
即便在系统设置里「输入监控」和「辅助功能」都勾着，跑 firectl 仍然
`not permitted`；Warp（Developer ID 签名、hardened runtime）就正常。
根因是 ad-hoc 签名的授权按 cdhash 绑定，终端一更新 cdhash 就变、授权静默失效
（开关看着还亮），即 [[macos-adhoc-sign-tcc-trap]] 那个坑，只是这次踩在**终端**上。
→ 但这条是**旧结论，已被 disclaim 方案取代**（见下条 ★）：firectl 现在自持授权，
从哪个终端跑都行，不用挑终端。这里保留只为说明当初为什么会误判。

**★ 血泪教训：`SetReport` 报 `0xE00002E2 not permitted` 是「输入监控」授权问题，
不是字节/长度问题。** TCC 把权限归责到**发起进程**：FireVibe.app 有自己的授权、
或在已授权过的终端（比如 Warp）里跑 firectl → 命令成功；在没授权的终端里跑 →
**任何 SetReport 都被拒**，连打开设备都可能失败。`sudo` 更糟 —— root 不在你的
图形登录会话里，拿不到授权，连设备都打不开。

我在这上面栽了整整一轮：全程在没授权的终端里测，把权限错误层层脑补成
「0xF2 载荷必须 1 字节」「系统重启后长度校验变严」「关麦会把麦克风死锁」，
据此删过 MIC_ON、拆过 probe、还把坏状态发给朋友测导致他那台的结论作废。
真相：开麦 **`MIC_ON=[F2,01,01]` 一直是对的**（授权环境里起流 ~50 帧/秒、电平在动）。
但关麦 v0.1.0 用的 `[F2,01,00]` **停不了流**（见下条）。
⚠️ 曾经代码写 `let _ = dev.write(&MIC_ON);` 把返回值丢了，失败看不出来 ——
**对设备的写操作一律看返回值。**

**★ 关麦命令是 `[F2,00]`(2B)，不是 `[F2,01,00]`(3B)。** `firectl --mic-off-test` 实测
（授权环境、控制变量、可复现）：`[F2,01,00]` 写成功但流照走 61/秒（**停不了**），
`[F2,00]` 立刻降到 ~0。开麦是 `[F2,01,01]`。所以关麦**不是**把开麦第 3 字节改成 0，
而是另一条 2 字节命令 —— v0.1.0 一直用 [F2,01,00] 当关麦，从来没真关上过，
这才是费电真凶（麦克风热着 50 帧/秒不停）。
遥控器麦克风是「热」的：发一次 MIC_ON 就一直吐流、与按键无关，用完必须发 MIC_OFF。
配套：**打开设备时补一发关麦**（上次若被强杀，麦克风还热着，`mic_was=false`
让循环以为不用关）；**自愈**：没开麦却 2 秒内还在收帧就补发关麦。
以后要改开关麦字节，先跑 `firectl --mic-off-test` 实测，别信描述符声明的长度。

**两条语音通路不是一回事**：Fire TV 3rd Gen 走 HID vendor report
（`0xF2` 开麦 / `0xF0` 收流）+ Opus；Android TV 系（小米那类）走 ATVV **GATT** + ADPCM。
「能在电视上用麦克风」只说明它按那台电视期望的方式说话，**不代表和我们同一条通路**。
判据用 **`firectl --probe-all`** —— 换遥控器的**唯一入口**，一条命令走完
环境自检 → 选设备（写配置）→ 描述符解析并和原厂逐字节比对 → 逐键认键（写配置）
→ 麦克风（发开麦 + 按住观察 0xF0）→ 电量，落一份
`~/Downloads/FireVibe 适配报告 <时间>.txt`，末尾带机器可读的 JSON 给后续脚本读。
⚠️ **未知/拼错的选项会报错，不再静默掉进默认引擎模式。** 以前 `firectl --proble-all`
(拼错) 不匹配任何 `if has(...)`，直接落到文件末尾的默认运行 —— 那个会 `Config::load()`、
打印方案、跑引擎，用户以为在跑自己想要的命令、还纳闷「怎么读了配置」。现在 run_cli 开头
有个 KNOWN 白名单 + 编辑距离「是不是想输 X」提示，未知 `--xxx` 直接退出码 2。
⚠️ **`--probe-all` 是纯硬件探测，全程不读也不写 app 配置。** 用内存里的
`Config::default()` + 选中的设备标识打开设备（从不落盘），认到的键、结论都只进报告。
**唯一会碰配置文件的是最后 `maybe_apply()`** —— 单独问「要不要把设备+键位写进
FireVibe 配置」，默认不写。这样适配过程和用户现有配置解耦，不会像早期那样把
用户方案搅乱（早期 step1 会 `Config::load/save`、step3 会 `set_slot+save`，已移除）。
⚠️ **firectl 需已加入「输入监控」**（它自持授权，任何终端都行；没加过就照 ★ 加一次）。
没授权时开麦 SetReport 会被 TCC 拒、测出来全零帧 —— 那是权限，不是设备毛病。
⚠️ **这台是「按住说话」硬件，不是热麦克风。** 实测（`firectl --mic`，MIC_ON 已成功发出
+ 每秒 keepalive，但**不按键**）→ **0 帧**。物理按住麦克风键才采音，松开即停。
所以 MIC_ON 只是「允许上报音频」，真正的门是那颗键。（原 NOTES 里「发一次 ON 就一直吐流、
与按键无关」的热麦克风说法，现在这台上不成立 —— 别再照那个设计。）
⚠️ **独占(seize)打开需要 root，普通用户 `privilege violation`(0xE00002C1)** —— 别用 `cfg.exclusive`。
（我试过，第一次碰巧成功是当时没人占设备，不可靠。）避免麦克风键弹 Spotlight 改用
**临时按键重映射**：测麦克风前 `hidremap::apply("rightoption")`（把 AC Search 映射成右⌥，
按下不弹 Spotlight，0xF0 音频照进），测完 `hidremap::clear()`。⚠️ 这是进程外系统状态，
中途退出会残留 —— probe 里在正常收尾、finish()、以及 **SIGINT 处理器**里都清了一遍
（SIGKILL 拦不住，但 Ctrl-C 能）。
⚠️ **测语音必须每秒补发开麦（keepalive）**：`[F2,01,01]` 会过期，单发一次、
等用户几秒后才按键就 0 帧（原厂遥控器实测栽过），按住期间每秒补发才行。

⚠️ **描述符可以照抄**：平替常把报告描述符原样搬过去，「声明了 0xF0」说明不了
固件实现了语音，必须看实收报文。
⚠️ **观察窗口要覆盖「松开」那一下**（按住 8 秒 + 松开后再收 2 秒），
否则松开报文落在窗外，看着像只发了 1 条。
⚠️ **不要对不明设备做 opcode 盲扫** —— 可能不可逆（GATT 那边 `CFBFA004` 上就有 WIPE）。
⚠️ **`voice.enabled` 字段已删除**（曾是个没界面、用户碰不到的死开关，默认永远 true）。
它当初唯一的作用是被探测命令临时设 false 来不建 sink，结果认键那步 `save()`
把 false 存进用户配置、把语音永久关掉了（app 里「测试语音」一直弹「语音链路
还没建起来」、按住说话 0 帧）。现在 sink 只要 loopback 就绪就无条件建；探测命令
不调 `start_voice()` 自然不建 sink，不再需要这个标志。`--no-voice` 改用局部布尔。
教训：没有 UI、只被内部代码翻转又会落盘的配置字段，就是纯坑，别留。

⚠️ **诊断命令只留一个**：`--probe-mic`/`--mic-test` 已并进 `--probe-all`，
别再拆出各测一半的实现（拆开过一次，漏发开麦把结论测反还发给了别人）。

⚠️ **国产遥控器（0x0171/0x041e）的语音到底行不行，目前零结论** ——
朋友那次跑的是我改坏、漏发开麦的版本，结果作废。要在**我方原厂遥控器上
把 `--probe-all` 验通**（确认能测出「语音可用」），再发给他重测。

## 四点五、HTTP 请求动作 + Shell 命令的坑

**`ActionType::Http`**（`config.rs`）：按键发一个 HTTP 请求。字段 `method`(GET/POST)、
`arg`(URL)、`body`、`retries`、`timeout_ms`。执行走 `runtime.rs::spawn_http`：
**用 `/usr/bin/curl` 直接传参数向量(不过 shell)**,`--retry`/`--max-time` 是 curl 原生的,
结果(HTTP 状态码/错误)回报到 `Event::Log`(会显示 toast)。UI 编辑器在 `editor.rs` 的
`ActionType::Http` 分支(GET/POST chip + URL + 请求体 + 重试 + 超时),EditState 加了
`post`/`body_in`/`retries_in`/`timeout_in`,存取在 `build_action()`。

⚠️ **为什么加它**:用户拿 `Shell` 动作跑 `curl` 发请求,命令字符串在配置里被换行拆断
(第一行结尾漏了续行反斜杠),`/bin/sh -c` 把它当成断命令,curl 从没拿到 URL、静默失败。
`Shell`/`AppleScript` 动作是 fire-and-forget(`let _ = ...spawn()`),连退出码都不看,
所以界面「测试」永远显示成功、用户完全看不到失败。要发 HTTP 请求就用 `Http` 动作,别用
shell+curl —— 免了 shell 引号/换行的坑,还有状态码反馈。

## 五、产品约定（用户明确要求过）

- **界面文案里不出现任何具体输入法/第三方 app 的品牌名**，统一写「第三方语音输入工具」。
  选项标签也一样（写「双击」，不写「双击（豆包默认）」）。代码注释里可以写具体名字。
- **文档只写做了的**，不列「还没接的功能」。「没做公证」这类是**安装步骤的一部分**，
  必须写（不写用户装不上），但要正面表述（「适用范围」而不是「已知限制」）。
- **README 面向使用者**（支持型号 → 能干什么 → 配对/安装/授权三步 → 常见问题），
  开发内容全在 `docs/DEVELOPMENT.md`。
- **FAQ 只留真会遇到的**。修好一个问题就把对应条目删掉 —— 留着等于告诉用户
  「你会遇到一个我们已经解决的问题」。
- **不做「一次性强交互向导」式的界面**（选设备、逐键认键这类）。那种放 CLI，
  界面里既占地方又不好交互。
- 交互式配置项**同一件事只在一个地方配**：硬件层映射是从「第三方语音输入」
  动作**推导**出来的，不做独立设置项。

## 五点五、窗口关闭会卡死（已修）

**点窗口红叉会让主线程死循环、要 force quit。** GPUI 0.2.2 默认关闭会 drop 掉
最后一个窗口，之后主线程陷入递归空转（`sample` 抓到 517 个采样全在一条自调用深栈，
不是等锁，是烧 CPU；release 已 strip，atos 还不出符号，但形态明确）。
⚠️ 注意 **Cmd-Q / `osascript quit` 走的是另一条路（on_app_quit），不卡** ——
所以只测退出复现不出来，必须测**点红叉**。用 `osascript ... click button 1 of window 1`
能复现（那次 osascript 调用会连带卡 2 分钟）。

修法：`window.on_window_should_close(cx, |_w, cx| { cx.hide(); false })` —— 返回 false
阻止 GPUI 默认关闭（不 drop 窗口就不会死循环），改成 `cx.hide()` 隐藏到后台。
点红叉=隐藏，Dock 点图标重新唤出（`Application::on_reopen` → `cx.activate(true)`）。

⚠️ **但只 hide 会让 app 退不掉**：顶部菜单栏原本是空的、Cmd-Q 没绑。必须配一套
应用菜单：`gpui::actions!(firevibe,[Quit])` + `cx.on_action(|_:&Quit,cx|cx.quit())`
+ `cx.bind_keys([KeyBinding::new("cmd-q",Quit,None)])` + `cx.set_menus(vec![Menu{
name:"FireVibe",items:vec![MenuItem::action("退出 FireVibe",Quit)]}])`。
`on_reopen` 在 `Application`（外层 builder）上，不在 run 闭包的 `App` 上 —— 要在 `.run()` 前调。
gpui 有原生菜单，**做菜单/退出不用碰 objc2**。
**右上角状态栏图标 + 窗口拖拽（都在 `core/src/tray.rs`）** —— `core/src/tray.rs::install()`，在 ui `main()`
的 run 闭包末尾（NSApp 起来后）调。菜单项走 **target=nil + 标准 selector**
（`unhide:` 显示 / `terminate:` 退出），响应链交给 NSApp，**不用自定义 Obj-C 类**。
图标是**自绘的遥控器剪影模板图**（不是 emoji、不是 SF Symbol）：`ui/assets/tray/tray@2x.png`
（36px 单色黑+alpha，PIL 生成，见提交历史里的画法），`include_bytes!` 内嵌 → NSData →
`NSImage::initWithData` → `setTemplate(true)` → `setSize(18,18)`，macOS 按深浅色自动着色。
菜单项「显示窗口」「Quit」。status item 和 menu 用 `mem::forget` 永久保活。
需要在 core 的 objc2-app-kit features 里加 NSStatusBar/NSStatusItem/NSStatusBarButton/
NSMenu/NSMenuItem/NSButton/NSControl/NSView/NSResponder/NSApplication。
⚠️ **拖窗：gpui 0.2.2 在 macOS 上 `window_control_area(Drag)` 和 `start_window_move()`
都是空实现**（platform trait 默认空，mac 没覆盖）。那条 40px 顶栏能拖，是因为它落在
AppKit 透明标题栏的真实拖拽区，不是 window_control_area 起作用。要让标题栏以外
（整个 header/背景）可拖，唯一原生办法是 `NSWindow.setMovableByWindowBackground(true)`
—— 见 `tray::make_windows_draggable()`，在 activate 后对 `NSApplication.windows()` 全设。
开了之后点任意非交互背景都能拖，按钮/输入框照常响应。

## 六、容易踩的杂项

- **GPUI**：`cx.open_window()` 不能在 `render()` 里调（重入绘制，静默 abort），
  放定时器的 `this.update` 闭包里。
- **`deferred()` 的浮层必须配 `.occlude()`**：deferred 只解决画在上面，
  挡不住鼠标 —— 点下拉菜单项会穿透到下面的按钮（实测点输入源下拉穿到了
  「添加按键」）。四个下拉（输入源/方案/卡片菜单/设置语言）都补过了。
- **`.when(self.x.lock().is_some(), |d| ...)` 会死锁。** 临时的 `parking_lot`
  guard 活到整条语句结束，闭包里再取同一把锁就是自锁（非重入）。
  先把锁里的东西取出来（`let st = { ... };`），闭包只用取出来的值。
- **macOS 26 会给所有窗口画圆角和边缘高光** —— 悬浮窗那圈「白框」是系统画的，
  不是渲染 bug。别自己再画圆角矩形去对抗它。
- **`include_dir!` 不声明 rerun-if-changed**：往 `ui/assets/icons/` 加 svg
  不会触发重编，新图标位置一片空白。加完 `touch ui/src/assets.rs`。
- **GitHub README 换图必须换文件名** —— camo 图片代理按 URL 缓存，
  只改内容读者看到的还是旧图。
- **配置迁移**用 `Config::schema`（现在 4）。改默认配置结构时记得写迁移，
  并且**先在 `FIREVIBE_CONFIG` 指向的副本上验**，别拿用户真配置试
  （我踩过：合成点击测试把用户的语言改成英文、删掉了一个槽位配置）。
- **方案顺序**：数组前面放预制（默认、Vibe），用户新建的 append 在后；
  下拉**倒序渲染**（新的在上）。`active` 存的是数组下标，动数组顺序要跟着改它。

## 遥控器有两种开麦模型（2026-08-26 实测）

支持的不是「一种遥控器」。Fire TV 固件里能查到 **16 个** `0x0171` PID
（两份名单的并集：`ConnectivityControllerService` 的 `res/raw/remote_config.json`
型号档案 13 个 + `BluetoothKeyMapLib` 的 `kml_supported_amazon_ble_remote_pids`
按键映射白名单 11 个，互相不是子集）。HID 报告描述符**完全相同**，但开麦方式分两派：

- **热麦克风**（已知：`0x0421`）：主机写 `SetReport(Output,0xF2,[01,01,…])`，
  之后一直吐流，**跟物理按键无关**；必须发 `[F2,00]` 才停。
- **PTT**（已知：`0x0425`）：**设备自发，只在物理麦克风键按住期间出流**，
  松手立停；`MIC_ON` 发了完全没反应，也不需要。

音频格式两派一样：report `0xF0`，81 字节，Opus 16 kHz / 20 ms。

❗ **PID 不代表机器，别拿它当设备档案。** `BluetoothKeyMapLib` 里每个 PID 各带一张
按键表：`0x0421` 那张 21 项、与官方 3rd Gen 的实体键一一对应；`0x0425` 那张 45 项
（多出数字键盘/红绿黄蓝/频道键），是一支带数字键盘的电视遥控 —— 但手上这台报
`0x0425` 的只有 21 颗键。**仿品会直接借用合法 PID。**
（早先据此断言「0x0425 不是山寨、是官方型号」——**只看了 PID 在不在名单里，是错的**。）
那张表也不含开麦方式：11 款都只有一个 `Voice` 条目。所以键位只能实地测绘、
开麦模型只能连上后实测 —— 两件 FireVibe 都自己做。

### app 自己认型号
`Runtime` 的 HID 线程起来时，若 `settings.mic_model == Unknown` 就探一次：
**没人碰遥控器时发 `MIC_ON`，看 1.5 秒内出不出流**。出流 = `Hot`，不出 = `Ptt`。
结果存 `settings.mic_model`，换设备时（`pick_device`）置回 `Unknown` 重探。

⚠️ 判成 Ptt 之后**照样继续发 MIC_ON/keepalive** —— 对 PTT 无害，万一判错
（探测时遥控器正好睡着）也不会把热麦克风弄瘫。判型只用来：关掉那个没意义的
自愈关麦、在界面上提醒绑定方式、以及在动作编辑器里收起短按槽的语音动作。

### 对配置的影响
PTT 遥控器上语音**配在长按槽**（长按 = 按住，语义直白）：
- 动作编辑器里，麦克风键的**短按**槽不再提供语音类动作，只显示一行说明
- 语音动作还留在短按槽时，主界面出提示条 +「移到长按」一键改
- `Profile::long_fires_on_press()`：某个槽配了长按、短按却是空的 → **按下即触发**，
  不等 `long_press_ms`。不然 PTT 上开头那截话就丢在虚拟声卡外面了

❗ 更正：早先记过「放长按槽会漏掉按下那一下、弹 Spotlight」——**错的**。
Spotlight 是 hidremap 认错设备导致的，和放哪个槽无关。

### 诊断命令
```
firectl --hid-list --all      # 每个 top-level collection 都列出来（macOS 会拆成多个）
firectl --collection-test     # 逐个 collection 发 MIC_ON，数 0xF0 帧
firectl --mic-listen          # 开麦蹲 20 秒，按 report id 统计收到什么
firectl --mic-listen --no-cmd # 对照组：一条命令都不发，只靠按键 → 分辨 PTT / 热麦克风
```
`battprobe --listen --secs=N` 是只读订阅所有 GATT notify（排查用，语音不走 GATT）。

⚠️ 遥控器空闲几十秒就休眠掉线，测之前先按个键唤醒；`--mic-listen` 自带等待。
⚠️ **别再把 probe-all 的麦克风测试改成「不用按实体按钮」** —— 对 PTT 遥控器
那等于保证测出 0 帧。现在 `--probe-all` 的第 ③ 步已经是两段对照
（A 段发 MIC_ON 不碰遥控器，B 段按住麦克风键），会直接判出开麦模型。
反过来也别改成「只让用户按住」：更早那版没建 sink，`push_pcm` 在
`passing=false` 时被丢弃，看着像「按了也没声音」。两个坑都踩过。

### hidremap 认设备（踩过）
`hidremap` 内部存着自己的一份 VID/PID，默认是出厂那台 `0x0421`。以前只有
「配对新遥控器」流程会 `set_ids`，所以启动时按配置换过遥控器的，映射一直下发给
一台没连的设备 —— 麦克风键没被接管：Spotlight 照弹，而且第三方语音工具拿不到
**硬件来源**的修饰键就不干活（合成事件对它无效，见 hidremap.rs 顶部注释）。
现在 `sync_hid_remap()` 每次都先按配置 `set_ids`。
❗ **2026-08-31 起映射改为全局（不带 --matching）**：按设备的映射只对下发
那一刻在线的设备生效，断开重连即失效 —— 遥控器 8 秒一睡，每次唤醒的第一下
按键到达时映射必然不在场（Spotlight 弹出、豆包拿不到硬件修饰键、要按两下）。
全局映射常驻事件系统、对之后接入的设备也生效，app 启动时遥控器睡着照样预埋成功。
源是 Consumer AC Search(0x0221)，普通键盘不发，无误伤面。
验证：`hidutil property --get UserKeyMapping`（不带 --matching）
应该看到 Src `0xC00000221`(AC Search) → Dst `0x7000000E6`(右 Option)。

⚠️ 改了配置要**重启 app** 才生效 —— runtime 是启动时读配置建 HID 线程的。
⚠️ 两个进程读同一台 HID 设备时报告可能只送到一个：用 firectl 测之前先退出 app。

## 发布（踩过的顺序坑）

**版本号只有一个来源**：根 `Cargo.toml` 的 `[workspace.package] version`。
`ui/Cargo.toml` 用 `version.workspace = true`，`package.sh` 从工作区读。
（以前 package.sh 读 ui/Cargo.toml、更新检查读 core 的 `CARGO_PKG_VERSION`，
两个能各说各话 —— 实测发生过：包自称 0.1.2-beta.1，更新检查拿 0.1.3 去比。）

**构建顺序不能反**：先 `./package.sh`（app），**再** `./build-cli.sh`。
package.sh 的工作区构建会覆盖 `target/release/firectl`，覆盖出来的是**未签名**的。

**打包必须用 `ditto`**，`zip` 会破坏签名：
```
ditto -c -k --sequesterRsrc --keepParent target/FireVibe.app  FireVibe-<VER>-macos-arm64.zip
ditto -c -k --sequesterRsrc target/release/firectl            firectl-macos-arm64.zip
```

**发之前逐项验签**（ad-hoc 会毁掉 TCC 授权，见 macos-adhoc-sign-tcc-trap）：
```
codesign --verify --deep --strict target/FireVibe.app     # 内嵌 battprobe 会一起 validated
codesign -dv --verbose=2 target/release/firectl           # 要看到 Authority=Apple Development
```
发完再从 GitHub 下回来解包复验一次 —— 那才是用户真正拿到的那份。

**tag 规则**：app = `vX.Y.Z`，CLI = `cli-vX.Y.Z`（CLI 发 `--prerelease`，
因为应用的更新检查走 `/releases/latest`，那个接口会忽略预发布）。

## 红外遥控（`ActionType::IrBlast`）

遥控器**自带红外发射管**（0x0425 实物拆开确认过）。发射不走 HID，走 **BLE GATT
的 KeyMap 服务** —— macOS 只对 app 隐藏 HID 服务 `0x1812`，`FE151500` 是自定义服务，
CoreBluetooth 直接可达（和读电量同一条路）。

**协议全部摸清了**，写在 `~/LocalDev/firetv-remote-mac/NOTES.md` 的「红外发射」一节：
服务/特征、控制码、五步发射流程、字节格式。来源是 Fire TV 固件的
`BluetoothKeyMapLib`（`KeyMapActionIr` / `BleKeyMapDeviceProxyV2` / `BleConfig`）。

### 现在做到哪儿

- ✅ `core/src/ir.rs` —— 码的解析 / 校验 / 编译成设备载荷，20 个单测覆盖格式细节
- ✅ 内置码库 `core/src/irdb.rs`（Flipper-IRDB，CC0，1605 设备 / 25767 条码 / 0.99 MB）
- ✅ `ActionType::IrBlast` + 编辑器里的粘贴框 + 码库搜索 + 卡片摘要
- ✅ **官方遥控器（0x0421）走 blast** —— 端到端验证过（Daikin152，CRC 正确）
- ✅ **仿品（0x0425）走 MAPPING 键位表** —— 端到端验证过（NEC `112121DE` 逐位一致）
- ❌ 两条路都还只在 `helper/irblast.swift` 里，**app 里没接**

### 还差什么

`helper/irblast.swift` 已经把两条路都跑通了（**独立进程**，
不能在 app 进程里建 CBCentralManager，见 battprobe 顶部注释）。
剩下的是把它接进 `runtime.rs::ir_blast()`：按 `settings.mic_model` 分流 ——
`Hot`(0x0421) 走 blast，`Ptt`(0x0425) 走 `--mapping`。
⚠️ MAPPING 那条是**改键位表**不是一次性发射：得先存一份当前表、
按键和红外码的对应关系要落进配置，不能每次触发都重写一遍表。

### 两条硬限制（来自固件，不是我们定的）

- **最多 2 段**原始码（`for i in 0..<2`）
- 时长是**有符号 int16**，单位是 **10 µs 一格** → 上限 **327670 µs**
  ❗ 早先写成「上限 32767 µs」，**错的**，害得码全长了 10 倍还发不对。
  单位是拿 StackChan 抓码回来 diff 出来的 —— 设备对错误单位照样 ACK 0x02，
  **只靠回执判断不了对错，必须实测抓码**。

❗ **更正**：早先写过「空调塞不进去」，错的。「2 段」限制的是 Pronto 的
intro/repeat，**和帧数无关** —— 多帧编成一段连续序列即可，帧间隔就是长 space；
上限卡的是单个条目，Daikin 帧间隔 25~29 ms 远在限内。
❗ **尺寸上限至今没搞清，别信任何一个"实测值"。** 三个数据点自相矛盾：
同一条 263 脉冲的码，放进**电视那张四行都带码的表**（1127 B）→ ✅ 抓码正确；
放进**我们生成的三行 NO_ACTION 占位表**（614 B）→ ❌ 发兜底乱码。
**更小的表反而失败**，所以不是「载荷 ≤ N」，还和表的形状有关。
现在 `MAX_PAYLOAD_BYTES` 保守压到 **180 B / 72 脉冲**（只覆盖验证过的区间）。
⚠️ 超限时设备**照样回 0x02**，那个键却发 NEC 地址 `027D` 的兜底码 ——
**回执永远不能当验证，必须抓码**。要放宽先二分 + 抓码确认。
空调是**有状态协议**，一个按键只能对一个固定状态，所以要的是几条预设码
（制冷 26 度 / 关机 / 制热 22 度…），抓一次就行，不需要编码器。
更复杂的空调控制仍建议 StackChan + `IRremoteESP8266` 的编码器，
抓码规格见 `~/LocalDev/firetv-remote-mac/daikin-capture-spec.md`。

### ⚠️ 安全边界

- **绝不碰 `CFBFA000` OTA 服务** —— `CFBFA004` 里有 WIPE(12) / WIPE_UNPAIR(14)
- CONTROL 只发 1 / 2 / 5，永不发 16(DELETE_TABLE) / 32(ENABLE_SDS)
- 不对不明设备做 opcode 盲扫

❗ **「绝不提交 MAPPING」这条已作废。** 仿品不实现 blast，MAPPING 是唯一通路，
用户明确同意（「能恢复感觉就不是问题」）。代价是覆盖持久化键位表 ——
**在电视上跑一次「设备控制」就恢复原样**，实测过。真正不可逆的只有 OTA 那条。
写之前先把当前表存一份（`firetv-remote-mac/tv_table.hex` 就是这么来的），随时能写回。

### 仿品（0x0425）的配方 —— 三个坑，都踩过

```bash
irblast "BLE_TEST" "$(cat 表.hex)" --mapping --uuid-rand --wait 80   # OK = 回 0x02
```
1. **每行必须正好两个动作，而且四行都得在。** 红外行 = 红外 + `BLE_KEYPRESS`
   (type 5, flags 0, len 1, 载荷 `0x00`)；**没红外的行用 `NO_ACTION`(type 0, 零长度)
   占位**，不能只写一个动作。只写一行（哪怕动作齐）也不生效。
   三种违规**设备一律回 0x02**，抓码才看得出没生效。
2. 写完要 **CONTEXT_SWITCH**（CONTROL opcode 1，18 字节，少最后那个字节设备不认）。
3. 表 UUID **随机就行**，不用固定值（电视那边也是服务端每个 activity 随机生成一个）。

**实现**：`core/src/irtable.rs`（表的构建，7 个单测）。其中
`和真机验证过的编码器逐字节一致` 把**真机抓码验证过的那串字节**钉死成黄金样本 ——
格式跑偏在设备上是静默失败（照样回 0x02 就是不发），只靠真机很难发现。
表的解析器和电视原表在 `firetv-remote-mac/{surg.py, tv_table.hex}`。

⚠️ **排查「写了没生效」时一定要有对照组**：中间重写一次已知能生效的表，
确认还能发，再继续查内容 —— 不然分不清是表错了还是设备进了坏状态。

⚠️ **一手资料的拿法**：`BleKeyMapDeviceProxyV2.writeTable()` 里那行
`Log.v(TAG,…)` 是**无条件写日志**的（TAG `SBleKeyMapDeviceProxy`），
`adb logcat` 期间在电视上跑「设备控制」就能抓到完整表的十六进制。
不用 `setprop log.tag.*`（会被 SELinux 挡）。**比在黑盒里试参数快一个数量级** ——
我先黑盒试了几小时，回去看日志 20 分钟拿到全部答案。

## ★★★ 「连上了但一个报文都收不到」：先怀疑机器，别先查代码（2026-09-19 结案）

**症状**：app 显示已连接、三个 collection 全部打开成功、hidremap 下发了、语音链路也建好了
—— 但按键毫无反应、软遥控不亮、`stats` 一次都不涨。而**系统自己收得一清二楚**
（遥控器方向键在 terminal 里照常翻历史、麦克风键照样唤起第三方语音工具）。

**真相**：**macOS 的 HID 读取通路会坏，而且坏得非常安静。** 那台 Mac 连续运行 12 天后，
**任何进程都读不到任何 HID 设备的 input report** —— 拿内置触控板做对照，画圈 25 秒
**零报文**；而 `IOHIDCheckAccess` 从头到尾回答「已授权」。**重启 Mac 立刻恢复**
（重启当天就正常记了 11 次按键）。统计上也对得起来：`by_day` 从 09-06 断到 09-18，
而上一次开机正是 09-05。

### 排查顺序（照这个走，能省一整晚）

1. `pgrep -f MacOS/firevibe` —— **app 在不在跑**。不在就看 `~/Library/Logs/DiagnosticReports/`
2. `sample <pid>` —— 主线程**卡没卡在 `runModal`**（崩溃后的「要恢复窗口吗」模态框，见下一节）
3. **拿内置触控板做对照**（关键的一步，我这次绕了很久才做）：
   ```bash
   python3 -c "import json,os;p=os.path.expanduser('~/Library/Application Support/FireVibe/config.json');c=json.load(open(p));c['settings']['device_vid']='0x05ac';c['settings']['device_pid']='0x0342';json.dump(c,open('/tmp/kb.json','w'))"
   FIREVIBE_CONFIG=/tmp/kb.json FIREVIBE_LC_UP=0x0001 FIREVIBE_LC_USAGE=0x02 firectl --linkcheck
   ```
   **画圈 25 秒零报文 → 机器坏了，重启 Mac**，别再查代码。
4. 以上都正常，才轮到 Runtime（stop 标志、collection 路由那些，见下面两节）

### ⚠️ 这次的误判，一条都别再走

- ❌ **「是输入监控授权失效」** —— 查了很久。`tccutil reset ListenEvent` 跑了（报成功、没用）、
  用户在系统设置里关开开关、**最后把整条记录删掉** —— app 重启后**照样报「已授权」**。
  **`IOHIDCheckAccess` 在这种坏状态下会说谎**：记录都删了它还说有，而实际一条报文不给。
  以后别拿它当判据，**判据只有「实际读到没读到」**。
  （旁证：用户说以前手动删那条记录时，app 会立刻提示权限失效；这次删了毫无反应。）
- ❌ **「app 和 firectl 是两个独立授权主体，都失败所以是系统问题」** —— 推理本身没错，
  但前提是错的：**`--sniff` 走的正是 Runtime 同一份代码**（旧版这份文档里「firectl 不走
  Runtime 那条路」已经过时）。两个一起失败说明不了任何事。要独立验证必须用**不走 Runtime**
  的路径 —— `firectl --linkcheck` 就是为此加的（直接 hidapi open + read）。
- ❌ 还怀疑过并排除了：旧句柄失效（重启 app 无效）、hidremap 吞报文（清空映射无效，
  且方向键根本不在映射里）、遥控器没电（换电池前后一样，且方向键在 terminal 里能用）、
  Karabiner（`org.pqrs.Karabiner-DriverKit-VirtualHIDDevice` 确实 activated，
  但 `karabiner_grabber` 没在跑，不 grab 设备）。
- ⚠️ **最大的时间黑洞是「你按一下我看看」**：零报文分不清「设备没发」还是「用户没按」，
  我为此来回问了七八轮。**改用触控板画圈**——高频、可持续、而且用户本来就在动它，
  一次就能定性。**任何「零事件」结论，先让探针自证**（`--linkcheck` 会打印所有
  collection 和授权状态；battprobe `--scan` 会打印「所有设备的广播条数」）。

## ★★★ 「连上了、但一个 HID 报文都收不到」—— stop 标志没复位

**症状特别像硬件坏了**：设备打开成功、`Connected` 事件照发、hidremap 照下发、
蓝牙电量也读得到、语音链路也建好了 —— 就是**软遥控器不亮、按键毫无反应、
语音没有音频**。
⚠️ **更正：`firectl --sniff` 走的正是 Runtime 同一份代码**（当年那句「它不走 Runtime
那条路」已经不成立了），所以它**不能**用来验证 Runtime 的 bug —— 两者会一起失败。
要独立验证用 `firectl --linkcheck`（直接 hidapi open + read，不碰 Runtime）。

根因：`Runtime.stop` 是挂在 Runtime 上的 `Arc<AtomicBool>`，**跨连接存活**，
而 `start()` 里**没有把它复位成 false**。`start_runtime` 又永远是
「先 stop 再 start」，于是新起的读线程第一圈就 `break`。

```rust
pub fn start(&self) -> Result<()> {
    self.stop.store(false, Ordering::Relaxed);   // ← 少了这行就全哑
```

⚠️ 这类「跨连接存活的状态没复位」在这个项目里出现过**四次**，形态都一样
（一次性闩锁，坏了就永久坏，且完全静默）：

| 状态 | 后果 |
|---|---|
| `Runtime.stop` 不复位 | 读线程立刻退出，收不到任何报文 |
| UI 的 `voice_ready` | 重连会 `stop_voice()` 拆掉 sink，标志还是 true → **永不重建** |
| UI 的 `voice_rx` | 建 sink 的线程 panic → `try_recv` 永远 `Disconnected`，只认 `Ok` 就永久卡住 |
| `Settings.prev_input_id` | 卡在虚拟声卡上而没有还原目标 → 所有 app 的麦克风永久哑掉 |

**判据**：凡是「重连/重试」路径上被置位的状态，都要问一句「谁把它清回去」。

## ★★★ 点连接偶发闪退（EXC_BREAKPOINT / PAC）—— 读线程的 drop 顺序反了

**crash 签名**（`~/Library/Logs/DiagnosticReports/firevibe-*.ips`，认 faulting thread 顶几帧）：
`__CFCheckCFInfoPACSignature > CFRunLoopAddSource > IOHIDDeviceScheduleWithRunLoop >
__IOHIDManagerDeviceAdded > IOHIDManagerSetDeviceMatchingMultiple`。0.2.0/0.2.1/0.2.2
签名一模一样 —— 长期潜伏，遥控器整晚不在、pump 反复重连时放大概率。

`start()` 明明有保护（等 `hid_threads` 归零再放行 `HidApi::new()` 枚举），却仍崩。漏洞在
**读线程退出时的析构顺序**：计数减在 `ThreadCount`/`Alive` 的 `Drop` 里（闭包体内**局部**
变量），而 `HidDevice`（`dev`/`sec`）是闭包**捕获**变量 —— 捕获变量在闭包体局部之后才析构。
于是计数先归零、`hid_close`（同步 `CFRunLoopStop`+join 掉这台设备的 run loop）还没跑，
`start()` 一放行就 enumerate，新 IOHIDManager 撞上没拆干净的旧设备 run loop → CF 对象写坏。
修法：把设备 move 成体内局部、且声明在计数 guard **之后**（逆序析构 → 设备先 drop、
计数后 drop）。主读线程 `let dev = dev;` 放 `_alive` 之后；副读线程 `let _count = count;
let sec = sec;`。所有退出路径（正常/`?`/return/panic）都走体末逆序，一并覆盖。
⚠️ 判据延伸：不只问「谁把标志清回去」，还要问「持有系统资源的对象和它的计数 guard，谁先析构」。

## ★★★ PAC 崩溃的真凶是 `HidApi::new()` 反复 new/drop（2026-09-14 复发，已根治）

上一节那个 drop 顺序的修复**没修干净** —— 同样的签名在 0.2.3 又崩了一次
（`~/Library/Logs/DiagnosticReports/firevibe-2026-09-14-031315.ips`，栈顶一模一样：
`__CFCheckCFInfoPACSignature → CFRunLoopAddSource → IOHIDDeviceScheduleWithRunLoop
→ __IOHIDManagerDeviceAdded → IOHIDManagerSetDeviceMatchingMultiple`）。

**真因（前后归因错了两次，第三次才拿到源码证据 —— 过程本身是教训）**：

```c
/* hidapi/mac/hid.c :: init_hid_manager() */
hid_mgr = IOHIDManagerCreate(kCFAllocatorDefault, kIOHIDOptionsTypeNone);
IOHIDManagerSetDeviceMatching(hid_mgr, NULL);
IOHIDManagerScheduleWithRunLoop(hid_mgr, CFRunLoopGetCurrent(), kCFRunLoopDefaultMode);
//                                       ^^^^^^^^^^^^^^^^^^^ 谁先调用 hid_init，就绑谁的 run loop
```

全局 `hid_mgr` 被**绑死在「第一个调用 hidapi 的线程」的 run loop** 上。而
`Runtime::start()` 跑在 `start_runtime` spawn 出来的**临时线程**里 —— 那个线程做完就
结束、run loop 随之销毁，manager 却还记着它。此后任何一次枚举只要**匹配到设备**，
`__IOHIDManagerDeviceAdded` 就会往那个**已经死掉的 run loop** 上 `CFRunLoopAddSource`
→ PAC 校验失败 → 进程当场被打死。

**修法**：`device::warm_up_hid_api()`，在 **`main()` 第一屏、主线程**上先把 hidapi
初始化掉 —— 主线程 run loop 和进程同寿，永不失效。⚠️ 它必须早于任何后台线程碰
`hid_api()` / `list_hid()`，否则 init 又落到别的线程上。

**两次错误归因，都别再走**：

1. ❌「`HidApi` 反复 new/drop 把 manager 拆了又建」→ 改成全局单例后**崩得更狠**：
   以前每次 new/drop 至少跟当次线程同生共死（偶发才撞），单例之后它**永久**绑在
   第一个临时线程的尸体上，于是**设备一出现就必崩**。
2. ❌「`refresh_devices()` 用 NULL 匹配、把系统每个 HID 设备都 schedule 进去」→
   这条**方向没错但不是根因**：改成 `reset_devices()` + `add_devices(vid, pid)` 后，
   设备不在时确实不崩了（匹配集为空、不触发 deviceAdded），可**设备一回来照样崩**，
   崩溃栈还多出一帧 `IOHIDManagerSetDeviceMatching`（带匹配字典那条路）。
   这个改动**保留**了（只碰自己那三个 collection，触发面更小、也更快），但别把它
   当解药。

**教训**：崩溃栈指向 `IOHIDManagerSetDeviceMatching*` 时，**别猜调用频率、别猜对象
生命周期，去读 `hid.c`**。两次错误归因各自都「讲得通」，而且第一次还伴随着一次
「三分钟没崩」的假阳性验证 —— 那次其实是 app 卡在恢复窗口的模态框上、压根没跑到
枚举（见下一条）。**验证崩溃修复前，先确认进程真的在跑那条路径。**

❗ **配套的第二个坑：崩溃之后下一次启动会卡死在「要恢复窗口吗」的模态框上。**
`_handleAEOpenEvent → NSPersistentUIRestorer promptToIgnorePersistentStateWithCrashHistory:
→ NSAlert runModal` —— 主线程停在 `runModal`，app 在进程列表里活着、状态栏图标也在，
但**什么都不干**：连不上遥控器、按键毫无反应。这和「HID 读线程失效」长得一模一样，
我为此往 HID 层查了很久。
- 判据：`sample <pid>` 看主线程，栈里有 `runModal` 就是它，不是 HID 的事
- ⚠️ **Info.plist 的 `NSQuitAlwaysKeepsWindows=false` 挡不住它**（实测，别再试）——
  那个键管的是「正常退出时保不保存窗口」，这个框走崩溃历史那条路。
- 根治：只有 `ApplePersistenceIgnoreState` 管用，所以 **app 自己写进自家 defaults**
  （`core/src/tray.rs::install()` 开头，下次启动生效；装机时没人会手动 defaults write）
- 救急（当场解卡）：`defaults write com.tankxu.firevibe ApplePersistenceIgnoreState -bool YES`
  \+ 删掉 `~/Library/Saved Application State/com.tankxu.firevibe.savedState`，再重启 app

❗ **排障顺序（这一晚又验证了一遍）**：遥控器"没反应"时，先 `pgrep` 看 app 在不在 →
不在就查 `~/Library/Logs/DiagnosticReports/` → 在就 `sample <pid>` 看主线程卡没卡。
**这两步都排除了再去查 HID**。

## HID collection：三个全开，别再赌哪个送按键（2026-08-31 起）

macOS 把这支遥控器拆成三个 top-level collection：
`0x0001/0x06 键盘` · `0x000c/0x01 消费类` · **`0x00ff/0x00 厂商`**。
「按键从哪个出来」会随枚举顺序 / 配对状态 / 写没写过键位表**漂移**，
写死任何一个（包括厂商 0x00ff）都可能变成「连上了但一个报文都收不到」。
现在 `Runtime::start()` **全部打开**：主 collection（优先 0x00ff，命令和
音频从它走）留在主读线程，其余各起一条副线程转发报文进主循环，按
report id 处理；按键状态集幂等，重复报文无害。`FIREVIBE_HID_USAGE_PAGE`
覆盖主 collection，`FIREVIBE_HID_SINGLE=1` 退回单开（都是排障用）。

## 掉线是遥控器固件常态，治不了只能无感化（2026-08-31 结案）

仿品每次连接请求 peripheral latency=49；bluetoothd **硬编码拒绝 >30**
（"so we drain our battery and they don't - refusing"，strings 扫过无任何
defaults 开关，插电照拒）。要不到打盹许可，它**约 8 秒没有物理按键就
主动断链省电**：按住键续命、主机侧任何写入（HID/GATT）都不算活动
（keepalive 实测无效）。Fire TV 批准 latency，所以在电视上"永远在线"。
「以前能保持连接」是 connected 标志不清零的 bug 造成的 UI 假常亮。
应对：断开只写日志不弹错误条、300ms 自动抓回、tap 无条件吞 0xb1
（唤醒键到达时 connected 必为 false）、隐式按住会话（PTT 只有实体麦克风
键按住才吐流 → "音频来了而没见过按下"=用户正按着，开音频闸门；对走
硬件层映射的键**绝不补合成** —— 豆包只认硬件来源，合成的还会把它的
热键状态机搞乱）。

## ★★★ 原厂遥控器待机耗电：链路是 macOS 维持的，用户空间断不掉（2026-09-10 结案）

**症状**：四天一次没按，电量从满掉到 12%。

实测（每条都有可信探针）：

| 检查 | 结果 |
|---|---|
| `system_profiler SPBluetoothDataType` | 四天没碰它，一直在 `Connected` |
| **把 FireVibe 整个退掉**，观察六分钟 | 照样 `Connected` |
| 麦克风热着吗（`firectl --sniff` 十秒） | **零报文**，没热着 |
| 系统休眠 | Amphetamine 按着 113 小时没睡 |
| `stats.by_day` | 最后一条就是四天前，确实没按 |

❗ **推翻旧结论两条**：

1. runtime.rs 里那句「这台遥控器只要 app 握着 HID 句柄就不休眠」——**不准确**，
   句柄放了（app 完全退出）它照样连着。链路是 **macOS** 维持的。
2. `23c4bdc`「非 PTT 连上过就不再自动重连，让它睡、不吵它」——**前提和后果都错了，
   已撤回**（`want_retry` 现在恒为 true，只是非 PTT 的重试间隔放到 1.5 秒）。
   前提错：原厂压根不会自己断链，不追它也照样在线，一点电都省不下来。
   后果错得更疼：链路**真断了**的时候（见下面「系统设置那个按钮是真能断的」），
   这条重试是**唯一**的恢复路径 —— 不追的话遥控器已经按键唤醒、macOS 也连回来了，
   FireVibe 那边还是「未连接」，只能手动点「连接」或重启 app。用户实际撞上了。
   会自己断链的只有仿品（PTT，要不到 latency，8 秒就断，见上一节）。

### ✅ 断链是**能做到**的（2026-09-19 实测通，推翻此前结论）

**配方**（`helper/battprobe.swift --disconnect` / `core/src/btlink.rs` / `firectl --disconnect`）：

1. `CBCentralManager.retrieveConnectedPeripherals(withServices:)` 找到它
2. **先 `connect(p)`** ← **关键的一步**
3. `cancelPeripheralConnection(p)`（公开 API）；三秒后还连着就降级到私有的
   `cancelPeripheralConnection:force:`（force=YES）

实测：三个探针同步归零（`ioreg -c IOHIDDevice` 节点消失、`blueutil --paired` 变
not connected、CB 的已连接列表里也没了），断开**保持三分钟以上不被拽回来**，
和手动点系统设置那个按钮效果一致。

❌ **此前错在哪（别再走）**：

- `IOBluetoothDevice.closeConnection()` 是**空转** —— 返回 `kIOReturnSuccess` 却什么
  都没发生。那套 API 是给 classic 蓝牙的，对已配对的 BLE HID 无效；`blueutil --disconnect`
  同理。**佐证**：系统设置的蓝牙面板（`/System/Library/ExtensionKit/Extensions/Bluetooth.appex`）
  `otool -L` 看下去**只链 CoreBluetooth，根本没链 IOBluetooth**。
- **只调 `cancelPeripheralConnection` 而不先 `connect`** —— 它的语义是「取消**我自己**
  的连接」，没 connect 过就无从取消，**静默无效**。我卡在这里，据此下了
  「断链在用户空间是死的」的结论，**并且把整个「闲时自动断链」功能删掉了** ——
  两件事都是错的。
- ⚠️ **`blueutil --is-connected` 对 BLE 设备会骗人**：它报 `0` 的同时
  `blueutil --paired` 和 `system_profiler` 都说 connected，`firectl --sniff` 还能正常
  打开这台设备。判据用 **`blueutil --paired` 那行的 connected 字段**，或
  **`ioreg -c IOHIDDevice | grep -c "Amazon Remote"`**。
- ⚠️ **IOBluetooth 现在内部包着 CoreBluetooth**：`+[IOBluetoothDevice pairedDevices]`
  → `IOBluetoothCoreBluetoothCoordinator` → `semaphore_wait`。在**没有蓝牙授权**的
  进程里它**永久挂住**，不报错不超时 —— 又是 battery.rs 顶部那套症状。
- 真正的功耗旋钮（connection interval / slave latency）由从设备请求、主机批准，
  而 `bluetoothd` 硬编码拒绝 latency > 30（见上一节），那条确实调不了。

**找到答案的方法**（比黑盒试快一个数量级，值得记）：去翻**系统设置自己**用了什么 ——
`otool -L` 看它链了哪些框架（只有 CoreBluetooth ⇒ IOBluetooth 那条路从一开始就不可能），
`otool -v -s __TEXT __objc_methname` 看它调了什么 selector（挖出 `disconnectWithCompletion:`），
再用 objc 运行时反查宿主类（`objc_copyClassNamesForImage` + `class_getInstanceMethod`；
⚠️ 别用 `NSStringFromClass` 扫全部类，碰到 `__NSGenericDeallocHandler` 会 abort，
用纯 C 的 `class_getName`）。

❗ **2026-09-16 补完机制（一小时实测，探针全程自证）**：断开之后遥控器是**真睡着**的
—— 一小时**零广播**（同期其他设备 39458 条，证明扫描一直在工作）、658 次状态采样
全程 `not connected`，**没有「自己周期性醒来招人连」这回事**。所以完整的因果是：

1. 它会休眠，睡着时不广播、基本不耗电；
2. 但**睡着 ≠ 断开** —— 只要链路还挂着，它作为从设备就得按 connection interval
   持续醒来应答，而它要的打盹许可被 `bluetoothd` 拒（latency 49 > 30），这才是掉电来源；
3. 链路一旦建立**不会自己散**（2026-09-14 03:12 那次是唯一见过的例外），macOS 也不主动断；
4. 唤醒只要物理碰一下：碰到 → 它广播 → macOS 见到已配对设备立刻连上 → 从此一直挂着。

**所以「几天没按却一直连着」= 某次碰到它之后就再没断过**，不是它在反复招人。
用户侧唯一有效的省电手段：不用时在系统设置点一次「断开连接」，它能一直睡到你按它。
用 `helper/battprobe.swift --scan`（只扫描不连接，带「所有设备广播数」自证）复验。

❗ **「系统设置 › 蓝牙 › 断开连接」那个按钮是真能断的**（2026-09-11 用户点、我在录）：
两个探针**同步归零**（`ioreg -c IOHIDDevice` 里的节点消失、`blueutil --paired` 变
not connected），而且**不碰遥控器它就一直断着**，macOS 不会自己把它拽回来；
按一下遥控器就正常连回来。对比 `closeConnection` 的「返回成功、什么都没发生」，
说明**系统里确实存在一条能断掉的路径，只是不是 IOBluetooth 那个 API**。
下次要做省电，从这里查（系统设置用的是哪条调用），别再碰 `closeConnection`。

### 结论：怎么省、各自值多少（别高估 app 能做的事）

| 手段 | 归谁 | 效果 | 证据 |
|---|---|---|---|
| **`firectl --disconnect`**（或手动点系统设置那个按钮） | 用户/CLI | **唯一真正有效**：它立刻真睡，一小时零广播、不会被连回来 | 一小时扫描 + 658 次状态采样；CLI 版 2026-09-19 实测通 |
| 让 Mac 睡 / 取消配对 | 用户 | 同理有效（链路没了它就睡） | 推论，未单独实测 |
| 麦克风别热着（开局补关麦 + 自愈） | app | 防「一两天吃掉 30%」那种量级的漏电 | 早就在做；本轮 `--sniff` 十秒零报文，确认没热着 |
| 电量轮询改按需 | app | 把**我们自己**制造的连接活动从一天 288 次降到 ≤48 次、长期不用趋近 0 | `spawn_tracker(1800, gate)`；实测 5.5 小时 11 轮 ≈ 30 分钟一轮 |
| 在 app 里做「一键断链 / 闲时自动断」 | app | **可以做**（底座已就绪，还没接进界面） | `btlink::disconnect()`，见上面的配方 |

⚠️ **别把电量轮询那条当成解药**：主因是**链路常驻时的持续应答**，轮询只是我们自己
额外叠上去的一点活动，**收益没有量化过**。真正把电量拉回来的是「不用就断开」。
（本轮实测佐证：用户换电池后 `last_battery` 73%，而这几天 app 侧行为没变。）

⚠️ 闸门有个已知缺陷：`battery::last().is_none()` 那条是为了「开机先拿一个值填界面」，
但**电量一直读不到时它每轮都会放行**（`last()` 永远是 None）。30 分钟一轮仍比原来的
5 分钟好 6 倍，先这样；真要修就加个「本次启动已尝试过 N 次就不再试」的计数。

✅ **「闲时自动断链」已实现**（2026-09-19，端到端验证过：连上 → 闲置 → 自动断开 →
按键唤醒 → app 自动抓回来）。

- 配置：`Settings.idle_sleep_min`，默认 **30 分钟**，设置页可调（10 分钟一档，0=不休眠）
- 逻辑：`ui/src/main.rs::poll_idle_sleep()`。触发条件 = **窗口收起**（`tray::is_hidden()`）
  + 非 PTT + 没在送流（`status.mic_on`）+ 距「最后一次按键 / 连上」超过阈值
- 断链在**后台线程**做（要等 helper 最多 20 秒，绝不能卡 UI），期间 `sleep_hold_until`
  抑制重连；断开后**必须保持自动重连**（`want_retry` 恒为 true），否则用户按键唤醒、
  macOS 都连回来了，FireVibe 还是「未连接」
- 排障开关：`FIREVIBE_IDLE_SEC=<秒>` 把阈值改成秒 —— 不然验一次要等半小时。
  ⚠️ **窗口开着不计时**，用这个开关测的时候记得先点红叉把窗口收起，否则永远不触发
  （我第一次就栽在这，以为逻辑没生效）

⚠️ **实现时踩的两个坑（都会表现成「断链失败」）**：

1. **别给断链加「优先公开 API」的降级**——除非中间**重新 connect**。
   公开的那次 `cancelPeripheralConnection` 会把**我们自己的连接引用取消掉**，
   紧接着调 force 变体时已经没有引用可 force，必然失败（退出码 5）。
   我加完这个"稳妥"的降级，原本验证通过的配方就一直断不掉。
2. **helper 的兜底超时要给够**。断链流程 ≈ 11 秒（connect 2s + 公开 cancel 3s +
   重连 2s + 检查 4s），而 battprobe 原来沿用读电量那个 **8 秒**兜底，会抢在流程完成前
   `exit(5)` —— 表现成**「报失败但设备其实断开了」**，极具误导性。现在断链模式单独用
   18 秒（父进程 `btlink.rs` 超时 20 秒）。

## 配置文件必须原子写

`save_to` 以前用 `std::fs::write`（**先截断再写**），而统计是**每按一次键就存一次**——
写到一半进程被杀就留下半个文件；`load()` 那边解析失败又**静默退回出厂默认**，
接着 `normalize` 触发保存，**把用户配置永久覆盖**。真出过：整套方案 + 累计统计全没了。

现在：临时文件 → `sync_all()` → `rename`；解析失败**保留原文件**为
`config.json.corrupt-<pid>` 并在 stderr 吼一声，绝不覆盖。

## 排障纪律（这一晚栽的）

- **先确认 app 在不在跑**。我对着一个已经退出的进程测了很久，据此推出
  「遥控器被写坏了」「Fire TV 在抢」「输入监控掉了」「该换电池了」
  —— 全是错的，还让用户抠电池、拔电源、重配三次。
- **探针要先自证**。`hidutil list` 那次一个设备都没列出来，我拿它的
  「0/200 在线」下了结论。换成 `ioreg -c IOHIDDevice` 并先验证它能列出
  已知设备之后，数据才可信。
- **判据别用 `awk '{print $NF}'`**。"FireVibe Mic" 的最后一个词是 "Mic"，
  `grep FireVibe` 直接判负 —— 差点把一次成功的切换判成失败。
- **失败路径要有日志**。连不上、语音链路建不起来，以前全都只写进界面，
  stderr 一片空白 —— 这是这一晚花掉最多时间的原因。
