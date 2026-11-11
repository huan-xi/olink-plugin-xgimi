import os, struct, sys

BIN = os.environ.get("XG_BIN", "/Applications/GmApp.app/Wrapper/GmApp.app/GmApp")
BASE = 0x100000000
TEXT_START, TEXT_END = 0x100004000, 0x1025afb00
CONST_START, CONST_END = 0x1026c5660, 0x1028e8520   # __const

data = open(BIN, "rb").read()

def off(a): return a - BASE

def read_i32(a): return struct.unpack_from("<i", data, off(a))[0]

# --- 1) 在 __const / 其他数据区里找 32 位相对指针指向目标字符串 ---
def const_refs(target):
    res = []
    for a in range(CONST_START, CONST_END - 4):
        try:
            v = read_i32(a)
        except Exception:
            break
        if a + v == target:
            res.append(a)
    return res

# --- 2) 扫描 __text 中的 ADRP+ADD 序列，找指向地址 X 的代码 ---
def adrp_add_refs(targets, window=8):
    tpages = {}
    for t in targets:
        tpages.setdefault(t & ~0xFFF, set()).add(t & 0xFFF)
    hits = {}
    code = data[off(TEXT_START):off(TEXT_END)]
    n = len(code)
    # 预解码所有 ADRP 位置
    for i in range(0, n - 3, 4):
        ins = struct.unpack_from("<I", code, i)[0]
        if (ins & 0x9F000000) != 0x90000000:
            continue
        rd = ins & 0x1F
        immlo = (ins >> 29) & 0x3
        immhi = (ins >> 5) & 0x7FFFF
        imm = (immhi << 2) | immlo
        if imm & (1 << 20): imm -= 1 << 21
        pc = TEXT_START + i
        page = (pc & ~0xFFF) + (imm << 12)
        if page not in tpages:
            continue
        # 往后找 ADD/ORR/LDR 引用同页目标
        for j in range(1, window + 1):
            k = i + 4 * j
            if k >= n: break
            ins2 = struct.unpack_from("<I", code, k)[0]
            # ADD (imm, 64-bit): 0x91000000 mask 0x7F800000
            if (ins2 & 0x7F800000) == 0x11000000:
                rn = (ins2 >> 5) & 0x1F
                rdd = ins2 & 0x1F
                imm12 = (ins2 >> 10) & 0xFFF
                sh = (ins2 >> 22) & 1
                if rn == rd:
                    addr = page + (imm12 << (12 if sh else 0))
                    if addr in tpages.get(page, ()) or (addr & ~0xFFF) == page:
                        if addr in targets:
                            hits.setdefault(addr, []).append(TEXT_START + k)
            # LDR (unsigned offset) 64-bit: 0xF9400000 mask 0xFFC00000
            if (ins2 & 0xFFC00000) == 0xF9400000:
                rn = (ins2 >> 5) & 0x1F
                imm12 = (ins2 >> 10) & 0xFFF
                if rn == rd:
                    addr = page + imm12 * 8
                    if addr in targets:
                        hits.setdefault(addr, []).append(TEXT_START + k)
    return hits

targets = {}
for s in sys.argv[1:]:
    b = s.encode()
    idx = data.find(b + b"\0")
    if idx == -1:
        print(f"!! 未找到字符串: {s}")
        continue
    targets[BASE + idx] = s

all_targets = set(targets)
# 加入 __const 中的静态字符串字面量结构体地址
for t in list(targets):
    for c in const_refs(t):
        all_targets.add(c)

hits = adrp_add_refs(all_targets)
for addr in sorted(hits):
    label = targets.get(addr)
    if label:
        print(f"[str] {label!r} @ {addr:#x}")
    else:
        print(f"[const] @ {addr:#x}")
    for h in hits[addr][:10]:
        print(f"    代码引用 @ {h:#x}")
