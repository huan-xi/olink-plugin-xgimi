#!/usr/bin/env python3
"""极米 App 登录协议测试脚本（协议见 docs/reference/2026-11-09-xgimi-account-login-protocol.md）

用法:
    python3 xgimi_login.py send-code --phone 13800138000
    python3 xgimi_login.py login --phone 13800138000 --code 123456

仅使用标准库。所有请求/响应会打印为 JSON，便于记录调试。
"""
import argparse
import hashlib
import json
import os
import sys
import time
import urllib.error
import urllib.request
import uuid

SIGN_SALT = "9y$B&E)H@McQeThW"

HOST_SUPERAPP = "https://superapp.xgimi.com"
HOST_MOBILE_API = "https://mobile-api.xgimi.com"

SESSION_PATH = os.environ.get("XG_SESSION", "/tmp/xgimi_session.json")

# 固定请求头（值可通过环境变量覆盖，便于比对抓包）
BASE_HEADERS = {
    "platform": "ios",
    "appVersion": os.environ.get("XG_APP_VERSION", "4.10.36"),
    "channel": os.environ.get("XG_CHANNEL", "appstore"),
    "superApp": os.environ.get("XG_SUPERAPP", "1"),
    "deviceId": os.environ.get("XG_DEVICE_ID", str(uuid.uuid4()).upper()),
    "deviceModel": os.environ.get("XG_DEVICE_MODEL", "iPhone"),
    "osName": os.environ.get("XG_OS_NAME", "iOS 26.0"),
}


def load_session() -> dict:
    try:
        with open(SESSION_PATH, encoding="utf-8") as f:
            return json.load(f)
    except Exception:
        return {}


def save_session(data: dict) -> None:
    try:
        with open(SESSION_PATH, "w", encoding="utf-8") as f:
            json.dump(data, f, ensure_ascii=False, indent=2)
        os.chmod(SESSION_PATH, 0o600)
    except Exception as e:
        print(f"!! 会话保存失败: {e}")


def with_auth(headers: dict) -> dict:
    """注入登录态请求头；缺少会话时报错退出。"""
    sess = load_session()
    if not sess.get("token"):
        sys.exit(f"!! 没有会话，请先 login（会话文件：{SESSION_PATH}）")
    headers.update({
        "token": sess["token"],
        "accessToken": sess.get("accessToken", ""),
        "openId": sess.get("openId", ""),
    })
    return headers


def sign(parameters: dict, salt: str = SIGN_SALT) -> tuple[dict, dict]:
    """返回 (headers, 签名用数据)。"""
    ts = str(int(time.time() * 1000))
    p = dict(parameters)
    p["timestamp"] = ts
    raw = "".join(f"{k}={p[k]}&" for k in sorted(p))
    sig = hashlib.md5((raw + salt).encode("utf-8")).hexdigest()
    return {"timestamp": ts, "sign": sig}, {"raw": raw, "digest_input": raw + salt}


def call(url: str, params: dict, need_sign: bool, verbose: bool = True,
         need_token: bool = False, method: str = "POST", query: dict | None = None) -> dict:
    headers = dict(BASE_HEADERS)
    headers["Accept-Encoding"] = "identity"
    if method != "GET":
        headers["Content-Type"] = "application/json"
    if need_token:
        with_auth(headers)
    debug = {}
    if need_sign:
        h, debug = sign(params)
        headers.update(h)
    if method == "GET":
        q = "&".join(f"{k}={v}" for k, v in (query or {}).items())
        req = urllib.request.Request(url + ("?" + q if q else ""), headers=headers, method="GET")
    else:
        body = json.dumps(params, ensure_ascii=False).encode("utf-8")
        req = urllib.request.Request(url, data=body, headers=headers, method=method)
    if verbose:
        print(f"→ POST {url}")
        print(f"  headers: {json.dumps(headers, ensure_ascii=False)}")
        print(f"  body   : {json.dumps(params, ensure_ascii=False)}")
        if debug:
            print(f"  sign   : raw={debug['raw']!r}")
    try:
        with urllib.request.urlopen(req, timeout=15) as resp:
            text = resp.read().decode("utf-8", "replace")
            status = resp.status
    except urllib.error.HTTPError as e:
        text = e.read().decode("utf-8", "replace")
        status = e.code
    except Exception as e:  # 网络层错误
        print(f"!! 请求失败: {e}")
        return {"ok": False, "error": str(e)}
    if verbose:
        print(f"← HTTP {status}")
        print(f"  {text[:2000]}")
    try:
        return {"ok": 200 <= status < 300, "status": status, "json": json.loads(text)}
    except json.JSONDecodeError:
        return {"ok": 200 <= status < 300, "status": status, "text": text}


def cmd_send_code(args):
    # type 场景: 2 = 手机号登录, 4 = 完善资料/绑定第三方
    params = {"mobile": args.phone, "type": args.type}
    return call(f"{HOST_MOBILE_API}/app/v4/account/sendSmsCode", params, need_sign=True)


