// 把 hud/ 下的页面和字体复制进 Wallpaper Engine 壁纸包 wallpaper-engine/telemetry-hud/，并打成发布用的 zip。
// 用法：node tools/package-wallpaper.mjs [另一个要同步的目录，例如 WE 的 projects\myprojects\telemetry-hud]
import { execFileSync } from 'node:child_process';
import { cpSync, existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';

const PKG = resolve('wallpaper-engine/telemetry-hud');
const version = readFileSync(resolve('helper/Cargo.toml'), 'utf8').match(/^version = "([^"]+)"/m)[1];

for (const dir of [PKG, ...process.argv.slice(2).map(p => resolve(p))]) {
  mkdirSync(dir, { recursive: true });
  if (dir !== PKG) {
    // 上传创意工坊后 WE 会在目标目录的 project.json 里记下 workshopid、visibility、评级、version 等字段，覆盖时保留：
    // 丢了 workshopid 下次更新会被当成新作品发布，visibility 被仓库里的 private 覆盖则会把已公开的作品改回私密
    const dst = join(dir, 'project.json');
    const pj = JSON.parse(readFileSync(join(PKG, 'project.json'), 'utf8'));
    const keep = existsSync(dst) ? Object.entries(JSON.parse(readFileSync(dst, 'utf8').replace(/^﻿/, ''))).filter(([k]) => !(k in pj) || k === 'visibility') : [];
    writeFileSync(dst, JSON.stringify({ ...pj, ...Object.fromEntries(keep) }, null, '\t'));
    for (const f of ['preview.jpg', 'README.md']) cpSync(join(PKG, f), join(dir, f));
  }
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
