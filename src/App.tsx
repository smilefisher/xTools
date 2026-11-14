import { For, Show, createMemo, createSignal, onMount } from "solid-js";
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

  let inputEl: HTMLInputElement | undefined;
  let toastTimer: number | undefined;
  const token = { current: 0 };

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
      case "Enter":
        event.preventDefault();
        void activate(rows()[selected()], event.ctrlKey);
        break;
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

    await refreshStatus();
    void invoke("frontend_ready").catch(() => {});
  });

  const iconFor = (hit: AppHit) =>
    icons()[hit.id] ?? "";

  const renderApp = (item: AppHit, index: number) => (
    <div
      class="row"
      classList={{ active: index === selected() }}
      onMouseEnter={() => setSelected(index)}
      onClick={() => void activate({ kind: "app", hit: item })}
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

  const renderFile = (item: FileHit, index: number) => (
    <div
      class="row"
      classList={{ active: index === selected() }}
      onMouseEnter={() => setSelected(index)}
      onClick={() => void activate({ kind: "file", hit: item })}
    >
      <span class="icon letter">{item.is_dir ? "▸" : "·"}</span>
      <span class="text">
        <span class="title">{item.name}</span>
        <span class="desc">{item.path}</span>
      </span>
      <Show when={index === selected()}>
        <span class="tag">↵</span>
      </Show>
    </div>
  );

  return (
    <div class="palette" onKeyDown={onKeyDown}>
      <div class="search">
        <span class="caret">⌘</span>
        <input
          ref={inputEl}
          autofocus
          spellcheck={false}
          placeholder="搜索应用与文件　·　f 文件名　a 应用名　j/{ JSON"
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

      <div class="list">
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