def cmd_login(args):
    params = {"mobile": args.phone, "code": args.code}
    r = call(f"{HOST_SUPERAPP}/api/v1/user/access", params, need_sign=False)
    if r.get("ok"):
        data = (r.get("json") or {}).get("data") or {}
        user = data.get("user") or {}
        access = user.get("access") or {}
        save_session({
            "token": data.get("token", ""),
            "accessToken": user.get("accessToken") or access.get("accessToken", ""),
            "refreshToken": access.get("refreshToken", ""),
            "openId": user.get("openId") or access.get("openId", ""),
            "uid": user.get("uid", ""),
            "deviceId": BASE_HEADERS["deviceId"],
            "savedAt": int(time.time()),
        })
        print(f"会话已保存到 {SESSION_PATH}")
    return r


def cmd_refresh(args):
    """续期：只需要 refreshToken，不需要 token 头；成功后更新会话文件。"""
    sess = load_session()
    rt = args.refresh_token or sess.get("refreshToken")
    if not rt:
        sys.exit("!! 缺少 refreshToken（可传 --refresh-token 或先 login）")
    r = call(f"{HOST_MOBILE_API}/app/v4/user/refreshToken",
             {"refreshToken": rt}, need_sign=True, need_token=False)
    if r.get("ok"):
        data = (r.get("json") or {}).get("data") or {}
        sess.update({
            "accessToken": data.get("accessToken", sess.get("accessToken", "")),
            "refreshToken": data.get("refreshToken", sess.get("refreshToken", "")),
            "savedAt": int(time.time()),
        })
        save_session(sess)
        print(f"续期成功，会话已更新：{SESSION_PATH}")
    return r


def cmd_userinfo(args):
    sess = load_session()
    open_id = args.open_id or sess.get("openId")
    if not open_id:
        sys.exit("!! 缺少 openId（可传 --open-id 或先 login）")
    return call(f"{HOST_SUPERAPP}/api/v2/userInfo/{open_id}", {}, need_sign=True,
                need_token=True, method="GET")


def cmd_devices(args):
    params = {"pageSize": args.page_size, "currentPage": args.page}
    return call(f"{HOST_MOBILE_API}/app/v4/device/new/list", params,
                need_sign=True, need_token=True)


def cmd_device_info(args):
    params = {"sn": args.sn, "dmCode": args.dm_code}
    return call(f"{HOST_MOBILE_API}/app/v4/myDevice/info", params,
                need_sign=True, need_token=True)


def cmd_device_config(args):
    params = {"sn": args.sn, "quickApps": []}
    return call(f"{HOST_MOBILE_API}/app/v4/myDevice/config", params,
                need_sign=True, need_token=True)


def main():
    ap = argparse.ArgumentParser(description="极米 App 登录协议测试")
    ap.add_argument("--device-id", default=None, help="固定 deviceId（发码与登录需一致时使用）")
    sub = ap.add_subparsers(dest="cmd", required=True)

    p1 = sub.add_parser("send-code", help="发送短信验证码")
    p1.add_argument("--phone", required=True)
    p1.add_argument("--type", type=int, default=2, help="短信场景，登录=2（默认），绑定=4")
    p1.set_defaults(func=cmd_send_code)

    p2 = sub.add_parser("login", help="手机号 + 验证码登录")
    p2.add_argument("--phone", required=True)
    p2.add_argument("--code", required=True)
    p2.set_defaults(func=cmd_login)

    p7 = sub.add_parser("refresh", help="token 续期（POST /app/v4/user/refreshToken）")
    p7.add_argument("--refresh-token", default=None)
    p7.set_defaults(func=cmd_refresh)

    p3 = sub.add_parser("userinfo", help="获取用户信息（GET /api/v2/userInfo/{openId}）")
    p3.add_argument("--open-id", default=None)
    p3.set_defaults(func=cmd_userinfo)

    p4 = sub.add_parser("devices", help="设备列表（POST /app/v4/device/new/list）")
    p4.add_argument("--page", type=int, default=1)
    p4.add_argument("--page-size", type=int, default=20)
    p4.set_defaults(func=cmd_devices)

    p5 = sub.add_parser("device-info", help="设备详情（POST /app/v4/myDevice/info）")
    p5.add_argument("--sn", required=True)
    p5.add_argument("--dm-code", required=True, help="物料编码，取自列表的 itemCode")
    p5.set_defaults(func=cmd_device_info)

    p6 = sub.add_parser("device-config", help="设备配置下发（POST /app/v4/myDevice/config）")
    p6.add_argument("--sn", required=True)
    p6.set_defaults(func=cmd_device_config)

    args = ap.parse_args()
    if args.device_id:
        BASE_HEADERS["deviceId"] = args.device_id
    result = args.func(args)
    print(json.dumps(result, ensure_ascii=False, indent=2)[:3000])


if __name__ == "__main__":
    main()
