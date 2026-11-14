# 高性能 Windows 启动器 · 方案设计（v0.2 定稿）

> 目标：uTools / ZTools 形态的 Windows 常驻效率工具。**本版只做四件事**：搜应用、搜文件、截图（含贴图）、JSON 工具。reference：ZTools（Electron 41 + Vue3 + electron-vite + LMDB + uiohook-napi）、uTools。
>
> 环境已核实：Win10 22H2 (19045)、Ryzen 5 7500F、Node 24.19、Rust 1.98 msvc、VS2019 BuildTools + Win10 SDK 22621、WebView2 157、Everything 已在运行（`E:\Everything`，db 297MB）。

---

## 0. 已锁定决策

| 决策项 | 结论 |
|---|---|
| 技术栈 | **Tauri 2 + Rust 核心 + SolidJS/CodeMirror 6** |
| 文件搜索 | **一步到位自研 MFT + USN 索引服务**（Everything 仅作为可选加速 backend，不进 v1 范围） |
| 功能范围 | 搜应用 / 搜文件 / 截图（含**贴图**，不做长截图） / JSON 工具 |
| 不做 | uTools 插件兼容、OCR、自动更新与代码签名（后续版本再说） |

---

## 1. 结论先行

选 Tauri 2 的三条硬理由：

1. **可控的性能**：空闲 50~80MB、冷启动 200~400ms、窗口预热后可 40ms 内出首帧；Electron 基线是 180~300MB / 800ms+，常驻托盘工具这个差距是体验级的。
2. **原生能力直达**：MFT/USN 读卷、DXGI 抓屏、`SetWindowDisplayAffinity`、`RegisterHotKey` 全部在 Rust 里裸调，不用跨 Node native addon 的版本地狱。
3. **包体**：安装包 ~10MB（WebView2 系统自带），Electron 方案 120MB 起步。

代价：Rust 迭代慢于 TS，插件生态要从零建。对本版范围可接受。

---

## 2. 性能目标（可度量、可回归）

| 指标 | 目标（p95） | 度量点 |
|---|---|---|
| 热键 → 首帧可见 | ≤ 60ms | 热键回调 QPC → 前端 `requestAnimationFrame` 回传 |
| 按键 → 应用结果 | ≤ 16ms | IPC 入参到列表 diff 完成 |
| 按键 → 文件结果（300 万文件库） | ≤ 30ms | 服务端 query 计时 |
| 冷启动到托盘可用 | ≤ 800ms | 进程 start → 托盘图标出现 |
| 空闲内存（主进程 RSS 合计） | ≤ 120MB | 300 万条目索引常驻 |
| 索引服务内存（3M 文件） | ≤ 180MB | 工作集 |
| 全量建索引（1M 文件） | ≤ 3s | 服务日志 |
| 增量（USN 单条） | ≤ 1ms | 服务日志 |
| 截图热键 → 冻结虚影 | ≤ 80ms | 抓帧完成 → overlay 可见 |
| 贴图窗口出现 | ≤ 50ms | |
| JSON 10MB 打开 / 格式化 | ≤ 600ms / ≤ 400ms | |

做法：Rust 内置 `bench` 子命令跑 100 次输出 p50/p95/p99，前端埋点回传，CI 对比基线，退化即报警。

---

## 3. 总体架构

```
┌────────────────────────── zlaunch.exe（主程序，普通权限，单实例） ──────────────────────────┐
│  hotkey      RegisterHotKey(默认 Alt+Space)；被占用自动换键 + 低层钩子兜底                    │
│  window      隐藏窗口预创建（永不销毁）；ShowWindow + 焦点；跟随鼠标所在屏、记忆上次位置       │
│  tray        托盘、开机自启（HKCU Run / 计划任务）、单实例唤醒                                 │
│  search      app_index / 查询解析 / 打分 / 结果动作                                            │
│  capture     DXGI 抓帧、多屏 overlay、选区、标注、贴图窗口                                     │
│  json        编辑器 + 树视图 + 查询（重活丢线程池，前端只渲染）                                │
│  store       SQLite(WAL)：应用索引、MRU、设置、截图历史                                       │
└───────────────────────────┬──────────────────────────────────────────────────────────────────┘
                            │ named pipe（bincode 帧，ACL 仅当前用户 SID）
┌───────────────────────────┴──────────────────────────────────────────────────────────────────┐
│                    zlaunch-index.exe（Windows 服务，SYSTEM，无 UI）                          │
│  MFT 全量扫描 → 路径树 + 名称索引 → 落盘 mmap 快照                                            │
│  USN Journal 监听 → 增量增删改（含 FRN 复用、父目录改名级联）                                  │
│  查询引擎：3-gram 倒排剪枝 + 前缀 trie + 打分；多卷（C:/D:/E:）各自独立状态                   │
│  只读服务：只回答查询，不做「打开/执行」，避免 SYSTEM 权限被滥用（LPE 风险）                   │
└──────────────────────────────────────────────────────────────────────────────────────────────┘
                            │ WebView2
              ┌─────────────┴──────────────┐
              │ SolidJS 面板 + 虚拟列表      │
              │ CodeMirror 6（JSON）        │
              │ Canvas（截图选区/标注/贴图）  │
              └────────────────────────────┘
```

