# tty7-pad

[tty7](https://github.com/l0ng-ai/tty7) 手机端（`mobile/`，Tauri + iroh）的 Android 平板定制版。
协议和 Rust 客户端不动，桌面端照常配对；改的是布局、实体键盘和安装身份。

## 和手机版的差别

- **分栏**：列表（机器 / 标签 / 设置）在侧栏，终端在主区，切换 pane 不离开列表。宽度 ≥ 900dp 时侧栏常驻，更窄（竖屏）时变成抽屉。
- **恢复**：回到某台机器时，自动重新打开上次留着的 pane。
- **实体键盘**：Ctrl / Alt 组合键、F1–F12、Home / End / PgUp / PgDn 都发到终端；接上键盘后隐藏屏幕按键行，焦点直接给终端。
- **默认值**：字号 14（档位 12 / 14 / 16 / 18），宽 pane 默认缩放到完整显示。
- **弹层**：底部 sheet 改成居中对话框。
- **独立安装**：`applicationId` 是 `dev.tty7.pad`，显示名 `tty7 Pad`，可与手机版共存，需要单独配对。

短边小于 600dp 的设备仍走手机布局。

## SSH 直连

不依赖桌面 tty7，填地址就能进终端，入口在 Machines 的 SSH 分组。

- **默认连接**：`DEFAULT_SSH`（`mobile/src/main.ts`）。平板打开时自动连它，除非上次留着的是 tty7 pane。
- **主机指纹**：首次连接要确认，指纹变化时拦下并告警。
- **密码**：只留在内存里。密码登录成功后，会把平板的公钥写进服务器的 `~/.ssh/authorized_keys`，之后免密码。
- **长按连接**：编辑，或复制平板公钥。
- **断线**：不自动重连，避免重复提交错误密码，也避免悄悄开一个新 shell。

## 终端主题

Tokyo Night（平板默认）、Catppuccin Mocha、Dracula、Nord、One Dark、Gruvbox Dark、Solarized Dark，也可以跟随 App。
在设置或终端 ⋯ 菜单里切换。平板的 App 外观默认深色。

## 快捷键

| 按键 | 作用 |
|---|---|
| Ctrl+Shift+B | 显示 / 隐藏侧栏 |
| Ctrl+Shift+T | 新建标签 |
| Ctrl+Shift+F | 在当前 pane 查找 |
| Ctrl+Shift+V | 粘贴 |
| Ctrl+Shift+[ / ] | 上一个 / 下一个 pane |
| Ctrl+Shift+1…9 | 跳到列表里第 n 个 pane |

## 构建与安装

```bash
cd mobile
npm ci
export ANDROID_HOME=/opt/homebrew/share/android-commandlinetools
export NDK_HOME=$ANDROID_HOME/ndk/28.2.13676358
unset CARGO_INCREMENTAL   # 本机开了 sccache，两者不能同时用
export CARGO_PROFILE_DEV_STRIP=symbols CARGO_PROFILE_DEV_DEBUG=0   # 不去符号的 debug 包 781MB，Wi-Fi adb 传到一半会掉线
npm run tauri android build -- --debug --apk --target aarch64
adb install -r src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk
```

## 配对

桌面 tty7：Settings → Mobile → Allow phone access → Show code，然后在平板上扫码或粘贴 `tty7pair:` 代码。

## 许可证

本仓库的改动采用 [MIT](LICENSE)。上游 tty7 的代码仍是 [Apache-2.0](LICENSE-APACHE)，详见 [NOTICE](NOTICE)。
