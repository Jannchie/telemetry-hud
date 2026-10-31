# Telemetry HUD

[English](README.md) · 简体中文 · [日本語](README.ja.md)

一款赛博朋克风格的系统监控壁纸，用于 [Wallpaper Engine](https://www.wallpaperengine.io/)。显示的是你自己电脑的实时数据：CPU（区分性能核与能效核）、GPU、每一根内存条、每一块硬盘、网络、主板插槽和功耗。硬件数据由一个很小的本地 helper 程序读取。

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

需要 Windows 10/11（x64）和 Wallpaper Engine。

1. **helper**：从 [Releases](https://github.com/Jannchie/telemetry-hud/releases) 下载 `telemetry-hud-helper-*-win-x64.zip`，解压后双击 `hud-telemetry.exe`。
   程序会把自己复制到 `%LOCALAPPDATA%\TelemetryHUD`，设为登录时自动启动（不需要管理员权限），并立刻在后台运行。之后下载的那份可以删掉。
   exe 没有代码签名，第一次运行时 Windows SmartScreen 可能会拦截，点 **更多信息 → 仍要运行** 即可。
2. **壁纸**：从同一个 Release 下载 `telemetry-hud-wallpaper-*.zip`，解压到
   `…\steamapps\common\wallpaper_engine\projects\myprojects\`，之后就能在已安装的壁纸里看到 **Telemetry HUD**。

不装 helper 壁纸也能运行，只是所有面板都会显示 NO SIGNAL。

### 可选：CPU 温度

Windows 不对普通程序开放 CPU 温度。想显示的话，运行 [LibreHardwareMonitor](https://github.com/LibreHardwareMonitor/LibreHardwareMonitor)，并打开 **Options → Remote Web Server**（端口 8085），helper 会自动读取。GPU 温度和功耗需要 NVIDIA 驱动。

## 设置

- **Wallpaper Engine 属性面板**：主题、后处理、曲率、辉光、扫描线、音量、环境底噪。
- **壁纸内的控制面板**：点右上角的按钮打开。修改会立即生效并保存，但每次加载壁纸时，Wallpaper Engine 属性面板里的值会覆盖同名设置。

## 卸载

再双击一次 `hud-telemetry.exe`，选 **是**。程序会停止 helper、删除开机自启项和已安装的文件。

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
