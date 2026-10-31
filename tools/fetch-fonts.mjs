// 下载 HUD 用到的 OFL 字体（只取拉丁字符子集的 woff2）到 hud/fonts/，并生成本地 @font-face 样式表 hud/fonts/fonts.css。
// 用法：node tools/fetch-fonts.mjs
import { mkdirSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';

const OUT = resolve('hud/fonts');
const FAMILIES = ['Michroma', 'Chakra+Petch:wght@400;600', 'JetBrains+Mono:wght@400;700'];
// Google Fonts 按 User-Agent 决定返回的格式，用较新的 Chrome UA 以拿到 woff2
const UA = 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0 Safari/537.36';

mkdirSync(OUT, { recursive: true });
const cssUrl = `https://fonts.googleapis.com/css2?${FAMILIES.map(f => 'family=' + f).join('&')}&display=swap`;
const css = await (await fetch(cssUrl, { headers: { 'User-Agent': UA } })).text();

// 每个 @font-face 块前有 /* subset */ 注释，只保留 latin
const blocks = [...css.matchAll(/\/\* ([\w-]+) \*\/\s*(@font-face \{[^}]+\})/g)].filter(m => m[1] === 'latin').map(m => m[2]);
// 可变字体的多个字重会指向同一个文件：合并成一条 @font-face，用字重范围表示
const faces = new Map();
for (const b of blocks) {
  const family = b.match(/font-family: '([^']+)'/)[1], weight = +b.match(/font-weight: (\d+)/)[1];
  const url = b.match(/url\((https:[^)]+)\)/)[1];
  const f = faces.get(url) || { family, url, weights: [] };
  f.weights.push(weight); faces.set(url, f);
}
let out = '/* OFL 授权字体，本地打包（由 tools/fetch-fonts.mjs 生成）：Michroma、Chakra Petch、JetBrains Mono */\n';
for (const f of faces.values()) {
  const lo = Math.min(...f.weights), hi = Math.max(...f.weights), w = lo === hi ? `${lo}` : `${lo} ${hi}`;
  const name = `${f.family.toLowerCase().replace(/\s+/g, '-')}-${w.replace(' ', '-')}.woff2`;
  const buf = Buffer.from(await (await fetch(f.url)).arrayBuffer());
  writeFileSync(join(OUT, name), buf);
  console.log(`${name}  ${(buf.length / 1024).toFixed(1)} KB`);
  out += `@font-face {
  font-family: '${f.family}';
  font-style: normal;
  font-weight: ${w};
  font-display: swap;
  src: url(${name}) format('woff2');
}
`;
}
writeFileSync(join(OUT, 'fonts.css'), out);
console.log(`fonts.css  ${faces.size} faces`);
