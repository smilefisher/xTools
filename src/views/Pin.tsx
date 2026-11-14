import { createSignal, onMount } from "solid-js";
import { invoke } from "@tauri-apps/api/core";

export default function Pin(props: { id: number }) {
  const [src, setSrc] = createSignal("");

  onMount(async () => {
    void invoke("log_frontend", { message: `pin view mounted id=${props.id}` }).catch(() => {});
    try {
      const buffer = await invoke<ArrayBuffer>("pin_image", { id: props.id });
      if (buffer && buffer.byteLength > 0) {
        setSrc(URL.createObjectURL(new Blob([buffer], { type: "image/png" })));
      }
    } catch {
      /* 贴图窗口拿不到图就保持透明 */
    }
  });

  return (
    <div class="pin-root">
      <img
        class="pin-image"
        src={src()}
        alt=""
        draggable={false}
        data-tauri-drag-region
        onDblClick={() => void invoke("pin_close", { id: props.id })}
      />
    </div>
  );
}
