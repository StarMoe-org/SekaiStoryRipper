# Changelog

## 0.1.0（未发布）

首个可用版本：从 CN 6.4.0 iOS CDN 抓取剧情回放所需的全部资源，并无损解包成 SekaiStoryExporter 可以直接读取的 library。

### 命令

- `ripper manifest`：读取 CDN 版本号，拉取并解密清单（key 由用户自行提供），按 `(app, ios{N})` 归档，并与上一版比较。也可以用 `--from-file` 导入已解密的清单。
- `ripper masterdata`：按配置里的 URL 模板拉取解析器所需的 masterdata 表。
- `ripper fetch`：按名字或前缀下载 bundle，并带上它的依赖；校验长度和 CRC（对解压后的条目计算）。
- `ripper unpack`：解包到 `library/<bundleName>/`；支持增量，也支持 `--keep-astc`。

### 输出格式（`ripper-format`）

- `sse-motion` v1：AnimationClip 的 StreamedClip 原始三次系数，f32 逐位无损，包含 binding 哈希和解析出的参数名，以及事件和淡入淡出时间。
- `ripper-unpack` v1：每个 bundle 的文件清单、来源 crc，以及未解析的 binding。
- `ripper-acb` v1：cue → waveform 映射，含 HCA 循环点；另附全部 UTF 表。

### 平台

macOS arm64、Windows x64、Linux x64/arm64（musl 静态链接）。要编译的 C 代码只有 zstd-sys 和 ring；TLS 用 rustls + webpki-roots。
