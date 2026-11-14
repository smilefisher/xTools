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
- **M2** 文件搜索：MFT + USN 索引服务，百万级毫秒响应
- **M3** 截图 + 贴图：冻结帧选区、标注、剪贴板、置顶贴图
- **M4** JSON：编辑器、树视图、格式化 / JSONPath / 大文件懒解析
- **M5** 打磨：开机自启、崩溃日志、安装包
