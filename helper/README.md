# hud-telemetry

给桌面壁纸 HUD 用的本地硬件遥测服务，也可以自己把 HUD 嵌到桌面图标后面当独立壁纸。Rust 单文件 exe（约 1 MB），服务只监听 `127.0.0.1:27182`，后台每秒采样一次，`GET /stats` 直接返回最新快照。不需要管理员权限。

## 安装（双击即可）

1. 双击 `hud-telemetry.exe`，对话框里选运行方式：“是”= 独立壁纸（`--wallpaper`），“否”= 只跑数据服务（`--background`，给 Wallpaper Engine / Lively 的网页壁纸用），“取消”= 不安装。
2. 程序会把自己复制到 `%LOCALAPPDATA%\TelemetryHUD\hud-telemetry.exe`，在 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` 写入 `TelemetryHUD` = `"<安装路径>" --wallpaper`（或 `--background`）作为开机自启，然后从安装位置启动，并弹窗提示安装完成。
3. 之后下载的那份 exe 就可以删掉了。

确认服务在运行：

```powershell
curl http://127.0.0.1:27182/stats
```

## 卸载

再双击一次 `hud-telemetry.exe`（下载的那份、安装目录里的那份都可以），在弹出的对话框里选“是”。程序会：

- 结束从安装路径运行的实例（按完整路径匹配，不会影响其他位置运行的副本）。独立壁纸会先收到 `WM_CLOSE` 自行退出（移除托盘图标、撤下桌面上的窗口），3 秒内没退出才强制结束
- 删除 Run 键里的 `TelemetryHUD`
- 删除已安装的 exe 和独立壁纸的 WebView2 数据目录 `webview`（含控制面板设置）；如果正在运行的就是安装文件本身，会交给 `cmd` 延迟约 2 秒删除
- 安装目录只有空了才删除

在对话框里选“否”就保持现状；如果这时没在运行，会按已安装的方式把它启动起来。

**升级**：在命令行执行 `hud-telemetry.exe --install` 会覆盖安装：先停掉旧实例，复制新版，再按原来的运行方式重新启动。直接双击的话，已安装时只会询问是否卸载，可以先卸载再双击安装。

## 命令行

| 参数 | 作用 |
| --- | --- |
| （无参数） | 双击时的安装 / 卸载对话框 |
| `--install` | 静默安装或覆盖安装，不弹窗；结果输出到 stdout，返回码 0 表示成功，非 0 表示失败。可加 `--wallpaper` / `--background` 指定运行方式；都不加时沿用已安装的方式，新装默认 `--background` |
| `--uninstall` | 静默卸载，规则同上（没安装时也返回 0） |
| `--background` | 只运行服务，不执行任何安装逻辑。端口已被占用时直接退出（返回码 1），所以不会产生多个实例 |
| `--wallpaper` | 独立壁纸（自带服务），不执行任何安装逻辑；单实例，见下文 |

程序是 windows 子系统（运行时没有黑窗口）：

- 在 cmd 里调用时不会等待程序结束；需要返回码可以用 `start /wait hud-telemetry.exe --install`。
- 在 PowerShell 里加个管道就会等它结束并拿到输出，例如 `.\hud-telemetry.exe --install | Out-Host`，之后 `$LASTEXITCODE` 就是返回码。
- 如果 stdout 没被重定向，程序会挂到父控制台上输出。

要停止服务，可以在任务管理器里结束 `hud-telemetry.exe`，或执行 `taskkill /IM hud-telemetry.exe`。

### 测试用环境变量

| 变量 | 作用 |
| --- | --- |
| `HUD_TEST_ROOT` | 代替 `%LOCALAPPDATA%`，安装到 `<HUD_TEST_ROOT>\TelemetryHUD` |
| `HUD_TEST_RUNKEY` | 代替 Run 键路径，只接受 `HKCU\...`，例如 `HKCU\Software\TelemetryHUD-Test\Run` |
| `HUD_PORT` | 服务端口（默认 27182）；安装时启动的后台实例也会继承这个变量 |
| `HUD_QUERY` | 独立壁纸：追加到页面网址的参数，例如 `theme=glacier&fps=30` |
| `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS` | WebView2 自己的变量；设为 `--remote-debugging-port=9447` 后可以用 CDP 查看壁纸页面的状态 |

## 其他参数

| 参数 / 环境变量 | 作用 |
| --- | --- |
| `--port 27182` | 监听端口（带这个参数也会直接进入服务模式） |
| `--lhm-port 8085` | LibreHardwareMonitor Web 服务端口 |
| `--probe-disks` | 打印各卷对应的盘类型后退出（排查用） |
| `--probe-board` | 打印 board 对象和扫描耗时后退出（加 `HUD_DEBUG=1` 时额外输出插槽 BDF 和 PCI 设备树） |
| `--probe-health` | 打印各盘的 health、power，以及 3 次 EMI 功率读数后退出 |
| `HUD_NO_NVML=1` | 强制走非 NVIDIA 的兜底路径（测试用） |

> 依赖选用 windows-sys 0.61（没有 build script），即使项目放在很深的路径下，链接时也不会因为路径超过 260 字符报 `LNK1104`。

## 独立壁纸（--wallpaper）

不依赖 Wallpaper Engine，把 HUD 直接嵌到桌面图标后面（`src/desktop.rs`）。取代了之前只能由 WE 托管的 `hud-wallpaper.exe`。

- **进程**：单实例（`Local\TelemetryHUD-Desktop`），进程内采样 + HTTP 服务（`/stats`，和 `--background` 一样）。27182 被占用时（例如还有一个 `--background` 实例）自动换空闲端口，并通过 `?endpoint=` 告诉页面。
- **页面**：`hud/index.html` 和 `hud/fonts/` 编译时内嵌，经 wry 自定义协议提供（页面来源固定为 `http://hud.localhost`，localStorage 里的设置不会随端口变化）。改了页面要重新编译。WebView2 数据目录是 `%LOCALAPPDATA%\TelemetryHUD\webview`（跟随 `HUD_TEST_ROOT`）。
- **嵌入桌面**：先向 `Progman` 发 `0x052C` 让它生成 WorkerW，再按层级选父窗口：
  - Windows 11 24H2 起：图标层 `SHELLDLL_DefView` 和画系统壁纸的 `WorkerW` 都是 `Progman` 的子窗口。HUD 窗口挂到 `Progman` 下，z 序紧贴在 `SHELLDLL_DefView` 后面（在 `WorkerW` 之上）。
  - 更早的版本：挂到 `SHELLDLL_DefView` 所在顶层窗口后面的那个顶层 `WorkerW`。
