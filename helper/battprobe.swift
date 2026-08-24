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

let want = CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : "Amazon"
// 结果写进这个文件 —— 由 LaunchServices 启动时拿不到 stdout
let outFile = CommandLine.arguments.count > 2 ? CommandLine.arguments[2] : ""

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
        let list = all.filter { ($0.name ?? "").contains(want) }
        if list.isEmpty { exit(3) }   // 3 = 蓝牙正常，但没这台设备
        targets = list
        for p in targets { p.delegate = self; c.connect(p, options: nil) }
    }
    func centralManager(_ c: CBCentralManager, didConnect p: CBPeripheral) {
        p.discoverServices([BATTERY])
    }
    func peripheral(_ p: CBPeripheral, didDiscoverServices e: Error?) {
        for s in p.services ?? [] { p.discoverCharacteristics([LEVEL], for: s) }
    }
    func peripheral(_ p: CBPeripheral, didDiscoverCharacteristicsFor s: CBService, error e: Error?) {
        for ch in s.characteristics ?? [] { p.readValue(for: ch) }
    }
    func peripheral(_ p: CBPeripheral, didUpdateValueFor ch: CBCharacteristic, error e: Error?) {
        if let b = ch.value?.first { emit(b) }
        exit(4)   // 4 = 连上了但读到空值
    }
}

note("probe 启动 want=\(want)")
let probe = Probe()
probe.central = CBCentralManager(delegate: probe, queue: nil)
// 别赖着不走：8 秒读不到就退
// 2 = 状态回调压根没来（多半是蓝牙没授权）；5 = 状态来了但没读完
DispatchQueue.main.asyncAfter(deadline: .now() + 8) { note("超时退出"); exit(gotState ? 5 : 2) }
RunLoop.main.run()
