# Changelog

## 0.1.0（2026-09-23）

首个可用版本：从 CN 6.4.0 iOS CDN 抓取剧情回放所需的全部资源，并无损解包成 SekaiStoryExporter 可以直接读取的 library。

### 命令

- `ripper manifest`：读取 CDN 版本号，拉取并解密清单（key 由用户自行提供），按 `(app, ios{N})` 归档，并与上一版比较。也可以用 `--from-file` 导入已解密的清单。
- `ripper masterdata`：按配置里的 URL 模板拉取解析器所需的 masterdata 表。
- `ripper fetch`：按名字或前缀下载 bundle，并带上它的依赖；校验长度和 CRC（对解压后的条目计算）。
- `ripper unpack`：解包到 `library/<bundleName>/`；支持增量，也支持 `--keep-astc`。
- `ripper plan <选择器…>`：把剧集解析成所需的 bundle，按类别汇总 warning，可用 `--report` 输出 JSON。
- `ripper rip <选择器…>`：完整导出剧集：下载并解包全部 bundle，写出 `episodes/…json` 索引和 `ripper.lock.json`。

### 输出格式（`ripper-format`）

- `sse-motion` v1：AnimationClip 的 StreamedClip 原始三次系数，f32 逐位无损，包含 binding 哈希和解析出的参数名，以及事件和淡入淡出时间。
- `ripper-unpack` v1：每个 bundle 的文件清单、来源 crc，以及未解析的 binding。
- `ripper-acb` v1：cue → waveform 映射，含 HCA 循环点；另附全部 UTF 表。
- `ripper-episode` v1：一话用到的全部资源，路径相对 library 根目录。

### 验证

- `plan`：主线、活动、特别篇全部 1807 话，1806 话规划成功。
- `rip`：主线全部 125 话，外加活动、特别篇、卡面抽样，共 129 话；31,979 条引用全部存在。
- UnityPy 交叉验证全部通过：动作系数 f32 逐位一致，PNG 逐像素一致，TextAsset 逐字节一致，typetree 语义一致。

### 已知限制

- **二进制**：本版只附带 macOS arm64。Windows x64 和 Linux x64/arm64 需要在 Gitea 上注册 Gitea Actions runner 后由 CI 构建（`.gitea/workflows/ci.yml`）。
- **HCA 解码未经独立验证**：cridecoder 的输出与 ffmpeg 相比极性相反，HCA v3 也还没有独立参考解码器验证（M0 报告 S6），留待 vgmstream 裁定。
- **不在 v1 范围内的**：
  - 区域对话、角色自我介绍（计划 v1.1）；
  - MV（只在索引里留占位）；
  - ipa 里独有的资源（对话框、Telop、shader 等），永远不做。
- **影片音频**：ADX 转 WAV 依赖外部 ffmpeg；找不到 ffmpeg 时只保留 `.adx`。
- **少量游戏数据问题**：以 warning 报告，例如 `self_mizuki` 有 4 条 VoiceId 与 cue 名不一致。
- **`card:all`**：会下载 `character/member/*` 共 2.7 GB 的卡面包，因为卡面剧本就放在这些包里。

### 平台

macOS arm64、Windows x64、Linux x64/arm64（musl 静态链接）。要编译的 C 代码只有 zstd-sys 和 ring；TLS 用 rustls + webpki-roots。
