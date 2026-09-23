# SekaiStoryRipper

为 **Project Sekai（CN 服 6.4.0，Unity 2022.3.62f3，iOS）** 的剧情回放抓取并解包所需资产的独立工具。
下游消费者是 [SekaiStoryExporter](https://github.com/StarMoe-org/SekaiStoryExporter)（sse）。

> 状态：**v0.1.0**：可以按剧集导出剧情回放所需的全部资源（主线、活动、卡面、特别篇）。M0 结论见 [`docs/spike/M0-report.md`](docs/spike/M0-report.md)。方案与全部已拍板决策见 [`docs/plan.md`](docs/plan.md)。

## 做什么

1. 从 CN CDN 匿名拉取 AssetBundle，完成反混淆、校验和缓存；
2. 根据 masterdata 和剧本，反推出某一话需要哪些 bundle；
3. 把 bundle 解成 sse 可以直接消费的**无损、版本化**中间格式。动作（AnimationClip）保留 StreamedClip 的原始多项式系数，**不转成 motion3**。

## 用法

最常用的是 `rip`：按剧集选择器导出一话需要的全部资源。

```bash
export RIPPER_AB_KEY=...  RIPPER_AB_IV=...   # 清单解密 key，需自行从合法持有的客户端取得（D4）
ripper manifest                              # 拉取并归档清单（每次游戏热更后运行一次）
ripper rip unit:school-refusal-story-chapter/1 event:120 card:1 special:2
ripper rip unit:all --report rip-report.json
ripper plan all --report plan.json           # 只做规划：列出需要的 bundle 和 warning，不下载资源本体
```

选择器：`unit:<章节 assetbundleName>[/<话>]`、`event:<eventId>[/<话 或 范围 1-4>]`、`card:<cardId>[/first|second]`、`special:<specialStoryId>[/<话>]`、`scenario:<scenarioId>`、`unit:all`、`all`。

输出：
- `out/library/<bundleName>/…`：解包后的资源，布局见下文；
- `out/episodes/<type>/<key>/<no>.json`：`ripper-episode` v1 索引，给出这一话用到的剧本、角色（模型、动作包、按游戏规则解析好的「动作名 → clip」）、背景、BGM、SE、语音、影片、特效，路径都相对 `out/library/`；
- `out/ripper.lock.json`：本次导出所用的工具版本、各格式版本、app/CDN/Unity 版本和 masterdata 来源。

底层命令：

```bash
cp ripper.example.toml ripper.toml        # 按需修改；ripper.toml 已被 git 忽略
export RIPPER_AB_KEY=...  RIPPER_AB_IV=... # 清单解密 key，需自行从合法持有的客户端取得（D4）

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
| 其他对象 | typetree JSON（带 GameObject 的 bundle 还会有 `_objects.json`） |

每个 bundle 目录里的 `_ripper.json` 记录了文件清单和来源 crc。

缓存布局：清单在 `cache/manifests/<app>/ios<N>.msgpack.zst`，bundle 在 `cache/bundles/<bundleName>.<crc>`（已反混淆的 UnityFS）。
每个 bundle 下载后都会校验长度（等于 `fileSize + 4`）和 CRC（对解压后的条目计算，与清单比对），通过后才原子写入缓存。

## 支持平台

macOS arm64、Windows x64、Linux x64、Linux arm64（Linux 为 musl 静态链接）。每个平台都在原生 CI runner 上构建和测试（`.gitea/workflows/ci.yml`）。

## 计划中的结构

Rust（edition 2024）多 crate workspace：

| crate | 职责 |
|---|---|
| `ripper-cdn` | 版本号、清单解密、下载、反混淆、缓存 |
| `ripper-unity` | UnityFS / SerializedFile / typetree 读取（基于 unity-rs-core，放在 trait 后面） |
| `ripper-convert` | Texture2D→PNG、AnimationClip→sse-motion、ACB→WAV、USM 解复用 |
| `ripper-resolve` | masterdata 加剧本得到 BundlePlan |
| `ripper-format` | 输出格式的 serde 类型与 schema 版本，供 sse 依赖 |
| `ripper-cli` | 命令行 |

`tools/oracle/` 放 UnityPy 交叉验证脚本。

## 法律与分发边界

- **本仓库和 release 不包含、也不分发任何游戏资产或派生资产。** 测试只使用本地缓存，仓库里只放哈希和统计值作为期望值。
- **清单解密 key（ABCrypt）不在代码里。** 需要用户从自己合法持有的客户端取得，通过配置文件或环境变量提供。
- 与游戏相关的商标和版权归各自权利人所有。本工具仅供个人研究与复刻使用。使用者须自行遵守游戏服务条款和所在地法律。

## 逆向工单

- [`docs/reverse/RE-R01-motion-bundle-mapping.md`](docs/reverse/RE-R01-motion-bundle-mapping.md)：Live2D 模型到动作包的映射规则（对应决策 D10）。**已完成**，结论见 [`docs/reverse/cn-6.4.0/live2d-bundle-resolution.md`](docs/reverse/cn-6.4.0/live2d-bundle-resolution.md)

## 许可证

可以任选以下两种许可证之一：

- Apache License 2.0（[LICENSE-APACHE](LICENSE-APACHE)）
- MIT License（[LICENSE-MIT](LICENSE-MIT)）
