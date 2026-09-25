# ADR-0010: 跨平台构建

- **状态**：accepted
- **日期**：2026-09-23

## 背景

目标平台是 macOS arm64、Windows x64、Linux x64 和 Linux arm64。依赖里的 C 代码（zstd-sys、ring）从 macOS 交叉编译到 Windows 时会因为缺少 Windows SDK 头文件而失败。

## 决策

- 每个平台都在原生 runner 上构建和测试。CI 同时提供 GitHub Actions（`.github/workflows/`）和 Gitea Actions（`.gitea/workflows/`）。
- Linux 使用 musl 静态链接。
- 只使用纯 Rust 或能在四个平台原生编译的依赖。HTTP 使用 reqwest + rustls（ring 后端）+ 内置 webpki-roots，不引入 OpenSSL 和 aws-lc。
- 输出文件名在 Windows 上必须合法（替换保留字符、避开保留名、去掉结尾的点和空格）；只差大小写的文件名视为冲突并报错；索引里的路径一律是 `/` 分隔的相对路径。
- 不依赖符号链接、硬链接和文件权限位。外部程序（ffmpeg）按平台查找，路径可配置。

## 后果

- 正面：四个平台产出一致的结果和静态二进制。
- 负面：需要为每个平台准备原生 runner。

## 复审条件

交叉编译工具链成熟到可以替代原生 runner 时。
