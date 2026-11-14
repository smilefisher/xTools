import { For, Show, createMemo, createSignal, onMount } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import "./App.css";

type AppHit = { id: number; name: string; target: string; score: number };

export default function App() {
  const [query, setQuery] = createSignal("");
  const [items, setItems] = createSignal<AppHit[]>([]);
  const [selected, setSelected] = createSignal(0);
  const [latency, setLatency] = createSignal<number | null>(null);
  const [toast, setToast] = createSignal("");
  const [composing, setComposing] = createSignal(false);
  const [icons, setIcons] = createSignal<Record<number, string>>({});

  let inputEl: HTMLInputElement | undefined;
  let toastTimer: number | undefined;

  const flash = (message: string) => {
    setToast(message);
    window.clearTimeout(toastTimer);
    toastTimer = window.setTimeout(() => setToast(""), 2000);
  };

  const requestId = { current: 0 };

  const search = async (text: string) => {
    const token = ++requestId.current;
    try {
      const hits = await invoke<AppHit[]>("app_search", { query: text, limit: 24 });
      if (token !== requestId.current) return; // 丢弃过期结果
      setItems(hits);
      setSelected(0);
      // 图标延后两帧再取：先让文字行画出来，避免图片解码拖慢首帧
      requestAnimationFrame(() => requestAnimationFrame(() => void loadIcons(hits)));
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

  const activate = async (hit: AppHit | undefined) => {
    if (!hit) return;
    void invoke("hide_palette");
    try {
      await invoke("app_launch", { id: hit.id });
    } catch (err) {
      flash(`启动失败：${String(err)}`);
    }
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
        setSelected((index) => Math.min(index + 1, items().length - 1));
        break;
      case "ArrowUp":
        event.preventDefault();
        setSelected((index) => Math.max(index - 1, 0));
        break;
      case "Enter":
        event.preventDefault();
        void activate(items()[selected()]);
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
    void invoke("frontend_ready").catch(() => {});
  });

  const placeholder = createMemo(() =>
    items().length === 0 && query() ? "没有匹配的应用" : "",
  );

  return (
    <div class="palette" onKeyDown={onKeyDown}>
      <div class="search">
        <span class="caret">⌘</span>
        <input
          ref={inputEl}
          autofocus
          spellcheck={false}
          placeholder="搜索应用、文件，或输入 { 编辑 JSON…"
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
        <For each={items()}>
          {(item, index) => (
            <div
              class="row"
              classList={{ active: index() === selected() }}
              onMouseEnter={() => setSelected(index())}
              onClick={() => void activate(item)}
            >
              <Show
                when={icons()[item.id]}
                fallback={<span class="icon letter">{item.name.slice(0, 1).toUpperCase()}</span>}
              >
                <img class="icon" src={icons()[item.id]} alt="" draggable={false} />
              </Show>
              <span class="text">
                <span class="title">{item.name}</span>
                <span class="desc">{item.target}</span>
              </span>
              <Show when={index() === selected()}>
                <span class="tag">↵</span>
              </Show>
            </div>
          )}
        </For>

        <Show when={placeholder()}>
          <div class="empty">{placeholder()}</div>
        </Show>
      </div>

      <div class="footer">
        <span class="keys">
          <kbd>↑</kbd>
          <kbd>↓</kbd> 选择 <kbd>↵</kbd> 打开 <kbd>Esc</kbd> 收起
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
