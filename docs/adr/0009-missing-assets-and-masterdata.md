# ADR-0009: 缺失资产的处理与 masterdata 来源

- **状态**：accepted
- **日期**：2026-09-23

## 背景

CDN 上的数据可能比客户端或 masterdata 新，剧本里也有少量游戏数据错误。masterdata 来自社区维护的公开仓库，更新有延迟。

## 决策

- 缺失或无法解析的资源默认只告警，写进 episode 索引的 `warnings`；`--strict` 时报错。
- masterdata 的来源是配置里的 URL 模板（Team-Haruki 的公开仓库：CN 默认 `haruki-sekai-sc-master`，日服默认 `haruki-sekai-master`），可以锁定到某个 ref，也可以指向本地目录。所用来源写进 `ripper.lock.json`。

## 后果

- 正面：少量数据问题不会阻断整批导出；结果可复现。
- 负面：默认模式下需要留意报告里的 warning。

## 复审条件

有官方或更及时的 masterdata 来源时。
