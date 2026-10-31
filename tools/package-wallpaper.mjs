// 把 hud/ 下的页面和字体复制进 Wallpaper Engine 壁纸包 wallpaper-engine/telemetry-hud/，并打成发布用的 zip。
// 用法：node tools/package-wallpaper.mjs [另一个要同步的目录，例如 WE 的 projects\myprojects\telemetry-hud]
import { execFileSync } from 'node:child_process';
import { cpSync, mkdirSync, readFileSync, rmSync } from 'node:fs';
import { join, resolve } from 'node:path';

const PKG = resolve('wallpaper-engine/telemetry-hud');
const version = readFileSync(resolve('helper/Cargo.toml'), 'utf8').match(/^version = "([^"]+)"/m)[1];

for (const dir of [PKG, ...process.argv.slice(2).map(p => resolve(p))]) {
  mkdirSync(dir, { recursive: true });
  if (dir !== PKG) for (const f of ['project.json', 'preview.jpg', 'README.md']) cpSync(join(PKG, f), join(dir, f));
  cpSync(resolve('hud/index.html'), join(dir, 'index.html'));
  rmSync(join(dir, 'fonts'), { recursive: true, force: true });
  cpSync(resolve('hud/fonts'), join(dir, 'fonts'), { recursive: true });
  console.log(`synced ${dir}`);
}

const zip = resolve(`dist/telemetry-hud-wallpaper-${version}.zip`);
mkdirSync(resolve('dist'), { recursive: true });
rmSync(zip, { force: true });
execFileSync('powershell', ['-NoProfile', '-Command', `Compress-Archive -Path '${PKG}' -DestinationPath '${zip}'`]);
console.log(`packed ${zip}`);
