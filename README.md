# Saya

> 出鞘，即知所想。 (Unsheathe your thoughts instantly.)

Rust 原生内核 + 本地 AI 语义检索的 macOS 启动器与剪贴板工具。MVP 聚焦验证「极致原生 + 本地语义」的核心假设。

---

## MVP 范围

**纳入**

| 模块 | 内容 |
|---|---|
| 平台 | macOS 15+ (Sequoia) / Apple Silicon Only |
| 应用启动器 | 全局快捷键唤起，键入秒级过滤启动 |
| 语义剪贴板 | 自动记录历史，字面量检索默认开启，向量化默认关闭 |
| 设置面板 | 快捷键、保留策略、向量化开关、模型状态 |
| 菜单栏 | SwiftUI `MenuBarExtra` 常驻 |
| CLI | `saya search` / `saya status` / `saya reindex` |

**推迟到 V1**

Windows 端、Wasm 插件、笔记/文件向量化、云同步、SQLCipher、遥测、多语言 SDK。

---

## 架构原则

核心业务全部 Rust 实现；Swift 仅承担 UI 渲染与必须在主 RunLoop 的系统集成。

- **Rust (`saya-core`)**: 数据库、AI 推理、剪贴板监听、应用扫描、图标提取（`objc2` 直调 NSWorkspace）、模型生命周期、CLI 逻辑
- **Swift (`saya-macos`)**: SwiftUI 视图、`MenuBarExtra`、全局快捷键注册、面板窗口生命周期
- **桥接**: UniFFI 函数调用 + Swift→Rust 事件回调

---

## 应用启动器

- 默认快捷键 `⌥ Space`，可配置
- 数据源：`/Applications`、`/System/Applications`、`~/Applications`
- 冷启动：Rust `rayon` 并行扫描 `.app` Bundle，构建前缀树
- 图标：Rust 通过 `objc2` 调 `NSWorkspace.shared.icon(forFile:)`，PNG 编码传 SwiftUI
- 增量：`notify` 订阅 FSEvents
- 匹配：大小写不敏感子序列模糊匹配 + MRU 加权
- UI：SwiftUI `LazyVStack`，最多 8 条，回车启动 / `⌘1-8` 快速选择
- 性能：唤起到可输入 < 100ms；键入到刷新 < 16ms

---

## 语义剪贴板

- **监听**: Rust 通过 `objc2` 轮询 `NSPasteboard.changeCount`（300ms 间隔），仅处理 `public.utf8-plain-text`
- **去重**: 与最近一条相同则不入库
- **阈值**: 单条 > 100KB 不入库；保留 30 天（可配置）
- **字面量检索**（默认开启）: `jieba-rs` 分词 + Tantivy BM25
- **向量化**（默认关闭）:
  - 模型 `all-MiniLM-L6-v2`（384 维，≈90MB）
  - 缓存复用 `~/.cache/huggingface/hub/`
  - Candle + Metal 推理；lazy load，空闲 5 分钟卸载
  - 开启后异步生成新条目向量 + 后台补齐历史
  - 关闭后保留已有向量但不新增
- **存储**: `rusqlite` + `sqlite-vec`，WAL，单写 + 读连接池 (`r2d2`, size 4)
- **检索**: 向量化开启时 BM25 + 余弦 RRF 融合；关闭时纯 BM25。10 万条 P50 < 50ms
- **UI**: 独立面板（`⌥⇧V`），列表 + 预览，回车回填前一个 App

---

## 设置面板

- 全局快捷键绑定（启动器 / 剪贴板）
- 剪贴板保留天数、最大条数
- 向量化开关；开启后展示模型下载状态、历史补齐进度、空闲卸载时间
- 数据库与日志路径（"在 Finder 中显示"）
- 开机自启动开关

---

## 菜单栏

SwiftUI `MenuBarExtra` 常驻状态栏。

- 菜单项：唤起启动器 / 唤起剪贴板 / 暂停监听 / 设置 / 退出
- 状态图标：正常 / 向量化进行中 / 错误

