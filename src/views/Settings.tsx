import { For, Show, createSignal, onMount } from "solid-js";
import { invoke } from "@tauri-apps/api/core";

type Settings = {
  hotkey: string;
  shot_hotkey: string;
  autohide_on_blur: boolean;
  theme: string;
  index_roots: string[];
};

type SaveResult = { settings: Settings; problems: string[] };
type AutostartState = { enabled: boolean; command: string | null; exe: string };
type FileStatus = { ready: boolean; entries: number; backend: string };
type ServiceStatus = {
  installed: boolean;
  run_level: string | null;
  running: boolean;
  elevated: boolean;
};

export default function SettingsView() {
  const [draft, setDraft] = createSignal<Settings | null>(null);
  const [autostartEnabled, setAutostartEnabled] = createSignal(false);
  const [exePath, setExePath] = createSignal("");
  const [status, setStatus] = createSignal("");
  const [problems, setProblems] = createSignal<string[]>([]);
  const [rootInput, setRootInput] = createSignal("");
  const [recording, setRecording] = createSignal<"hotkey" | "shot_hotkey" | null>(null);

  /// 把 "ctrl+alt+a" 显示成 "Ctrl + Alt + A"
  const prettyHotkey = (spec: string) => {
    if (spec.trim().toLowerCase() === "dblshift") return "双击 Shift";
    return spec
      .split("+")
      .map((part) => {
        const p = part.trim().toLowerCase();
        if (p === "ctrl" || p === "control") return "Ctrl";
        if (p === "alt") return "Alt";
        if (p === "shift") return "Shift";
        if (p === "meta" || p === "win" || p === "super") return "Win";
        if (p === "space") return "Space";
        if (p === "esc" || p === "escape") return "Esc";
        if (/^f\d{1,2}$/.test(p)) return p.toUpperCase();
        return p.length === 1 ? p.toUpperCase() : p.charAt(0).toUpperCase() + p.slice(1);
      })
      .join(" + ");
  };

  /// 从键盘事件生成 "ctrl+alt+a" 形式的字符串；只按了修饰键返回 null
  const hotkeyFromEvent = (event: KeyboardEvent): string | null => {
    const key = event.key;
    if (!key || ["Control", "Alt", "Shift", "Meta"].includes(key)) return null;

    const named: Record<string, string> = {
      " ": "space",
      Escape: "esc",
      Enter: "enter",
      Tab: "tab",
      Backspace: "backspace",
      ArrowUp: "up",
      ArrowDown: "down",
      ArrowLeft: "left",
      ArrowRight: "right",
      Delete: "delete",
      Home: "home",
      End: "end",
      PageUp: "pageup",
      PageDown: "pagedown",
      Insert: "insert",
    };

    let name = named[key];
    if (!name) {
      if (key.length === 1) name = key.toLowerCase();
      else if (/^F\d{1,2}$/i.test(key)) name = key.toLowerCase();
      else name = key.toLowerCase();
    }

    const parts: string[] = [];
    if (event.ctrlKey) parts.push("ctrl");
    if (event.altKey) parts.push("alt");
    if (event.shiftKey) parts.push("shift");
    if (event.metaKey) parts.push("meta");
    if (parts.length === 0) return null;

    return [...parts, name].join("+");
  };

  let lastShiftAt = 0;

  const recordHotkey = (which: "hotkey" | "shot_hotkey", event: KeyboardEvent) => {
    event.preventDefault();
    event.stopPropagation();
    if (event.key === "Escape") {
      setRecording(null);
      return;
    }

    // 连按两次 Shift = 双击 Shift（走键盘钩子，不是普通热键）
    if (event.key === "Shift" && !event.ctrlKey && !event.altKey && !event.metaKey) {
      const now = performance.now();
      if (now - lastShiftAt < 420) {
        lastShiftAt = 0;
        patch({ [which]: "dblshift" } as Partial<Settings>);
        setRecording(null);
        setStatus("已记录「双击 Shift」，点保存生效");
      } else {
        lastShiftAt = now;
        setStatus("再按一次 Shift 即可设为「双击 Shift」");
      }
      return;
    }

    const spec = hotkeyFromEvent(event);
    if (!spec) {
      setStatus("请至少按住 Ctrl / Alt / Win 之一再按其它键");
      return;
    }
    patch({ [which]: spec } as Partial<Settings>);
    setRecording(null);
    setStatus(`已记录 ${prettyHotkey(spec)}，点「保存」生效`);
  };
  const [fileStatus, setFileStatus] = createSignal<FileStatus | null>(null);
  const [serviceStatus, setServiceStatus] = createSignal<ServiceStatus | null>(null);

  const backendLabel = (backend: string) =>
    backend === "service"
      ? "索引服务（xtools-indexd）"
      : backend === "cache"
        ? "本地索引（扫描 + 落盘快照）"
        : "构建中…";

  const refreshIndexState = async () => {
    try {
      setFileStatus(await invoke<FileStatus>("file_status"));
      setServiceStatus(await invoke<ServiceStatus>("index_service_status"));
    } catch {
      /* 忽略 */
    }
  };

  const reindex = async () => {
    await invoke("file_reindex").catch(() => {});
    setStatus("已开始重建索引（后台进行，约 30 秒）");
    window.setTimeout(() => void refreshIndexState(), 3000);
  };

  const installService = async () => {
    try {
      const message = await invoke<string>("index_service_install");
      setStatus(message);
      void refreshIndexState();
    } catch (err) {
      setStatus(String(err));
    }
  };

  const uninstallService = async () => {
    try {
      setStatus(await invoke<string>("index_service_uninstall"));
      void refreshIndexState();
    } catch (err) {
      setStatus(String(err));
    }
  };

  onMount(async () => {
    void invoke("log_frontend", { message: "settings view mounted" }).catch(() => {});
    try {
      const loaded = await invoke<Settings>("settings_get");
      void invoke("log_frontend", { message: `settings loaded hotkey=${loaded.hotkey} shot=${loaded.shot_hotkey}` }).catch(() => {});
      setDraft(loaded);
      const auto = await invoke<AutostartState>("autostart_get");
      setAutostartEnabled(auto.enabled);
      setExePath(auto.exe);
      await refreshIndexState();
    } catch (err) {
      void invoke("log_frontend", { message: `settings load FAILED ${String(err)}` }).catch(() => {});
      setStatus(`读取设置失败：${String(err)}`);
    }
  });

  const patch = (change: Partial<Settings>) => {
    const current = draft();
    if (current) setDraft({ ...current, ...change });
  };

  const save = async () => {
    const current = draft();
    if (!current) return;
    try {
      const result = await invoke<SaveResult>("settings_save", { next: current });
      setDraft(result.settings);
      setProblems(result.problems);
      setStatus(result.problems.length > 0 ? "已保存（部分热键未生效）" : "已保存，热键已重新注册");
    } catch (err) {
      setStatus(`保存失败：${String(err)}`);
    }
  };

  const toggleAutostart = async () => {
    const next = !autostartEnabled();
    try {
      const applied = await invoke<boolean>("autostart_set", { enabled: next });
      setAutostartEnabled(applied);
      setStatus(applied ? "已加入开机自启（HKCU Run）" : "已取消开机自启");
    } catch (err) {
      setStatus(`开机自启设置失败：${String(err)}`);
    }
  };

  const addRoot = () => {
    const value = rootInput().trim();
    if (!value) return;
    const current = draft();
    if (!current) return;
    if (current.index_roots.includes(value)) {
      setRootInput("");
      return;
    }
    patch({ index_roots: [...current.index_roots, value] });
    setRootInput("");
  };

  return (
    <div class="settings-root">
      <Show when={draft()} fallback={<div class="settings-loading">读取中…</div>}>
        {(settings) => (
          <>
            <section class="settings-group">
              <h3>热键</h3>
              <div class="settings-row">
                <span>唤起面板</span>
                <button
                  class="hotkey-field"
                  classList={{ recording: recording() === "hotkey" }}
                  onMouseDown={() => setRecording("hotkey")}
                  onKeyDown={(event) => recordHotkey("hotkey", event)}
                  onBlur={() => setRecording(null)}
                  title="点击后按下组合键"
                >
                  {recording() === "hotkey"
                    ? "按下组合键…（Esc 取消）"
                    : prettyHotkey(settings().hotkey)}
                </button>
              </div>
              <div class="settings-row">
                <span>截图</span>
                <button
                  class="hotkey-field"
                  classList={{ recording: recording() === "shot_hotkey" }}
                  onMouseDown={() => setRecording("shot_hotkey")}
                  onKeyDown={(event) => recordHotkey("shot_hotkey", event)}
                  onBlur={() => setRecording(null)}
                  title="点击后按下组合键"
                >
                  {recording() === "shot_hotkey"
                    ? "按下组合键…（Esc 取消）"
                    : prettyHotkey(settings().shot_hotkey)}
                </button>
              </div>
              <p class="settings-tip">
                点一下输入框，然后直接按组合键即可记录（必须含 Ctrl / Alt / Win）；连按两次 Shift 可设为「双击 Shift」。
                截图默认 Ctrl+Alt+A，避开中文系统的 Alt+Shift 输入法切换。改完点「保存」立即重新注册。
              </p>
              <Show when={settings().hotkey === settings().shot_hotkey}>
                <p class="settings-tip warn">两个热键不能相同，否则只有一个能生效。</p>
              </Show>

              <label class="settings-row">
                <span>失焦自动收起</span>
                <input
                  type="checkbox"
                  checked={settings().autohide_on_blur}
                  onChange={(event) => patch({ autohide_on_blur: event.currentTarget.checked })}
                />
              </label>
            </section>

            <section class="settings-group">
              <h3>文件索引</h3>
              <Show when={fileStatus()}>
                {(status) => (
                  <p class="settings-tip">
                    当前：{status().entries.toLocaleString()} 条 · {backendLabel(status().backend)}
                  </p>
                )}
              </Show>
              <Show when={serviceStatus()}>
                {(svc) => (
                  <p class="settings-tip">
                    索引服务：{svc().installed ? `已安装（${svc().run_level ?? "未知"}）` : "未安装"}
                   　运行中：{svc().running ? "是" : "否"}
                  </p>
                )}
              </Show>
              <div class="settings-add">
                <button onClick={() => void reindex()}>重建索引</button>
                <button onClick={() => void installService()}>安装索引服务</button>
                <button onClick={() => void uninstallService()}>卸载服务</button>
                <button onClick={() => void refreshIndexState()}>刷新状态</button>
              </div>
              <p class="settings-tip">
                索引根目录留空则索引 `%USERPROFILE%`；改动保存后需重启程序生效。
              </p>
              <For each={settings().index_roots}>
                {(root, index) => (
                  <div class="settings-root-item">
                    <span title={root}>{root}</span>
                    <button
                      onClick={() =>
                        patch({
                          index_roots: settings().index_roots.filter((_, i) => i !== index()),
                        })
                      }
                    >
                      移除
                    </button>
                  </div>
                )}
              </For>
              <div class="settings-add">
                <input
                  value={rootInput()}
                  onInput={(event) => setRootInput(event.currentTarget.value)}
                  onKeyDown={(event) => {
                    if (event.key === "Enter") addRoot();
                  }}
                  placeholder="D:\Projects"
                />
                <button onClick={addRoot}>添加</button>
              </div>
            </section>

            <section class="settings-group">
              <h3>开机自启</h3>
              <label class="settings-row">
                <span>登录时启动 xTools</span>
                <input
                  type="checkbox"
                  checked={autostartEnabled()}
                  onChange={() => void toggleAutostart()}
                />
              </label>
              <p class="settings-tip">当前程序：{exePath()}</p>
            </section>

            <div class="settings-footer">
              <button class="primary" onClick={() => void save()}>
                保存
              </button>
              <button onClick={() => void invoke("settings_close")}>关闭</button>
              <span class="settings-status">{status()}</span>
            </div>

            <Show when={problems().length > 0}>
              <ul class="settings-problems">
                <For each={problems()}>{(item) => <li>{item}</li>}</For>
              </ul>
            </Show>
          </>
        )}
      </Show>
    </div>
  );
}
