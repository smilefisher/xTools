import { For, Show, createEffect, createSignal, onCleanup, onMount } from "solid-js";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

type Rect = { x: number; y: number; width: number; height: number };
type Box = { x: number; y: number; w: number; h: number };

type Op =
  | { kind: "rect"; x: number; y: number; w: number; h: number }
  | { kind: "arrow"; x1: number; y1: number; x2: number; y2: number }
  | { kind: "mosaic"; x: number; y: number; w: number; h: number }
  | { kind: "pen"; points: number[][] };

type Tool = "rect" | "arrow" | "mosaic" | "pen";

const ACCENT = "#eb4034";

export default function Shot() {
  const [rect, setRect] = createSignal<Rect | null>(null);
  const [selection, setSelection] = createSignal<Box | null>(null);
  const [dragging, setDragging] = createSignal<"select" | "draw" | null>(null);
  const [tool, setTool] = createSignal<Tool | null>(null);
  const [ops, setOps] = createSignal<Op[]>([]);
  const [draft, setDraft] = createSignal<Op | null>(null);
  const [busy, setBusy] = createSignal(false);
  const [toast, setToast] = createSignal("");
  let toastTimer: number | undefined;

  const flash = (message: string) => {
    setToast(message);
    window.clearTimeout(toastTimer);
    toastTimer = window.setTimeout(() => setToast(""), 2000);
  };

  let start = { x: 0, y: 0 };
  let canvas: HTMLCanvasElement | undefined;

  const dpr = () => window.devicePixelRatio || 1;

  onMount(async () => {
    void invoke("log_frontend", { message: "shot view mounted" }).catch(() => {});
    fitCanvas();
    window.addEventListener("resize", fitCanvas);
    const current = await invoke<Rect | null>("shot_rect").catch(() => null);
    if (current) setRect(current);

    const unlisten = await listen<Rect>("shot:begin", (event) => {
      setRect(event.payload);
      reset();
    });
    onCleanup(() => unlisten());

    window.addEventListener("keydown", onKeyDown);
    onCleanup(() => window.removeEventListener("keydown", onKeyDown));

    // 自测通道：用固定选区走一遍真实提交（复制 + 贴图）
    const probeOff = await listen("shot:probe", async () => {
      const origin = rect();
      if (!origin) {
        void invoke("log_frontend", { message: "shot:probe 无屏幕信息" }).catch(() => {});
        return;
      }
      const scale = dpr();
      setSelection({ x: 100, y: 100, w: 320, h: 200 });
      try {
        const result = await invoke<Record<string, unknown>>("shot_commit", {
          x: Math.round(origin.x + 100 * scale),
          y: Math.round(origin.y + 100 * scale),
          width: Math.round(320 * scale),
          height: Math.round(200 * scale),
          pin: true,
          ops: [],
        });
        void invoke("log_frontend", {
          message: `shot:probe 提交结果 ${JSON.stringify(result)}`,
        }).catch(() => {});
      } catch (err) {
        void invoke("log_frontend", { message: `shot:probe 失败 ${String(err)}` }).catch(() => {});
      }
    });
    onCleanup(() => probeOff());
  });

  const reset = () => {
    setSelection(null);
    setOps([]);
    setDraft(null);
    setTool(null);
    setBusy(false);
  };

  const fitCanvas = () => {
    if (!canvas) return;
    const scale = dpr();
    canvas.width = Math.floor(window.innerWidth * scale);
    canvas.height = Math.floor(window.innerHeight * scale);
    canvas.style.width = `${window.innerWidth}px`;
    canvas.style.height = `${window.innerHeight}px`;
    canvas.getContext("2d")?.setTransform(scale, 0, 0, scale, 0, 0);
  };

  const onKeyDown = (event: KeyboardEvent) => {
    if (event.key === "Escape") {
      event.preventDefault();
      void invoke("shot_cancel");
      return;
    }
    if (event.key === "Enter") {
      event.preventDefault();
      void commit(event.ctrlKey);
      return;
    }
    if (event.ctrlKey && event.key.toLowerCase() === "z") {
      event.preventDefault();
      setOps((list) => list.slice(0, -1));
    }
  };

  const insideSelection = (x: number, y: number) => {
    const sel = selection();
    if (!sel) return false;
    return x >= sel.x && y >= sel.y && x <= sel.x + sel.w && y <= sel.y + sel.h;
  };

  const onPointerDown = (event: PointerEvent) => {
    if (busy()) return;
    (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
    start = { x: event.clientX, y: event.clientY };

    const activeTool = tool();
    if (selection() && activeTool && insideSelection(start.x, start.y)) {
      setDragging("draw");
      setDraft(makeOp(activeTool, start.x, start.y, start.x, start.y));
      return;
    }

    setDragging("select");
    setOps([]);
    setDraft(null);
    setSelection({ x: start.x, y: start.y, w: 0, h: 0 });
  };

  const makeOp = (kind: Tool, x1: number, y1: number, x2: number, y2: number): Op => {
    switch (kind) {
      case "rect":
      case "mosaic":
        return {
          kind,
          x: Math.min(x1, x2),
          y: Math.min(y1, y2),
          w: Math.abs(x2 - x1),
          h: Math.abs(y2 - y1),
        } as Op;
      case "arrow":
        return { kind, x1, y1, x2, y2 };
      default:
        return { kind: "pen", points: [[x1, y1]] };
    }
  };

  const onPointerMove = (event: PointerEvent) => {
    const mode = dragging();
    if (!mode) return;

    if (mode === "select") {
      setSelection({
        x: Math.min(start.x, event.clientX),
        y: Math.min(start.y, event.clientY),
        w: Math.abs(event.clientX - start.x),
        h: Math.abs(event.clientY - start.y),
      });
      return;
    }

    const current = draft();
    if (!current) return;
    if (current.kind === "pen") {
      setDraft({ ...current, points: [...current.points, [event.clientX, event.clientY]] });
    } else {
      const fresh = makeOp(tool()!, start.x, start.y, event.clientX, event.clientY);
      setDraft(fresh);
    }
  };

  const onPointerUp = () => {
    const mode = dragging();
    setDragging(null);
    if (mode !== "draw") return;

    const current = draft();
    setDraft(null);
    if (!current) return;

    const tiny =
      current.kind === "arrow"
        ? Math.hypot(current.x2 - current.x1, current.y2 - current.y1) < 6
        : current.kind === "pen"
          ? current.points.length < 2
          : current.w < 6 && current.h < 6;
    if (!tiny) setOps((list) => [...list, current]);
  };

  // 预览：canvas 上重绘所有标注
  createEffect(() => {
    const sel = selection();
    const list = ops();
    const pending = draft();
    const box = sel;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    ctx.clearRect(0, 0, window.innerWidth, window.innerHeight);
    if (!box || box.w < 1) return;

    if (pending) drawOp(ctx, pending, true);

    for (const op of list) {
      drawOp(ctx, op, true);
    }
  });

  const drawOp = (ctx: CanvasRenderingContext2D, op: Op, preview: boolean) => {
    ctx.save();
    ctx.strokeStyle = ACCENT;
    ctx.fillStyle = ACCENT;
    ctx.lineWidth = 2;

    if (op.kind === "rect") {
      ctx.strokeRect(op.x, op.y, op.w, op.h);
    } else if (op.kind === "arrow") {
      const angle = Math.atan2(op.y2 - op.y1, op.x2 - op.x1);
      const head = 12;
      ctx.beginPath();
      ctx.moveTo(op.x1, op.y1);
      ctx.lineTo(op.x2, op.y2);
      ctx.stroke();
      ctx.beginPath();
      ctx.moveTo(op.x2, op.y2);
      ctx.lineTo(
        op.x2 - head * Math.cos(angle - 0.45),
        op.y2 - head * Math.sin(angle - 0.45),
      );
      ctx.lineTo(
        op.x2 - head * Math.cos(angle + 0.45),
        op.y2 - head * Math.sin(angle + 0.45),
      );
      ctx.closePath();
      ctx.fill();
    } else if (op.kind === "mosaic") {
      ctx.fillStyle = "rgba(235, 64, 52, 0.12)";
      ctx.fillRect(op.x, op.y, op.w, op.h);
      ctx.strokeRect(op.x, op.y, op.w, op.h);
      if (preview) {
        ctx.fillStyle = "rgba(255,255,255,0.55)";
        ctx.font = "12px Segoe UI";
        ctx.fillText("马赛克", op.x + 6, op.y + 16);
      }
    } else if (op.kind === "pen") {
      ctx.beginPath();
      op.points.forEach((point, index) => {
        if (index === 0) ctx.moveTo(point[0], point[1]);
        else ctx.lineTo(point[0], point[1]);
      });
      ctx.stroke();
    }
    ctx.restore();
  };

  const scaleOp = (op: Op, origin: { x: number; y: number }): Op => {
    const s = dpr();
    const fx = (value: number) => Math.round((value - origin.x) * s);
    const fy = (value: number) => Math.round((value - origin.y) * s);

    switch (op.kind) {
      case "rect":
      case "mosaic":
        return {
          kind: op.kind,
          x: fx(op.x),
          y: fy(op.y),
          w: Math.round(op.w * s),
          h: Math.round(op.h * s),
        } as Op;
      case "arrow":
        return {
          kind: "arrow",
          x1: fx(op.x1),
          y1: fy(op.y1),
          x2: fx(op.x2),
          y2: fy(op.y2),
        };
      default:
        return { kind: "pen", points: op.points.map((p) => [fx(p[0]), fy(p[1])]) };
    }
  };

  const commit = async (pin: boolean) => {
    const sel = selection();
    const origin = rect();
    if (!sel || !origin || sel.w < 4 || sel.h < 4 || busy()) return;

    setBusy(true);
    const scale = dpr();
    try {
      const result = await invoke<Record<string, unknown>>("shot_commit", {
        x: Math.round(origin.x + sel.x * scale),
        y: Math.round(origin.y + sel.y * scale),
        width: Math.round(sel.w * scale),
        height: Math.round(sel.h * scale),
        pin,
        ops: ops().map((op) => scaleOp(op, { x: sel.x, y: sel.y })),
      });
      if (result && result.clipboard === false) {
        flash("已保存，但剪贴板写入被拒绝");
      }
    } catch (err) {
      flash(`提交失败：${String(err)}`);
      setBusy(false);
    }
  };

  const tools: { id: Tool; label: string }[] = [
    { id: "rect", label: "矩形" },
    { id: "arrow", label: "箭头" },
    { id: "pen", label: "画笔" },
    { id: "mosaic", label: "马赛克" },
  ];

  return (
    <div
      class="shot-layer"
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
    >
      <canvas class="shot-canvas" ref={canvas} width={0} height={0} />

      <Show when={selection()}>
        {(sel) => (
          <>
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

            <Show when={sel().w > 8 && sel().h > 8 && !dragging()}>
              <div
                class="shot-toolbar"
                style={{
                  left: `${sel().x}px`,
                  top: `${Math.max(sel().y - 46, 8)}px`,
                }}
                // 关键：不阻止 pointerdown 的话，点按钮会被外层当成「重新拉选区」，
                // 选区被重置成 0×0，随后 commit 直接判「选区太小」
                onPointerDown={(event) => event.stopPropagation()}
                onPointerUp={(event) => event.stopPropagation()}
                onClick={(event) => event.stopPropagation()}
              >
                <For each={tools}>
                  {(item) => (
                    <button
                      classList={{ active: tool() === item.id }}
                      onClick={(event) => {
                        event.stopPropagation();
                        setTool(tool() === item.id ? null : item.id);
                      }}
                    >
                      {item.label}
                    </button>
                  )}
                </For>
                <button
                  onClick={(event) => {
                    event.stopPropagation();
                    setOps((list) => list.slice(0, -1));
                  }}
                  disabled={ops().length === 0}
                >
                  撤销
                </button>
                <span class="shot-divider" />
                <button
                  class="primary"
                  onClick={(event) => {
                    event.stopPropagation();
                    void commit(false);
                  }}
                >
                  复制
                </button>
                <button
                  onClick={(event) => {
                    event.stopPropagation();
                    void commit(true);
                  }}
                >
                  贴图
                </button>
              </div>
            </Show>
          </>
        )}
      </Show>

      <div class="shot-hint">
        <Show when={toast()}>
          <span class="shot-toast">{toast()}</span>
        </Show>
        <Show
          when={selection() && selection()!.w > 4}
          fallback={"拖动选择区域　·　Esc 取消"}
        >
          <Show
            when={tool()}
            fallback={"选工具后可标注　·　Enter 复制　Ctrl+Enter 贴图"}
          >
            在选区内拖动绘制　·　Ctrl+Z 撤销
          </Show>
        </Show>
      </div>
    </div>
  );
}
