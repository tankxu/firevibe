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
