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
// --scan：**只扫描广播，绝不连接**。用来回答「遥控器断开之后，是它自己在不停
// 广播（macOS 见到已配对设备就连回去），还是 macOS 主动去连它」。
// 连接会污染实验（把它拉回在线），所以这个模式下一次 connect 都不发。
let scanMode = args.contains("--scan")
// --disconnect：真正能断开 BLE 链路的那条路（2026-09-19 实测通）。
// 公开的 `cancelPeripheralConnection:` 只取消**自己**建立的连接，对系统持有的
// HID 链路无效；而系统设置里那个「断开连接」按钮用的是私有的
// `CBConnection.disconnectWithCompletion:`（从 Bluetooth.appex 的 selector 里挖出来的）。
// CBCentralManager 上还有个私有变体 `cancelPeripheralConnection:force:`，
// force=YES 才是「连不是我连的也断」。
let forceDisconnect = args.contains("--disconnect")
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
/// --scan 收到多少条广播（目标设备 / 所有设备）
var advCount = 0
var advAll = 0
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
        if forceDisconnect {
            let every = c.retrieveConnectedPeripherals(withServices: [
                CBUUID(string: "1800"), CBUUID(string: "1801"), CBUUID(string: "180A"),
                BATTERY, CBUUID(string: "1812"), CBUUID(string: "FE03"),
            ])
            note("系统已连接的外设: \(every.map { $0.name ?? "?" })")
            guard let p = every.first(where: { ($0.name ?? "").contains(want) }) else {
                note("没找到名字含「\(want)」的已连接外设"); exit(3)
            }
            targets = [p]; p.delegate = self

            // ⚠️ **必须先 connect**：`cancelPeripheralConnection` 的语义是「取消**我自己**
            // 的连接」，没 connect 过就无从取消 —— 少了这一步它静默无效（我先这么试了
            // 半天，以为整条路都是死的）。连上之后 cancel 才会真的把链路拆掉。
            note("先 connect（拿到自己的连接引用）…")
            c.connect(p, options: nil)
            Thread.sleep(forTimeInterval: 2.0)
            note("connect 后 state=\(p.state.rawValue)")

            func stillConnected() -> Bool {
                c.retrieveConnectedPeripherals(withServices: [
                    CBUUID(string: "1800"), CBUUID(string: "1801"), BATTERY, CBUUID(string: "1812"),
                ]).contains { ($0.name ?? "").contains(want) }
            }

            // 先用**公开 API**，够用就不碰私有接口（将来 macOS 改了也不会一下子全废）
            note("① cancelPeripheralConnection:（公开 API）…")
            c.cancelPeripheralConnection(p)
            Thread.sleep(forTimeInterval: 3.0)
            if !stillConnected() {
                note("✅ 公开 API 就断掉了"); exit(0)
            }
            // ⚠️ 上面那次 cancel **已经把我们自己的连接引用取消掉了**，此时直接
            // 调 force 变体必然无效（没有引用可 force）—— 踩过：加了「优先公开 API」
            // 这步之后，原本验证通过的配方就一直失败（退出码 5）。所以必须**重新 connect**。
            note("公开 API 没断掉，重新 connect 后再试私有变体…")
            c.connect(p, options: nil)
            Thread.sleep(forTimeInterval: 2.0)
            note("重连后 state=\(p.state.rawValue)")
            // 不够就降级到私有变体：force=YES 才连「不是我建立的连接」也断
            let sel = NSSelectorFromString("cancelPeripheralConnection:force:")
            guard c.responds(to: sel) else {
                note("❌ 公开 API 没断掉，而这版 CoreBluetooth 没有 force 变体"); exit(4)
            }
            note("② cancelPeripheralConnection:force:（私有变体，force=YES）…")
            typealias Fn = @convention(c) (AnyObject, Selector, AnyObject, Bool) -> Void
            let m = class_getInstanceMethod(type(of: c), sel)!
            let f = unsafeBitCast(method_getImplementation(m), to: Fn.self)
            f(c, sel, p, true)
            DispatchQueue.main.asyncAfter(deadline: .now() + 4) {
                let still = stillConnected()
                note(still ? "❌ 4 秒后它还在已连接列表里" : "✅ 已断开")
                exit(still ? 5 : 0)
            }
            return
        }
        if scanMode {
            let conn = c.retrieveConnectedPeripherals(withServices: [
                CBUUID(string: "1800"), CBUUID(string: "1801"), BATTERY, CBUUID(string: "1812"),
            ])
            note("开扫前，系统认为已连接的: \(conn.map { $0.name ?? "?" })")
            note("开始扫描（只看广播，不连接）…")
            c.scanForPeripherals(withServices: nil,
                                 options: [CBCentralManagerScanOptionAllowDuplicatesKey: true])
            return
        }
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
    func centralManager(_ c: CBCentralManager, didDiscover p: CBPeripheral,
                        advertisementData ad: [String: Any], rssi: NSNumber) {
        guard scanMode else { return }
        // ⚠️ 探针自证用：不过滤地数一遍。「目标 0 条」必须配合「总数 > 0」才
        // 说明是它没广播，而不是扫描压根没工作。
        advAll += 1
        let n = p.name ?? (ad[CBAdvertisementDataLocalNameKey] as? String) ?? ""
        guard n.contains(want) else { return }
        advCount += 1
        // 前 20 条逐条打（够看清广播节奏），之后每 20 条打一次
        if advCount <= 20 || advCount % 20 == 0 {
            let conn = ad[CBAdvertisementDataIsConnectable] as? Bool
            note(String(format: "+%6.1fs 广播 #%d  「%@」 RSSI %@  connectable=%@",
                        Date().timeIntervalSince(t0), advCount, n, rssi,
                        conn.map { $0 ? "是" : "否" } ?? "?"))
        }
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
if scanMode {
    note("扫描 \(Int(listenSecs)) 秒 —— 期间请不要碰遥控器")
    DispatchQueue.main.asyncAfter(deadline: .now() + listenSecs) {
        note("──────── 汇总 ────────")
        note("扫描自证：这段时间总共收到 \(advAll) 条广播（所有设备）")
        if advCount == 0 {
            note(advAll > 0
                 ? "目标一条广播都没有 → 它是真睡着了，没在招人连它"
                 : "⚠️ 总广播也是 0 —— 扫描本身没工作，这轮数据作废")
        } else {
            note(String(format: "共收到 %d 条广播（%.1f 条/秒）→ 它在持续招人连它",
                        advCount, Double(advCount) / listenSecs))
        }
        exit(gotState ? 0 : 2)
    }
} else if readAllSvc != nil {
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
} else if forceDisconnect {
    // ⚠️ 断链流程比读电量慢得多：connect(2s) + 公开 cancel(3s) + 重连(2s) + 检查(4s)
    // ≈ 11 秒。用默认那个 8 秒兜底会**抢在流程完成前 exit(5)** —— 表现成「报失败
    // 但设备其实断开了」，踩过。这里给足余量（父进程 btlink.rs 的超时是 20 秒）。
    DispatchQueue.main.asyncAfter(deadline: .now() + 18) { note("断链超时退出"); exit(6) }
} else {
    // 别赖着不走：8 秒读不到就退
    // 2 = 状态回调压根没来（多半是蓝牙没授权）；5 = 状态来了但没读完
    DispatchQueue.main.asyncAfter(deadline: .now() + 8) { note("超时退出"); exit(gotState ? 5 : 2) }
}
RunLoop.main.run()
