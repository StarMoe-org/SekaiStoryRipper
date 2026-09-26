# 架构决策记录（ADR）

本目录记录 SekaiStoryRipper 的架构决策，每条决策一个文件，编号递增、不复用。
决策变更时新建一篇 ADR，并把旧 ADR 的状态改为 `superseded by ADR-NNNN`。

| 编号 | 标题 | 状态 |
|---|---|---|
| [0001](0001-record-architecture-decisions.md) | 用 ADR 记录架构决策 | accepted |
| [0002](0002-rust-and-unity-rs.md) | 实现语言：Rust + unity-rs-core | accepted |
| [0003](0003-cli-and-ripper-format.md) | 集成形态：独立 CLI + `ripper-format` crate | accepted |
| [0004](0004-license.md) | 许可证：MIT OR Apache-2.0 | accepted |
| [0005](0005-keys-and-assets.md) | 密钥由用户提供，不分发任何游戏资产 | accepted |
| [0006](0006-scope.md) | 范围：剧情类型与客户端内置资源 | accepted |
| [0007](0007-output-formats.md) | 输出格式：无损、版本化的中间格式 | accepted |
| [0008](0008-live2d-resolution.md) | Live2D 包解析与 binding 命名 | accepted |
| [0009](0009-missing-assets-and-masterdata.md) | 缺失资产的处理与 masterdata 来源 | accepted |
| [0010](0010-cross-platform.md) | 跨平台构建 | accepted |
| [0011](0011-regions.md) | 区服：CN 匿名 CDN 与日服游客登录 | accepted |
| [0012](0012-s3-output.md) | S3 作为输出存储 | accepted |
| [0013](0013-storage-contract.md) | 存储即接口：library + `ripper-format` + `formats` 表 | accepted |

模板见 [`0000-template.md`](0000-template.md)。
