# 薛定谔的化裁间

一款使用 Rust 和 Tauri 开发的 DNF 时装搭配桌面工具，提供时装浏览、角色预览、染色编辑、搭配收藏和换装补丁导出等功能。

本仓库仅包含项目源代码，不包含 `avatar_data.dll`、游戏资源、用户配置或构建产物。软件完全免费。

## 主要功能

- 浏览不同职业的时装套装、单件时装和武器装扮。
- 实时合成角色外观，支持静态预览与动画预览。
- 调整时装颜色并保存染色方案。
- 将常用搭配保存到时装橱窗，支持分组、备注和收藏。
- 设置当前穿戴与目标外观，分析差异并生成换装补丁。
- 导出动态 WebP、合成 NPK 和染色补丁。

## 运行准备

- Windows 10 或 Windows 11。
- 可用的 DNF `ImagePacks2` 目录。
- 与当前源码兼容的 `avatar_data.dll`。

开发环境下请将 `avatar_data.dll` 放到 `src-tauri\avatar_data.dll`。该文件不会纳入 Git。

## 开发与构建

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

## 工具信息

- 工具 ID：`dnf_avatar_studio`
- 分类：图像工具
- 当前版本：`1.7.0`
- 应用入口：`bin/run.cmd`
