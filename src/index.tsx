/* @refresh reload */
import { render } from "solid-js/web";

import App from "./App";
import Pin from "./views/Pin";
import Shot from "./views/Shot";

const root = document.getElementById("root");
if (!root) throw new Error("#root 不存在");

// 一个前端包，三种视图：主面板 / 截图覆盖层 / 贴图窗
const hash = window.location.hash;

if (hash.startsWith("#pin/")) {
  const id = Number(hash.slice("#pin/".length)) || 0;
  render(() => <Pin id={id} />, root);
} else if (hash === "#shot") {
  render(() => <Shot />, root);
} else {
  render(() => <App />, root);
}
