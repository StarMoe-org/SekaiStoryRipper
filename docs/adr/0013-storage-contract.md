# ADR-0013: 存储即接口：library + `ripper-format` + `formats` 表

- **状态**：accepted
- **日期**：2026-09-26
- **相关**：ADR-0003、ADR-0007、ADR-0012

## 背景

sse 与 ripper 之间只通过 ripper 的输出交换数据：本地目录，或经 S3 发布的同一布局（ADR-0012）。sse 只以 git tag 依赖 `ripper-format` 的 serde 类型，不链接 ripper 的实现。
但之前有两处缺口：

- 特效用的 `_objects.json` 没有格式名和版本；
- `ripper.lock.json` 虽然列出了各格式的版本，却没有读取方校验。

结果是 `ripper-unpack` 升到 v2（新增 `_textures/`）后，用旧 library 渲染时，SpriteMask 遮住的内容会静默消失，而不是报错。

## 决策

- **接口就是存储**：library（`library/`、`episodes/`、`ripper.lock.json`）是 ripper 与读取方之间唯一的接口，本地目录与 S3 前缀的布局完全相同。读取方只依赖 `ripper-format`。
- **ripper 自己定义的文档都有格式名和版本**，类型都在 `ripper-format` 中：

  | 格式 | 文件 |
  |---|---|
  | `ripper-episode` | `episodes/…json` |
  | `ripper-unpack` | `_ripper.json`（含 `_textures/`） |
  | `ripper-objects` | `_objects.json` |
  | `sse-motion` | `.sse-motion.json` |
  | `ripper-acb` | `.cues.json` |
  | `ripper-lock` | `ripper.lock.json` |

- **原始载荷的结构由游戏决定**，不属于本项目的格式：
  - 单个资产的 typetree JSON、`_objects.json` 里的 `tree`；
  - PNG、WAV、moc3、`model3.json`；
  - `.tables.json`（ACB 的 UTF 表原样导出）。

  它们的结构由 lock 里的 `unityVersion` / `appVersion` 确定。
- **`formats` 表**：每次运行最后写 `ripper.lock.json`。其中 `formats` 等于写入时 `ripper_format::formats()` 的结果，即全部格式及其版本。
- **读取方的规则**：打开 library 时先读 lock，把 `formats` 与自己编译时所用 `ripper-format` 的版本逐项比对；有任何不同就拒绝，并提示用对应版本的 ripper 重新导出。逐个文档读取时，仍然检查该文档自己的 `format` / `version`。
- **演进**：任何文档结构的变化都要提升对应格式的版本，并发布新的 ripper tag。sse 升级 tag 是一次显式变更。

## 后果

- 正面：格式不匹配在打开 library 时就暴露，不会渲染出静默错误的结果；sse 可以只拿到一个 S3 前缀，不需要 ripper 的任何代码或网络能力。
- 负面：任何格式升级都要求 library 用新版 ripper 重新导出（已解包的 bundle 会因 record 版本不同而重新解包）。

## 复审条件

需要让一个读取方同时兼容多个格式版本时。
