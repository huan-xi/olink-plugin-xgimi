#!/usr/bin/env python3
"""极米神灯（CP01）局域网协议探针。

用法：
  python3 -I scripts/xgimi_lan_probe.py discover
  python3 -I scripts/xgimi_lan_probe.py state   <ip>
  python3 -I scripts/xgimi_lan_probe.py raw     <ip> '<json>'
  python3 -I scripts/xgimi_lan_probe.py set     <ip> <type> <value>
  python3 -I scripts/xgimi_lan_probe.py features <ip>

协议见 docs/reference/2026-11-09-xgimi-cp01-lan-protocol.md
"""
import json
import socket
import struct
import sys
import threading
import time

UDP_DISCOVER_PORT = 16750   # 广播请求
UDP_REPLY_PORT = 16751      # 设备回包落在本机这个端口
TCP_DEFAULT_PORT = 13145

BASE = {
    "msgid": "1234567890",
    "appid": "com.xgimi.XAssistant",
    "version": "4.10.36",
    "packageName": "com.xgimi.XAssistant",
}
TYPES = {"0": "lightSwitch", "1": "lightColorTemp", "2": "lightBrightness", "3": "ceilingLampMode"}


def frame(obj):
    body = json.dumps(obj, separators=(",", ":"), ensure_ascii=False).encode()
    return b"\xE0\x4A" + struct.pack(">I", len(body)) + body


def read_frames(sock, timeout):
    sock.settimeout(timeout)
    buf, out, end = b"", [], time.time() + timeout
    while time.time() < end:
        try:
            d = sock.recv(65536)
        except socket.timeout:
            break
        if not d:
            break
        buf += d
        while True:
            i = buf.find(b"\xE0\x4A")
            if i < 0 or len(buf) < i + 6:
                break
            n = struct.unpack(">I", buf[i + 2:i + 6])[0]
            if len(buf) < i + 6 + n:
                break
            out.append(json.loads(buf[i + 6:i + 6 + n]))
            buf = buf[i + 6 + n:]
    return out


def discover(secs=5):
    found = []

    def listen():
        s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        try:
            s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEPORT, 1)
        except OSError:
            pass
        s.bind(("0.0.0.0", UDP_REPLY_PORT))
        s.settimeout(0.5)
        end = time.time() + secs
        while time.time() < end:
            try:
                d, a = s.recvfrom(65536)
            except socket.timeout:
                continue
            try:
                j = json.loads(d)
            except Exception:
                continue
            if j.get("action") == 9999:
                found.append((a[0], j))
        s.close()

    th = threading.Thread(target=listen)
    th.start()
    time.sleep(0.2)
    s = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    s.setsockopt(socket.SOL_SOCKET, socket.SO_BROADCAST, 1)
    req = json.dumps({"action": 9998, **BASE}, separators=(",", ":")).encode()
    for _ in range(3):
        s.sendto(req, ("255.255.255.255", UDP_DISCOVER_PORT))
        time.sleep(0.4)
    s.close()
    th.join()
    return found


class Device:
    def __init__(self, host, port=TCP_DEFAULT_PORT):
        self.s = socket.create_connection((host, port), timeout=5)

    def send(self, obj):
        self.s.sendall(frame(obj))

    def call(self, obj, wait=2.5):
        self.send(obj)
        return read_frames(self.s, wait)

    def ping(self):
        return self.call({"action": 10002, **BASE}, 1.5)

    def state(self):
        out = {}
        for t in TYPES:
            for j in self.call({"action": 50040, **BASE, "commonCmd": {"type": t}}):
                if j.get("action") == 50040:
                    fdi = j.get("featureDetailInfo", {})
                    out[fdi.get("key", t)] = fdi.get("value", fdi.get("curChildItem"))
        return out

    def set(self, typ, value):
        return self.call({"action": 50041, **BASE, "commonCmd": {"type": typ, "value": str(value)}})

    def features(self):
        for j in self.ping():
            if j.get("action") == 50022:
                return j["deviceFunctionsInfo"]
        return None

    def close(self):
        self.s.close()


def main():
    if len(sys.argv) < 2:
        print(__doc__)
        return
    cmd = sys.argv[1]
    if cmd == "discover":
        for ip, j in discover():
            di, ag = j["deviceInfo"], j.get("agreement", {})
            print(f"{ip}  {di.get('device')}  {di.get('productName')}  pid={di.get('pid')}  "
                  f"isTcp={ag.get('isTcp')} port={ag.get('port')}")
        return
    if cmd not in ("state", "raw", "set", "features", "ping"):
        print(__doc__)
        return
    dev = Device(sys.argv[2])
    if cmd == "state":
        print(json.dumps(dev.state(), ensure_ascii=False, indent=2))
    elif cmd == "raw":
        print(json.dumps(dev.call(json.loads(sys.argv[3])), ensure_ascii=False, indent=2))
    elif cmd == "set":
        print(json.dumps(dev.set(sys.argv[3], sys.argv[4]), ensure_ascii=False, indent=2))
    elif cmd == "features":
        print(json.dumps(dev.features(), ensure_ascii=False, indent=2))
    elif cmd == "ping":
        print(json.dumps(dev.ping(), ensure_ascii=False, indent=2))
    dev.close()


if __name__ == "__main__":
    main()
