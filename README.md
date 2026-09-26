# SekaiStoryRipper

[![CI](https://github.com/StarMoe-org/SekaiStoryRipper/actions/workflows/ci.yml/badge.svg)](https://github.com/StarMoe-org/SekaiStoryRipper/actions/workflows/ci.yml)

为 **Project Sekai**（CN 服 6.4.0 / 日服 6.8.1，iOS）的剧情回放下载并解包所需资源的命令行工具。
下游是 [SekaiStoryExporter](https://github.com/StarMoe-org/SekaiStoryExporter)（sse），它把这些资源渲染成视频。

> English summary at the [end of this page](#english-summary).

## 做什么

1. 从 CN CDN 匿名下载、或以日服游客账号登录后下载 AssetBundle，完成反混淆、校验和缓存；
2. 根据 masterdata 和剧本，反推出某一话需要哪些 bundle；
3. 把 bundle 解成 sse 可以直接读取的**无损、版本化**中间格式。动作（AnimationClip）保留 StreamedClip 的原始多项式系数，**不转成 motion3**。

CN 与日服的支持程度相同：主线、活动、卡面、特别篇都可以导出。

## 快速上手

**1. 安装。** 从 [Releases](https://github.com/StarMoe-org/SekaiStoryRipper/releases) 下载对应平台的 `ripper`，或者从源码构建（Rust 版本由 `rust-toolchain.toml` 固定）：

```bash
cargo build --release -p ripper-cli     # 产物在 target/release/ripper
```

**2. 提供密钥。** 清单（日服还包括 API）的 AES key/IV 不随本工具分发，需要你从自己合法持有的客户端取得：

```bash
export RIPPER_AB_KEY=...  RIPPER_AB_IV=...   # 16 个字符，或 32 位十六进制
```

**3. 导出一话。**

```bash
ripper manifest                                        # 拉取并归档清单（游戏每次热更后运行一次）
ripper rip unit:school-refusal-story-chapter/1         # 下载并解包第 1 章第 1 话需要的全部资源

# 日服：加 --region jp。首次运行会注册一个游客账号，保存在 cache/jp/account.json 后复用
ripper --region jp manifest
ripper --region jp rip event:185/1
```

结果在 `out/`（日服 `out/jp/`）：`library/` 是解包后的资源，`episodes/` 是每一话的索引。把这个目录交给 sse 即可：

```bash
sse --library out export unit:school-refusal-story-chapter/1 -o ep1.mp4 --ui <ui-dir>
```

## 用法

最常用的是 `rip`：按剧集选择器导出一话需要的全部资源。

```bash
export RIPPER_AB_KEY=...  RIPPER_AB_IV=...   # 清单解密 key，需自行从合法持有的客户端取得（[ADR-0005](docs/adr/0005-keys-and-assets.md)）
ripper manifest                              # 拉取并归档清单（每次游戏热更后运行一次）
ripper rip unit:school-refusal-story-chapter/1 event:120 card:1 special:2
ripper rip unit:all --report rip-report.json
ripper plan all --report plan.json           # 只做规划：列出需要的 bundle 和 warning，不下载资源本体
```

日服：加 `--region jp`（或在配置里写 `[cdn] region = "jp"`）。首次运行会注册一个游客账号并存到 `cache/jp/account.json`，之后复用；key 同样通过 `RIPPER_AB_KEY` / `RIPPER_AB_IV` 提供（日服 API 和清单共用一对）。输出在 `out/jp/`。

```bash
ripper --region jp manifest
ripper --region jp masterdata
ripper --region jp rip unit:school-refusal-story-chapter/1
```

选择器：`unit:<章节 assetbundleName>[/<话>]`、`event:<eventId>[/<话 或 范围 1-4>]`、`card:<cardId>[/first|second]`、`special:<specialStoryId>[/<话>]`、`scenario:<scenarioId>`、`unit:all`、`all`。

输出：
- `out/library/<bundleName>/…`：解包后的资源，布局见下文；
- `out/episodes/<type>/<key>/<no>.json`：`ripper-episode` v1 索引，给出这一话用到的剧本、角色（模型、动作包、按游戏规则解析好的「动作名 → clip」）、背景、BGM、SE、语音、影片、特效，路径都相对 `out/library/`；
- `out/ripper.lock.json`：本次导出所用的工具版本、各格式版本（`formats`，读取方据此校验，见 ADR-0013）、app/CDN/Unity 版本和 masterdata 来源。

底层命令：

```bash
cp ripper.example.toml ripper.toml        # 按需修改；ripper.toml 已被 git 忽略
export RIPPER_AB_KEY=...  RIPPER_AB_IV=... # 清单解密 key，需自行从合法持有的客户端取得（[ADR-0005](docs/adr/0005-keys-and-assets.md)）

ripper manifest                           # 读 CDN 版本号 → 拉取并解密清单 → 归档 → 与上一版 diff
ripper manifest --diff-out diff.json      # 同时把 diff 写成 JSON
ripper manifest --from-file decrypted.json --asset-version 10   # 导入已解密的清单，无需 key

ripper fetch live2d/model/01ichika_normal sound/scenario/voice/nightcode_01_01
ripper fetch --prefix scenario/effect/     # 按前缀批量下载；默认跟随清单里的 dependencies

ripper unpack live2d/model/01ichika_normal live2d/motion/01ichika_motion_base   # 缺的先下载，再解包到 out/library/
ripper unpack --prefix scenario/unitstory/ --force                             # 忽略已有结果，重新解包
ripper unpack --keep-astc scenario/background/bg_a000001                       # 额外保留原始 ASTC（.astc，Unity 行序）
```

解包布局：`out/library/<bundleName>/<container 相对路径>`。

| 类别 | 输出 |
|---|---|
| TextAsset | 原样输出，去掉 `.bytes` |
| Texture2D | PNG（Alpha8 贴图，如字体 atlas，写成白色 + alpha） |
| AnimationClip | `.sse-motion.json` |
| ACB | 原始 `.acb`、`.cues.json`、`.tables.json`，以及 `.audio/` 下每个 waveform 一个 WAV |
| 影片 | `.m2v` 和 `.adx`，另有经 ffmpeg 转出的 `.wav` |
| Font | `.otf` / `.ttf` |
| 其他对象 | typetree JSON（带 GameObject 的 bundle 还会有 `_objects.json` 对象图和 `_textures/` 依赖贴图） |

每个 bundle 目录里的 `_ripper.json` 记录了文件清单和来源 crc。

缓存布局：清单在 `cache/manifests/<app>/ios<N>.msgpack.zst`，bundle 在 `cache/bundles/<bundleName>.<crc>`（已反混淆的 UnityFS）。
每个 bundle 下载后先校验，通过后才原子写入缓存：CN 校验长度（等于 `fileSize + 4`）和 CRC（对解压后的条目计算，与清单比对）；日服校验 `Content-Length` 和能否解压（见 [ADR-0011](docs/adr/0011-regions.md)）。

## 输出到 S3

`--out` 可以直接写成对象存储地址（AWS S3，或 MinIO、Cloudflare R2 等兼容服务）：

```bash
export AWS_ACCESS_KEY_ID=...  AWS_SECRET_ACCESS_KEY=...
export AWS_ENDPOINT_URL=https://<account>.r2.cloudflarestorage.com   # 非 AWS 时设置；AWS 用 AWS_REGION
ripper --out s3://my-bucket/sekai/cn rip unit:school-refusal-story-chapter/1
```

- 先解包到本地暂存 `<cache>/s3-out/<bucket>/<prefix>`，结束后只上传新增或变化的文件。暂存目录可以随时删除。
- 暂存为空时（例如换了一台机器），会从 S3 取回已有 bundle 的 record 和 JSON，已发布的内容不会重复解包、重复上传。
- 端点、区域、寻址方式可写在配置的 `[s3]` 段（见 `ripper.example.toml`）；凭据只从环境变量读取。设计见 [ADR-0012](docs/adr/0012-s3-output.md)。

SekaiStoryExporter 可以直接读取这个地址：`sse --library s3://my-bucket/sekai/cn …`。

## 支持平台

macOS arm64、Windows x64、Linux x64、Linux arm64（Linux 为 musl 静态链接）。每个平台都在原生 runner 上构建和测试，GitHub Actions 和 Gitea Actions 各有一份配置（[ADR-0010](docs/adr/0010-cross-platform.md)）。

## 项目结构

Rust（edition 2024）多 crate workspace：

| crate | 职责 |
|---|---|
| `ripper-cdn` | 版本号、登录（日服）、清单解密、下载、反混淆、缓存 |
| `ripper-unity` | UnityFS / SerializedFile / typetree 读取（基于 unity-rs-core，放在 trait 后面） |
| `ripper-convert` | Texture2D→PNG、AnimationClip→sse-motion、ACB→WAV、USM 解复用 |
| `ripper-resolve` | masterdata 加剧本得到 BundlePlan |
| `ripper-format` | 输出格式的 serde 类型与格式版本，供 sse 依赖 |
| `ripper-cli` | 命令行 |

`tools/oracle/library/` 是用 UnityPy 交叉验证解包结果的脚本。架构决策见 [`docs/adr/`](docs/adr/)，版本变化见 [`CHANGELOG.md`](CHANGELOG.md)。

## 法律与分发边界

- **本仓库和 release 不包含、也不分发任何游戏资产或派生资产。** 测试只使用合成数据或本地缓存，仓库里只放哈希和统计值作为期望值。
- **密钥不在代码里。** 需要用户从自己合法持有的客户端取得，通过配置文件或环境变量提供。
- 本项目与 SEGA、Colorful Palette 及 Craft Egg 没有任何关联。与游戏相关的商标和版权归各自权利人所有。本工具仅供个人研究使用，使用者须自行遵守游戏服务条款和所在地法律。

## 许可证

可以任选以下两种许可证之一：

- Apache License 2.0（[LICENSE-APACHE](LICENSE-APACHE)）
- MIT License（[LICENSE-MIT](LICENSE-MIT)）

## English summary

**SekaiStoryRipper** (`ripper`) downloads and unpacks the assets needed to replay Project Sekai
stories (CN 6.4.0 and JP 6.8.1, iOS) into a lossless, versioned intermediate format consumed by
[SekaiStoryExporter](https://github.com/StarMoe-org/SekaiStoryExporter).

- `ripper manifest` fetches and archives the asset manifest; `ripper rip <selector>` resolves an
  episode to its bundles, downloads, verifies and unpacks them, and writes an episode index.
- Both regions are supported at the same level. CN downloads anonymously; JP logs in once with a
  guest account (kept in `cache/jp/account.json`) to obtain the CDN cookie. Pass `--region jp`.
- **No keys and no game assets are shipped.** Supply the AES key/IV from a client you own via
  `RIPPER_AB_KEY` / `RIPPER_AB_IV`.
- `--out s3://bucket/prefix` publishes the output to S3 or an S3-compatible store (MinIO, R2, ...),
  uploading only what changed; credentials come from `AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY`.
- Builds natively on macOS arm64, Windows x64 and Linux x64/arm64 (musl).
- Not affiliated with SEGA, Colorful Palette or Craft Egg. Licensed under MIT OR Apache-2.0.

Design decisions are recorded in [`docs/adr/`](docs/adr/) (in Chinese). Issues and pull requests
are welcome in English or Chinese.
