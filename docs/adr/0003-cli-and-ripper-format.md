# ADR-0003: 集成形态：独立 CLI + `ripper-format` crate

- **状态**：accepted
- **日期**：2026-09-23

## 背景

下游 SekaiStoryExporter（sse）需要读取 ripper 的输出。输出格式一旦漂移，应当在编译期暴露，而不是在运行时才发现。

## 决策

- ripper 是独立的命令行工具和独立仓库。
- 输出格式的 serde 类型和格式版本号放在单独的 `ripper-format` crate 里，sse 以 git 依赖引用它。
- 仓库是多 crate 的 workspace：`ripper-cdn`（下载、清单、缓存）、`ripper-unity`（Unity 读取）、`ripper-convert`（格式转换）、`ripper-resolve`（剧集 → bundle 解析）、`ripper-format`（输出格式）、`ripper-cli`。

## 后果

- 正面：sse 不带网络和解包依赖；格式变化由类型系统和版本号双重把关。
- 负面：格式版本号要人工维护。

## 复审条件

sse 需要按需拉取资源、必须把 ripper 作为库链接时。
