/* @refresh reload */
import { render } from "solid-js/web";

import App from "./App";
import JsonTool from "./views/JsonTool";
import Pin from "./views/Pin";
import Shot from "./views/Shot";

const root = document.getElementById("root");
if (!root) throw new Error("#root 不存在");

// 一个前端包，四种视图：主面板 / 截图覆盖层 / 贴图窗 / JSON 工具
const hash = window.location.hash;

if (hash.startsWith("#pin/")) {
  const id = Number(hash.slice("#pin/".length)) || 0;
  render(() => <Pin id={id} />, root);
} else if (hash === "#shot") {
  render(() => <Shot />, root);
} else if (hash === "#json") {
  render(() => <JsonTool />, root);
} else {
  render(() => <App />, root);
}
