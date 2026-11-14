import { Show, createSignal, onCleanup, onMount } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

type Rect = { x: number; y: number; width: number; height: number };
type Selection = { x: number; y: number; w: number; h: number };

export default function Shot() {
  const [rect, setRect] = createSignal<Rect | null>(null);
  const [selection, setSelection] = createSignal<Selection | null>(null);
  const [dragging, setDragging] = createSignal(false);
  const [busy, setBusy] = createSignal(false);
  let start = { x: 0, y: 0 };

  const dpr = () => window.devicePixelRatio || 1;

  onMount(async () => {
    void invoke("log_frontend", { message: "shot view mounted" }).catch(() => {});
    const current = await invoke<Rect | null>("shot_rect").catch(() => null);
    if (current) setRect(current);

    const unlisten = await listen<Rect>("shot:begin", (event) => {
      setRect(event.payload);
      setSelection(null);
      setBusy(false);
    });
    onCleanup(() => unlisten());

    window.addEventListener("keydown", onKeyDown);
    onCleanup(() => window.removeEventListener("keydown", onKeyDown));
  });

  const onKeyDown = (event: KeyboardEvent) => {
    if (event.key === "Escape") {
      event.preventDefault();
      void invoke("shot_cancel");
      return;
    }
    if (event.key === "Enter") {
      event.preventDefault();
      void commit(event.ctrlKey);
    }
  };

  const onPointerDown = (event: PointerEvent) => {
    if (busy()) return;
    start = { x: event.clientX, y: event.clientY };
    setDragging(true);
    setSelection({ x: start.x, y: start.y, w: 0, h: 0 });
    (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
  };

  const onPointerMove = (event: PointerEvent) => {
    if (!dragging()) return;
    const x = Math.min(start.x, event.clientX);
    const y = Math.min(start.y, event.clientY);
    setSelection({
      x,
      y,
      w: Math.abs(event.clientX - start.x),
      h: Math.abs(event.clientY - start.y),
    });
  };

  const onPointerUp = () => {
    setDragging(false);
  };

  const commit = async (pin: boolean) => {
    const sel = selection();
    const origin = rect();
    if (!sel || !origin || sel.w < 4 || sel.h < 4 || busy()) return;

    setBusy(true);
    const scale = dpr();
    try {
      await invoke("shot_commit", {
        x: Math.round(origin.x + sel.x * scale),
        y: Math.round(origin.y + sel.y * scale),
        width: Math.round(sel.w * scale),
        height: Math.round(sel.h * scale),
        pin,
      });
    } catch (err) {
      console.error(err);
      setBusy(false);
    }
  };

  return (
    <div
      class="shot-layer"
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
    >
      <Show when={selection()}>
        {(sel) => (
          <div
            class="shot-selection"
            style={{
              left: `${sel().x}px`,
              top: `${sel().y}px`,
              width: `${sel().w}px`,
              height: `${sel().h}px`,
            }}
          >
            <span class="shot-size">
              {Math.round(sel().w * dpr())} × {Math.round(sel().h * dpr())}
            </span>
          </div>
        )}
      </Show>

      <div class="shot-hint">
        <Show when={selection() && selection()!.w > 4} fallback={"拖动选择区域　·　Esc 取消"}>
          <kbd>Enter</kbd> 复制　<kbd>Ctrl</kbd>+<kbd>Enter</kbd> 贴图　<kbd>Esc</kbd> 取消
        </Show>
      </div>
    </div>
  );
}
