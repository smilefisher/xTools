import { Show, createSignal, onMount } from "solid-js";
import { invoke } from "@tauri-apps/api/core";

export default function Pin(props: { id: number }) {
  const [src, setSrc] = createSignal("");
  const [stageZoom, setStageZoom] = createSignal(1);
  const [natural, setNatural] = createSignal<{ w: number; h: number } | null>(null);
  const [opacity, setOpacity] = createSignal(1);
  const [menu, setMenu] = createSignal<{ x: number; y: number } | null>(null);
  const [hint, setHint] = createSignal("");

  onMount(async () => {
    void invoke("log_frontend", { message: `pin view mounted id=${props.id}` }).catch(() => {});
    try {
      const buffer = await invoke<ArrayBuffer>("pin_image", { id: props.id });
      if (buffer && buffer.byteLength > 0) {
        setSrc(URL.createObjectURL(new Blob([buffer], { type: "image/png" })));
      }
    } catch {
      /* 拿不到图就保持透明 */
    }

    // 高 DPI 下浏览器会把图片先缩小再放大，边缘就发虚、出现白边。
    // 只对图片容器做 1/dpr 的布局缩放，让 1 CSS px == 1 设备像素；右键菜单不受影响。
    const dpr = window.devicePixelRatio || 1;
    setStageZoom(1 / (dpr || 1));

    window.addEventListener("keydown", (event) => {
      if (event.key === "Escape") void invoke("pin_close", { id: props.id });
      if (event.key === "+" || event.key === "=") void zoom(1.1);
      if (event.key === "-") void zoom(1 / 1.1);
    });
  });

  let flashTimer: number | undefined;
  const flash = (message: string) => {
    setHint(message);
    window.clearTimeout(flashTimer);
    flashTimer = window.setTimeout(() => setHint(""), 1500);
  };

  const zoom = async (factor: number) => {
    try {
      await invoke("pin_zoom", { id: props.id, factor });
    } catch (err) {
      flash(String(err));
    }
  };

  const onWheel = (event: WheelEvent) => {
    event.preventDefault();
    if (event.altKey) {
      const next = Math.min(1, Math.max(0.2, opacity() - Math.sign(event.deltaY) * 0.08));
      setOpacity(next);
      return;
    }
    void zoom(event.deltaY < 0 ? 1.1 : 1 / 1.1);
  };

  const copy = async () => {
    setMenu(null);
    try {
      await invoke("pin_copy", { id: props.id });
      flash("已复制");
    } catch (err) {
      flash(`复制失败：${String(err)}`);
    }
  };

  return (
    <div class="pin-root" onWheel={onWheel}>
      <div
        class="pin-stage"
        style={{ zoom: String(stageZoom()) }}
        data-tauri-drag-region
      >
        <img
          class="pin-image"
          src={src()}
          alt=""
          style={{
            opacity: opacity(),
            width: natural() ? `${natural()!.w}px` : "100%",
            height: natural() ? `${natural()!.h}px` : "100%",
          }}
          draggable={false}
          data-tauri-drag-region
          onLoad={(event) => {
            const img = event.currentTarget;
            setNatural({ w: img.naturalWidth, h: img.naturalHeight });
          }}
          onDblClick={() => void invoke("pin_close", { id: props.id })}
          onContextMenu={(event) => {
            event.preventDefault();
            setMenu({ x: event.clientX, y: event.clientY });
          }}
          onClick={() => setMenu(null)}
        />
      </div>

      <Show when={hint()}>
        <div class="pin-hint">{hint()}</div>
      </Show>

      <Show when={menu()}>
        {(position) => (
          <div
            class="pin-menu"
            style={{ left: `${position().x}px`, top: `${position().y}px` }}
            onClick={(event) => event.stopPropagation()}
          >
            <button onClick={() => void copy()}>复制</button>
            <button onClick={() => void zoom(1.25)}>放大</button>
            <button onClick={() => void zoom(0.8)}>缩小</button>
            <button onClick={() => setOpacity(opacity() > 0.9 ? 0.5 : 1)}>
              {opacity() > 0.9 ? "半透明" : "不透明"}
            </button>
            <button
              onClick={() => {
                setMenu(null);
                void invoke("pin_close", { id: props.id });
              }}
            >
              关闭
            </button>
          </div>
        )}
      </Show>
    </div>
  );
}
