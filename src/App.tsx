import { For, Show, createMemo, createSignal, onMount } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

import "./App.css";

type Item = {
  id: string;
  title: string;
  desc: string;
  icon: string;
  hint?: string;
  ready: boolean;
};

const ITEMS: Item[] = [
  {
    id: "app",
    title: "应用搜索",
    desc: "开始菜单 / UWP / App Paths / 便携应用，支持中文与拼音首字母",
    icon: "▣",
    hint: "M1",
    ready: false,
  },
  {
    id: "file",
    title: "文件搜索",
    desc: "MFT + USN 索引，百万级文件毫秒级响应",
    icon: "⌕",
    hint: "M2",
    ready: false,
  },
  {
    id: "shot",
    title: "截图 / 贴图",
    desc: "冻结帧选区、标注、复制、贴到屏幕",
    icon: "⛶",
    hint: "M3",
    ready: false,
  },
  {
    id: "json",
    title: "JSON 工具",
    desc: "格式化、压缩、排序、JSONPath、大文件懒解析",
    icon: "{}",
    hint: "M4",
    ready: false,
  },
  {
    id: "settings",
    title: "设置",
    desc: "热键、开机自启、索引范围",
    icon: "⚙",
    ready: false,
  },
];

export default function App() {
  const [query, setQuery] = createSignal("");
  const [selected, setSelected] = createSignal(0);
  const [latency, setLatency] = createSignal<number | null>(null);
  const [toast, setToast] = createSignal("");
  const [composing, setComposing] = createSignal(false);

  let inputEl: HTMLInputElement | undefined;
  let toastTimer: number | undefined;

  const items = createMemo(() => {
    const q = query().trim().toLowerCase();
    if (!q) return ITEMS;
    return ITEMS.filter(
      (item) =>
        item.title.toLowerCase().includes(q) || item.desc.toLowerCase().includes(q),
    );
  });

  const flash = (message: string) => {
    setToast(message);
    window.clearTimeout(toastTimer);
    toastTimer = window.setTimeout(() => setToast(""), 2200);
  };

  onMount(async () => {
    void invoke("log_frontend", { message: "mounted" }).catch(() => {});
    await listen("palette:shown", () => {
      void invoke("log_frontend", { message: "palette:shown received" }).catch(() => {});
      setQuery("");
      setSelected(0);
      setToast("");
      inputEl?.focus();
      inputEl?.select();

      // 等到真正画出一帧再回报，测的是「可见」而不是「逻辑上已显示」
      requestAnimationFrame(() => {
        requestAnimationFrame(async () => {
          try {
            const ms = await invoke<number | null>("first_frame");
            if (ms != null) setLatency(Math.round(ms * 10) / 10);
          } catch (err) {
            void invoke("log_frontend", { message: `first_frame failed: ${String(err)}` }).catch(
              () => {},
            );
          }
        });
      });
    });
    void invoke("log_frontend", { message: "listener ready" }).catch(() => {});
    void invoke("frontend_ready").catch(() => {});
  });

  const activate = (item: Item | undefined) => {
    if (!item) return;
    if (!item.ready) {
      flash(`${item.title}：预留入口，${item.hint ?? "后续里程碑"} 接入`);
      return;
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
        activate(items()[selected()]);
        break;
    }
  };

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
            setQuery(event.currentTarget.value);
            setSelected(0);
          }}
          onCompositionStart={() => setComposing(true)}
          onCompositionEnd={() => setComposing(false)}
        />
        <Show when={query()}>
          <button
            class="clear"
            onClick={() => {
              setQuery("");
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
              onClick={() => activate(item)}
            >
              <span class="icon">{item.icon}</span>
              <span class="text">
                <span class="title">{item.title}</span>
                <span class="desc">{item.desc}</span>
              </span>
              <Show when={item.hint}>
                <span class="tag">{item.hint}</span>
              </Show>
            </div>
          )}
        </For>

        <Show when={items().length === 0}>
          <div class="empty">没有匹配项</div>
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
