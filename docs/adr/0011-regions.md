# ADR-0011: 区服：CN 匿名 CDN 与日服游客登录

- **状态**：accepted
- **日期**：2026-09-25

## 背景

- CN 服的清单和 bundle 可以匿名下载。
- 日服 CDN 要求 CloudFront 签名 cookie，cookie 只能在登录后取得。日服的 API 和清单共用一对 AES key。
- 日服 CDN 上有少数 bundle 与清单里的 `fileSize` / `crc` 不一致，而游戏客户端本身不校验这两项。

## 决策

- 用 `--region cn|jp`（或配置 `cdn.region`）选择区服。区服决定整套默认值：CDN 地址、masterdata 来源、Unity 版本，以及独立的 `cache/<region>`、`out/<region>` 目录。配置文件叠加在区服预设之上。
- 日服按客户端新装流程登录：版本 API → 注册游客账号 → `PUT user/{id}/auth` → `POST api/signature` 取 cookie。游客账号只注册一次并保存在 `<cache>/account.json`，之后复用；它不在游戏内激活，只用于下载资源。
- 日服的 key 同样由用户提供（ADR-0005）。
- 日服下载只校验传输完整（`Content-Length`）和能否解压；CRC 不符只打印提示。CN 仍然校验长度和 CRC。
- asset version 统一用字符串表示（CN `"10"`，JP `"6.8.0.50"`）。

## 后果

- 正面：两个区服用同一套命令和输出格式。
- 负面：日服依赖登录流程，协议变化时需要跟进。

## 复审条件

日服改变认证方式，或 CN 开始要求登录时。
