# Telemetry HUD

[English](README.md) · 简体中文 · [日本語](README.ja.md)

一款赛博朋克风格的系统监控壁纸，可以单独运行在桌面图标后面，也可以放进 [Wallpaper Engine](https://www.wallpaperengine.io/) 使用。显示的是你自己电脑的实时数据：CPU（区分性能核与能效核）、GPU、每一根内存条、每一块硬盘、网络、主板插槽和功耗。硬件数据由一个很小的本地 helper 程序读取。

![Telemetry HUD](docs/screenshot.jpg)

## 功能

- **CPU / GPU**：占用、频率、温度、功耗；Intel 混合架构 CPU 分别显示大核和小核。
- **内存**：使用中、已缓存、已压缩、已提交、可用；每根内存条单独一张卡片（容量、频率、厂商、型号），空槽位也会显示。
- **硬盘**：每块盘单独显示读写速度、忙碌率和剩余空间。NVMe、SATA SSD、HDD 的画法各不相同；硬盘能报告时，还会显示寿命和通电时长。
- **主板**：PCIe / M.2 插槽及上面插的设备，包括实际链路宽度和速率。
- **电源**：交流电或电池、电源计划、CPU 封装功耗、整机功耗（硬件提供时）。
- **告警**：CPU、GPU 或内存跑满，温度过高，磁盘瓶颈，大流量上传或下载，各有专属的颜色和提示音。
- **不造假数据**：读不到的数据显示 **NO SIGNAL**，绝不编一个数字填上。数据恢复时，对应面板会播放挂载动画。
- 组件依次载入的开机动画、合成音效、辉光 / 扫描线 / 屏幕曲率后处理，以及四套配色主题。

## 安装

需要 Windows 10/11（x64）。下载 [`hud-telemetry.exe`](https://github.com/Jannchie/telemetry-hud/releases/latest/download/hud-telemetry.exe)，双击运行，它会问你用哪种方式：

- **是：独立壁纸**，不需要 Wallpaper Engine。
- **否：只装 helper**，配合 Wallpaper Engine 的壁纸使用。

两种方式都会把程序复制到 `%LOCALAPPDATA%\TelemetryHUD`，设为登录时自动启动（不需要管理员权限），并立刻运行起来。之后下载的那份可以删掉。

exe 没有代码签名，第一次运行时 Windows SmartScreen 可能会拦截，点 **更多信息 → 仍要运行** 即可。

### 独立壁纸

HUD 显示在桌面图标后面，每个显示器各一份。它用的是 Microsoft Edge WebView2 Runtime，Windows 11 和保持更新的 Windows 10 都自带。

- 某个显示器上有窗口最大化或全屏（比如游戏）时，这块屏会暂停渲染，看不见的时候不占资源。
- 托盘图标可以打开设置、暂停或退出；双击托盘图标也能打开设置。
- 只有主显示器上的那份会出声。
- 虽然在图标后面，视差效果照样跟随鼠标。

如果 Wallpaper Engine 里也在放这张壁纸，先把它关掉，不然会有两份同时在画。

### Wallpaper Engine

安装对话框里选 **否**，只装 helper。再从同一个 Release 下载 `telemetry-hud-wallpaper-*.zip`，解压到
`…\steamapps\common\wallpaper_engine\projects\myprojects\`，之后就能在已安装的壁纸里看到 **Telemetry HUD**。

不装 helper，Wallpaper Engine 里的壁纸也能运行，只是所有面板都会显示 NO SIGNAL。

以后想换方式，运行 `hud-telemetry.exe --install --wallpaper`（独立壁纸）或 `hud-telemetry.exe --install --background`（只装 helper）即可。

### 可选：CPU 温度

Windows 不对普通程序开放 CPU 温度。想显示的话：

1. 下载 [LibreHardwareMonitor](https://github.com/LibreHardwareMonitor/LibreHardwareMonitor/releases)，**以管理员身份**运行（读取 CPU 传感器需要管理员权限）。
2. 打开 **Options → Remote Web Server → Run**（端口 8085）。
3. 想让它重启后也自动运行，再勾选 **Options → Run On Windows Startup** 和 **Start Minimized**。

helper 会在大约 15 秒内自动连上它。在 Windows 不提供 CPU 封装功耗的机器上，这个数据也会从 LibreHardwareMonitor 读取。GPU 温度和功耗需要 NVIDIA 驱动。

## 设置

- **Wallpaper Engine 属性面板**：主题、后处理、曲率、辉光、扫描线、音量、环境底噪。
- **壁纸内的控制面板**：点右上角的按钮打开。修改会立即生效并保存，但每次加载壁纸时，Wallpaper Engine 属性面板里的值会覆盖同名设置。
- **独立壁纸**：右键托盘图标选 **设置**，控制面板会在单独的窗口里打开，改动会立刻同步到桌面上。

## 卸载

再双击一次 `hud-telemetry.exe`，选 **是**。程序会停止 helper 或独立壁纸，删除开机自启项和已安装的文件，独立壁纸的设置也会一并删除。

## 隐私

helper 只监听 `127.0.0.1:27182`，不联网，不向外发送任何数据。壁纸也只从这个本机地址读取数据。

## 在浏览器里试用

克隆仓库后，把 `hud` 目录作为静态站点运行，例如 `python -m http.server 5174 -d hud`，然后打开：

- `http://localhost:5174/`：来自 helper 的实时数据
- `http://localhost:5174/?demo=tour`：64 秒的功能演示，使用模拟数据，并在界面上明确标注

## 开发

见 [docs/DEVELOPMENT.md](docs/DEVELOPMENT.md)。

## 许可

代码：[MIT](LICENSE)。附带的字体（Michroma、Chakra Petch、JetBrains Mono）：SIL Open Font License 1.1。
