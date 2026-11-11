import os, struct, sys
BIN=os.environ.get("XG_BIN", "/Applications/GmApp.app/Wrapper/GmApp.app/GmApp")
BASE=0x100000000
text_start, text_end = 0x100004000, 0x1025afb00
data=open(BIN,"rb").read()
targets={int(a,16) for a in sys.argv[1:]}
hits={t:[] for t in targets}
for a in range(text_start, text_end, 4):
    w = struct.unpack_from("<I", data, a-BASE)[0]
    if (w & 0xFC000000) != 0x94000000:
        continue
    imm = w & 0x3FFFFFF
    if imm & (1<<25): imm -= (1<<26)
    t = a + imm*4
    if t in hits:
        hits[t].append(a)
for t in sorted(targets):
    print(f"target {t:#x}: {len(hits[t])} 处调用")
    for h in hits[t][:20]:
        print(f"   {h:#x}")