**统一命令模型**：列表每一项都是 `Command { id, title, subtitle, icon, score, actions[] }`；输入框前缀路由（`f ` 文件 / `a ` 应用 / `j ` JSON），`{`、`[` 开头自动进 JSON。后面要接插件，就是给这个模型加个 provider 而已。

---

## 4. 搜应用

- **来源**：开始菜单（系统 + 用户，`.lnk` 用 `IShellLink` 解析 Target/Args/IconLocation）、`App Paths` 注册表、UWP（`PackageManager.FindPackagesForUser` + AppxManifest，`IApplicationActivationManager` 启动）、可选 Steam/Epic。
- **索引**：启动全量 1~3k 条（< 40ms）→ SQLite；`ReadDirectoryChangesW` 监听开始菜单增量；无定时轮询。
- **图标**：`SHGetFileInfo` 取 256px → PNG 缓存 `%LOCALAPPDATA%\zlaunch\icons\`，前端懒加载。
- **匹配**：中文名 + 拼音全拼/首字母（`wx` → 微信）；打分 `文本分*0.6 + 频次*0.3 + 最近*0.1`，MRU 异步落库。
- **坑**：UWP 图标在 Assets 里带 scale 后缀；`RunAsAdministrator` 的 lnk 要加盾标；卸载后需清理残留。

---

## 5. 搜文件：自研 MFT + USN 索引服务（本方案技术核心）

### 5.1 为什么必须提权、以及怎么提

读 `$MFT` 和 `FSCTL_READ_USN_JOURNAL` 都要求对卷有读权限 → 必须管理员。做法对齐 Everything：

- 首次使用引导安装 **Windows 服务** `ZLaunchIndex`（`sc create`，一次 UAC，SYSTEM 运行，开机自启，失败可回滚卸载）。
- 服务只做「索引 + 只读查询」，**不做任何按路径打开/执行**，管道 ACL 限定当前交互用户 SID——避免"系统级服务 + 任意命令"这种提权面。
- 用户拒绝提权：主程序自动降级为「按需遍历指定目录」模式（索引存 SQLite，`ReadDirectoryChangesW` 增量），并在设置页明示能力差异。

### 5.2 索引数据与内存布局

按卷存三块，mmap 到文件以便秒级重启（免重建）：

| 结构 | 内容 | 1M 文件占用 |
|---|---|---|
| `Meta[]` | parent_idx(u32)、name_off(u32)、flags(u16)、size(u64)、mtime(i64) | ~24MB |
| `Names` | 名称池，前向压缩（front-coding）+ UTF-8 | ~20MB |
| `QGram` | 3-gram → 文件号倒排（分块 + delta 编码，前缀剪枝） | ~60MB |

- 路径按 `.lnk` 式「父引用 + 名字偏移」存，不存完整路径字符串（省 5~10 倍内存），展示时回溯拼装（O(depth)）。
- 查询：`3-gram 求交剪枝 → 候选集 → 精确子串/前缀校验 → 打分排序 → top 50`。1M 文件典型 3~8ms。
- 打分：完全匹配 > 前缀 > 词界（`-`/`_`/`.`/驼峰）> 子串；同级按 路径深度、mtime、用户打开历史 加权。

### 5.3 全量与增量

- **全量**：`FSCTL_GET_NTFS_VOLUME_DATA` 拿 MFT 起始/大小 → 裸读解析 `FILE` 记录（需处理 `ATTRIBUTE_LIST` 扩展记录、硬链接多父、重解析点跳过）。1M 文件 1~3s。
- **增量**：独立线程阻塞在 `FSCTL_READ_USN_JOURNAL`；`USN_RECORD_V2/V3` 给 (FRN, ParentFRN, Name, Reason)。改名/移动只要 `FILE_CREATE|RENAME_OLD_NAME` 等掩码组合；父目录改名靠 FRN 引用天然不变，无需级联。
- **健壮性**：记录 `journal_id` + `next_usn` 落盘；重启时校验 journal 是否被重建/回绕（`FSCTL_QUERY_USN_JOURNAL` 比对 id），不一致就重新全量。卷离线/热插拔按卷粒度重建。
- **省电**：USN 线程阻塞等待，空闲 CPU 0%；不做轮询。

### 5.4 查询语法

空格 AND、`ext:pdf`、`path:d:\work`、`"精确"`、`re:` 正则开关、`size:>10m`、`dm:7d`。结果动作一次做全：打开、打开所在文件夹（`explorer /select,`）、复制路径、VSCode 打开、管理员运行、删除到回收站。

### 5.5 可选加速 backend（不进 v1）

`trait FileSearchBackend`，若检测到 Everything 在跑，可一键切到 Everything IPC（亚毫秒级）；作为对照与兜底保留接口，不写实现。

---

## 6. 截图 + 贴图

1. 热键 → `IDXGIOutputDuplication` 抓所有屏（4K×2 ≈ 33MB）→ 合成虚拟桌面位图。
2. 每屏弹一个置顶无边框窗，背景 = 刚抓的**冻结帧** + 40% 遮罩（先抓后显，无闪黑）。
3. 位图走 `CreateFileMapping` **共享内存零拷贝**给前端 canvas（33MB 走 IPC 会卡）。
4. 选区：十字 + 放大镜（像素 + HEX 取色）+ 尺寸角标 + 窗口边界吸附（枚举可见窗 `GetWindowRect`）。
5. 松手 → 工具条：矩形/椭圆/箭头/画笔/文字/马赛克/序号 → 回车复制到剪贴板（`CF_DIBV5` 保 alpha），可选另存。
6. **贴图**：`Ctrl+T` 或按钮 → 生成一个置顶无边框「贴图窗」贴在原位；可拖动、滚轮缩放、`Alt`+滚轮调透明度、双击关闭、右键菜单（复制/另存/关闭）；多张互不干扰，随屏幕 DPI 正确缩放，`Esc` 逐个关闭。
7. 细节：Per-Monitor-V2 DPI（混合 DPI 必做，否则选区偏移）；HDR 屏 tonemap 防发灰；截图期间自身窗口 `SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)`。

---

## 7. JSON 工具

- 入口：粘贴/拖入/文件关联；输入 `{` 或 `j ` 直接进。
- **解析**：Rust `serde_json`（严格）/ `json5`（宽松，开关）。≤20MB 建树；>20MB 走 **tape 模式**：只解析顶层，记录子树字节偏移，展开时按需解析 + 虚拟列表，100MB 也能开。
- **视图**：树（折叠/类型着色/路径面包屑）+ 原文（CodeMirror 6，高亮、错误行内标记）。
- **操作**：格式化（2/4 空格/Tab）、压缩、键排序、转义/反转义、JSONPath 子集查询（`$.a.b[*]`）、全文搜索（高亮 + 上下跳）、校验（行列 + 中文提示）、双栏 diff。
- **性能**：解析/格式化全在 Rust 线程池，大结果分片回传。

---

## 8. 性能工程清单

1. 窗口永不复建，`show` 只做 `SetWindowPos` + `ShowWindow`；DOM 不重载。
2. 焦点：注册热键的线程自带前台权限；兜底 `AttachThreadInput` + 短暂 `keybd_event`；失败率打点。
3. 大对象零 IPC：图片共享内存，列表 bincode，首屏 50 条按需追加。
4. 首屏只渲染 50 行 + 虚拟列表 + 图标懒加载；禁 `backdrop-filter`，毛玻璃走 DWM。
5. 每次按键 `AbortHandle` 上一次查询；Rust 线程池，不占 UI 线程。
6. 索引常驻内存，SQLite 只做持久化。
7. 空闲零定时器（事件驱动）。
8. 启动分帧：托盘先起（<300ms），索引/图标检查延后到 idle。

---

## 9. 目录结构

```
E:\ztools\
├─ src\                # 前端（Vite + SolidJS + TS）
│  ├─ App.tsx  index.tsx  App.css
│  └─ views\{Launcher,Json,Shot}  components\        # M1 起逐层铺开
├─ src-tauri\          # 主程序（Tauri 2）
│  ├─ src\ lib.rs main.rs hotkey.rs win.rs tray.rs settings.rs cmd.rs bench.rs
│  │      search\{app_index.rs, file_client.rs, rank.rs, pinyin.rs}       # M1/M2
│  │      capture\{grab.rs, overlay.rs, annotate.rs, pin.rs, clipboard.rs} # M3
│  │      json\{engine.rs, tape.rs, query.rs, format.rs}                  # M4
│  └─ tauri.conf.json  capabilities\  icons\
├─ crates\             # M2 起：workspace 成员
│  ├─ indexer\         # MFT/USN/查询引擎（服务与降级模式共用）
│  └─ index-svc\       # xtools-index.exe：服务宿主 + named pipe
├─ docs\PLAN.md
├─ Cargo.toml          # cargo workspace
└─ package.json
```

仓库：<https://github.com/smilefisher/xTools>

---

## 10. 里程碑（单人，约 3~4 周）

| 阶段 | 内容 | 验收 |
|---|---|---|
| **M0 骨架** 1~2d | Tauri 初始化、单实例、托盘、热键、预热窗口、SQLite 设置、bench 打点、IPC 协议 | 热键→首帧 p95 ≤ 60ms；空闲 ≤ 80MB |
| **M1 应用搜索** 2~3d | 三来源索引、图标缓存、拼音匹配、MRU、启动/管理员启动 | 1000+ 应用；按键→结果 ≤ 16ms；`wx` 能出微信 |
| **M2 索引服务** 4~6d | MFT 全量解析、USN 增量、内存索引 + mmap 快照、查询引擎、服务安装/卸载/自启、管道 ACL、降级模式 | 1M 文件建索引 ≤ 3s；300 万查询 p95 ≤ 30ms；重启秒级可用；改文件 1s 内可搜到 |
| **M3 截图+贴图** 3~4d | 多屏抓帧、overlay、选区/吸附/放大镜、标注、剪贴板/另存、贴图窗 | 热键→虚影 ≤ 80ms；4K 双屏无偏移；贴图拖动缩放正常 |
| **M4 JSON** 2~3d | 编辑器、树视图、格式化/压缩/排序、JSONPath、搜索、tape 大文件、diff | 10MB ≤ 600ms；100MB 不崩；滚动 60fps |
| **M5 打磨** 1~2d | 开机自启、崩溃日志、首启引导（含提权说明）、NSIS 打包 | 安装包 ≤ 15MB；卸载干净（服务、自启、缓存） |

---

## 11. 风险与对策

| 风险 | 对策 |
|---|---|
| 服务安装被拒/被杀软拦 | 一键卸载回滚；降级模式可用；文档写清「只读索引、不执行命令」 |
| USN journal 回绕/journal 重建 | 校验 journal_id + next_usn，不一致自动全量重建 |
| MFT 解析边界（ATTRIBUTE_LIST、硬链接、稀疏/压缩文件） | 参考 `ntfs` crate 实现；用 10 万条样本与 Everything 结果对拍，差异 > 0.1% 阻断发版 |
| FRN 复用导致幽灵条目 | 以 `(FRN, seq)` 为键，USN 记录的 seq 校验 |
| 全局热键被占 | 自动换键 + 设置可改 + 提示占用者 |
| 混合 DPI / HDR | Per-Monitor-V2 + DXGI 色彩转换，三屏真机验证 |
| 大 JSON 内存 | tape + 上限提示 + 分片渲染 |
| 未签名被 SmartScreen 拦 | v1 自用可接受；对外分发再补签名 |

---

## 12. 下一步

M0 可以立刻开工：`npm create tauri-app` → 接热键/托盘/预热窗口 → 打通 bench 打点，先拿到「热键→首帧」的真实数字，再往上叠功能。
