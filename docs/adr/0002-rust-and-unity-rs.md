# ADR-0002: 实现语言：Rust + unity-rs-core

- **状态**：accepted
- **日期**：2026-09-23

## 背景

需要读取 UnityFS / SerializedFile / typetree，解码 ASTC 贴图，解析 CRI ACB/AWB/HCA/USM，并把 AnimationClip 无损转成中间格式。

- Rust 侧有 `unity-rs-core`（AssetStudio 的无头 Rust 重写，支持显式指定 Unity 版本、AnimationClip 绑定、内嵌 typetree、符合 Khronos 规范的 ASTC 解码）和 `cridecoder`（纯 Rust 的 CRI 解码）。
- C# 侧可以复用 AssetStudio，但 CRI 解码没有成熟方案，且只能通过 JSON Schema 与下游 sse（Rust）对接。
- AnimationClip 的 StreamedClip 无损转换两边都要自己写。

## 决策

用 Rust 实现。Unity reader 放在 trait 后面；unity-rs-core 不能满足时换成自研的最小 reader，不回退到 C#。不采用「Rust 编排 + C# FFI」的混合方案。

实测结论：显式版本覆盖、AnimationClip 原始字段、内嵌 typetree、ASTC 解码均满足需求。

## 后果

- 正面：单个静态二进制；能与 sse 共享 `ripper-format` 类型；CRI 链路完整。
- 负面：unity-rs-core 仍是 Beta，主要由一人维护，API 可能变动。

## 复审条件

unity-rs-core 停止维护，或出现它无法读取的 Unity 版本。
