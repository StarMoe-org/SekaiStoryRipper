# ADR-0008: Live2D 包解析与 binding 命名

- **状态**：accepted
- **日期**：2026-09-23

## 背景

剧本只给出 `CostumeType` 和 `Character2dId`。模型包与动作包之间没有直接的数据字段关联；社区工具使用启发式匹配，在一部分组合上会静默选错包。AnimationClip 的 binding 只存 CRC32 哈希，需要参数 ID 表才能还原名字。

## 决策

- 按客户端的实际规则解析：模型包 `live2d/model/<CostumeType>`，动作包 `live2d/motion/<character2ds[Character2dId].assetName>_motion_base`；动作名先查模型包、再查动作包，不区分大小写。规则写成常量，见 `ripper-resolve::live2d`。
- 只保留「客户端规则 + 配置覆盖表」两层，不做启发式；解析不出来就报 warning。
- binding 名字由 ripper 用全部模型的参数和部件 ID 全集反查，同时始终保留原始哈希；sse 仍可以按实际模型重新解析。

## 后果

- 正面：不会静默选错包；解析结果与游戏一致。
- 负面：游戏规则变化时需要更新常量。

## 复审条件

新版本客户端改变了包命名规则时。
