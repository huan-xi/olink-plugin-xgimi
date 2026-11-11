# xgimi 开发模式（dev_roots）

本机 Host 与本目录同机时，可跳过 `pack.sh` / 安装包，直接联调。

在 Host `data/application.toml`：

```toml
[plugins]
dev_mode = true
allow_unsigned_local = true
dev_roots = { xgimi = "/ABS/PATH/olink-plugins/xgimi" }
dev_assets = { xgimi = "http://127.0.0.1:5174" }
```

```bash
# 后端
cd olink-plugins/xgimi && cargo build

# UI（可选，需 Vite + CORS）
# cd ui && pnpm dev

# 管理页启用 xgimi 后：改代码 → cargo build → 重启插件
```

不要 pack / install。同 domain 正式安装会被 `PLUGIN_DEV_ROOT_ACTIVE` 拒绝。

## UI 注意

**不需要单独编译极米前端。** `ui/entry.js` 是宿主直接加载的 ESM；改完刷新/重启插件即可。

`ui/entry.js` 必须用宿主绝对路径导入共享依赖，例如：

```js
import { createElement, useState } from '/plugin-runtime/react.js'
import { usePluginContext } from '/plugin-runtime/plugin-sdk.js'
```

不要写裸 `from 'react'` / `from '@olink/plugin-sdk'`。

`/plugin-runtime/*.js` 由 **olink-front** 提供（生产 `dist/plugin-runtime/`，开发态 Vite 需有 facade 中间件）。若开发态拿到 HTML 而不是 JS，重启 `olink-front` 的 `pnpm dev`。

`/plugin-assets/{domain}/{version}/...` 映射到源码 `ui/`。

若 Console 仍报裸 `react` 但磁盘 `entry.js` 已改成 `/plugin-runtime/...`：多半是浏览器缓存了旧 entry（Host 曾对 assets 下 `immutable`）。DevTools 勾 Disable cache 后硬刷新，或清该 URL 缓存。

上位规格：`docs/superpowers/specs/2026-11-09-olink-plugin-dev-roots-design.md`。