- **多显示器**：每个显示器一个窗口。HUD 按 1920×1080 设计、等比适配，一个窗口铺满虚拟桌面会被拉伸并横跨屏幕边框，各屏 DPI 也可能不同；分开后还能只暂停被挡住的那块屏。只有主显示器的页面出声（`?mute=1`）。显示器布局或缩放变化后自动重建。
- **暂停**：每秒检查一次。某块屏上有最大化的窗口，或有窗口铺满整块屏（全屏应用、无边框游戏）时，暂停这块屏；`SHQueryUserNotificationState` 报告独占全屏 D3D 时全部暂停。最小化、隐身（其他虚拟桌面、后台 UWP）、鼠标穿透的覆盖层和工具窗口不算。暂停时页面停止绘制（画面停在最后一帧）并挂起音频，见页面里的 `hudPause`。
- **Explorer 重启**：收到 `TaskbarCreated` 后重新添加托盘图标、重建窗口；父窗口失效时也会自动重建。
- **托盘**：设置（双击图标也行）/ 暂停 / 退出。设置在普通窗口里打开一份带控制面板的 HUD（`?panel=1&mute=1&intro=0`），和壁纸共用 WebView2 数据目录，改动经 `storage` 事件同步到各屏。
- **鼠标视差**：嵌在桌面里的窗口收不到鼠标事件，每 33 ms 轮询一次 `GetCursorPos`，光标移动时把归一化坐标交给光标所在屏的页面（`hudPointer`）。不用低级鼠标钩子，免得钩子线程卡住时拖慢全局输入。

> 如果 `target\release\hud-telemetry.exe` 正在运行，cargo 无法覆盖它，可以先指定另一个输出目录再构建：`$env:CARGO_TARGET_DIR="tw"; cargo build --release`

## 字段来源

