# SekaiStoryRipper

为 **Project Sekai（CN 服 6.4.0，Unity 2022.3.62f3，iOS）** 的剧情回放抓取并解包所需资产的独立工具。
下游消费者是 [SekaiStoryExporter](https://github.com/StarMoe-org/SekaiStoryExporter)（sse）。

> 状态：**规划完成，尚未开始实现**。方案与全部已拍板决策见 [`docs/plan.md`](docs/plan.md)。

## 做什么

1. 从 CN CDN 匿名拉取 AssetBundle，完成反混淆、校验和缓存；
2. 根据 masterdata 和剧本，反推出某一话需要哪些 bundle；
3. 把 bundle 解成 sse 可以直接消费的**无损、版本化**中间格式。动作（AnimationClip）保留 StreamedClip 的原始多项式系数，**不转成 motion3**。

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

- [`docs/reverse/RE-R01-motion-bundle-mapping.md`](docs/reverse/RE-R01-motion-bundle-mapping.md)：Live2D 模型到动作包的映射规则（对应决策 D10）

## 许可证

可以任选以下两种许可证之一：

- Apache License 2.0（[LICENSE-APACHE](LICENSE-APACHE)）
- MIT License（[LICENSE-MIT](LICENSE-MIT)）
