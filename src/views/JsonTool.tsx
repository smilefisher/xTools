import { For, Show, createMemo, createSignal, onCleanup, onMount } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { basicSetup } from "codemirror";
import { EditorView } from "@codemirror/view";
import { json } from "@codemirror/lang-json";
import { oneDark } from "@codemirror/theme-one-dark";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";

type JsonResult = {
  ok: boolean;
  output: string;
  error: { message: string; line: number; column: number } | null;
  elapsed_ms: number;
  input_bytes: number;
  output_bytes: number;
};

type JsonTreeNode = {
  id: number;
  parent: number;
  depth: number;
  key: string;
  kind: string;
  preview: string;
  path: string;
  has_children: boolean;
};

type JsonTree = {
  ok: boolean;
  nodes: JsonTreeNode[];
  total: number;
  truncated: boolean;
  elapsed_ms: number;
  error: { message: string; line: number; column: number } | null;
};

type JsonOutlineNode = {
  depth: number;
  key: string;
  kind: string;
  start: number;
  end: number;
  bytes: number;
};

type JsonOutline = {
  ok: boolean;
  nodes: JsonOutlineNode[];
  truncated: boolean;
  root_kind: string;
  elapsed_ms: number;
  error: { message: string; line: number; column: number } | null;
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
  const [showTree, setShowTree] = createSignal(false);
  const [tree, setTree] = createSignal<JsonTree | null>(null);
  const [collapsed, setCollapsed] = createSignal<Set<number>>(new Set());
  const [outlineData, setOutlineData] = createSignal<JsonOutline | null>(null);
  const [currentPath, setCurrentPath] = createSignal<string | null>(null);
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

  const visibleNodes = createMemo(() => {
    const data = tree();
    if (!data) return [] as JsonTreeNode[];
    const byId = new Map(data.nodes.map((node) => [node.id, node]));
    const hidden = collapsed();

    const isHidden = (node: JsonTreeNode) => {
      let cursor: JsonTreeNode | undefined = node;
      while (cursor && cursor.parent >= 0) {
        const parent = byId.get(cursor.parent);
        if (!parent) return false;
        if (hidden.has(parent.id)) return true;
        cursor = parent;
      }
      return false;
    };

    return data.nodes.filter((node) => !isHidden(node));
  });

  const toggleCollapse = (id: number) => {
    const next = new Set(collapsed());
    if (next.has(id)) next.delete(id);
    else next.add(id);
    setCollapsed(next);
  };

  const toggleTree = async () => {
    if (showTree()) {
      setShowTree(false);
      return;
    }
    try {
      const result = await invoke<JsonTree>("json_tree", { text: text(), limit: 3000 });
      if (!result.ok) {
        const err = result.error;
        setError(err ? `${err.message}（第 ${err.line} 行）` : "解析失败");
        return;
      }
      setTree(result);
      setCollapsed(new Set<number>());
      setShowTree(true);
      setStatus(
        `树视图：${result.nodes.length} 个节点${result.truncated ? `（共 ${result.total}，已截断）` : ""} · ${result.elapsed_ms.toFixed(1)} ms`,
      );
    } catch (err) {
      setError(String(err));
    }
  };

  const toggleOutline = async () => {
    if (outlineData()) {
      setOutlineData(null);
      setShowTree(false);
      return;
    }
    try {
      const started = performance.now();
      const result = await invoke<JsonOutline>("json_outline", {
        text: text(),
        depth: 2,
        limit: 500,
      });
      if (!result.ok) {
        const err = result.error;
        setError(err ? `${err.message}（第 ${err.line} 行）` : "解析失败");
        return;
      }
      setOutlineData(result);
      setShowTree(false);
      setStatus(
        `大纲：${result.nodes.length} 个节点${result.truncated ? "（已截断）" : ""} · ${result.elapsed_ms.toFixed(1)} ms（端到端 ${(performance.now() - started).toFixed(0)} ms）`,
      );
    } catch (err) {
      setError(String(err));
    }
  };

  const openSlice = async (node: JsonOutlineNode) => {
    try {
      const result = await invoke<JsonResult>("json_slice", {
        text: text(),
        start: node.start,
        end: node.end,
        indent: "2",
      });
      if (!result.ok) {
        setError(result.error ? result.error.message : "片段解析失败");
        return;
      }
      replace(result.output);
      setOutlineData(null);
      setStatus(`已展开 ${node.key}（${(node.bytes / 1024).toFixed(1)} KB）`);
    } catch (err) {
      setError(String(err));
    }
  };

  const openFile = async () => {
    try {
      const picked = await openDialog({
        multiple: false,
        filters: [{ name: "JSON", extensions: ["json", "jsonc", "txt"] }],
      });
      if (!picked || Array.isArray(picked)) return;
      const file = await invoke<{ path: string; size: number; text: string }>(
        "json_read_file",
        { path: picked },
      );
      replace(file.text);
      setCurrentPath(file.path);
      setStatus(`已打开 ${file.path}（${(file.size / 1024).toFixed(1)} KB）`);
      setError("");
    } catch (err) {
      setError(`打开失败：${String(err)}`);
    }
  };

  const saveFile = async () => {
    try {
      const target = await saveDialog({
        defaultPath: currentPath() ?? "data.json",
        filters: [{ name: "JSON", extensions: ["json"] }],
      });
      if (!target) return;
      const bytes = await invoke<number>("json_write_file", {
        path: target,
        content: text(),
      });
      setCurrentPath(target);
      setStatus(`已保存 ${target}（${(bytes / 1024).toFixed(1)} KB）`);
      setError("");
    } catch (err) {
      setError(`保存失败：${String(err)}`);
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

    // 界面自测通道：跑一遍全部操作并把结果打进日志
    const probeOff = await listen("json:probe", async () => {
      const report: string[] = [];
      try {
        const formatted = await invoke<JsonResult>("json_format", { text: text() });
        report.push(`format ok=${formatted.ok} out=${JSON.stringify(formatted.output.slice(0, 40))}`);
        if (formatted.ok) replace(formatted.output);
      } catch (err) {
        report.push(`format THREW ${String(err)}`);
      }
      try {
        const minified = await invoke<JsonResult>("json_minify", { text: text() });
        report.push(`minify ok=${minified.ok} len=${minified.output.length}`);
      } catch (err) {
        report.push(`minify THREW ${String(err)}`);
      }
      try {
        const valid = await invoke<JsonResult>("json_validate", { text: text() });
        report.push(`validate ok=${valid.ok} msg=${valid.ok ? valid.output : valid.error?.message}`);
      } catch (err) {
        report.push(`validate THREW ${String(err)}`);
      }
      try {
        const queried = await invoke<JsonResult>("json_query", { text: text(), path: "$.name" });
        report.push(`query ok=${queried.ok} out=${JSON.stringify(queried.output.slice(0, 30))}`);
      } catch (err) {
        report.push(`query THREW ${String(err)}`);
      }
      try {
        const treeResult = await invoke<JsonTree>("json_tree", { text: text(), limit: 200 });
        report.push(`tree ok=${treeResult.ok} nodes=${treeResult.nodes.length}`);
        setTree(treeResult);
        setShowTree(true);
      } catch (err) {
        report.push(`tree THREW ${String(err)}`);
      }
      try {
        const outlineResult = await invoke<JsonOutline>("json_outline", {
          text: text(),
          depth: 2,
          limit: 200,
        });
        report.push(`outline ok=${outlineResult.ok} nodes=${outlineResult.nodes.length}`);
      } catch (err) {
        report.push(`outline THREW ${String(err)}`);
      }
      const editorHeight = host?.clientHeight ?? -1;
      const editorWidth = host?.clientWidth ?? -1;
      report.push(`editor box ${editorWidth}x${editorHeight}`);
      report.push(`toolbar buttons=${document.querySelectorAll(".json-toolbar button").length}`);
      void invoke("log_frontend", { message: `JSON PROBE | ${report.join(" | ")}` }).catch(() => {});
    });
    onCleanup(() => probeOff());

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
        <button onClick={() => void openFile()}>打开</button>
        <button onClick={() => void saveFile()}>保存</button>
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
        <button onClick={() => void toggleTree()}>{showTree() ? "收起树" : "树视图"}</button>
        <button onClick={() => void toggleOutline()}>
          {outlineData() ? "收起大纲" : "大纲"}
        </button>
      </div>

      <div class="json-body" classList={{ split: showTree() || outlineData() !== null }}>
        <div class="json-editor" ref={host} />

        <Show when={outlineData()}>
          <div class="json-tree">
            <For each={outlineData()!.nodes}>
              {(node) => (
                <div
                  class="tree-row"
                  style={{ "padding-left": `${8 + node.depth * 14}px` }}
                  onClick={() => void openSlice(node)}
                  title={`点开只解析这一段（${node.bytes} 字节）`}
                >
                  <span class="tree-caret"> </span>
                  <span class="tree-key">{node.key}</span>
                  <span class={`tree-kind kind-${node.kind}`}>{node.kind}</span>
                  <span class="tree-preview">{(node.bytes / 1024).toFixed(1)} KB</span>
                </div>
              )}
            </For>
          </div>
        </Show>

        <Show when={showTree() && tree()}>
          <div class="json-tree">
            <For each={visibleNodes()}>
              {(node) => (
                <div
                  class="tree-row"
                  style={{ "padding-left": `${8 + node.depth * 14}px` }}
                  onClick={() => setQuery(node.path)}
                  title={node.path}
                >
                  <span
                    class="tree-caret"
                    onClick={(event) => {
                      event.stopPropagation();
                      if (node.has_children) toggleCollapse(node.id);
                    }}
                  >
                    {node.has_children ? (collapsed().has(node.id) ? "▸" : "▾") : " "}
                  </span>
                  <span class="tree-key">{node.key || "$"}</span>
                  <span class={`tree-kind kind-${node.kind}`}>{node.kind}</span>
                  <span class="tree-preview">{node.preview}</span>
                </div>
              )}
            </For>
          </div>
        </Show>
      </div>

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
