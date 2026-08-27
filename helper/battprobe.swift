// 读遥控器电量，只往 stdout 打一个整数（读不到就什么都不打、退出码 1）。
//
// 为什么要独立成一个小程序：在 FireVibe 进程内用 objc2 建 CBCentralManager，
// 状态永远停在 Unknown(0)、delegate 回调一次都不来（selector 确认已注册、
// 也给了专属 dispatch 队列、主线程/后台线程都试过）。同样的逻辑在独立进程里
// 一次就成。原因没查清，所以走这条能用的路。
//
// TCC 归责到父进程：由 FireVibe.app 启动它，用的就是 FireVibe 的
// NSBluetoothAlwaysUsageDescription。
import Foundation
import CoreBluetooth

let args = Array(CommandLine.arguments.dropFirst())
// --dump：枚举全部 GATT 服务/特征（只读，不写任何特征），用来判断语音走不走 GATT
let dumpMode = args.contains("--dump")
// --listen：订阅所有 notify 特征，把推过来的字节按时间打出来。**只读**：不写任何
// 特征、不发任何厂商命令。用来验证「国产遥控器的语音是不是走它自己的 FFF0 私有服务」——
// 那一系（Amlogic/Telink 方案）按下麦克风键就自己推流，不需要主机下命令。
let listenMode = args.contains("--listen")
// --readall=<服务UUID前缀>：把该服务下所有**可读**特征的当前值读出来打印。
// 纯读，不写任何东西 —— 用来判断固件是真实现了这个服务还是只挂了个空壳。
let readAllSvc: String? = args.first(where: { $0.hasPrefix("--readall=") }).map {
    String($0.dropFirst(10)).uppercased()
}
// 监听时长，默认 30 秒
let listenSecs: Double = {
    guard let a = args.first(where: { $0.hasPrefix("--secs=") }),
          let v = Double(a.dropFirst(7)) else { return 30 }
    return v
}()
let want = args.first(where: { !$0.hasPrefix("--") }) ?? "Amazon"
// 结果写进这个文件 —— 由 LaunchServices 启动时拿不到 stdout
let positional = args.filter { !$0.hasPrefix("--") }
let outFile = positional.count > 1 ? positional[1] : ""

// 诊断落到 <outFile>.log —— 由 LaunchServices 启动时 stderr 是拿不到的
let logFile = outFile.isEmpty ? "" : outFile + ".log"
func note(_ m: String) {
    if logFile.isEmpty { FileHandle.standardError.write((m + "\n").data(using: .utf8)!) }
    else if let h = FileHandle(forWritingAtPath: logFile) {
        h.seekToEndOfFile(); h.write((m + "\n").data(using: .utf8)!); h.closeFile()
    } else {
        try? (m + "\n").write(toFile: logFile, atomically: true, encoding: .utf8)
    }
}

func emit(_ v: UInt8) {
    if outFile.isEmpty { print(v) }
    else { try? "\(v)".write(toFile: outFile, atomically: true, encoding: .utf8) }
    exit(0)
}
let BATTERY = CBUUID(string: "180F")
let LEVEL = CBUUID(string: "2A19")

// 状态回调来没来 —— 用来把「蓝牙没授权」和「设备没连」分开
var gotState = false

// ── --listen 用 ──
let t0 = Date()
/// 每个特征收到多少条、多少字节 —— 结束时汇总，一眼看出哪条在推音频
var tally: [String: (n: Int, bytes: Int)] = [:]
func hex(_ d: Data) -> String { d.map { String(format: "%02x", $0) }.joined() }

final class Probe: NSObject, CBCentralManagerDelegate, CBPeripheralDelegate {
    var central: CBCentralManager!
    // ⚠️ 必须持有外设 —— CBCentralManager 不强引用它，不存下来连接过程中会被释放，
    // didConnect 永远不来（老 BleProbe 就是靠这个 targets 才成功的）。
    var targets: [CBPeripheral] = []

