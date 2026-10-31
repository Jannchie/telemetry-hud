# Telemetry HUD · Wallpaper Engine 壁纸

赛博风格的系统监控 HUD：CPU（区分大小核）、GPU、内存条与内存构成、磁盘（NVMe / SSD / HDD）、网络。

## 安装

1. 把整个 `telemetry-hud` 文件夹复制到 Wallpaper Engine 的本地项目目录：

   ```
   E:\SteamLibrary\steamapps\common\wallpaper_engine\projects\myprojects\
   ```

2. 打开 Wallpaper Engine，在已安装的壁纸列表里就能看到 **Telemetry HUD**。

## 真实硬件数据

壁纸本身只负责显示，数据来自本地 helper 程序 `hud-telemetry.exe`（与本文件夹分开放置，见 `helper/README.md`）。

- helper 运行时，顶部状态行显示 `● LIVE`。
- helper 没运行或某项数据读不到时，对应区域显示 NO SIGNAL，不会用模拟数据填充；helper 启动后各组件会播放挂载动画接入。
- Wallpaper Engine 不能替壁纸启动外部程序。双击 `hud-telemetry.exe` 会把它安装到 `%LOCALAPPDATA%\TelemetryHUD` 并设为开机自启，再次双击可卸载。

## 调整

**Wallpaper Engine 属性面板**：主题、后处理开关、曲率、辉光、扫描线、音量、环境底噪。

**壁纸内控制面板**：右上角的小按钮或按 `P` 打开，`Esc` 关闭；`T` 轮换主题，`F` 开关后处理。
需要先在 Wallpaper Engine 设置里允许壁纸接收鼠标和键盘输入。
控制面板的修改会保存在本地，但 Wallpaper Engine 每次加载壁纸时会用属性面板的值覆盖同名设置。

**帧率**：跟随 Wallpaper Engine 的帧率设置。

## 字体

Michroma、Chakra Petch、JetBrains Mono，均为 SIL Open Font License，已打包在 `fonts/` 里，离线可用，可随壁纸分发。

## 更新

本文件夹里的 `index.html` 和 `fonts/` 是 `hud/` 下同名文件的副本。修改源文件后重新复制过来即可。
