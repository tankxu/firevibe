// 让遥控器打一发红外。
//
// 走 BLE GATT 的 KeyMap 服务（`FE151500`）—— macOS 只对 app 隐藏 HID 服务 `0x1812`，
// 这个自定义服务 CoreBluetooth 直接可达，和读电量同一条路。
//
// 为什么又是个独立小程序：和 battprobe 同一个原因 —— 在 FireVibe 进程里用 objc2 建
// CBCentralManager，状态永远停在 Unknown、delegate 一次都不回调。独立进程一次就成。
// 蓝牙授权归责到父进程，所以由 FireVibe.app 启动它。
//
// 流程抄自固件 `BleKeyMapDeviceProxyV2.blastCommand()`：
//   ① CONTROL 写 [2][1][16 字节 UUID][u16-le 0][u16-le 表长度]  —— 申请开一张暂存表
//   ② 订阅 BLAST 的 notify
//   ③ 表数据分片写 BLAST，**每片定长 200 字节**（最后一片补零，不截断）
//   ④ CONTROL 写 [5] —— 提交并发射
//   ⑤ 等 notify 或轮询读 BLAST，首字节 == 2 即成功
//
// ⚠️ 只碰 CONTROL 的 2 / 5 两个操作码和 BLAST 特征。
//    绝不写 MAPPING（`FE151501`，写坏持久化按键映射要重新配对才能恢复），
//    绝不碰 OTA 服务（`CFBFA000`，里面有 WIPE / WIPE_UNPAIR）。
//
// 用法：irblast <设备名片段> <表的十六进制> [--scan-id N]
import Foundation
import CoreBluetooth

let args = Array(CommandLine.arguments.dropFirst())
guard args.count >= 2 else {
    FileHandle.standardError.write("用法: irblast <设备名> <表 hex>\n".data(using: .utf8)!)
    exit(64)
}
let wantName = args[0]
let tableHex = args[1]

// 几个只能靠试的点，做成开关：
//   --verify N   开表命令第 2 个字节（固件写 1；BleConfig 里 0=NONE 1=SHA2）
//   --sha        表数据后面附 32 字节 SHA-256，并把长度算进去
//   --uuid-rand  开表用随机 UUID（固件是表 id 的 UUID，我们默认全零）
func flagInt(_ k: String, _ dflt: Int) -> Int {
    guard let i = args.firstIndex(of: k), i + 1 < args.count else { return dflt }
    return Int(args[i + 1]) ?? dflt
}
let verifyByte = UInt8(flagInt("--verify", 1))
let wantSha = args.contains("--sha")
let randUuid = args.contains("--uuid-rand")

func note(_ m: String) {
    FileHandle.standardError.write((m + "\n").data(using: .utf8)!)
}

func fromHex(_ s: String) -> Data? {
    let clean = s.filter { !$0.isWhitespace }
    guard clean.count % 2 == 0 else { return nil }
    var d = Data()
    var i = clean.startIndex
    while i < clean.endIndex {
        let j = clean.index(i, offsetBy: 2)
        guard let b = UInt8(clean[i..<j], radix: 16) else { return nil }
        d.append(b)
        i = j
    }
    return d
}

// Swift 的类不能闭包捕获外层局部变量，所以做成全局常量
let table: Data = {
    guard let d = fromHex(tableHex), !d.isEmpty else {
        note("表的十六进制解不开")
        exit(65)
    }
    return d
}()

import CryptoKit
/// 真正要写下去的字节。`--sha` 时在表后附 32 字节 SHA-256。
let payload: Data = {
    guard wantSha else { return table }
    var d = table
    d.append(Data(SHA256.hash(data: table)))
    return d
}()

let SVC = CBUUID(string: "FE151500-5E8D-11E6-8B77-86F30CA893D3")
let CTRL = CBUUID(string: "FE151502-5E8D-11E6-8B77-86F30CA893D3")
let BLAST = CBUUID(string: "FE151503-5E8D-11E6-8B77-86F30CA893D3")
let CHUNK = 200

// 控制码。只用这两个 —— 16(删表) / 32 一律不碰。
let CTRL_START_TABLE: UInt8 = 2
let CTRL_COMMIT_BLAST: UInt8 = 5

var gotState = false

/// 退出前一定要断开。连了不断，攒上十几个会话之后设备就开始拒收所有写入
/// （CONTROL 报 Unknown ATT error），只能等它自己回收或者按一下遥控器唤醒。
/// 这是实测踩到的，别删。
func bail(_ code: Int32) -> Never {
    if let p = liveConn.0, let c = liveConn.1 {
        c.cancelPeripheralConnection(p)
        Thread.sleep(forTimeInterval: 0.15)
    }
    exit(code)
}
var liveConn: (CBPeripheral?, CBCentralManager?) = (nil, nil)

final class Blaster: NSObject, CBCentralManagerDelegate, CBPeripheralDelegate {
    var central: CBCentralManager!
    // ⚠️ 必须持有外设，否则 didConnect 永远不来（battprobe 踩过）
    var targets: [CBPeripheral] = []
    var ctrl: CBCharacteristic?
    var blast: CBCharacteristic?
    var step = 0
    var lastSeen: UInt8 = 0xFF
    var stepName = "init"

    func centralManagerDidUpdateState(_ c: CBCentralManager) {
        gotState = true
        guard c.state == .poweredOn else {
            note("蓝牙没开或没授权 state=\(c.state.rawValue)")
            bail(2)
        }
        let all = c.retrieveConnectedPeripherals(withServices: [SVC])
        let hit = all.filter { ($0.name ?? "").contains(wantName) }
        guard let p = hit.first else {
            note("没找到名字含「\(wantName)」且带 KeyMap 服务的设备（在线的：\(all.map { $0.name ?? "?" })）")
            bail(3)
        }
        targets = [p]
        p.delegate = self
        c.connect(p, options: nil)
    }

