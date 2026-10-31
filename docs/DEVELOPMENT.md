# 开发说明

## 目录

| 路径 | 内容 |
| --- | --- |
| `hud/index.html` | 整个 HUD：单文件 Canvas2D 绘制 + WebGL2 后处理 + WebAudio 合成音效 |
| `hud/fonts/` | 本地打包的 OFL 字体，由 `tools/fetch-fonts.mjs` 生成 |
| `helper/` | Rust 写的 `hud-telemetry.exe`：硬件遥测服务，以及不依赖 Wallpaper Engine 的独立壁纸（`--wallpaper`，把 `hud/` 编译进 exe），见 [helper/README.md](../helper/README.md) |
| `wallpaper-engine/telemetry-hud/` | Wallpaper Engine 网页壁纸包：`project.json`（属性面板）、`preview.jpg`、包内 README；`index.html` 和 `fonts/` 由打包脚本复制 |
| `tools/` | 开发脚本：字体下载、壁纸打包、演示视频导出 |

## 本地预览

```bash
python -m http.server 5174 -d hud
```

网址参数：

| 参数 | 作用 |
| --- | --- |
| `demo=tour` | 64 秒脚本化巡演，展示所有功能（模拟数据，界面上会标注） |
| `demo=1` | 随机模拟数据 |
| `intro=0` | 跳过开屏动画 |
| `endpoint=` | helper 地址，默认 `http://127.0.0.1:27182/stats` |
| `theme=` | `ember` / `glacier` / `phosphor` / `classic` |
| `fx=0` | 关闭后处理 |
| `curve=` `glow=` `scan=` `sfx=` | 后处理强度与音量（0–1.5） |
| `parallax=` | 鼠标视差强度（0–1，0 关闭，默认 0.5） |
| `hum=0` | 关闭环境底噪 |
| `fps=` | 帧率上限，0 为跟随显示器 |
| `bottom=` | 屏幕底部预留高度（设计坐标，默认 80） |
| `panel=1` | 载入时打开控制面板（独立壁纸的设置窗口用） |
| `mute=1` | 本页面不出声，不影响保存的音量（独立壁纸的副屏和设置窗口用） |

数据原则：正式模式下只显示 helper 实测到的值，读不到的一律显示 NO SIGNAL，不推测、不填充模拟数据。

## 打包壁纸

```bash
node tools/package-wallpaper.mjs "E:\SteamLibrary\steamapps\common\wallpaper_engine\projects\myprojects\telemetry-hud"
```

把 `hud/index.html` 和 `hud/fonts/` 复制进壁纸包（参数里的目录也会一起同步），并生成 `dist/telemetry-hud-wallpaper-<版本>.zip`。

## 构建 helper

```bash
cd helper
cargo build --release
```

产物是 `helper/target/release/hud-telemetry.exe`，Release 里直接发布这个 exe（不打包），用户说明见根目录 README。

## 发布

改 `helper/Cargo.toml` 的版本号并提交，然后推送同名的 annotated tag，更新说明写在 tag 消息里（每个 `-m` 一段）：

```bash
git tag -a v0.1.2 -m "v0.1.2" -m "- First change." -m "- Second change."
git push origin main v0.1.2
```

`.github/workflows/release.yml` 会检查 tag 和版本号是否一致，构建 helper、打包壁纸 zip、生成 `SHA256SUMS.txt`，并创建 GitHub Release。

## 导出演示视频

```bash
node tools/export-video.mjs 64 video/telemetry-hud-demo.mp4
```

需要 Microsoft Edge、PATH 里的 ffmpeg，以及正在运行的本地预览服务器。脚本用无头 Edge 接管页面时钟，逐帧截图，并用页面自己的合成器离线渲染音轨，最后合成 1080p60 带音效的 mp4。

## 字体

```bash
node tools/fetch-fonts.mjs
```

从 Google Fonts 下载 Michroma、Chakra Petch、JetBrains Mono 的拉丁字符子集，写入 `hud/fonts/`。三者均为 SIL Open Font License 1.1。