| 字段 | 来源 | 条件 |
| --- | --- | --- |
| `cpu.usage` / `cores[]` | PDH `Processor Information\% Processor Utility`（与任务管理器口径一致） | 总是可用 |
| `cpu.coreTypes[]` | `GetSystemCpuSetInformation` 的 EfficiencyClass | 仅混合架构（Intel 12 代及以后）；非混合时省略 |
| `cpu.freq` | PDH `% Processor Performance` × `Processor Frequency` | 总是可用 |
| `cpu.name` / `spec.pCores/eCores/threads/cacheMB/baseClock` | 注册表、`GetLogicalProcessorInformationEx`、`CallNtPowerInformation` | 启动时读一次 |
| `cpu.spec.boostClock` / `tdp` | — | 不需要驱动就拿不到，始终省略 |
| `cpu.temp` | LibreHardwareMonitor `http://127.0.0.1:8085/data.json`（CPU Package / Tctl） | 需要 LHM 正在运行，并在 Options → Remote Web Server 里开启；否则省略。LHM 不在时每 15 s 探测一次 |
| `gpu.*`（usage、vram、temp、power、clock、name、spec.driver/busWidth/boostClock/tdp） | NVML（动态加载 `nvml.dll`） | 需要 NVIDIA 驱动 |
| 无 NVML 时的 `gpu.usage` / `vramUsed` / `vramTotal` / `name` / `spec.driver` | PDH `GPU Engine`、`GPU Adapter Memory` + 显示适配器注册表 | 通用兜底；temp、power、clock 省略 |
| `gpu.spec.memType` | — | NVML 不提供，始终省略 |
| `mem.used/total/available` | `GlobalMemoryStatusEx`（used = total − available） | 单位 GiB，下同 |
| `mem.committed/commitLimit/pagedPool/nonPagedPool` | `GetPerformanceInfo` | |
| `mem.cached` | PDH `\Memory\Standby Cache {Core,Normal Priority,Reserve} Bytes` + `Modified Page List Bytes`（与任务管理器「已缓存」口径一致） | |
| `mem.compressed` | `Memory Compression` 进程的工作集，取自 `NtQuerySystemInformation` 的进程列表 | 每 5 s 刷新一次 |
| `mem.slots` / `mem.modules[]` | SMBIOS 原始表（`GetSystemFirmwareTable('RSMB')`）的 Type 16 / Type 17，也就是 Win32_PhysicalMemoryArray / Win32_PhysicalMemory 的数据来源 | 启动时读一次；`status` 在 SMBIOS 里没有对应项，WMI 里通常也是空的，所以省略 |
| `disks[]` | 固定卷：`GetDiskFreeSpaceExW`（每 10 s 刷新）；`read` / `write` 取自 PDH `LogicalDisk(盘符)`，按卷单独统计 | |
| `disks[].type` | 卷 → 物理盘号（`IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS`）→ `IOCTL_STORAGE_QUERY_PROPERTY`：BusType=NVMe 报 `NVMe`；否则看 SeekPenalty，有则 `HDD`、无则 `SSD` | 启动时和卷集合变化时探测；无法判断时省略 |
| `board.vendor/product/bios` | SMBIOS Type 2 / Type 0 | 启动时读一次 |
| `board.slots[]` | SMBIOS Type 9：插槽名、类型（PCIe / M.2 / other）、宽度、代数（插槽类型没有标明代数时为 null）、Current Usage | 很多主板的 Type 9 不完整，有什么报什么 |
| `board.slots[].device/link` | SetupAPI 枚举全部设备；Type 9 给出的 BDF 一般是根端口，就找父设备是这个 BDF 的 PCI 设备；`link` 取自 `DEVPKEY_PciDevice_CurrentLinkWidth/Speed` | 每 30 s 重扫（一次约 60 ms）。匹配到设备时 `inUse` 以实际为准记为 true |
| `board.devices[]` | 主板设备树：PCIe 端点（显卡、NVMe、网卡、无线网卡；USB / 雷电控制器只列扩展卡）、SATA 盘、芯片组集成的声卡 / 网卡 / 显卡、USB 声卡和网卡。`where` 写出位置：匹配到的插槽名，M.2 / 板载设备写根端口名（如 `M.2 · PEG60`），SATA 盘写 `SATA n`（n 是 AHCI 端口号，从 0 开始，取自 `IOCTL_SCSI_GET_ADDRESS` 的 PathId，可能和主板丝印的编号差 1）；`disk` 是这块盘上的盘符 | 同上 |
| `disks[].busy` | 100 − PDH `\LogicalDisk(盘符)\% Idle Time`，结果钳制在 0–100 | 每秒；拿不到计数器时省略 |
| `disks[].health` | `IOCTL_STORAGE_QUERY_PROPERTY` 的 protocol-specific 查询（以 0 访问权限打开，无需管理员）：NVMe 读健康日志（Log 0x02），得到 lifeUsed、spare、powerOnHours、tbw、temp；SATA 读 ATA Device Statistics（Log 0x04 第 1 / 7 页），得到 powerOnHours、tbw、lifeUsed（HDD 只有 powerOnHours） | 每 5 分钟读一次；读不到就整个省略，不推测 |
| `net` | `GetIfTable2`，汇总所有已连接的物理网卡（排除过滤驱动和虚拟网卡） | B/s |
| `power.ac/battery` | `GetSystemPowerStatus` | 每秒采样；没有电池时 battery 为 null |
| `power.plan` / `power.mode` | `PowerGetActiveScheme` + `PowerReadFriendlyName`；Windows 11 电源模式取自 `PowerGetEffectiveOverlayScheme`，从 powrprof 动态加载，只认三个已知 GUID，其他情况为 null | 每 10 s 刷新 |
| `power.cpuW` | 优先读 Windows Energy Meter Interface（EMI）的 `RAPL_Package0_PKG` 通道，用两次读数的能量差除以时间差算功率；没有时读 LibreHardwareMonitor 的 CPU Package 功率；都没有就是 null。`power.source.cpuW` 标明来源（`emi` / `lhm`） | 只用实测值 |
| `power.systemW` | 用电池供电时取 `IOCTL_BATTERY_QUERY_STATUS` 的放电功率；否则用 EMI 的 PSYS / PLATFORM 通道；都没有就是 null。`power.source.systemW` 标明来源（`battery` / `emi`） | 只用实测值，不估算也不把各部分加总 |
