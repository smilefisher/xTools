#!/usr/bin/env node
/**
 * Server酱（方糖）推送：构建失败 / 运行异常时把问题推到手机。
 *
 * 用法：
 *   node scripts/notify.mjs "标题" "正文（支持 Markdown）"
 *   npm run notify -- "标题" "正文"
 *
 * SendKey 来源（优先级从高到低）：
 *   1. 环境变量 SERVERCHAN_KEY
 *   2. .secrets/serverchan.key（已在 .gitignore 中，绝不入库）
 */
import { existsSync, readFileSync } from "node:fs";
import { request } from "node:https";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");

function resolveKey() {
  const fromEnv = process.env.SERVERCHAN_KEY?.trim();
  if (fromEnv) return fromEnv;

  const file = join(root, ".secrets", "serverchan.key");
  if (!existsSync(file)) {
    throw new Error(
      "找不到 SendKey：请设置环境变量 SERVERCHAN_KEY，或把 key 写入 .secrets/serverchan.key",
    );
  }
  return readFileSync(file, "utf8").trim();
}

/** 用 node:https 直连（agent:false），避免 keep-alive 在退出时触发 libuv 断言 */
function post(url, body) {
  return new Promise((resolve, reject) => {
    const req = request(
      url,
      {
        method: "POST",
        agent: false,
        headers: {
          "Content-Type": "application/x-www-form-urlencoded",
          "Content-Length": Buffer.byteLength(body),
        },
      },
      (res) => {
        let text = "";
        res.setEncoding("utf8");
        res.on("data", (chunk) => (text += chunk));
        res.on("end", () => resolve({ status: res.statusCode ?? 0, text }));
      },
    );
    req.setTimeout(15000, () => req.destroy(new Error("请求超时")));
    req.on("error", reject);
    req.end(body);
  });
}

const argv = process.argv.slice(2);
if (argv.length === 0) {
  console.error('用法: node scripts/notify.mjs "标题" "正文"');
  process.exit(2);
}

const title = argv[0].slice(0, 32);
const desp = argv.slice(1).join(" ") || title;
const key = resolveKey();
if (!/^SCT[A-Za-z0-9]+$/.test(key)) {
  console.error("[notify] 警告：SendKey 不像 Server酱 Turbo 的 SCTxxx 格式，仍会尝试发送");
}

const body = new URLSearchParams({ title, desp }).toString();

let result;
try {
  result = await post(`https://sctapi.ftqq.com/${key}.send`, body);
} catch (err) {
  console.error(`[notify] 推送失败：${err.message}`);
  process.exit(1);
}

let payload;
try {
  payload = JSON.parse(result.text);
} catch {
  payload = { code: -1, raw: result.text.slice(0, 200) };
}

if (result.status >= 200 && result.status < 300 && payload?.code === 0) {
  console.log(`[notify] 已推送：${title}`);
  process.exit(0);
}

console.error(`[notify] 推送被拒绝：HTTP ${result.status} ${JSON.stringify(payload)}`);
process.exit(1);
