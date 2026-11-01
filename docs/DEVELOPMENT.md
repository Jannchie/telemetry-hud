# 开发说明

## 目录

| 路径 | 内容 |
| --- | --- |
| `hud/index.html` | 整个 HUD：单文件 Canvas2D 绘制 + WebGL2 后处理 + WebAudio 合成音效 |
| `hud/player.html` | 音乐播放器 HUD：同一套配色、字体与后处理的单文件页面，见下文「音乐播放器」（暂不进入发布流程） |
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
| `theme=` | `ember` / `glacier` / `phosphor` / `classic` / `amber` / `synthwave` / `sakura` / `cobalt` / `mono` |
| `fx=0` | 关闭后处理 |
| `curve=` `glow=` `scan=` `sfx=` | 后处理强度与音量（0–1.5） |
| `parallax=` | 鼠标视差强度（0–1，0 关闭，默认 0.5） |
| `hum=0` | 关闭环境底噪 |
| `fps=` | 帧率上限，0 为跟随显示器 |
| `bottom=` | 屏幕底部预留高度（设计坐标，默认 80） |
| `panel=1` | 载入时打开控制面板（独立壁纸的设置窗口用） |
| `mute=1` | 本页面不出声，不影响保存的音量（独立壁纸的副屏和设置窗口用） |

数据原则：正式模式下只显示 helper 实测到的值，读不到的一律显示 NO SIGNAL，不推测、不填充模拟数据。

## 音乐播放器

`hud/player.html` 是独立的音乐播放器页面，用同一个预览服务器打开：

- `http://localhost:5174/player.html`：拖入音频文件或文件夹（或按 O / 点 LOAD FILES 选择），同名 `.lrc` 自动匹配
- `http://localhost:5174/player.html?demo=1`：用 Web Audio 离线合成三首示例曲（写成带 INFO 标签的 WAV，走和本地文件相同的解析与播放流程）和内置示例歌词

构图是侧视的飞翼：圆环在前缘（进度环、当前曲目、播放控制、音量，环外上半圈是左右声道频谱），上下蒙皮向右收拢到后缘，左声道示波器沿上蒙皮、右声道沿下蒙皮流出（背后是 6 秒 RMS 历史）；翼梁上挂歌词（时间戳 + 打字机效果，点击行可跳转），播放列表与文件规格从下蒙皮垂下。

- 音频图：`<audio>` → `MediaElementSource` → 音量 → 输出；另一路先上混成立体声，再经 `ChannelSplitterNode` 进左右两个 `AnalyserNode`（接在音量之前，显示的是文件本身的信号）
- 元数据：页面内的轻量解析器读取 ID3v1 / ID3v2（含 USLT）、FLAC 与 Ogg 的 Vorbis comment、MP4 `ilst`、RIFF INFO，以及 MP3 帧头 / Xing / VBRI、FLAC STREAMINFO、WAV `fmt`、MP4 `mvhd` 得到的采样率、声道与码率（VBR / 平均码率按实际音频字节算）。格式、大小、MIME 取浏览器给出的文件信息
- 读不到的字段显示 NO SIGNAL；没有标题标签时依次退回 LRC 的 `[ti:]`、文件名，并在标题上方注明来源。歌词优先同名 `.lrc`，其次是文件内嵌、带时间戳的歌词
- 快捷键：空格播放 / 暂停，← → 快进 5 秒，B / N 切歌，↑ ↓ 音量，O 打开文件，T 切换主题，F 开关后处理，P 控制面板
- 参数：`theme=` `fx=0` `curve=` `glow=` `scan=` `fps=` `bottom=` 与遥测 HUD 相同；设置单独存在 `hud-player-settings-v1`，不影响遥测 HUD

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
