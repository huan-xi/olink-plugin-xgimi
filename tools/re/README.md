# 逆向辅助脚本

> **免责声明**：本目录及相关分析文档全部由 AI 分析产出，仅供学习参考，不构成任何官方说明或使用建议。

分析 `/Applications/GmApp.app`（极米 4.10.36，iOS 包）时用的小工具，配套文档：

- 协议：`docs/reference/2026-11-09-xgimi-account-login-protocol.md`
- 调试记录：`docs/debug/2026-11-09-gmapp-login.md`

全部基于 Xcode 命令行工具（`llvm-objdump` / `otool`）+ Ghidra，无第三方依赖。

## 脚本

| 文件 | 用途 |
|---|---|
| `xref.py` | 扫描 `__text` 里的 `ADRP+ADD` / `ADRP+LDR`，找出引用某个字符串常量的代码地址 |
| `blxref.py` | 扫描 `BL` 指令，找出调用某个函数的代码地址 |
| `listing.py` | 指定地址区间输出反汇编，并把 `bl` 目标解析成符号、把内联的 Swift 小字符串解码成文本 |
| `DumpAt.java` | Ghidra 脚本：反编译包含指定地址的函数，打印 C 伪代码 |
| `stubs.txt` | `listing.py` 自动生成的间接符号表（不入库） |

公共环境变量：

```bash
export XG_BIN=/Applications/GmApp.app/Wrapper/GmApp.app/GmApp   # 目标二进制
export XG_STUBS=tools/re/stubs.txt                              # 间接符号表缓存
```

## 用法

```bash
# 1) 找出引用某个接口路径/常量的代码位置
python3 tools/re/xref.py '/api/v1/user/access' '/app/v4/account/sendSmsCode'

# 2) 找出某个函数的调用方
python3 tools/re/blxref.py 0x1005d284c

# 3) 看某个地址区间的反汇编（带符号与字符串标注）
python3 tools/re/listing.py 0x1005d592c 0x1005d5e60
```

Ghidra 反编译：

```bash
export JAVA_HOME=/Users/huanxi/Library/Java/JavaVirtualMachines/azul-25.0.1/Contents/Home
export GHIDRA_HEADLESS_MAXMEM=8G
GH=~/tools/ghidra_12.2_PUBLIC/support/analyzeHeadless

# 首次：导入并全量分析（约 13 分钟，工程 ~890MB）
$GH /tmp/ghproj gm -import "$XG_BIN" -overwrite

# 之后：反编译包含指定地址的函数（可传多个地址）
$GH /tmp/ghproj gm -process GmApp -noanalysis \
    -scriptPath tools/re -postScript DumpAt.java 0x1005d592c 0x1005d5de4
```

## 已知坑

1. Ghidra 12.2 的官方 zip 里 `Features/Decompiler/os/` 只有 linux/win 反编译器，
   macOS 会报 `os/mac_arm_64/decompile does not exist`。
   解决办法：从 `ghidra_12.1.4_PUBLIC` 等旧版拷贝 `os/mac_arm_64/decompile` 过去（版本握手兼容）。
2. `llvm-objdump --start-address` 对"非函数起始"的地址不生效，会从段首开始输出。
   `listing.py` 内部已按地址过滤；大范围分析建议先导出全量再检索。
3. Swift 短字符串（≤15 字节）内联在 `mov/movk` 立即数里，不出现在 `__cstring`；
   `listing.py` 会解码并在行尾标注 `; str='...'`。
4. 长字符串在 `__cstring`，其"字面量结构体"位于字符数据前 `0x20` 字节处；
   `xref.py` 已同时匹配这两种引用方式。
