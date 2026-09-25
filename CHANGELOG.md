# Changelog

## 0.2.0（2026-09-25）

### 日服（JP 6.8.1）

- 新增 `--region jp`（或配置里的 `cdn.region = "jp"`）。区服决定整套默认值：CDN、masterdata（`haruki-sekai-master`）、Unity 版本（2022.3.62f2），以及独立的 `cache/jp`、`out/jp` 目录。配置文件叠加在区服预设之上，命令行的 `--region` 优先。
- 日服 CDN 只认 CloudFront 签名 cookie。工具按游戏新装流程登录：版本 API → 注册游客账号（只注册一次，账号存在 `<cache>/account.json`）→ `PUT user/{id}/auth` → `POST api/signature` 取 cookie。版本号和 assetHash 取自登录响应。
- 日服只有一对 AES key，同时用于 API 和清单，仍由用户通过 `RIPPER_AB_KEY` / `RIPPER_AB_IV` 提供。
- 清单 URL 为 `…/api/version/{assetVersion}/{assetHash}/os/ios`，bundle URL 为 `{host}/{assetVersion}/{assetHash}/ios/{bundleName}`。日服清单没有 `downloadPath`，由清单库里的 `.meta.json`（记录 assetHash）在加载时补上。
- 日服 CDN 上有少数 bundle 与清单里的 `fileSize`/`crc` 不一致（内容完整，只是另一版）。游戏客户端本身不校验这两项，所以日服下载只校验 `Content-Length` 和能否解压，CRC 不符只打印提示，缓存也不按大小判定。

### 变更

- asset version 改为字符串（CN `"10"`，JP `"6.8.0.50"`），按点分数字排序；`--asset-version`、`ripper.lock.json` 和报告里的 `assetVersion` 随之变为字符串。`ripper.lock.json` 新增 `region`。

### 修复

- `ripper-convert`：Transform 的动画绑定按分量展开成多条曲线（位置 3 / 旋转 4 / 缩放 3 / 欧拉角 3），修复部分特效 prefab 的 AnimationClip 报 “N bindings for M curves” 而转换失败的问题。

### 项目

- 公开到 GitHub（[StarMoe-org/SekaiStoryRipper](https://github.com/StarMoe-org/SekaiStoryRipper)）。
- 架构决策整理为 ADR（`docs/adr/`），替代原来的规划文档。
- CI 同时提供 GitHub Actions 与 Gitea Actions 配置，四个平台原生构建；推送 `v*` tag 时自动构建并发布 release。

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

- **二进制**：本版只附带 macOS arm64。
- **HCA 解码未经独立验证**：cridecoder 的输出与 ffmpeg 相比极性相反，HCA v3 也还没有独立参考解码器验证，留待 vgmstream 裁定。
- **不在 v1 范围内的**：
  - 区域对话、角色自我介绍（计划 v1.1）；
  - MV（只在索引里留占位）；
  - ipa 里独有的资源（对话框、Telop、shader 等），永远不做。
- **影片音频**：ADX 转 WAV 依赖外部 ffmpeg；找不到 ffmpeg 时只保留 `.adx`。
- **少量游戏数据问题**：以 warning 报告，例如 `self_mizuki` 有 4 条 VoiceId 与 cue 名不一致。
- **`card:all`**：会下载 `character/member/*` 共 2.7 GB 的卡面包，因为卡面剧本就放在这些包里。

### 平台

macOS arm64、Windows x64、Linux x64/arm64（musl 静态链接）。要编译的 C 代码只有 zstd-sys 和 ring；TLS 用 rustls + webpki-roots。
