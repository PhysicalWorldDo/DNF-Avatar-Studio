# 薛定谔的化裁间

Rust/Tauri 桌面工具，用于从内置加密数据文件读取时装、武器装扮、礼盒套装成员和图层 img，并从游戏 `ImagePacks2` 的 NPK 中解码 IMG 帧合成预览。

本仓库只包含源代码，不包含 `src-tauri/avatar_data.dll`。需要运行数据查询、预览或资源探针时，请自行准备兼容的数据 DLL 并放入该路径；该文件不会纳入 Git。

## 默认路径

- NPK 目录：`E:\DNF游戏原始NPK`
- 内置数据：运行时从程序目录下的 `avatar_data.dll` 加载（公开源码不附带）
- NPK/IMG 解码：静态接入 `dnflib`

## 功能

- 选择 NPK 目录。
- 从内置加密数据载入礼盒套装、单件时装、武器装扮、icon img 和图层 img。
- 套装和单件列表只读取摘要数据；选中目标后才解密对应图层分片。
- 点击套装或单件后，从 NPK 解码 IMG 并按图层偏移合成预览。
- 读取 `.equ` 的 `[hide equipment]` 隐藏关系，在权威图层排序前统一解析可见性；主预览、导出和一键换装使用同一规则。
- 严格区分 `[enable dye]` 的 mask 通道授权和可见合成：mask 可作为获准资源被定位、编辑和单目标检查，但无论是否存在染色配方，都不得进入整件、部位或一键换装的普通 RGBA 合成；可见染色画面只使用经过配方修改的 base IMG。
- 支持 V4/V6 调色板时装染色，主 IMG 与同名 `_maskN.img` 可独立或相对同步调色，并可导出自动命名的独立染色补丁 NPK。
- 支持将带完整染色配方的搭配保存到时装橱窗，管理自定义栏目、名称、备注和收藏，并兼容读取旧版橱窗/分享数据。
- 支持保存带染色配方的当前穿戴与目标外观，分析同职业搭配差异并生成独立换装补丁 NPK；换装补丁的可见像素只固化目标 base IMG 的最终结果，任何 raw/edited mask 都不烘焙进载体，需要清空的当前获准 mask 入口写为透明。
- 支持动态 WebP 导出以及普通合成/原图 NPK 导出。

## 开发命令

```powershell
npm install
npm run build
cd src-tauri
cargo build --release
cd avatar-data-dll
cargo build --release
cd ..\..
npm run tauri:build
```

## 资源快速排查

准备好本地 `src-tauri\avatar_data.dll` 后，运行 `npm run probe:build` 会生成 `src-tauri\target\release\avatar_resource_probe.exe`，把该 DLL 复制到同目录并校验 SHA-256。探针直接按 DLL 索引批量查询物品并检查 NPK/IMG，不必反复搜索 `Script_export`：

```powershell
src-tauri\target\release\avatar_resource_probe.exe 415560060 415560061 415560062 415560063
src-tauri\target\release\avatar_resource_probe.exe "星辰命运长发" --game-dir E:\DNF游戏原始NPK
```

JSON 报告会列出 `variation`、实际 Palette、各 Palette 帧哈希，以及 mask 的“已授权 / 文件存在 / 默认激活”状态。工具使用有界 NPK 缓存和累计解码预算，并在进入 native IMG 打开/解码前先校验入口大小、Palette/帧头、LINK、压缩载荷和像素尺寸；单个坏资源写入该图层的 `errors` 后继续批量排查，错误 NPK 目录则直接报错。完整约束以源码实现为准。

## 工具信息

- 工具 ID：`dnf_avatar_studio`
- 分类：`图像工具`
- 版本：`1.7.0`
- 入口：`bin/run.cmd`