    func centralManagerDidUpdateState(_ c: CBCentralManager) {
        gotState = true
        note("state=\(c.state.rawValue)")
        guard c.state == .poweredOn else { return }
        let all = c.retrieveConnectedPeripherals(withServices: [BATTERY])
        note("带电池服务的已连接外设: \(all.map { $0.name ?? "?" })")
        // dump / listen 模式：不限定电池服务，列出所有已连接外设再按名字挑
        if dumpMode || listenMode || readAllSvc != nil {
            let every = c.retrieveConnectedPeripherals(withServices: [
                CBUUID(string: "1800"), CBUUID(string: "1801"), CBUUID(string: "180A"),
                BATTERY, CBUUID(string: "1812"), CBUUID(string: "FE03"),
                CBUUID(string: "AB5E0001-5A21-4F05-BC7D-AF01F617B664"),
            ])
            note("所有已连接外设: \(every.map { $0.name ?? "?" })")
            let picked = every.filter { ($0.name ?? "").contains(want) }
            if picked.isEmpty { note("没有名字含 \(want) 的"); exit(3) }
            targets = picked
            for p in targets { p.delegate = self; c.connect(p, options: nil) }
            return
        }
        let list = all.filter { ($0.name ?? "").contains(want) }
        if list.isEmpty { exit(3) }   // 3 = 蓝牙正常，但没这台设备
        targets = list
        for p in targets { p.delegate = self; c.connect(p, options: nil) }
    }
    func centralManager(_ c: CBCentralManager, didConnect p: CBPeripheral) {
        p.discoverServices((dumpMode || listenMode || readAllSvc != nil) ? nil : [BATTERY])
    }
    func peripheral(_ p: CBPeripheral, didDiscoverServices e: Error?) {
        if let want = readAllSvc {
            for s in p.services ?? [] where s.uuid.uuidString.uppercased().hasPrefix(want) {
                note("=== 服务 \(s.uuid.uuidString) ===")
                p.discoverCharacteristics(nil, for: s)
            }
            return
        }
        if listenMode {
            note("=== \(p.name ?? "?") 服务 \((p.services ?? []).map { $0.uuid.uuidString }) ===")
            for s in p.services ?? [] { p.discoverCharacteristics(nil, for: s) }
            return
        }
        if dumpMode {
            note("=== \(p.name ?? "?") 的服务 ===")
            for s in p.services ?? [] {
                note("SERVICE \(s.uuid.uuidString)")
                p.discoverCharacteristics(nil, for: s)
            }
            return
        }
        for s in p.services ?? [] { p.discoverCharacteristics([LEVEL], for: s) }
    }
    func peripheral(_ p: CBPeripheral, didDiscoverCharacteristicsFor s: CBService, error e: Error?) {
        if readAllSvc != nil {
            for ch in s.characteristics ?? [] where ch.properties.contains(.read) {
                p.readValue(for: ch)   // 只读
            }
            return
        }
        if listenMode {
            for ch in s.characteristics ?? [] {
                if ch.properties.contains(.notify) || ch.properties.contains(.indicate) {
                    note("订阅 \(ch.uuid.uuidString) (svc \(s.uuid.uuidString))")
                    p.setNotifyValue(true, for: ch)   // 只订阅，不写业务数据
                }
            }
            return
        }
        if dumpMode {
            for ch in s.characteristics ?? [] {
                var props: [String] = []
                let pr = ch.properties
                if pr.contains(.read) { props.append("read") }
                if pr.contains(.write) { props.append("write") }
                if pr.contains(.writeWithoutResponse) { props.append("writeNR") }
                if pr.contains(.notify) { props.append("notify") }
                if pr.contains(.indicate) { props.append("indicate") }
                note("  CHAR \(ch.uuid.uuidString)  [\(props.joined(separator: ","))]  (svc \(s.uuid.uuidString))")
            }
            return
        }
        for ch in s.characteristics ?? [] { p.readValue(for: ch) }
    }
    func peripheral(_ p: CBPeripheral, didUpdateValueFor ch: CBCharacteristic, error e: Error?) {
        if readAllSvc != nil {
            if let err = e {
                note("  \(ch.uuid.uuidString)  读失败: \(err.localizedDescription)")
            } else if let d = ch.value {
                let txt = String(data: d, encoding: .utf8).map {
                    $0.allSatisfy { $0.isASCII && !$0.isNewline } ? "  \"\($0)\"" : ""
                } ?? ""
                note("  \(ch.uuid.uuidString)  \(d.count)B  \(hex(d))\(txt)")
            } else {
                note("  \(ch.uuid.uuidString)  空值")
            }
            return
        }
        if listenMode {
            guard let d = ch.value else { return }
            let k = ch.uuid.uuidString
            var t = tally[k] ?? (0, 0)
            t.n += 1; t.bytes += d.count
            tally[k] = t
            // 前 60 条逐条打全（够看清帧头/帧长），之后只计数免得刷屏
            if t.n <= 60 {
                note(String(format: "+%6.3f  %@  %3dB  %@", Date().timeIntervalSince(t0),
                            k, d.count, hex(d)))
            } else if t.n % 50 == 0 {
                note(String(format: "+%6.3f  %@  …已收 %d 条 / %d 字节", Date().timeIntervalSince(t0),
                            k, t.n, t.bytes))
            }
            return
        }
        if let b = ch.value?.first { emit(b) }
        exit(4)   // 4 = 连上了但读到空值
    }
}

note("probe 启动 want=\(want)")
let probe = Probe()
probe.central = CBCentralManager(delegate: probe, queue: nil)
if readAllSvc != nil {
    DispatchQueue.main.asyncAfter(deadline: .now() + 8) { note("读完退出"); exit(0) }
} else if listenMode {
    note("监听 \(Int(listenSecs)) 秒 —— 现在按住遥控器麦克风键说话")
    DispatchQueue.main.asyncAfter(deadline: .now() + listenSecs) {
        note("──────── 汇总 ────────")
        if tally.isEmpty {
            note("没收到任何 notify。")
        } else {
            for (k, v) in tally.sorted(by: { $0.value.bytes > $1.value.bytes }) {
                let bps = Double(v.bytes) / listenSecs
                note(String(format: "  %@  %d 条 / %d 字节 (%.0f B/s)", k, v.n, v.bytes, bps))
            }
        }
        exit(gotState ? 0 : 2)
    }
} else {
    // 别赖着不走：8 秒读不到就退
    // 2 = 状态回调压根没来（多半是蓝牙没授权）；5 = 状态来了但没读完
    DispatchQueue.main.asyncAfter(deadline: .now() + 8) { note("超时退出"); exit(gotState ? 5 : 2) }
}
RunLoop.main.run()