---

## CLI

```
saya search <query>      # 混合检索剪贴板，输出 JSON / TSV
saya status              # 内存、DB 大小、向量化队列、模型状态
saya reindex             # 模型升级时重建向量
```

与 GUI 共享同一 SQLite 文件，靠 WAL + 文件锁协调，不引入 daemon IPC。

---

## 性能指标

| 指标 | 目标 |
|---|---|
| 静默内存（向量化关闭） | < 25MB |
| 静默内存（向量化开启 + 空闲） | < 30MB |
| 推理峰值内存 | < 250MB，任务后 5 分钟内卸载回落 |
| 静默 CPU | < 0.1%（剪贴板轮询） |
| 启动器首次响应 | < 100ms |
| 剪贴板检索 P50 | < 50ms（10 万条） |
| 单包二进制 | < 60MB（不含模型） |

---

## 隐私

- 100% 本地，仅模型下载时联网（且需用户开启向量化）
- 无遥测
- 数据位于 `~/Library/Application Support/Saya/`，权限 0700
- MVP 不启用 SQLCipher

---

## 工程结构

```text
saya/
├── Cargo.toml
├── crates/
│   ├── saya-core/
│   │   ├── database/        # rusqlite + sqlite-vec
│   │   ├── ai/              # Candle + hf-hub + 模型生命周期
│   │   ├── clipboard/       # NSPasteboard 监听 (objc2)
│   │   ├── launcher/        # 应用扫描 + 图标提取 (objc2)
│   │   ├── search/          # Tantivy + jieba-rs + 混合检索
│   │   └── ffi/             # UniFFI
│   └── saya-cli/
└── apps/
    └── saya-macos/          # SwiftUI 外壳
```

---

## 里程碑

| 阶段 | 退出条件 |
|---|---|
| M1 内核 Alpha | `saya search` 跑通完整链路 |
| M2 GUI Beta | 启动器 + 剪贴板面板可日用 |
| M3 MVP Release | 设置面板 + 自启动 + 签名公证 + DMG 分发 |

---

## 决议日志

> Append-only。每条记录一个已敲定的技术或范围决策，按时间倒序追加。

