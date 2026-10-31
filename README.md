# Telemetry HUD

English · [简体中文](README.zh-CN.md) · [日本語](README.ja.md)

A cyberpunk system-monitor wallpaper for [Wallpaper Engine](https://www.wallpaperengine.io/). It shows live data from your own PC: CPU (performance and efficiency cores), GPU, every memory module, every disk, network, motherboard slots and power draw. A tiny local helper program reads the hardware.

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

Requires Windows 10/11 (x64) and Wallpaper Engine.

1. **Helper**: download [`hud-telemetry.exe`](https://github.com/Jannchie/telemetry-hud/releases/latest/download/hud-telemetry.exe) and double-click it.
   It copies itself to `%LOCALAPPDATA%\TelemetryHUD`, starts automatically when you sign in (no administrator rights needed), and runs in the background right away. You can delete the downloaded file afterwards.
   Windows SmartScreen may warn you on first run because the exe isn't code-signed. Click **More info → Run anyway**.
2. **Wallpaper**: download `telemetry-hud-wallpaper-*.zip` from the same release and unzip it into
   `…\steamapps\common\wallpaper_engine\projects\myprojects\`. **Telemetry HUD** then appears in your installed wallpapers.

Without the helper the wallpaper still runs, but every panel shows NO SIGNAL.

### Optional: CPU temperature

Windows doesn't give regular programs the CPU temperature. To show it:

1. Download [LibreHardwareMonitor](https://github.com/LibreHardwareMonitor/LibreHardwareMonitor/releases) and run it **as administrator** (it needs admin rights to read CPU sensors).
2. Turn on **Options → Remote Web Server → Run** (port 8085).
3. To keep it running after a restart, also turn on **Options → Run On Windows Startup** and **Start Minimized**.

The helper picks it up automatically within about 15 seconds. LibreHardwareMonitor also supplies CPU package power on CPUs where Windows doesn't expose it. GPU temperature and power need an NVIDIA driver.

## Settings

- **Wallpaper Engine properties**: theme, post-processing, curvature, glow, scanlines, volume, ambient hum.
- **In-wallpaper panel**: click the button in the top-right corner. Settings are saved and take effect immediately, but the Wallpaper Engine properties override them each time the wallpaper loads.

## Uninstall

Double-click `hud-telemetry.exe` again and choose **Yes**. This stops the helper, removes the autostart entry and deletes the installed copy.

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
