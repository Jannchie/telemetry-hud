// 逐帧导出 HUD 演示视频：无界面 Edge 打开巡演页面，接管页面时钟，每帧精确推进 1/fps 秒后取画面，交给 ffmpeg 编码。
// 音效：导出期间记录每次音效触发的时刻与底噪负载曲线，画面导出后用页面自己的合成器离线渲染整段音轨，再与画面合成。
// 用法：node tools/export-video.mjs [秒数=64] [输出=video/telemetry-hud-demo.mp4] [页面地址=http://localhost:5174/?demo=tour]
// 默认按 2 倍像素比渲染、输出 3840×2160：HUD 里大量 1px 细线和 8–9px 小字，1080p 下会糊；环境变量 HUD_SCALE=1 输出 1080p
// 依赖：Microsoft Edge、ffmpeg（在 PATH 中）、页面所在的本地静态服务器。只用 Node 自带的 fetch / WebSocket，无需安装依赖。
import { spawn } from 'node:child_process';
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, join, resolve } from 'node:path';

const SECONDS = +(process.argv[2] || 64);
const OUT = resolve(process.argv[3] || 'video/telemetry-hud-demo.mp4');
// 保留页面默认的底部预留：设备树和内存条卡片会画进这块区域，设成 bottom=0 会被裁掉
const URL_ = process.argv[4] || 'http://localhost:5174/?demo=tour';
const FPS = 60, W = 1920, H = 1080, SCALE = +(process.env.HUD_SCALE || 2), PORT = 9333;
const EDGE = 'C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe';

// 在页面脚本运行前注入：用虚拟时钟替换 performance.now 与 requestAnimationFrame，__step(dt) 推进一帧
const CLOCK = `(() => {
  let t = 0; let q = [];
  performance.now = () => t;
  window.requestAnimationFrame = (cb) => { q.push(cb); return q.length; };
  window.cancelAnimationFrame = () => {};
  window.__step = (dt) => {
    t += dt;
    if (typeof tour === 'function' && typeof TOUR !== 'undefined' && TOUR) tour();
    const cbs = q; q = []; for (const cb of cbs) cb(t);
  };
})();`;
// 包装音效方法：导出时不播放，只记录 [方法名, 参数, 虚拟时间]；开机扫频音在页面里由定时器触发，这里按 0.3 秒补一条
const HOOK_SFX = `(() => {
  window.__sfxEv = [['boot', [], 0.3]]; window.__loads = []; window.__sfxOrig = {};
  for (const k of ['log', 'status', 'stage', 'alert', 'mount', 'land', 'update']) __sfxOrig[k] = sfx[k];
  for (const k of ['log', 'status', 'stage', 'alert', 'mount', 'land']) sfx[k] = (...a) => { __sfxEv.push([k, a, performance.now() / 1000]); };
  sfx.update = (l) => { const t = performance.now() / 1000, L = __loads; if (!L.length || t - L[L.length - 1][0] >= .1) L.push([t, l]); };
  return true;
})()`;
const GRAB = `(() => {
  __step(${1000 / FPS});
  const fx = window.hudFx && window.hudFx.on ? document.querySelectorAll('canvas')[1] : document.getElementById('c');
  return fx.toDataURL('image/jpeg', 0.97).slice(23);
})()`;

const sleep = (ms) => new Promise(r => setTimeout(r, ms));
const profile = mkdtempSync(join(tmpdir(), 'hud-export-'));
const edge = spawn(EDGE, ['--headless=new', `--remote-debugging-port=${PORT}`, `--user-data-dir=${profile}`,
  `--window-size=${W},${H}`, '--hide-scrollbars', '--mute-audio',
  // 临时配置里不要自动登录、同步账号或加载扩展
  '--no-first-run', '--no-default-browser-check', '--disable-extensions', '--disable-sync', '--disable-features=msImplicitSignin,msEdgeSyncConsent',
  'about:blank'], { stdio: 'ignore' });

