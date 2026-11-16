# xTools

高性能 Windows 启动器（uTools / ZTools 形态）：**搜应用 · 搜文件 · 截图贴图 · JSON 工具**。

## 下载

最新安装包：**[xTools_0.1.1_x64-setup.exe](https://github.com/smilefisher/xTools/releases/latest)**

用户级安装，不需要管理员；安装包内含主程序与索引助手，约 1.9 MB。打 tag 会由 GitHub Actions 自动构建并发布（[Release 页面](https://github.com/smilefisher/xTools/releases)）。

### 版本记录

- **0.1.1**：修复从面板打开 JSON 工具 / 设置窗口 / 截图覆盖层时的**主线程死锁**（表现为窗口打不开、整个界面假死）；窗口创建统一改为后台线程
- **0.1.0**：首个可用版本（搜应用含 UWP、文件搜索含 MFT、截图贴图、JSON 工具）

四个功能均已实现并在真机验证；仅「MFT 提权索引」一条路径仍待一次 UAC 实测（见文末检查清单）。

## 技术栈

- 壳：Tauri 2（Rust + WebView2），release 单文件约 5.8 MB
- 前端：SolidJS + TypeScript + Vite + CodeMirror 6
- 索引：自研紧凑内存索引（父节点引用 + 字节池），落盘快照，命名管道服务化

## 快捷键

| 操作 | 键 |
|---|---|
| 唤起面板 | `Alt+Space`（被占用时自动回退 `Ctrl+Space` → `Alt+Shift+Space` → `Ctrl+Alt+Space`） |
| 截图 | `Alt+Shift+A` |
| 面板内 | `↑↓` 选择 · `PgUp/PgDn/Home/End` 翻页 · `↵` 打开 · `Ctrl+↵` 定位文件 / 以管理员运行 · `Ctrl+C` 复制路径 · 右键动作菜单 · `Esc` 收起 |
| 截图内 | 拖选 → 工具（矩形/箭头/画笔/马赛克）→ `↵` 复制 · `Ctrl+↵` 贴图 · `Ctrl+Z` 撤销 · `Esc` 取消 |
| 贴图窗 | 拖动 · 滚轮缩放 · `Alt+滚轮` 透明度 · 双击关闭 · 右键菜单 |

## 开发与构建

```bash
npm install
npm run app        # tauri dev（热重载）
npm run dist       # NSIS 安装包：target/release/bundle/nsis/xTools_0.1.1_x64-setup.exe（约 1.9MB）
npm run typecheck  # 前端类型检查
cargo test --workspace   # Rust 单测（19 个）
```

安装包为**用户级安装**（`currentUser`，不需要管理员），默认装到 `%LOCALAPPDATA%\Programs\xTools`，界面简中，含 `xTools.exe` 与 `xtools-indexd.exe`。

## 自检与诊断

```bash
xTools.exe --self-check          # 环境/索引/JSON/截图/剪贴板状态，写入 logs\self-check.txt
xTools.exe --probe-apps          # 应用索引与查询样例
xTools.exe --probe-files         # 文件索引（优先读落盘缓存）与查询样例
xTools.exe --probe-mft           # MFT 枚举（需管理员）
xTools.exe --probe-capture       # 抓屏 → 裁剪 → PNG → 剪贴板
xTools.exe --probe-uwp           # UWP 应用枚举与图标提取
xTools.exe --probe-json          # JSON 窗口 + 树视图
xTools.exe --probe-outline       # 30MB 文档的结构扫描
xTools.exe --probe-pipe          # 索引服务连通性
xTools.exe --install-index-service / --uninstall-index-service   # 计划任务（需管理员）
```

## 已实现

| 能力 | 说明 |
|---|---|
| 应用搜索 | 开始菜单 `.lnk` + `App Paths` 注册表 + UWP/Store（`Get-StartApps`，AUMID 启动，图标走 shell 命名空间）；中文/拼音首字母/子序列匹配；图标按需提取并缓存；使用频次加权 |
| 文件搜索 | 目录遍历建索引 + MFT 快速枚举 + USN 日志增量；内存紧凑布局 + rayon 并行扫描；落盘快照 6 小时；`ext:` 过滤、空格 AND；结果含大小与修改日期 |
| 索引服务 | `xtools-indexd.exe` 独立进程，命名管道只读查询；主程序探活成功即不建本地索引 |
| 截图 / 贴图 | 全屏选区、矩形/箭头/画笔/马赛克标注、剪贴板（CF_DIB）、贴图窗（缩放/透明度/右键菜单） |
| JSON 工具 | CodeMirror 6 编辑器、保序格式化、压缩、键排序、流式校验、JSONPath 查询、树视图、30MB 级文档的结构大纲（按区间懒解析） |
| 面板 | 命令前缀（`a ` 应用 / `f ` 文件 / `j ` 或 `{` JSON）、右键菜单、选中跟随滚动 |
| 设置 | 托盘 →「设置」：热键、失焦收起、索引目录、开机自启；保存后热键立即重注册 |
| 常驻 | 单实例、托盘、预热窗口（唤起只做显示+定位+抢焦点）、跟随鼠标屏、开机自启（HKCU Run） |

## 实测数据（Win10 22H2 / Ryzen 5 7500F / release）

| 指标 | 结果 |
|---|---|
| 热键 → 首帧 | p50 **17.5 ms**，p95 **18.1 ms**（空列表）；带 24 行结果约 p50 31 ms |
| 应用索引 | 273 条 / 24~31 ms 建成 / 查询 0.04~0.06 ms |
| 文件索引规模 | 目录扫描 2,408,604 条（`%USERPROFILE%`）；MFT **3,363,054 条**（整个 C: 卷） |
| 建索引耗时 | 目录扫描 35.8 s；**MFT 7.3 s**（含索引组装共 8.1 s） |
| 落盘 / 加载 | 120 MB / 秒级；常驻 124 MB（2.4M 条）/ 194 MB（3.36M 条） |
| 文件查询 | `cargo` 9.3 ms · `.rs` 6.8 ms · `ext:png` 4.3 ms（240 万条） |
| 抓屏 / 裁剪 / PNG | 800×600 抓取 10.3 ms · 裁剪 0.1 ms · 落盘 2.4 ms |
| 一次截图提交 | 187~231 ms（含 80 ms 等待覆盖窗消失） |
| JSON 大纲 | 29.5 MB 文档结构扫描 **225 ms**，展开节点 1.0 s |
| 主进程内存 | 31 MB（服务模式）/ 159 MB（自建 124 MB 索引） |

## 架构要点

- **预热窗口**：启动即隐藏创建，唤起只做 `SetWindowPos` + `ShowWindow` + `AttachThreadInput` 抢焦点，不重建 WebView
- **索引内存布局**：不存完整路径，存「父节点下标 + 名字在字节池的偏移」，展示时回溯拼装
- **查询**：`memchr2` 跳字节 + rayon 并行 + 有界 top-k 堆（避免命中几十万条时的全排序）
- **索引服务**：命名管道一行一 JSON，只接受 `ping`/`search`，不接受执行类指令
- **截图合成**：前端只画预览，最终由 Rust 把矢量操作栅格化到抓取的像素上，避免 DPR/透明通道误差

## 目录

```
src/                前端（SolidJS）：面板、截图层、贴图窗、JSON 工具、设置
src-tauri/          主程序（Rust）：窗口/热键/托盘/截图/JSON/索引调度
crates/indexer/     索引与查询引擎（MFT、USN、落盘、并行查询）
crates/pipe/        命名管道传输
crates/indexd/      索引助手进程
docs/PLAN.md        方案设计与里程碑
scripts/            Server酱 推送、安装脚本、打包辅助
```

## 验证状态

| 项目 | 验证方式 | 状态 |
|---|---|---|
| 应用搜索 / UWP / 图标 | `--probe-apps`、`--probe-uwp` | ✅ 真机通过 |
| 文件索引 / 查询 / 落盘 | `--probe-files`、`--probe-pipe` | ✅ 真机通过 |
| 索引服务（管道） | PowerShell 客户端 + `--probe-pipe` | ✅ 真机通过 |
| 截图 / 标注 / 剪贴板 | `--probe-shot` + 逐像素校验 + .NET 读剪贴板 | ✅ 真机通过（锁屏时会被系统拒绝，属预期） |
| JSON 全部能力 | 19 个单测 + `--probe-json` / `--probe-outline` | ✅ 真机通过 |
| 设置 / 开机自启 | `--probe-autostart` + 注册表核对 | ✅ 真机通过 |
| 安装包 | 7-Zip 解包核对文件清单 | ✅ 构建通过；**安装向导未跑**（火绒 HIPS 会拦一次，需人工放行） |
| **MFT 全量枚举** | 管理员运行 `--probe-mft` | ✅ 真机通过：336 万条 / 7.3 s |
| USN 增量读取 | 管理员运行 `--probe-usn` | ⏳ 逻辑已单测；真机读取待跑 |

## 待办

1. 管理员运行 `scripts/probe-mft.cmd`：已验证 MFT 全量枚举，脚本现在会顺带跑 USN 增量读取（造文件 → 改名 → 删除，再读变更）
2. 双击安装包完成一次安装/卸载（火绒会弹窗，放行即可）
3. 若需要开机即用 MFT：`scripts/install-service.cmd` 装计划任务（一次 UAC），之后登录自动拉起索引助手

环境变量：`XTOOLS_INDEX_ROOTS`（分号分隔索引根）、`XTOOLS_DEBUG=1`（详细日志）。
