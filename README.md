# Telemetry HUD

English · [简体中文](README.zh-CN.md) · [日本語](README.ja.md)

A cyberpunk system-monitor wallpaper. It runs on its own behind your desktop icons, or inside [Wallpaper Engine](https://www.wallpaperengine.io/). It shows live data from your own PC: CPU (performance and efficiency cores), GPU, every memory module, every disk, network, motherboard slots and power draw. A tiny local helper program reads the hardware.

![Telemetry HUD](docs/screenshot.jpg)

## Features

- **CPU / GPU**: load, clocks, temperatures and power, with P-core / E-core bars on hybrid Intel CPUs.
- **Memory**: in use, cached, compressed, committed and available, plus a card for each DIMM (capacity, speed, vendor, part number) and the empty slots.
- **Disks**: per-drive read/write, busy time and free space. NVMe, SATA SSD and HDD are drawn differently, and drive wear and power-on hours are shown when the drive reports them.
- **Motherboard**: PCIe / M.2 slots and the devices in them, including link width and speed.
- **Power**: AC or battery, power plan, CPU package power and system power, when the hardware exposes them.
- **Alerts**: saturated CPU, GPU or memory, overheating, disk bottlenecks and large uploads or downloads each get their own color and sound.
- **No fake numbers**: if a value can't be read, its panel shows **NO SIGNAL** and never a made-up value. When the data comes back, the panel plays a short mount animation.
- **Staged boot animation**, synthesized sound effects, glow / scanline / screen-curvature post-processing, and four color themes.

## Install

Requires Windows 10/11 (x64). Download [`hud-telemetry.exe`](https://github.com/Jannchie/telemetry-hud/releases/latest/download/hud-telemetry.exe) and double-click it. It asks how you want to use it:

- **Yes: standalone wallpaper**, no Wallpaper Engine needed.
- **No: helper only**, for the Wallpaper Engine wallpaper.

Either way it copies itself to `%LOCALAPPDATA%\TelemetryHUD`, starts automatically when you sign in (no administrator rights needed), and starts running right away. You can delete the downloaded file afterwards.

Windows SmartScreen may warn you on first run because the exe isn't code-signed. Click **More info → Run anyway**.

### Standalone wallpaper

The HUD appears behind your desktop icons, with one copy on each monitor. It uses the Microsoft Edge WebView2 Runtime, which comes with Windows 11 and up-to-date Windows 10.

- Rendering pauses on a monitor while a window there is maximized or full screen, for example a game, so it costs nothing while you can't see it.
- Use the tray icon to open the settings, pause the wallpaper or exit. Double-clicking the tray icon also opens the settings.
- Only the primary monitor plays sound.
- The HUD follows your mouse for the parallax effect, even though it sits behind the icons.

If you already run the wallpaper in Wallpaper Engine, turn that off first, or you'll have two copies drawing at once.

### Wallpaper Engine

Choose **No** in the install dialog to install just the helper. Then download `telemetry-hud-wallpaper-*.zip` from the same release and unzip it into
`…\steamapps\common\wallpaper_engine\projects\myprojects\`. **Telemetry HUD** then appears in your installed wallpapers.

Without the helper the Wallpaper Engine wallpaper still runs, but every panel shows NO SIGNAL.

To switch between the two modes later, run `hud-telemetry.exe --install --wallpaper` (standalone) or `hud-telemetry.exe --install --background` (helper only).

### Optional: CPU temperature

Windows doesn't give regular programs the CPU temperature. To show it:

1. Download [LibreHardwareMonitor](https://github.com/LibreHardwareMonitor/LibreHardwareMonitor/releases) and run it **as administrator** (it needs admin rights to read CPU sensors).
2. Turn on **Options → Remote Web Server → Run** (port 8085).
3. To keep it running after a restart, also turn on **Options → Run On Windows Startup** and **Start Minimized**.

The helper picks it up automatically within about 15 seconds. LibreHardwareMonitor also supplies CPU package power on CPUs where Windows doesn't expose it. GPU temperature and power need an NVIDIA driver.

## Settings

- **Wallpaper Engine properties**: theme, post-processing, curvature, glow, scanlines, volume, ambient hum.
- **In-wallpaper panel**: click the button in the top-right corner. Settings are saved and take effect immediately, but the Wallpaper Engine properties override them each time the wallpaper loads.
- **Standalone**: right-click the tray icon and choose **Settings**. The panel opens in its own window, and every change shows on the desktop right away.

## Uninstall

Double-click `hud-telemetry.exe` again and choose **Yes**. This stops the helper or the standalone wallpaper, removes the autostart entry and deletes the installed copy along with the standalone wallpaper's settings.

## Privacy

The helper listens only on `127.0.0.1:27182`, makes no network connections and sends nothing anywhere. The wallpaper reads from that local address only.

## Try it in a browser

Clone the repo and serve the `hud` folder, for example with `python -m http.server 5174 -d hud`, then open:

- `http://localhost:5174/`: live data from the helper
- `http://localhost:5174/?demo=tour`: a 64-second scripted tour of every feature, using simulated data that is clearly labeled as such

## Development

See [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md) (in Chinese).

## License

Code: [MIT](LICENSE). Bundled fonts (Michroma, Chakra Petch, JetBrains Mono): SIL Open Font License 1.1.
