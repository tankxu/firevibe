#!/usr/bin/env python3
"""把 Flipper-IRDB 打包成 FireVibe 内置的红外码库。

用法：
    python3 tools/build-irdb.py [--repo <本地克隆路径>]
产物：
    core/assets/irdb.jsonl.gz   （提交进仓库，约 2 MB）

数据源：https://github.com/Lucaslhm/Flipper-IRDB  —— CC0-1.0（公有领域），
所以能直接内置，不用担心许可。Flipper Zero 社区攒的，用户抓到码就提 PR。

## 只收两类码，其余丢掉

  · type: raw          —— 本来就是时长序列，直接用，零风险
  · protocol: NEC/NECext —— 合成时序。时序常数拿库里 1175 条真实 raw 反解验证过，
    0 条结构不符（见 tools/ 里的验证脚本历史）

Samsung32 / RC5 / SIRC / Kaseikyo 这些**故意不做** —— 时序只能凭记忆写，没有验证
手段。等 StackChan 那边能收码了，发一条合成的让它解回来比对，验完再加。
宁可少一半，不要塞一堆发不出去的码。

## 还丢掉

  · `_Converted_/`（6944 个文件、55 MB）—— 机器从别的库转换来的，质量参差，
    而且是体积大头。人工整理的那 1957 个文件才 7.2 MB。
  · 单个时长 > 32767 µs 的码 —— 遥控器那边是有符号 int16，装不下（见 core/src/ir.rs）
"""
import argparse
import gzip
import json
import os
import re
import subprocess
import sys

REPO_URL = "https://github.com/Lucaslhm/Flipper-IRDB.git"
OUT = "core/assets/irdb.jsonl.gz"
MAX_US = 327670  # 设备侧是 int16 「格」，一格 10 µs → 327 ms（和 core/src/ir.rs 对齐）

# NEC 家族标准时序（µs）。这几个数字是拿库里 1175 条真实 raw 反解验证过的。
NEC_HDR_MARK, NEC_HDR_SPACE = 9000, 4500
NEC_BIT, NEC_ONE, NEC_ZERO = 560, 1690, 560


def nec_frame(data_bytes):
    """一帧 NEC：引导 + 每位（mark + 长/短 space）+ 收尾 mark。共 2+64+1 = 67 项。"""
    out = [NEC_HDR_MARK, NEC_HDR_SPACE]
    for b in data_bytes:
        for i in range(8):  # 每字节 LSB 先发
            out += [NEC_BIT, NEC_ONE if (b >> i) & 1 else NEC_ZERO]
    out.append(NEC_BIT)
    return out


def synth(proto, addr, cmd):
    """parsed → 时长序列。不支持的协议返回 None。"""
    if proto == "NEC":  # 地址 + 取反、命令 + 取反
        a, c = addr[0], cmd[0]
        return nec_frame([a, a ^ 0xFF, c, c ^ 0xFF])
    if proto == "NECext":  # 16 位地址原样，命令 + 取反
        if len(addr) < 2:
            return None
        return nec_frame([addr[0], addr[1], cmd[0], cmd[0] ^ 0xFF])
    return None


def parse_ir(path):
    """读一个 .ir，产出 [(按键名, 频率, 时长序列, 来源标记)]"""
    txt = open(path, encoding="utf-8", errors="ignore").read()
    out = []
    for blk in re.split(r"\n(?=name:)", txt):
        name = re.search(r"^name:\s*(.+)$", blk, re.M)
        kind = re.search(r"^type:\s*(\w+)", blk, re.M)
        if not (name and kind):
            continue
        nm = name.group(1).strip()
        if kind.group(1) == "raw":
            d = re.search(r"^data:\s*([\d ]+)", blk, re.M)
            f = re.search(r"^frequency:\s*(\d+)", blk, re.M)
            if not d:
                continue
            out.append((nm, int(f.group(1)) if f else 38000,
                        [int(x) for x in d.group(1).split()], "raw"))
        else:
            p = re.search(r"^protocol:\s*(\S+)", blk, re.M)
            a = re.search(r"^address:\s*([\dA-Fa-f ]+)", blk, re.M)
            c = re.search(r"^command:\s*([\dA-Fa-f ]+)", blk, re.M)
            if not (p and a and c):
                continue
            seq = synth(p.group(1),
                        [int(x, 16) for x in a.group(1).split()],
                        [int(x, 16) for x in c.group(1).split()])
            if seq:
                out.append((nm, 38000, seq, p.group(1)))
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--repo", default="")
    args = ap.parse_args()

    repo = args.repo
    if not repo:
        repo = "/tmp/flipper-irdb"
        if not os.path.isdir(repo):
            print(f"克隆 {REPO_URL} …")
            subprocess.run(["git", "clone", "--depth", "1", "-q", REPO_URL, repo], check=True)

    devices, codes, dropped_long, skipped, trimmed = [], 0, 0, 0, 0
    for dp, _, fns in os.walk(repo):
        if "_Converted_" in dp or f"{os.sep}.git" in dp:
            continue
        rel = os.path.relpath(dp, repo).split(os.sep)
        if len(rel) < 2 or rel[0] in (".", ""):
            continue  # 目录结构是 <分类>/<品牌>/<型号>.ir
        cat, brand = rel[0], rel[1]
        for fn in sorted(fns):
            if not fn.endswith(".ir"):
                continue
            buttons = []
            for nm, freq, seq, src in parse_ir(os.path.join(dp, fn)):
                # 末尾那一项常常是「帧后间隔」，对单次发射没有信息量 —— 砍掉就能收下，
                # 不必因为它丢掉整条码。中间的长间隔不行：两段的语义是 Pronto 的
                # intro/repeat，不是「两帧」，拆开会丢掉间隔时长。
                if len(seq) > 1 and seq[-1] > MAX_US and max(seq[:-1]) <= MAX_US:
                    seq = seq[:-1]
                    trimmed += 1
                if max(seq) > MAX_US:
                    dropped_long += 1
                    continue
                buttons.append({"n": nm, "f": freq, "s": seq, "p": src})
            if not buttons:
                skipped += 1
                continue
            devices.append({
                "c": cat.replace("_", " "),
                "b": brand.replace("_", " "),
                "m": os.path.splitext(fn)[0].replace("_", " "),
                "k": buttons,
            })
            codes += len(buttons)

    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    with gzip.open(OUT, "wt", encoding="utf-8", compresslevel=9) as f:
        for d in devices:
            f.write(json.dumps(d, ensure_ascii=False, separators=(",", ":")) + "\n")

    size = os.path.getsize(OUT)
    print(f"✓ {OUT}")
    print(f"  设备 {len(devices)} 个 · 码 {codes} 条 · {size/1048576:.2f} MB")
    print(f"  丢弃：中间有超 {MAX_US}µs 长间隔的 {dropped_long} 条 · "
          f"一条可用码都没有的文件 {skipped} 个")
    print(f"  砍掉末尾帧后间隔救回：{trimmed} 条")


if __name__ == "__main__":
    sys.exit(main())
