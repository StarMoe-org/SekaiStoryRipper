# ADR-0007: 输出格式：无损、版本化的中间格式

- **状态**：accepted
- **日期**：2026-09-23

## 背景

第三方解包产物不能直接复用：有的不导出 AnimationClip，有的在转成 motion3 时把三次曲线段写成了线性，偏差明显。sse 需要逐位可复现的输入。

## 决策

所有格式都定义在 `ripper-format` 中，带格式名和版本号。

- **动作**：`sse-motion` JSON，保留 StreamedClip 的原始三次系数（f32 最短往返表示，逐位可逆），同时保留 binding 哈希和解析出的名字、事件、淡入淡出时间。不转成 motion3。
- **贴图**：PNG（RGBA8），解码器符合 Khronos 规范；`--keep-astc` 可以额外保留原始 ASTC 数据块。
- **音频**：每个物理 waveform 输出一个 WAV，另附 `cues.json`（cue → waveform，含循环点）和 `tables.json`（全部 UTF 表，含分块 BGM 的 block → track → waveform），并保留原始 ACB。
- **影片**：USM 只解复用成原始视频流和 ADX 音频，ADX 通过外部 ffmpeg 转成 WAV；不转码视频。
- **其他对象**：typetree JSON；带 GameObject 的 bundle 额外输出整个对象图（`_objects.json`，`ripper-objects`），以及没有 container 路径、只能经对象图引用到的 Texture2D（`_textures/<名字>.<pathId>.png`，例如 SpriteMask 用的 `Square`），供 sse 实现特效 prefab。
- **布局**：`library/<bundleName>/<container 相对路径>`，保留原始文件名，`model3.json` 里的相对引用可以直接解析；每话一个 `episodes/…json` 索引，外加 `ripper.lock.json`（`ripper-lock`，含全部格式版本的 `formats` 表，见 ADR-0013）。

## 后果

- 正面：下游可以逐位复现游戏数据；格式可以演进而不破坏旧产物。
- 负面：PNG 和 WAV 比原始数据大数倍。

## 复审条件

体积成为主要问题时，考虑 FLAC 或保留压缩纹理。
