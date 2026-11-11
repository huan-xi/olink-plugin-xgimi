import os, re, subprocess, sys

BIN = os.environ.get("XG_BIN", "/Applications/GmApp.app/Wrapper/GmApp.app/GmApp")
start, stop = int(sys.argv[1], 16), int(sys.argv[2], 16)

STUBS = os.environ.get("XG_STUBS", os.path.join(os.path.dirname(os.path.abspath(__file__)), "stubs.txt"))
if not os.path.exists(STUBS):
    out = subprocess.run(["otool", "-Iv", BIN], capture_output=True, text=True).stdout
    with open(STUBS, "w") as f:
        for ln in out.splitlines()[2:]:
            parts = ln.split()
            if len(parts) == 3 and parts[0].startswith("0x"):
                f.write(f"{parts[0]} {parts[2]}\n")
    print(f"[i] 已生成间接符号表: {STUBS}", file=sys.stderr)

stubs = {}
for ln in open(STUBS):
    p = ln.split()
    if len(p) == 2:
        stubs[int(p[0], 16)] = p[1]

out = subprocess.run(["xcrun","llvm-objdump","--disassemble","--no-show-raw-insn",
                      f"--start-address={hex(start)}", f"--stop-address={hex(stop)}", BIN],
                     capture_output=True, text=True).stdout
lines = out.splitlines()

# demangle bl 目标
targets = set()
for ln in lines:
    m = re.search(r"\bbl\s+0x([0-9a-f]+)", ln)
    if m and int(m.group(1),16) in stubs:
        targets.add(int(m.group(1),16))
dem = {}
if targets:
    origs = [stubs[t] for t in sorted(targets)]
    p = subprocess.run(["xcrun","swift-demangle","-compact"], input="\n".join(origs), capture_output=True, text=True)
    for o, d in zip(origs, p.stdout.splitlines()):
        dem[o] = d

regs = {}
prev_mnemonic = None
for ln in lines:
    m = re.match(r"\s*([0-9a-f]+):\s+(\S+)(.*)", ln)
    if not m:
        continue
    addr, mn, rest = int(m.group(1),16), m.group(2), m.group(3)
    note = ""
    # 解码 mov/movk
    mm = re.match(r"\s+x(\d+),\s+#(-?0x[0-9a-f]+)(?:,\s*lsl #(\d+))?", rest)
    if mn in ("mov","movz","movk","movn") and mm:
        r, val, sh = mm.group(1), int(mm.group(2),16), int(mm.group(3) or 0)
        if val < 0: val += 1<<64
        if mn == "movk":
            regs[r] = (regs.get(r,0) & ~(0xFFFF<<sh)) | ((val & 0xFFFF)<<sh)
        elif mn == "movn":
            regs[r] = (~val<<sh) & 0xFFFFFFFFFFFFFFFF
        else:
            regs[r] = (val<<sh) & 0xFFFFFFFFFFFFFFFF
        b = regs[r].to_bytes(8,"little")
        cands = []
        if (b[7] & 0xF0) in (0xE0,0xD0): cands.append(b[:7].rstrip(b"\x00"))
        cands.append(b.rstrip(b"\x00"))
        for c in cands:
            if len(c) >= 3 and all(32 <= x < 127 for x in c):
                note = f"   ; str={c.decode()!r}"
                break
    else:
        if mn.startswith("mov") and re.match(r"\s+x(\d+),\s+x", rest):
            pass
    # 标注 bl
    mb = re.search(r"\bbl\s+0x([0-9a-f]+)", ln)
    if mb:
        t = int(mb.group(1),16)
        if t in stubs:
            note = f"   ; {dem.get(stubs[t], stubs[t])}"
        else:
            note = f"   ; sub_{t:x}"
    print(f"{addr:#x}: {mn:8s}{rest}{note}")
