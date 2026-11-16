import { For, Show, createEffect, createMemo, createSignal, onMount } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import "./App.css";

type AppHit = { id: number; name: string; target: string; score: number };
type FileHit = {
  idx: number;
  score: number;
  path: string;
  name: string;
  is_dir: boolean;
  size: number;
  mtime: number;
};
type FileStatus = { ready: boolean; entries: number };

type Row =
  | { kind: "app"; hit: AppHit }
  | { kind: "file"; hit: FileHit }
  | { kind: "json"; text: string };

type Mode = "" | "a" | "f" | "j";

/** 前缀路由：`a ` 只搜应用、`f ` 只搜文件、`j ` 进 JSON 工具，其余走混合搜索。 */
function parseQuery(raw: string): { mode: Mode; text: string } {
  const match = /^([afj])\s+(.*)$/i.exec(raw);
  if (match) {
    return { mode: match[1].toLowerCase() as Mode, text: match[2] };
  }
  const trimmed = raw.trim().toLowerCase();
  if (trimmed === "f" || trimmed === "a" || trimmed === "j") {
    return { mode: trimmed as Mode, text: "" };
  }
  return { mode: "", text: raw };
}

export default function App() {
  const [query, setQuery] = createSignal("");
  const [apps, setApps] = createSignal<AppHit[]>([]);
  const [files, setFiles] = createSignal<FileHit[]>([]);
  const [selected, setSelected] = createSignal(0);
  const [latency, setLatency] = createSignal<number | null>(null);
  const [toast, setToast] = createSignal("");
  const [composing, setComposing] = createSignal(false);
  const [icons, setIcons] = createSignal<Record<number, string>>({});
  const [fileStatus, setFileStatus] = createSignal<FileStatus>({ ready: false, entries: 0 });
  const [menu, setMenu] = createSignal<{ x: number; y: number; row: Row } | null>(null);

  let inputEl: HTMLInputElement | undefined;
  let listEl: HTMLDivElement | undefined;
  let toastTimer: number | undefined;
  const token = { current: 0 };

  // 选中项滚进可视区：列表长了以后方向键不会把选中项走出视野
  createEffect(() => {
    const index = selected();
    const row = listEl?.querySelector<HTMLElement>(`[data-row="${index}"]`);
    row?.scrollIntoView({ block: "nearest" });
  });

  const parsed = createMemo(() => parseQuery(query()));

  const jsonIntent = createMemo<string | null>(() => {
    const { mode, text } = parsed();
    if (mode === "j") return text;
    if (mode !== "") return null;
    const q = text.trim();
    if (q.startsWith("{") || q.startsWith("[")) return q;
    return null;
  });

  const rows = createMemo<Row[]>(() => [
    ...(jsonIntent() !== null ? [{ kind: "json" as const, text: jsonIntent()! }] : []),
    ...apps().map((hit) => ({ kind: "app" as const, hit })),
    ...files().map((hit) => ({ kind: "file" as const, hit })),
  ]);

  const flash = (message: string) => {
    setToast(message);
    window.clearTimeout(toastTimer);
    toastTimer = window.setTimeout(() => setToast(""), 2200);
  };

  const refreshStatus = async () => {
    try {
      setFileStatus(await invoke<FileStatus>("file_status"));
    } catch {
      /* ignore */
    }
  };

  const search = async (raw: string) => {
    const mine = ++token.current;
    const { mode, text } = parseQuery(raw);
    const wantApps = mode === "" || mode === "a";
    const wantFiles = mode === "f" || (mode === "" && text.trim().length > 0);

    try {
      const [appHits, fileHits] = await Promise.all([
        wantApps
          ? invoke<AppHit[]>("app_search", { query: text, limit: 12 })
          : Promise.resolve<AppHit[]>([]),
        wantFiles
          ? invoke<FileHit[]>("file_search", { query: text, limit: 20 })
          : Promise.resolve<FileHit[]>([]),
      ]);
      if (mine !== token.current) return;
      setApps(appHits);
      setFiles(fileHits);
      setSelected(0);
      requestAnimationFrame(() => requestAnimationFrame(() => void loadIcons(appHits)));
    } catch (err) {
      flash(`搜索失败：${String(err)}`);
    }
  };

  const loadIcons = async (hits: AppHit[]) => {
    const known = icons();
    const missing = hits.filter((hit) => !(hit.id in known));
    if (missing.length === 0) return;

    const next: Record<number, string> = { ...known };
    await Promise.all(
      missing.map(async (hit) => {
        try {
          const buffer = await invoke<ArrayBuffer>("app_icon", { id: hit.id });
          if (!buffer || buffer.byteLength === 0) return;
          next[hit.id] = URL.createObjectURL(new Blob([buffer], { type: "image/png" }));
        } catch {
          /* 没图标就用首字母兜底 */
        }
      }),
    );
    setIcons(next);
  };

  const activate = async (row: Row | undefined, reveal = false) => {
    if (!row) return;
    void invoke("hide_palette");
    try {
      if (row.kind === "json") {
        await invoke("json_open", { text: row.text || null });
      } else if (row.kind === "app") {
        await invoke("app_launch", { id: row.hit.id });
      } else if (reveal) {
        await invoke("file_reveal", { path: row.hit.path });
      } else {
        await invoke("file_open", { path: row.hit.path });
      }
    } catch (err) {
      flash(`打开失败：${String(err)}`);
    }
  };

  const move = (delta: number) => {
    const size = rows().length;
    if (size === 0) return;
    setSelected((index) => Math.min(Math.max(index + delta, 0), size - 1));
  };

  const onKeyDown = (event: KeyboardEvent) => {
    if (composing() || event.isComposing || event.keyCode === 229) return;

    const size = rows().length;

    switch (event.key) {
      case "Escape":
        event.preventDefault();
        void invoke("hide_palette");
        break;
      case "ArrowDown":
        event.preventDefault();
        move(1);
        break;
      case "ArrowUp":
        event.preventDefault();
        move(-1);
        break;
      case "PageDown":
        event.preventDefault();
        setSelected((index) => Math.min(index + 8, Math.max(size - 1, 0)));
        break;
      case "PageUp":
        event.preventDefault();
        setSelected((index) => Math.max(index - 8, 0));
        break;
      case "Home":
        event.preventDefault();
        setSelected(0);
        break;
      case "End":
        event.preventDefault();
        setSelected(Math.max(size - 1, 0));
        break;
      case "Enter":
        event.preventDefault();
        void activate(rows()[selected()], event.ctrlKey);
        break;
      default:
        if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "c") {
          event.preventDefault();
          void copySelection();
        }
    }
  };

  const copySelection = async () => {
    const row = rows()[selected()];
    if (!row) return;
    await copyRow(row);
  };

  const copyRow = async (row: Row) => {
    const text =
      row.kind === "app" ? row.hit.target : row.kind === "file" ? row.hit.path : row.text;
    try {
      await invoke("copy_text", { text });
      flash("已复制");
    } catch (err) {
      flash(`复制失败：${String(err)}`);
    }
  };

  const runMenuAction = async (row: Row, action: string) => {
    setMenu(null);
    try {
      if (row.kind === "app") {
        if (action === "open") await invoke("app_launch", { id: row.hit.id, admin: false });
        if (action === "admin") await invoke("app_launch", { id: row.hit.id, admin: true });
        if (action === "copy") await copyRow(row);
        if (action === "open" || action === "admin") void invoke("hide_palette");
      } else if (row.kind === "file") {
        if (action === "open") await invoke("file_open", { path: row.hit.path });
        if (action === "reveal") await invoke("file_reveal", { path: row.hit.path });
        if (action === "copy") await copyRow(row);
        if (action === "open" || action === "reveal") void invoke("hide_palette");
      } else if (action === "open") {
        await invoke("json_open", { text: row.text || null });
        void invoke("hide_palette");
      }
    } catch (err) {
      flash(`操作失败：${String(err)}`);
    }
  };

  onMount(async () => {
    await listen("palette:shown", () => {
      setQuery("");
      setSelected(0);
      setToast("");
      inputEl?.focus();
      void search("");
      void refreshStatus();

      requestAnimationFrame(() => {
        requestAnimationFrame(async () => {
          try {
            const ms = await invoke<number | null>("first_frame");
            if (ms != null) setLatency(Math.round(ms * 10) / 10);
          } catch {
            /* 打点失败不影响使用 */
          }
        });
      });
    });

    await listen<string>("ui:query", async (event) => {
      const text = event.payload;
      setQuery(text);
      await search(text);
      void invoke("log_frontend", {
        message: `ui query=${JSON.stringify(text)} apps=${apps().length} files=${files().length} first=${rows()[0]?.kind ?? "none"}`,
      }).catch(() => {});
    });

    // 复现「从面板打开 JSON 工具」的完整路径（含先隐藏面板）
    await listen<string>("ui:json", async (event) => {
      void invoke("log_frontend", { message: "ui:json 收到，开始打开 JSON 工具" }).catch(() => {});
      await activate({ kind: "json", text: event.payload });
      void invoke("log_frontend", { message: "ui:json 已调用 json_open" }).catch(() => {});
    });

    await refreshStatus();
    void invoke("frontend_ready").catch(() => {});
  });

  const iconFor = (hit: AppHit) =>
    icons()[hit.id] ?? "";

  const renderApp = (item: AppHit, index: number) => (
    <div
      class="row"
      data-row={index}
      classList={{ active: index === selected() }}
      onMouseEnter={() => setSelected(index)}
      onClick={() => void activate({ kind: "app", hit: item })}
      onContextMenu={(event) => {
        event.preventDefault();
        setSelected(index);
        setMenu({ x: event.clientX, y: event.clientY, row: { kind: "app", hit: item } });
      }}
    >
      <Show
        when={iconFor(item)}
        fallback={<span class="icon letter">{item.name.slice(0, 1).toUpperCase()}</span>}
      >
        <img class="icon" src={iconFor(item)} alt="" draggable={false} />
      </Show>
      <span class="text">
        <span class="title">{item.name}</span>
        <span class="desc">{item.target}</span>
      </span>
      <Show when={index === selected()}>
        <span class="tag">↵</span>
      </Show>
    </div>
  );

  const formatSize = (bytes: number, isDir: boolean) => {
    if (isDir) return "";
    if (!bytes) return "";
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    if (bytes < 1024 * 1024 * 1024) return `${(bytes / 1048576).toFixed(1)} MB`;
    return `${(bytes / 1073741824).toFixed(1)} GB`;
  };

  const formatDate = (seconds: number) => {
    if (!seconds) return "";
    const date = new Date(seconds * 1000);
    const now = new Date();
    const sameYear = date.getFullYear() === now.getFullYear();
    const pad = (n: number) => String(n).padStart(2, "0");
    return sameYear
      ? `${pad(date.getMonth() + 1)}-${pad(date.getDate())}`
      : `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
  };

  const renderFile = (item: FileHit, index: number) => (
    <div
      class="row"
      data-row={index}
      classList={{ active: index === selected() }}
      onMouseEnter={() => setSelected(index)}
      onClick={() => void activate({ kind: "file", hit: item })}
      onContextMenu={(event) => {
        event.preventDefault();
        setSelected(index);
        setMenu({ x: event.clientX, y: event.clientY, row: { kind: "file", hit: item } });
      }}
    >
      <span class="icon letter" classList={{ folder: item.is_dir }}>
        {item.is_dir ? "▸" : "·"}
      </span>
      <span class="text">
        <span class="title">
          {item.name}
          <span class="meta">
            {item.is_dir ? "文件夹" : formatSize(item.size, false)}
            <Show when={formatDate(item.mtime)}>
              <Show when={item.is_dir || formatSize(item.size, false)}> · </Show>
              {formatDate(item.mtime)}
            </Show>
          </span>
        </span>
        <span class="desc">{item.path}</span>
      </span>
      <Show when={index === selected()}>
        <span class="tag">↵</span>
      </Show>
    </div>
  );

  return (
    <div class="palette" onKeyDown={onKeyDown} onClick={() => setMenu(null)}>
      <Show when={menu()}>
        {(state) => (
          <div
            class="ctx-menu"
            style={{
              left: `${Math.min(state().x, 520)}px`,
              top: `${Math.min(state().y, 320)}px`,
            }}
            onClick={(event) => event.stopPropagation()}
          >
            <Show when={state().row.kind === "app"}>
              <button onClick={() => void runMenuAction(state().row, "open")}>打开</button>
              <button onClick={() => void runMenuAction(state().row, "admin")}>
                以管理员身份运行
              </button>
              <button onClick={() => void runMenuAction(state().row, "copy")}>复制目标路径</button>
            </Show>
            <Show when={state().row.kind === "file"}>
              <button onClick={() => void runMenuAction(state().row, "open")}>打开</button>
              <button onClick={() => void runMenuAction(state().row, "reveal")}>
                打开所在文件夹
              </button>
              <button onClick={() => void runMenuAction(state().row, "copy")}>复制路径</button>
            </Show>
            <Show when={state().row.kind === "json"}>
              <button onClick={() => void runMenuAction(state().row, "open")}>
                在 JSON 工具中打开
              </button>
            </Show>
          </div>
        )}
      </Show>
      <div class="search">
        <span class="caret">⌘</span>
        <input
          ref={inputEl}
          autofocus
          spellcheck={false}
          placeholder="搜索应用与文件　·　f 文件名　a 应用名　is:dir 只看文件夹"
          value={query()}
          onInput={(event) => {
            const text = event.currentTarget.value;
            setQuery(text);
            void search(text);
          }}
          onCompositionStart={() => setComposing(true)}
          onCompositionEnd={(event) => {
            setComposing(false);
            void search(event.currentTarget.value);
          }}
        />
        <Show when={query()}>
          <button
            class="clear"
            onClick={() => {
              setQuery("");
              void search("");
              inputEl?.focus();
            }}
          >
            ✕
          </button>
        </Show>
      </div>

      <div class="list" ref={listEl}>
        <Show when={jsonIntent() !== null}>
          <div
            class="row"
            classList={{ active: selected() === 0 }}
            onMouseEnter={() => setSelected(0)}
            onClick={() => void activate({ kind: "json", text: jsonIntent() ?? "" })}
          >
            <span class="icon letter">{"{}"}</span>
            <span class="text">
              <span class="title">在 JSON 工具中打开</span>
              <span class="desc">
                {jsonIntent()?.trim() ? "把这段内容带进编辑器" : "打开空编辑器"}
              </span>
            </span>
            <Show when={selected() === 0}>
              <span class="tag">↵</span>
            </Show>
          </div>
        </Show>

        <For each={apps()}>
          {(item, index) => renderApp(item, index() + (jsonIntent() !== null ? 1 : 0))}
        </For>

        <Show when={files().length > 0}>
          <div class="section">
            文件
            <span class="section-count">
              {fileStatus().entries > 0
                ? `索引 ${fileStatus().entries.toLocaleString()} 条`
                : ""}
            </span>
          </div>
          <For each={files()}>
            {(item, index) =>
              renderFile(
                item,
                (jsonIntent() !== null ? 1 : 0) + apps().length + index(),
              )
            }
          </For>
        </Show>

        <Show when={rows().length === 0 && query()}>
          <div class="empty">没有匹配项</div>
        </Show>
      </div>

      <div class="footer">
        <span class="keys">
          <kbd>↑</kbd>
          <kbd>↓</kbd> 选择 <kbd>↵</kbd> 打开 <kbd>Ctrl</kbd>+<kbd>↵</kbd> 定位{" "}
          <kbd>Esc</kbd> 收起
        </span>
        <span class="status">
          <Show when={toast()}>
            <span class="toast">{toast()}</span>
          </Show>
          <Show when={latency() != null}>
            <span class="latency" title="热键 → 首帧">
              {latency()} ms
            </span>
          </Show>
          <kbd>Alt</kbd>
          <kbd>Space</kbd>
        </span>
      </div>
    </div>
  );
}
