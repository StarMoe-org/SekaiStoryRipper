# ADR-0004: 许可证：MIT OR Apache-2.0

- **状态**：accepted
- **日期**：2026-09-23

## 背景

依赖全部是 MIT / Apache-2.0。下游 sse 是 AGPL-3.0-or-later，`ripper-format` 会被它引用，也可能被第三方复用。ripper 不链接 Live2D Cubism Core。

## 决策

采用 MIT OR Apache-2.0 双许可。其他项目（例如 GPL 的 sekai-viewer）只借鉴路径规律这类事实，不复制代码。

## 后果

- 正面：与 AGPL 兼容，第三方可以自由复用格式 crate。
- 负面：他人可以闭源再利用。

## 复审条件

引入了许可证不兼容的依赖时。
