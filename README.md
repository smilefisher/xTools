# xTools

高性能 Windows 启动器（uTools / ZTools 形态）：**搜应用 · 搜文件 · 截图贴图 · JSON 工具**。

当前进度：**M0 骨架**（单实例 / 托盘 / 全局热键 / 预热面板 / 首帧打点）。

## 技术栈

- 壳：Tauri 2（Rust + WebView2）
- 前端：SolidJS + TypeScript + Vite
- 文件索引：自研 MFT + USN 服务（M2 落地，见 [docs/PLAN.md](docs/PLAN.md)）

## 开发

```bash
npm install
npm run app        # tauri dev：热重载开发
npm run dist       # 打 NSIS 安装包（release）
npm run typecheck  # 前端类型检查
```

首次 `npm run app` 会编译 Rust 依赖，几分钟；之后增量编译很快。

## 安装包

```bash
npm run dist       # 产物：target/release/bundle/nsis/xTools_0.1.0_x64-setup.exe（约 1.8MB）
```

- 用户级安装（`currentUser`），**不需要管理员**，默认装到 `%LOCALAPPDATA%\Programs\xTools`
- 安装界面简体中文（内置简中 + 英文），卸载走「设置 → 应用」或安装目录的 `uninstall.exe`
- 依赖 WebView2 运行时：Win11 自带；Win10 若缺，安装器会引导安装

## 性能测量

```bash
# 自驱 20 轮「唤起 → 首帧」，输出 p50/p95，并写 bench-latency.json
npm run tauri -- dev -- --bench=20
```

面板右下角会实时显示最近一次「热键 → 首帧」耗时，方便肉眼回归。

**M0 实测（Win10 22H2 / Ryzen 5 7500F / release 构建）**

| 指标 | 结果 |
|---|---|
| 热键 → 首帧 p50 | **17.5 ms** |
| 热键 → 首帧 p95 | **18.1 ms** |
| 首帧 min / max | 16.5 ms / 30.2 ms |
| 主进程体积 | 4.5 MB（exe） |
| 主进程内存 | 30.5 MB |
| 含 WebView2 子进程总内存 | 140.7 MB |

## 已实现

| 能力 | 说明 |
|---|---|
| 应用搜索 | 开始菜单 `.lnk` + `App Paths` 注册表，中文/拼音首字母/子序列匹配，图标按需提取并缓存，使用频次加权 |
| 文件搜索 | 目录遍历建索引（默认 `%USERPROFILE%`），内存紧凑布局 + rayon 并行扫描，落盘缓存 6 小时，`ext:` 过滤、空格 AND |
| 截图 / 贴图 | `Alt+Shift+A` 唤起全屏选区 → 回车复制到剪贴板、`Ctrl+回车` 贴到屏幕；截图自动存一份到 `%LOCALAPPDATA%\xtools\shots` |
| JSON 工具 | 独立窗口（CodeMirror 6）：格式化（保序/2·4 空格/Tab）、压缩、键排序、校验（报行列）、JSONPath 查询；面板里输入 `{` 可直接带内容打开 |
| 设置窗口 | 托盘右键 →「设置」：热键、失焦收起、索引目录增删、开机自启；保存后热键**立即重新注册**，索引目录改动重启生效 |
| 开机自启 | 写 `HKCU\...\Run`（无需管理员），设置里一键开关 |
| 单实例 | 重复启动不会开第二个进程，直接把已有面板叫到前台 |
| 托盘常驻 | 左键唤起，右键菜单可重新注册热键 / 退出；关窗只隐藏不退出 |
| 全局热键 | 默认 `Alt+Space`，失败自动回退 `Ctrl+Space` → `Alt+Shift+Space` → `Ctrl+Alt+Space` |
| 预热窗口 | 启动即创建隐藏窗口，唤起只做「定位 + 显示 + 抢焦点」，不重建窗口 |
| 跟随屏幕 | 在鼠标所在显示器居中偏上弹出 |
| 抢焦点 | AttachThreadInput 绕过前台锁，热键唤起后可直接输入 |
| 首帧打点 | Rust 记录唤起时刻，前端首帧回调，实时显示 + bench 模式统计 |
| 设置 | `%APPDATA%\xtools\settings.json`（热键、失焦收起、主题） |

## 目录

```
src/            前端（SolidJS）
src-tauri/      Tauri 主程序（Rust）
docs/PLAN.md    方案设计与里程碑
```

## 路线图

- **M0** 骨架 ✅
- **M1** 应用搜索 ✅（开始菜单 / App Paths / 拼音 / 图标 / MRU；UWP 待补）
- **M2** 文件搜索：目录索引 + 落盘缓存 + 查询引擎 ✅；MFT 枚举 ✅（待提权验证）；USN 日志增量 ✅（提权时启用）；提权索引服务 🚧
- **M3** 截图 + 贴图：冻结帧选区、标注、剪贴板、置顶贴图 ✅（选区/复制/贴图已通，标注待做）
- **M4** JSON：编辑器、树视图、格式化 / JSONPath / 大文件懒解析 ✅（编辑器/格式化/压缩/排序/校验/JSONPath；树视图与 tape 待做）
- **M5** 打磨：开机自启、崩溃日志、安装包

### 文件搜索实测（release）

| 指标 | 结果 |
|---|---|
| 索引规模 | 2,408,604 条（`%USERPROFILE%`） |
| 首次扫描建索引 | 35.8 s |
| 落盘大小 / 加载耗时 | 120 MB / 秒级 |
| 常驻内存 | 124.3 MB |
| 查询延迟（`cargo` / `.rs` / `ext:png`） | 9.3 / 6.8 / 4.3 ms |

环境变量：`XTOOLS_INDEX_ROOTS`（分号分隔的索引根）、`XTOOLS_DEBUG=1`（详细日志）。