let ws, seq = 0;
const pending = new Map();
const send = (method, params = {}) => new Promise((res, rej) => {
  const id = ++seq; pending.set(id, { res, rej }); ws.send(JSON.stringify({ id, method, params }));
});
const evaluate = async (expression) => {
  const r = await send('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true });
  if (r.exceptionDetails) throw new Error(r.exceptionDetails.text + ' ' + (r.exceptionDetails.exception?.description || ''));
  return r.result.value;
};

try {
  let target;
  for (let i = 0; i < 50 && !target; i++) {
    try { target = (await (await fetch(`http://127.0.0.1:${PORT}/json/list`)).json()).find(t => t.type === 'page'); } catch {}
    if (!target) await sleep(200);
  }
  if (!target) throw new Error('无法连接到 Edge 的调试端口');
  ws = new WebSocket(target.webSocketDebuggerUrl);
  await new Promise((res, rej) => { ws.onopen = res; ws.onerror = rej; });
  ws.onmessage = (e) => {
    const m = JSON.parse(e.data);
    if (m.id && pending.has(m.id)) { const p = pending.get(m.id); pending.delete(m.id); m.error ? p.rej(new Error(m.error.message)) : p.res(m.result); }
  };
  await send('Page.enable');
  await send('Emulation.setDeviceMetricsOverride', { width: W, height: H, deviceScaleFactor: SCALE, mobile: false });
  await send('Page.addScriptToEvaluateOnNewDocument', { source: CLOCK });
  await send('Page.navigate', { url: URL_ });
  // 等页面和网络字体就绪，避免开头几帧用后备字体
  for (let i = 0; i < 100; i++) {
    if (await evaluate(`document.readyState === 'complete' && typeof __step === 'function' && typeof draw === 'function'`)) break;
    await sleep(100);
  }
  await evaluate(`document.fonts.ready.then(() => Promise.all(['24px Michroma', '12px "Chakra Petch"', '11px "JetBrains Mono"'].map(f => document.fonts.load(f)))).then(() => true)`);

  await evaluate(HOOK_SFX);
  mkdirSync(dirname(OUT), { recursive: true });
  const work = mkdtempSync(join(tmpdir(), 'hud-av-')), vidTmp = join(work, 'video.mp4'), wavTmp = join(work, 'audio.wav');
  const ff = spawn('ffmpeg', ['-y', '-loglevel', 'error', '-f', 'image2pipe', '-framerate', String(FPS), '-c:v', 'mjpeg', '-i', '-',
    '-c:v', 'libx264', '-preset', 'slow', '-crf', '17', '-pix_fmt', 'yuv420p', vidTmp], { stdio: ['pipe', 'inherit', 'inherit'] });
  const frames = Math.round(SECONDS * FPS), t0 = Date.now();
  for (let i = 0; i < frames; i++) {
    const b64 = await evaluate(GRAB);
    if (!ff.stdin.write(Buffer.from(b64, 'base64'))) await new Promise(r => ff.stdin.once('drain', r));
    if (i % (FPS * 4) === 0) process.stdout.write(`frame ${i}/${frames}  ${((Date.now() - t0) / 1000).toFixed(0)}s\n`);
  }
  ff.stdin.end();
  await new Promise((res, rej) => ff.on('close', c => c === 0 ? res() : rej(new Error('ffmpeg exit ' + c))));

  // 离线渲染音轨（单声道 48kHz，16 位 PCM），写成 WAV 后与画面合成。
  // 回放事件前先换回原来的音效方法，否则回放会再次被记录，边遍历边追加，永远结束不了
  const SR = 48000;
  const pcm = Buffer.from(await evaluate(`Object.assign(sfx, __sfxOrig), sfx.renderOffline(__sfxEv, __loads, ${SECONDS}, ${SR}).then((ch) => {
    const i16 = new Int16Array(ch.length);
    for (let i = 0; i < ch.length; i++) i16[i] = Math.max(-1, Math.min(1, ch[i])) * 32767;
    const u8 = new Uint8Array(i16.buffer); let s = '';
    for (let i = 0; i < u8.length; i += 0x8000) s += String.fromCharCode.apply(null, u8.subarray(i, i + 0x8000));
    return btoa(s);
  })`), 'base64');
  const hdr = Buffer.alloc(44);
  hdr.write('RIFF', 0); hdr.writeUInt32LE(36 + pcm.length, 4); hdr.write('WAVE', 8); hdr.write('fmt ', 12);
  hdr.writeUInt32LE(16, 16); hdr.writeUInt16LE(1, 20); hdr.writeUInt16LE(1, 22); hdr.writeUInt32LE(SR, 24);
  hdr.writeUInt32LE(SR * 2, 28); hdr.writeUInt16LE(2, 32); hdr.writeUInt16LE(16, 34); hdr.write('data', 36); hdr.writeUInt32LE(pcm.length, 40);
  writeFileSync(wavTmp, Buffer.concat([hdr, pcm]));
  const events = await evaluate('__sfxEv.length');
  await new Promise((res, rej) => spawn('ffmpeg', ['-y', '-loglevel', 'error', '-i', vidTmp, '-i', wavTmp, '-c:v', 'copy', '-c:a', 'aac', '-b:a', '192k',
    '-shortest', '-movflags', '+faststart', OUT], { stdio: 'inherit' }).on('close', c => c === 0 ? res() : rej(new Error('ffmpeg mux exit ' + c))));
  rmSync(work, { recursive: true, force: true });
  console.log(`done: ${OUT}  (${frames} frames, ${events} sound events, ${((Date.now() - t0) / 1000).toFixed(0)}s)`);
} finally {
  try { ws && ws.close(); } catch {}
  edge.kill();
  await sleep(500);
  try { rmSync(profile, { recursive: true, force: true }); } catch {}
}