    func centralManager(_ c: CBCentralManager, didConnect p: CBPeripheral) {
        liveConn = (p, c)
        p.discoverServices([SVC])
    }

    func peripheral(_ p: CBPeripheral, didDiscoverServices e: Error?) {
        guard let s = (p.services ?? []).first(where: { $0.uuid == SVC }) else {
            note("这台设备没有 KeyMap 服务")
            bail(4)
        }
        p.discoverCharacteristics([CTRL, BLAST], for: s)
    }

    func peripheral(_ p: CBPeripheral, didDiscoverCharacteristicsFor s: CBService, error e: Error?) {
        for ch in s.characteristics ?? [] {
            if ch.uuid == CTRL { ctrl = ch }
            if ch.uuid == BLAST { blast = ch }
        }
        guard let ctrl, let blast else {
            note("KeyMap 服务里缺 CONTROL 或 BLAST 特征")
            bail(4)
        }
        p.setNotifyValue(true, for: blast)
        // --reset：只发一个裸 [2]（RESET_STAGING_TABLE）把暂存表状态机复位。
        // 会话没走完就退出会让设备后续拒收（写 CONTROL 报 Unknown ATT error）。
        if args.contains("--reset") {
            note("复位暂存表")
            p.writeValue(Data([CTRL_START_TABLE]), for: ctrl, type: .withResponse)
            DispatchQueue.main.asyncAfter(deadline: .now() + 1.0) {
                print("RESET")
                bail(0)
            }
            return
        }
        // --ctx：先发一次上下文切换（控制码 1）。电视配对后也走这一步（日志里的
        // state_switch），可能是 blast 的前置条件。
        if args.contains("--ctx") {
            note("⓪ 上下文切换")
            p.writeValue(Data([1]), for: ctrl, type: .withResponse)
            Thread.sleep(forTimeInterval: 0.3)
        }

        // ① 申请开一张暂存表
        var cmd = Data([CTRL_START_TABLE, verifyByte])
        cmd.append(randUuid ? Data((0..<16).map { _ in UInt8.random(in: 0...255) })
                            : Data(repeating: 0, count: 16))
        cmd.append(contentsOf: [0, 0])                                   // u16-le 0
        cmd.append(contentsOf: withUnsafeBytes(of: UInt16(payload.count).littleEndian) { Array($0) })
        stepName = "开表后"
        note("① 申请开表 verify=\(verifyByte) sha=\(wantSha) 长度 \(payload.count)")
        p.writeValue(cmd, for: ctrl, type: .withResponse)
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.2) { p.readValue(for: ctrl) }
    }

    func peripheral(_ p: CBPeripheral, didWriteValueFor ch: CBCharacteristic, error e: Error?) {
        if let e {
            note("写 \(ch.uuid.uuidString) 失败：\(e.localizedDescription)")
            bail(5)
        }
        guard let ctrl, let blast else { return }

        // ③ 分片写数据。每片定长 200，最后一片补零 —— 固件就是这么干的，
        //    设备靠 ① 里声明的长度定真实边界。
        let chunks = (payload.count + CHUNK - 1) / CHUNK
        if step < chunks {
            var piece = Data(repeating: 0, count: CHUNK)
            let lo = step * CHUNK
            let hi = min(lo + CHUNK, payload.count)
            piece.replaceSubrange(0..<(hi - lo), with: payload[lo..<hi])
            note("③ 写第 \(step + 1)/\(chunks) 片")
            step += 1
            stepName = "第\(step)片后"
            p.writeValue(piece, for: blast, type: .withResponse)
            p.readValue(for: ctrl)
            return
        }
        if step == chunks {
            step += 1
            note("④ 提交发射")
            stepName = "提交后"
            p.writeValue(Data([CTRL_COMMIT_BLAST]), for: ctrl, type: .withResponse)
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.3) { p.readValue(for: ctrl) }
            // ⑤ 等回执。固件是先等 notify，收不到就轮询最多 9 次、间隔递增 ——
            //    照抄这个节奏，一次读到 0x00 不代表失败，可能只是还没执行完。
            for i in 1...9 {
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.25 * Double(i)) {
                    p.readValue(for: blast)
                }
            }
        }
    }

    func peripheral(_ p: CBPeripheral, didUpdateValueFor ch: CBCharacteristic, error e: Error?) {
        // 调试：CONTROL 上有真状态（首字节 + 16 字节表 UUID），把每步之后的都打出来
        if ch.uuid == CTRL, let v = ch.value {
            note("   CTRL[\(stepName)] = \(v.map { String(format: "%02x", $0) }.joined())")
            return
        }
        guard ch.uuid == BLAST, let v = ch.value, let first = v.first else { return }
        // 首字节 == 2 就是成功（固件的 tableWriteCompleted）。
        // 0x00 是「还没结果」，继续等 —— 别一读到就判失败。
        if first == 2 {
            print("OK")
            bail(0)
        }
        if first != 0 {
            note("设备回了 0x\(String(format: "%02X", first))（成功应该是 0x02）")
            bail(6)
        }
        lastSeen = first
    }
}

let b = Blaster()
b.central = CBCentralManager(delegate: b, queue: nil)
DispatchQueue.main.asyncAfter(deadline: .now() + 12) {
    note("等回执超时，最后读到 0x\(String(format: "%02X", b.lastSeen))")
    exit(gotState ? 7 : 2)
}
RunLoop.main.run()
