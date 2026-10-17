<p align="center"><img src="assets/app-icon/icon-256.png" width="96" alt="LuminaCraft"></p>

# LuminaCraft

Rust / egui Windows 摄影工作台。照片库、非破坏性调色、人像修饰与本地导出集中在独立桌面 App 中，可按需打开完整 LightCraft 高级界面。

当前为 **0.1.0 开发预览版**，此仓库提供源代码，尚未提供可直接安装的发行包。

## 功能

- 照片库：文件夹浏览、选片、星级、标记、颜色、相册与目录迁移。
- 修图：基础调色、曲线、裁剪、人像区域与画笔、本次会话操作历史。
- 导入：在原位置添加、复制、移动，支持预览、去重与归档命名。
- 导出：JPEG / TIFF 本地成片，以及独立 `.lumcat` 照片库目录。
- 风格化：配置图片模型后使用提示词生成独立版本，密钥保存在 Windows 凭据存储。

原片和原 XMP 不回写，编辑保存在独立工程 / catalog 中；导出禁止覆盖已有文件。用户主动选择导入“移动”时，仅在目标验证及 catalog 持久保存成功后移除来源。磁盘删除单独确认并使用 Windows 回收站。

## 构建与启动

Windows x64，需要 Rust stable **GNU 工具链**（`stable-x86_64-pc-windows-gnu`）和 LLVM-MinGW；将 LLVM-MinGW 的 `bin` 加入 PATH，确保 `clang`、`windres` 可用。按本机安装路径设置 Cargo 的 GNU linker，例如：

```powershell
rustup default stable-x86_64-pc-windows-gnu
$env:CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER = 'clang'
& .\scripts\cargo.ps1 check -j 1
& .\scripts\cargo.ps1 test -j 1
& .\scripts\cargo.ps1 build -j 1
& .\scripts\build-launcher.ps1
```

双击根目录 `LuminaCraft.exe` 启动。`启动 LuminaCraft.cmd` 保留兼容，也可以运行 `target/debug/luminacraft.exe`。使用 `--advanced` 打开高级窗口。根目录的小型 exe 入口依赖构建出的主程序，不能单独复制到其他电脑。

主界面运行时读取 Windows 系统字体，不附带或分发系统字体。高级窗口的可选中日文字体按 LightCraft 的 `CRAFT_FONTS_DIR` 构建说明配置，见 `upstream/lightcraft/README.md`。

## 使用说明与边界

参见 [照片库管理](docs/主页面照片库管理.md)、[目录保存与迁移](docs/照片库目录保存与迁移.md)、[删除与移除](docs/照片删除与移除.md)、[本地导出](docs/本地导出.md)、[操作历史](docs/操作历史.md) 和 [AI 风格化](docs/AI风格化.md)。

AI 需要自行配置有效接口与模型；测试 mock 不代表真实模型效果，用户账户远程生成尚未验收。Lumina 连接 / 发送入口当前停用，保留 API 适配源码，不直接访问网站数据库。Lightroom `.lrcat` 导出为实验功能，实际 Lightroom Classic 打开尚待验证。真实模型降噪、大 RAW 性能和部分异常恢复路径仍在验证中。

## 来源与许可

LuminaCraft 自有代码使用 [MIT](LICENSE)。LightCraft / PhotoCraft 的固定来源见 [UPSTREAMS.json](UPSTREAMS.json)，整合说明见 [完整上游接入](docs/完整上游接入.md)。`vendor/` 保留固定快照，`upstream/lightcraft/` 为整合后的 owned 副本；第三方代码和素材保留各自许可与归属记录。应用素材见 [assets/ATTRIBUTION.md](assets/ATTRIBUTION.md)。
