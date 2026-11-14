import { Show, createSignal, onCleanup, onMount } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { basicSetup } from "codemirror";
import { EditorView } from "@codemirror/view";
import { json } from "@codemirror/lang-json";
import { oneDark } from "@codemirror/theme-one-dark";

type JsonResult = {
  ok: boolean;
  output: string;
  error: { message: string; line: number; column: number } | null;
  elapsed_ms: number;
  input_bytes: number;
  output_bytes: number;
};

const SAMPLE = `{
  "name": "xTools",
  "version": "0.1.0",
  "features": ["app-search", "file-search", "screenshot", "json"],
  "performance": { "hotkeyToFirstFrameMs": 17.5, "fileQueryMs": 9.3 }
}`;

export default function JsonTool() {
  const [status, setStatus] = createSignal("就绪");
  const [error, setError] = createSignal("");
  const [meta, setMeta] = createSignal("");
  const [query, setQuery] = createSignal("$.features[0]");
  let host: HTMLDivElement | undefined;
  let view: EditorView | undefined;

  const text = () => view?.state.doc.toString() ?? "";

  const replace = (next: string) => {
    if (!view) return;
    view.dispatch({
      changes: { from: 0, to: view.state.doc.length, insert: next },
    });
  };

  const apply = (result: JsonResult, label: string) => {
    if (!result.ok) {
      const err = result.error;
      setError(err ? `${err.message}（第 ${err.line} 行，第 ${err.column} 列）` : "解析失败");
      setStatus(`${label}失败`);
      return;
    }
    setError("");
    setStatus(`${label}完成 · ${result.elapsed_ms.toFixed(1)} ms`);
    if (label !== "校验") {
      replace(result.output);
    } else {
      setMeta(result.output);
    }
  };

  const run = async (action: "format" | "minify" | "validate" | "query") => {
    const source = text();
    if (!source.trim()) {
      setError("内容为空");
      return;
    }
    try {
      const result =
        action === "query"
          ? await invoke<JsonResult>("json_query", { text: source, path: query() })
          : await invoke<JsonResult>(`json_${action}`, { text: source });
      apply(result, action === "format" ? "格式化" : action === "minify" ? "压缩" : action === "validate" ? "校验" : "查询");
      if (action === "query" && result.ok) {
        setStatus(`查询完成 · ${result.elapsed_ms.toFixed(1)} ms`);
      }
    } catch (err) {
      setError(String(err));
    }
  };

  const sortKeys = async () => {
    try {
      const result = await invoke<JsonResult>("json_format", {
        text: text(),
        indent: "2",
        sortKeys: true,
      });
      apply(result, "键排序");
    } catch (err) {
      setError(String(err));
    }
  };

  onMount(() => {
    void init();
  });
  const init = async () => {
    void invoke("log_frontend", { message: "json view mounted" }).catch(() => {});
    view = new EditorView({
      doc: SAMPLE,
      extensions: [
        basicSetup,
        json(),
        oneDark,
        EditorView.theme({
          "&": { height: "100%", fontSize: "13px", background: "transparent" },
          ".cm-scroller": { fontFamily: "Cascadia Code, Consolas, monospace" },
          ".cm-gutters": { background: "transparent", border: "none" },
        }),
        EditorView.updateListener.of((update) => {
          if (update.docChanged) setError("");
        }),
      ],
      parent: host!,
    });

    const unlisten = listen<string>("json:set", (event) => {
      replace(event.payload);
      setStatus("已载入外部内容");
    });
    onCleanup(() => unlisten.then((off) => off()));

    const pending = await invoke<string | null>("json_take_pending").catch(() => null);
    if (pending && pending.trim()) {
      replace(pending);
      setStatus("已载入外部内容");
    }

    window.addEventListener("keydown", onKeyDown);
    onCleanup(() => window.removeEventListener("keydown", onKeyDown));
  };

  const onKeyDown = (event: KeyboardEvent) => {
    if ((event.ctrlKey || event.metaKey) && event.key === "Enter") {
      event.preventDefault();
      void run("format");
    }
    if (event.key === "Escape") {
      event.preventDefault();
      void invoke("json_close");
    }
  };

  return (
    <div class="json-root">
      <div class="json-toolbar">
        <button onClick={() => void run("format")}>格式化</button>
        <button onClick={() => void run("minify")}>压缩</button>
        <button onClick={() => void sortKeys()}>键排序</button>
        <button onClick={() => void run("validate")}>校验</button>
        <input
          class="json-query"
          value={query()}
          onInput={(event) => setQuery(event.currentTarget.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter") void run("query");
          }}
          placeholder="$.path.to.value  ·  $.list[*].name"
        />
        <button onClick={() => void run("query")}>查询</button>
      </div>

      <div class="json-editor" ref={host} />

      <div class="json-status">
        <span class="json-hint">{status()}</span>
        <Show when={error()}>
          <span class="json-error">{error()}</span>
        </Show>
        <Show when={meta()}>
          <span class="json-hint">{meta()}</span>
        </Show>
        <span class="json-keys">
          <kbd>Ctrl</kbd>+<kbd>Enter</kbd> 格式化　<kbd>Ctrl</kbd>+<kbd>F</kbd> 搜索　<kbd>Esc</kbd> 关闭
        </span>
      </div>
    </div>
  );
}