- **2026-05-20** 字面量车道升级为 Tantivy BM25 + jieba-rs 分词。LIKE 占位移除；`Database::insert_entry` 双写 SQLite + Tantivy（Tantivy 失败仅 warn 不阻塞），`delete_older_than` 两阶段（先取 ids → 删 Tantivy → 删 SQLite）。索引目录 `<db_dir>/text_index/`，内存模式用 `Index::create_in_ram` 给测试用
- **2026-05-20** 自定义 `JiebaTokenizer` 实现 `tantivy::tokenizer::Tokenizer`，调 `jieba.tokenize(_, Search, true)` 拿到带 byte offset 的 token（不用 `cut_for_search` 因为后者结果有重叠，byte offset 不单调）。不做英文词干化 —— jieba 不擅长，且 SwiftUI 前端用户体验更可预测
- **2026-05-20** Launcher FSEvents 增量更新策略：事件只作为 hint，真值以 `path.exists()` 重新校验。FSEvents 在 .app 包内文件变动时疯狂喷子事件，路径折叠到 .app 边界后用 filesystem stat 决定 insert/remove，自然去重
- **2026-05-20** Launcher MRU 评分：recency 阶梯（<7d +400 / <30d +200 / <90d +50 / 老 0）+ freq capped（`min(count,20)*10` ≤ +200），合计 ≤ +500 < prefix bonus (+1000)。冷启动键入"saf"时新装的 Safari 仍胜过老用过 100 次的 Stackoverflow App
- **2026-05-20** 浮窗使用 `NSPanel` + `.nonactivatingPanel` + `hidesOnDeactivate`，Spotlight 居中定位（屏幕水平中线，垂直偏上 1/3），Esc 经 `cancelOperation` 收回。非激活样式确保不抢前一个 App 焦点 —— 粘贴时关键
- **2026-05-20** `MenuBarExtra` 改用 `.menu` 风格（不再是 popover），三个面板独立 NSPanel 实例，由 PanelController 管理。原因：popover 锚定菜单栏位置无法实现 Spotlight 居中体验
- **2026-05-20** 应用启动走 `/usr/bin/open` 而非 NSWorkspace API。新版 `openApplicationAtURL:configuration:completionHandler:` 是异步 callback，跨 FFI 不友好；老版 `openURL:` 已弃用。fork+exec ~5ms 相对 App 启动延迟可忽略
- **2026-05-20** 拆 `crates/saya-ffi` 为独立 crate（uniffi 桥接 + DTO + opaque Saya + bindgen 二进制）。saya-core 退回纯 rlib，`cargo test -p saya-core` 不再链接 staticlib/cdylib；为多端绑定（未来 Kotlin/Python）预留平行 crate
- **2026-05-20** Xcode 项目用 [xcodegen](https://github.com/yonaskolb/XcodeGen) 从 `project.yml` 生成（.xcodeproj 已 gitignore）。pbxproj 不版本化，但项目配置版本化。比手写 pbxproj / swiftc+bash / Tuist 更适合 MVP
- **2026-05-20** Rust staticlib 用 `$(SAYA_FFI_STATICLIB)` Xcode 变量传完整路径而非 `-lsaya_ffi`。Cargo 必须同时输出 staticlib（Swift 用）和 cdylib（`uniffi-bindgen --library` 内省用），无法只产其中之一；`-lsaya_ffi` 会让 ld 优先挑 cdylib
- **2026-05-20** Swift 绑定用 Objective-C 桥接头（`SWIFT_OBJC_BRIDGING_HEADER`）而非 modulemap。一行 build setting vs 模块发现路径链 + clang 配置
- **2026-05-20** 链接需补 `SystemConfiguration.framework`。reqwest/ureq 用系统代理 + 可达性 API；初次链接报 `_SCDynamicStoreCopyProxies` 等未定义符号
- **2026-05-20** 混合检索用 Reciprocal Rank Fusion（k=60，标准值），每条车道过取 `4×limit`（至少 20）给融合留排序空间。无 embedder 时退化为纯字面量车道
- **2026-05-20** SQLite 初始化分两阶段：先单连接 bootstrap（设 WAL + 跑 schema migration）→ 关闭 → 再建 r2d2 池。原因：WAL 模式切换需短暂排他锁，r2d2 启动期并发开 4 连接会偶发 `database is locked`
- **2026-05-20** Rust edition 2024
- **2026-05-20** 跳过 M0 PoC 基准测试阶段，直接进入 M1 开发；性能指标改由真实使用反馈验证
- **2026-05-20** 平台收紧为 macOS 15+ / Apple Silicon Only，放弃 Intel 与低版本兼容
- **2026-05-20** 向量化默认关闭，用户在设置中显式启用后才下载模型
- **2026-05-20** 模型缓存复用 HuggingFace 默认目录 `~/.cache/huggingface/hub/`，不自建 CDN
- **2026-05-20** 中文分词使用 `jieba-rs`
- **2026-05-20** 全局快捷键在 Swift 侧用 [`KeyboardShortcuts`](https://github.com/sindresorhus/KeyboardShortcuts) 库（Carbon 注册，可拦截按键）
- **2026-05-20** 菜单栏用 SwiftUI `MenuBarExtra`（macOS 13+ 原生）
- **2026-05-20** 图标提取放 Rust 侧，用 `objc2` 直调 NSWorkspace，避免跨 FFI 碎片
- **2026-05-20** 架构原则：核心业务全部 Rust（含平台 API 直调）；Swift 仅 UI 渲染 + 主 RunLoop 系统集成
- **2026-05-20** MVP 性能指标放宽：静默内存 25MB（向量化关闭），二进制 60MB（原 REQ 10MB/20MB 在引入 Candle 后不可达）
- **2026-05-20** CLI 与 GUI 通过 WAL + 文件锁共享 SQLite，不引入 daemon IPC
