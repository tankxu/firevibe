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
验证：`hidutil property --matching '{"ProductID":0x0425,"VendorID":0x0171}' --get UserKeyMapping`
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

- ✅ `core/src/ir.rs` —— 码的解析 / 校验 / 编译成设备载荷，7 个单测覆盖格式细节
- ✅ `ActionType::IrBlast` + 编辑器里的粘贴框（边填边校验）+ 卡片摘要
- ❌ **发射通道没接** —— 触发时只校验+编译，然后如实说「通道还没接通」

### 还差什么

一个独立的 CoreBluetooth 小进程（照 `helper/battprobe.swift` 的架子，
**不能在 app 进程里建 CBCentralManager**，见 battprobe 顶部注释），执行：
`requestStartNewTable` → 订阅 BLAST → 分块写（200 字节/片）→ `commitBlast`(opcode 5) → 等 notify。
外层 `compileAction` / `compileAsBlast` 的确切帧格式还没读完（都在反编译产物里）。

### 两条硬限制（来自固件，不是我们定的）

- **最多 2 段**原始码（`for i in 0..<2`）
- 时长是**有符号 int16**（`(short) data`）→ 上限 **32767 µs**

→ 只适合电视/音响那类短码。**空调塞不进去**：Daikin 经典 ARC 每次发 3 帧、
帧间隔 25~35 ms。空调交给 StackChan + `IRremoteESP8266` 的编码器，
抓码规格见 `~/LocalDev/firetv-remote-mac/daikin-capture-spec.md`。

### ⚠️ 安全边界

- 只走 blast（一次性、有暂存表，`RESET_STAGING_TABLE`=2 可回滚）
- **绝不提交 `FE151501` MAPPING** —— 写坏持久化按键映射要重新配对才能恢复
- **绝不碰 `CFBFA000` OTA 服务** —— `CFBFA004` 里有 WIPE(12) / WIPE_UNPAIR(14)
- CONTROL 只发 1 / 2 / 5，永不发 16(DELETE_TABLE) / 32
