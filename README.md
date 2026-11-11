# olink-plugin-xgimi

极米设备插件（神灯 / 投影仪等；当前先落地神灯控制与云账号接入）。

- 产品设计：[`docs/superpowers/specs/2026-11-09-olink-plugin-xgimi-design.md`](../../docs/superpowers/specs/2026-11-09-olink-plugin-xgimi-design.md)
- OOP 骨架：[`docs/superpowers/specs/2026-11-09-olink-plugin-xgimi-oop-refactor-design.md`](../../docs/superpowers/specs/2026-11-09-olink-plugin-xgimi-oop-refactor-design.md)
- 账号/设备 UI：经 Host proxy 打插件私有 API（无 Host 标准 login）— [`docs/superpowers/specs/2026-11-09-olink-plugin-ui-proxy-onboard-design.md`](../../docs/superpowers/specs/2026-11-09-olink-plugin-ui-proxy-onboard-design.md)
- 云登录与勾选接入：[`docs/superpowers/specs/2026-11-09-olink-plugin-xgimi-cloud-onboard-design.md`](../../docs/superpowers/specs/2026-11-09-olink-plugin-xgimi-cloud-onboard-design.md)
- 云协议（实测）：[`docs/reference/2026-11-09-xgimi-account-login-protocol.md`](../../docs/reference/2026-11-09-xgimi-account-login-protocol.md)
- 局域网协议（实测，含实现与验证）：[`docs/reference/2026-11-09-xgimi-cp01-lan-protocol.md`](../../docs/reference/2026-11-09-xgimi-cp01-lan-protocol.md)
- 局域网发现与通道化接入：[`docs/superpowers/specs/2026-11-10-xgimi-lan-discover-design.md`](../../docs/superpowers/specs/2026-11-10-xgimi-lan-discover-design.md)
- 局域网发现弹窗（任务池）：[`docs/superpowers/specs/2026-11-10-xgimi-lan-discover-dialog-design.md`](../../docs/superpowers/specs/2026-11-10-xgimi-lan-discover-dialog-design.md)

## 局域网控制

**不需要登录**：同网段直接 TCP `13145` + `E0 4A` 帧 + JSON 信封即可读写灯状态。
账号只用于云端设备列表与外网遥控。

- 实现：`src/lan.rs` 的 `XgimiLan`（帧编解码 / 连接复用 / 50040·50041 / 模式处理）
- 设备：`devices.json` 里 `host` 填设备 IP（如 `172.21.10.36`）
- 探针：`scripts/xgimi_lan_probe.py`（发现 / 读 / 写 / 能力树）
- 真机测试：
  ```bash
  XGIMI_HOST=<ip> cargo test --test lan_protocol -- --ignored
  XGIMI_HOST=<ip> XGIMI_LIVE_WRITE=1 cargo test --test lan_protocol -- --ignored
  ```

## 局域网发现（Settings）

页头 **局域网发现** → Dialog 搜索约 10s（挂 Host 异步任务池）→ 多选 → 添加。无页内局域网卡片。与云导入并存：同一物理机可各有 `_light_lan` / `_light_cloud`。

私有 API（经 Host proxy）：

| 方法 | 路径 | 说明 |
|---|---|---|
| POST | `/devices/lan/search` | 开搜，返回 `task_id`；后台循环 UDP discover |
| GET | `/devices/lan/search/{task_id}` | 轮询 phase / devices |
| POST | `/devices/lan/search/{task_id}/cancel` | 取消 |
| GET | `/devices/lan/discover` | 一次性发现（调试保留） |
| POST | `/devices/lan/import` | 勾选接入 LAN 灯（写 `devices.json` + `register_light`） |

设备 id 公式：`{pid}_{type}_{channel}`（例 `SWX5PZA97YWC_light_lan`）；系统 did = `xgimi.{id}`。`pid` 经 sanitize（非 `[A-Za-z0-9_-]` → `_`，整 key ≤ 64）。详见规格。

## 云账号接入（Settings）

1. 设置页输入手机号 → 发送验证码 → 登录（会话落盘 `account.json`）。
2. 「发现云设备」→ 勾选 → 「导入勾选设备」（写 `devices.json` + Host `device.register`）。
3. JWT 约 7 天过期后需重新短信登录；可用 `POST /account/refresh` 续 AT/RT。

私有 API：`/account/status|sms|login|logout|refresh`、`/devices/discover`、`/devices/import`（云 id 为 `{pid}_light_cloud`）。

本地协议调试脚本：`scripts/xgimi_login.py`。

## 仿写新插件

1. 复制本目录，改 `plugin.json` 的 `domain` / `name` / `version`。
2. 实现自己的 `LanBackend`（参考 `XgimiLan`；`StubLan` 仅用于单测）。
3. 按需增删 `src/attr.rs` 里的 `LightAttr` 实现，并挂到 registry。
4. `DeviceService` / `PluginEnv` / `main` 通常只改默认 domain 与 `HostGateway::register_*` 的 schema。
5. `cargo test` 通过后，用 `scripts/pack.sh` 打包。

## 本机调试（免安装）

见 [`DEV.md`](./DEV.md)：配置 Host `dev_roots` / `dev_assets` 后，`cargo build` + 重启插件即可，无需 pack/install。

## 打包

```bash
./scripts/pack.sh
```
