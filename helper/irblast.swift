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

let SVC = CBUUID(string: "FE151500-5E8D-11E6-8B77-86F30CA893D3")
let CTRL = CBUUID(string: "FE151502-5E8D-11E6-8B77-86F30CA893D3")
let BLAST = CBUUID(string: "FE151503-5E8D-11E6-8B77-86F30CA893D3")
let CHUNK = 200

// 控制码。只用这两个 —— 16(删表) / 32 一律不碰。
let CTRL_START_TABLE: UInt8 = 2
let CTRL_COMMIT_BLAST: UInt8 = 5

var gotState = false

final class Blaster: NSObject, CBCentralManagerDelegate, CBPeripheralDelegate {
    var central: CBCentralManager!
    // ⚠️ 必须持有外设，否则 didConnect 永远不来（battprobe 踩过）
    var targets: [CBPeripheral] = []
    var ctrl: CBCharacteristic?
    var blast: CBCharacteristic?
    var step = 0

    func centralManagerDidUpdateState(_ c: CBCentralManager) {
        gotState = true
        guard c.state == .poweredOn else {
            note("蓝牙没开或没授权 state=\(c.state.rawValue)")
            exit(2)
        }
        let all = c.retrieveConnectedPeripherals(withServices: [SVC])
        let hit = all.filter { ($0.name ?? "").contains(wantName) }
        guard let p = hit.first else {
            note("没找到名字含「\(wantName)」且带 KeyMap 服务的设备（在线的：\(all.map { $0.name ?? "?" })）")
            exit(3)
        }
        targets = [p]
        p.delegate = self
        c.connect(p, options: nil)
    }

    func centralManager(_ c: CBCentralManager, didConnect p: CBPeripheral) {
        p.discoverServices([SVC])
    }

    func peripheral(_ p: CBPeripheral, didDiscoverServices e: Error?) {
        guard let s = (p.services ?? []).first(where: { $0.uuid == SVC }) else {
            note("这台设备没有 KeyMap 服务")
            exit(4)
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
            exit(4)
        }
        p.setNotifyValue(true, for: blast)

        // ① 申请开一张暂存表。UUID 用全零 —— 一次性发射不需要索引到具体表。
        var cmd = Data([CTRL_START_TABLE, 1])
        cmd.append(Data(repeating: 0, count: 16))
        cmd.append(contentsOf: [0, 0])                                   // u16-le 0
        cmd.append(contentsOf: withUnsafeBytes(of: UInt16(table.count).littleEndian) { Array($0) })
        note("① 申请开表，长度 \(table.count)")
        p.writeValue(cmd, for: ctrl, type: .withResponse)
    }

    func peripheral(_ p: CBPeripheral, didWriteValueFor ch: CBCharacteristic, error e: Error?) {
        if let e {
            note("写 \(ch.uuid.uuidString) 失败：\(e.localizedDescription)")
            exit(5)
        }
        guard let ctrl, let blast else { return }

        // ③ 分片写数据。每片定长 200，最后一片补零 —— 固件就是这么干的，
        //    设备靠 ① 里声明的长度定真实边界。
        let chunks = (table.count + CHUNK - 1) / CHUNK
        if step < chunks {
            var piece = Data(repeating: 0, count: CHUNK)
            let lo = step * CHUNK
            let hi = min(lo + CHUNK, table.count)
            piece.replaceSubrange(0..<(hi - lo), with: table[lo..<hi])
            note("③ 写第 \(step + 1)/\(chunks) 片")
            step += 1
            p.writeValue(piece, for: blast, type: .withResponse)
            return
        }
        if step == chunks {
            step += 1
            note("④ 提交发射")
            p.writeValue(Data([CTRL_COMMIT_BLAST]), for: ctrl, type: .withResponse)
            // ⑤ 收不到 notify 就主动读一次兜底
            DispatchQueue.main.asyncAfter(deadline: .now() + 0.4) {
                p.readValue(for: blast)
            }
        }
    }

    func peripheral(_ p: CBPeripheral, didUpdateValueFor ch: CBCharacteristic, error e: Error?) {
        guard ch.uuid == BLAST, let v = ch.value, let first = v.first else { return }
        // 首字节 == 2 就是成功（固件的 tableWriteCompleted）
        if first == 2 {
            print("OK")
            exit(0)
        }
        note("设备回了 0x\(String(format: "%02X", first))（成功应该是 0x02）")
        exit(6)
    }
}

let b = Blaster()
b.central = CBCentralManager(delegate: b, queue: nil)
DispatchQueue.main.asyncAfter(deadline: .now() + 12) {
    note("超时")
    exit(gotState ? 7 : 2)
}
RunLoop.main.run()
