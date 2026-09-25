# 第三方组件

`build.sh` 会在构建时从上游 clone
[BlackHole](https://github.com/ExistentialAudio/BlackHole)（**GPL-3.0**），
改一处传输类型后编译成 `FireVibeMic.driver`。

**本仓库不包含 BlackHole 的任何源码** —— 它是构建时拉取的。对它的改动只有一处，
完整记录在 `build.sh` 里那段 patch（把硬编码的 `kAudioDeviceTransportTypeVirtual`
改成编译期常量 `kTransportType`，然后按 USB 编译）。

为什么要改：第三方语音输入工具会把传输类型为「虚拟」的设备从麦克风候选里滤掉，
自称 USB 的实例才会被列出来。

编出来的 `.driver` 是 GPL-3 作品的衍生品，分发它需要遵守 GPL-3。
FireVibe 自身（本仓库代码）是 MIT，驱动作为独立的 CoreAudio 插件运行在
`coreaudiod` 进程里，不与 FireVibe 链接。

---

## 仓库里为什么有一份编好的 `prebuilt/FireVibeMic.driver`

发布产物由 GitHub Actions 在一次性干净环境里构建（见 `.github/workflows/release.yml`），
而 **runner 上没有代码签名证书**，`build.sh` 会直接拒绝编 —— HAL 驱动必须有有效签名，
`coreaudiod` 才肯加载。所以这一份是在本机用真证书编好后提交进来的。

`package.sh` 的取用顺序是 `driver/out/`（本地刚编的）→ `driver/prebuilt/`（这一份）。
实测 **`codesign --deep` 不会重签 `Contents/Resources/` 里的 bundle**，所以即便发布产物
本身走 ad-hoc 签名，这个驱动仍然带着证书签名到达用户手里。

**更新它**（上游升级、或换了签名证书时）：

```bash
./driver/build.sh
ditto driver/out/FireVibeMic.driver driver/prebuilt/FireVibeMic.driver
```

### 对应源码（GPL-3 要求）

这份二进制是 GPL-3 作品 BlackHole 的衍生物。对应源码 =
**上游 <https://github.com/ExistentialAudio/BlackHole> 的 `v0.7.1`（commit `62953f5`）**
\+ `build.sh` 里那段 patch（唯一的改动：把硬编码的 `kAudioDeviceTransportTypeVirtual`
改成编译期常量 `kTransportType`，再按 USB 编译）。跑一次 `./driver/build.sh` 即可
从上游重建出同一份产物 —— 脚本会自己 clone 上游并应用那段 patch。
