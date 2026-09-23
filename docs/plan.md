# SekaiStoryRipper 规划

## Context

SekaiStoryExporter（sse，Rust，AGPL-3.0-or-later + Cubism 链接例外）要做 Live2D 剧情复刻和视频导出。目前它只有骨架：13 个 crate 只写了职责注释，`tools/fetch` 是空目录，也还没定义资产目录布局。已经查过的第三方解包源都不能用：exmeaning 没导出 AnimationClip；sekai.best 的 motion3 把两端切线为 0 的三次段写成了线性，最大偏差约 9.5 个参数单位。所以需要一个独立工具 **SekaiStoryRipper**，负责：

1. 从 CN CDN 匿名拉取 bundle，做反混淆和缓存；
2. 根据 masterdata 和剧本，反推出「某一话需要哪些 bundle」；
3. 把这些 bundle 解成 sse 能直接消费的**无损、版本化**中间格式。动作必须保留 StreamedClip 的原始多项式系数。

目标：版本锁定 CN 6.4.0 / Unity 2022.3.62f3 / iOS，同时保持可配置。

---

## 0. 本次调研新增的事实（补充你给的 1–6 条）

| # | 事实 | 来源 |
|---|---|---|
| F1 | 清单里和剧情相关的前缀与体量：`scenario/background` 1244 个 / 1.27 GB，`scenario/effect` 324 个（依赖 `shader/particles`），`scenario/movie` 139 个 / 1.56 GB，`sound/scenario/voice` 1930 个 / 5.8 GB，`sound/scenario/bgm` 389 个 / 0.49 GB，`sound/scenario/se/{se,se_pack00001,se_pack00001_b}` 3 个大包共约 120 MB，`sound/card_scenario/voice` 2454 个 / 4.3 GB，`event_story/<ab>/{scenario,scenario_se,episode_image,screen_image}`，`scenario/{unitstory,special,actionset,profile}`，`live2d/model` 648 个 / 785 MB，`live2d/motion` 187 个 / 24 MB，`font/common`（依赖 `custom_profile/font`）。全量清单 77.7 GB | 本地 manifest.json 统计 |
| F2 | **剧本 bundle 是一个章节或一个活动一个包**，里面有多个 ScenarioSceneData：`scenario/unitstory/<chapter.assetbundleName>`、`event_story/<eventStory.assetbundleName>/scenario`、`scenario/special/<ab>`、`scenario/actionset/group<id/100>`、`scenario/profile` | 清单 + sekai-viewer `storyLoader.ts` |
| F3 | **语音 bundle = `sound/scenario/voice/<ScenarioId>`**，已实测 `nightcode_01_01`、`mmj_01_01`、`event_01_01`。卡面剧情是 `sound/card_scenario/voice/<scenarioId>`。另有 `vs<ScenarioId>` 变体和 `part_voice*`，含义待确认 | 清单 |
| F4 | **`AppearCharacters[].CostumeType` 本身就是模型 bundle 名**：`live2d/model/<CostumeType>`，例如 `18mafuyu_black`、`sub_kanadefather`、`21miku_night`。`Character2dId` 查 `character2ds` 只能得到 characterId 和 assetName，其中 146 条 mob 没有 assetName | 剧本样本 + character2ds |
| F5 | 动作 bundle 是 `live2d/motion/<base>_motion_base`，按角色共享。模型到动作包的映射**没有直接字段**。sekai-viewer 靠启发式回退（去 `v2_clb\d{2}_`、归一 `_back`、去掉尾部数字、逐段缩短） | sekai-viewer `live2dLoader.ts:320-402` |
| F6 | BGM 用了 CRI **AISAC 和 BlockIndex**（`SoundPlayMode` 4/5/6、`SetFirstBGMBlockIndex`），SE 按 cue 名在大包里查。**只解成 WAV 会丢掉 block 和循环信息** | scenario-player.md §9 |
| F7 | 对话框、Telop、Sekai 转场粒子、相机后处理 shader、TMP 材质、GlobalMaskTexture 都在 **ipa 的 `Data/data.unity3d`**（`resources.assets\|N`）里，**不在 CDN 上** | sse 逆向文档 |
| F8 | 字体的结论已经定了：复刻端必须用思源黑体自己生成 SDF。游戏的 OnDemand atlas 只覆盖 78%，只能拿来做像素对拍 | text.md §2 |
| F9 | Rust 生态比预期成熟：**`unity-rs-core` 0.5.1**（MIT，seiunx-dev）是 AssetStudio 的无头 Rust 重写，把 **Team-Haruki/AssetStudio 当差分 oracle**。它支持显式传入 Unity 版本来覆盖被抹掉的版本号、AnimationClip 和绑定图、内嵌或外部 schema 的 MonoBehaviour、符合 Khronos 规范的 ASTC 解码（其语料统计是 100% ASTC_RGB_6x6，疑似就是 Sekai），Live2D 包也支持。还是 Beta，主要由一个人维护。同一作者还有 **`cridecoder` 0.3.5**（MIT，纯 Rust，ACB/AWB/HCA/USM，Moe 在生产环境用它） | crates.io、README、REWRITE_STATUS |
| F10 | Moe_Assets_updater_NEXT **不导出 Live2D 和 AnimationClip**（AnimationClip 被列为不可读类型）。本地的 AssetStudio-haruki 的 FFI 里也没有 live2d 路径，`CubismMotion3Converter` 完全不读 `binding.attribute`。能复用的只是：StreamedClip 解析类（`AnimationClip.cs:498-620`，2022.3.19 以上 curveCount 按 u16 读），以及 CRC32 反查思路（`CubismMotion3Converter.cs:20-25,118-155`） | 调研 agent |
| F11 | Moe 可以借鉴的工程做法：清单快照用 msgpack+zstd 存盘并按 crc 做 diff；下载器是 `JoinSet` + 信号量组成的滑动窗口；key/iv 走环境变量（`HARUKI_SHARED_AES_KEY_HEX`）；按 crc 跳过已下载项 | `bundle_diff.rs`、`asset_execution.rs` |
| F12 | masterdata GitHub 源 7 张表都能直接取：eventStories、unitStories、character2ds、cardEpisodes、actionSets、specialStories、episodeCharacters | 实测 curl |

---

## 1. 范围：剧情回放需要的资产与反推规则

| 资产 | bundle 名来源（反推规则） | 里面是什么 | v1 | 输出 |
|---|---|---|---|---|
| 剧本 | 按 masterdata 表定位（F2），按 `ScenarioId` 选对象 | MonoBehaviour ScenarioSceneData | ✅ | typetree JSON，字段名与 C# 一致，格式和现有 `assets/scenario/*.json` 相同 |
| 背景 | `NeedBundleNames` 里的 `scenario/background/*`，以及 `FirstBackground` 和 EffectType 7 的 `StringVal`（取并集，防止 NeedBundleNames 漏项） | Texture2D/Sprite（ASTC） | ✅ | PNG |
| Live2D 模型 | `AppearCharacters[].CostumeType`，以及 LayoutData 里出现的 CostumeType，拼成 `live2d/model/<x>` | moc3 / physics3 / model3（TextAsset）、texture_00、BuildModelData | ✅ | 原样文件 + PNG + BuildModelData JSON |
| Live2D 动作 | `live2d/motion/<character2ds[Character2dId].assetName>_motion_base`（RE-R01）；动作名先查模型包、再查动作包 | AnimationClip ×N、MotionMetaData、BuildMotionData | ✅ | **sse-motion JSON**（§3） |
| 语音 | `sound/scenario/voice/<ScenarioId>`（卡面剧情用 `sound/card_scenario/voice/…`）；FullScreenText 的 `StringValSub`；part_voice 规则待确认 | TextAsset `.acb` | ✅ | 按 cue 输出 WAV + cue 索引 |
| BGM | `FirstBgm` 和 `SoundData.Bgm`，拼成 `sound/scenario/bgm/<bgm>` | `.acb` | ✅ | WAV + 原 ACB + block/loop 元数据 |
| SE | `SoundData.Se`，通过「cue 名 → 包」的**权威索引**定位（索引来自扫描 3 个 SE 包和 `event_story/*/scenario_se` 的 cue 表，不猜规则） | `.acb` | ✅ | WAV（只输出被引用的 cue） |
| 剧情特效 | EffectType 15/16/22 的 `StringValSub`，以及 `IncludeSoundDataBundleNames`，外加 bundle 自身的依赖 `shader/particles` | prefab（ParticleSystem 等）+ 贴图 + 可能带 ACB | ⚠️ 只做结构导出 | 对象图 typetree JSON + 贴图 PNG（sse 自己决定怎么烘焙） |
| 剧情影片 | `NeedBundleNames` 里的 `scenario/movie/*`（EffectType 19） | USM | ⚠️ 解复用 | 视频流原样 + 音频 WAV，不转码 |
| MV | `EpisodeMusicVideoId` / EffectType 37 | 3D MV | ❌ 范围外 | 在报告里给出占位条目 |
| 字体参考 | `font/common`（+`custom_profile/font`） | TMP FontAsset + Alpha8 atlas | ⚠️ 仅作参考 | glyph 表 JSON + atlas PNG |
| 对话框、Telop、转场、shader | ipa `data.unity3d`（F7） | — | ❌（见 D6） | — |

反推流程（resolver）：

```
masterdata(可配置源) ──► 剧集索引 (storyType, id) → (scenarioBundle, ScenarioId, voiceBundle)
                              │
                              ▼  拉取并解剧本 bundle
ScenarioSceneData ──► 抽取引用：background / costume / motion+facial 名 / voice cue / bgm / se / effect / movie
                              │
                              ▼  对照清单做存在性校验（缺失 → 报告；--strict 时报错）
BundlePlan（有序、去重、带 crc/size）──► fetch ──► unpack ──► emit + episode 索引
```

---

## 2. Rust vs C#

| 维度 | Rust | C#（直接复用 AssetStudio-haruki） |
|---|---|---|
| UnityFS / SerializedFile / typetree | `unity-rs-core`（F9，Beta，拿 AssetStudio 做 oracle）；备选 `rabex`，或者自己写一个最小 reader（约 1.5–2k 行，只需 UnityFS+LZ4+SF v22+typetree） | AssetStudio：成熟，Sekai 社区在生产环境长期使用 |
| 被抹掉的 Unity 版本 | unity-rs 支持调用方显式指定版本（REWRITE_STATUS:89），需要在 spike 里实测 | `CustomUnityVersion`，已被 Moe 验证 |
| AnimationClip 原始系数 | 两边都要**自己写** StreamedClip 到分段的转换（约 100 行）；Rust 可以参考 AssetStudio 的算法（MIT） | 现成的 `StreamedClip` 类，但 motion3 转换器用不上 |
| MonoBehaviour 缺 typetree 时 | unity-rs 支持外部 schema（可以从 dump.cs 生成） | AssemblyLoader + DummyDll（需要 Il2CppDumper 的产物） |
| ASTC 6x6 | unity-rs 内置，符合 Khronos 规范；或用 `texture2ddecoder`（MIT/Apache） | `Kyaru.Texture2DDecoder` 原生库，属 AssetStudio 谱系，**已知与 astcenc 有 LDR 舍入差异**（unity-rs 文档 :861-878） |
| ACB/HCA/USM | `cridecoder`（纯 Rust，Moe 在生产环境用） | 没有现成方案：VGAudio 对 HCA v3/HFR 的支持存疑，或者得 FFI 调 Rust |
| LZ4 / LZMA / AES / msgpack | `lz4_flex` / `lzma-rs` / `aes+cbc` / `rmp-serde`，都很成熟 | BCL + 第三方，同样成熟 |
| 分发 | 单个静态二进制，交叉编译 macOS arm64 / Windows x64 | NativeAOT，要按 RID 分别构建；原生 ASTC 库得一起带上 |
| 与 sse 集成 | **能共享 `ripper-format` crate**（serde 类型 + schema 版本），sse 编译期就能发现格式漂移；以后也可以把 ripper 当库链进 sse | 只能靠 CLI + JSON Schema，两种语言各写一套类型，有漂移风险 |
| 许可证 | 依赖都是 MIT/Apache | AssetStudio 是 MIT。两种语言都和 sse 的 AGPL 兼容 |
| 主要风险 | unity-rs 是 Beta、主要一人维护、API 还在变 | 两种语言的工具链；CRI 解码这块短板 |

**推荐：Rust。** 理由如下：

- CRI 这条链路只有 Rust 有成熟方案；
- 能和 sse 共享类型；
- 是单二进制；
- AnimationClip 的无损转换两边都得自己写，C# 在这一点上没有实质优势。

风险的应对：M0 先做 spike。Unity reader 放在 trait 后面，unity-rs 不行就换成自研最小 reader，**不回退到 C#**。
Moe 那种「Rust 编排 + C# NativeAOT FFI」的混合方案复杂度最高，不推荐。

---

## 3. 输出格式设计

### 3.1 目录布局（共享库 + 每话索引，对齐 sse 规划的 CAS 和 `assets.lock`）

```
<out>/
  ripper.lock.json              # app 版本、CDN N、清单 sha256、masterdata commit sha、工具版本、format 版本
  library/
    scenario/<bundle>/<ScenarioId>.json
    live2d/model/<costume>/{<x>.moc3, <x>.physics3.json, <x>.model3.json, texture_00.png, build_model_data.json, params.json}
    live2d/motion/<base>/{index.json, clips/<name>.sse-motion.json}
    background/<name>.png
    audio/{bgm,se,voice}/<bundle>/<cue>.wav + <bundle>.cues.json (+ 可选 <bundle>.acb)
    effect/<name>/{objects.json, textures/*.png}
    movie/<name>/{video.<ext>, audio_*.wav, meta.json}
    font/common/{glyphs.json, atlas_*.png}
  episodes/<storyType>/<id>.json  # 这一话引用的 library 相对路径 + 缺失清单 + 未解析项
```

### 3.2 `sse-motion` 无损中间格式（JSON，f32 用最短往返表示，保证逐位可逆）

```jsonc
{
  "format": "sse-motion", "version": 1,
  "name": "w-cute-nod05", "sampleRate": 60, "length": 2.35,
  "loop": { "loopTime": false, "startTime": 0, "stopTime": 2.35 },     // 来自 m_MuscleClip
  "fade": { "in": 0.5, "out": 0.5 },                                   // 来自 Live2DBuildMotionMetaData
  "category": "Motions|Facials|Arms|Poses",                             // 来自 BuildMotionData
  "curves": [
    { "binding": { "pathHash": 123, "attrHash": 3702945584, "typeId": 114,
                   "resolved": { "target": "Parameter", "id": "ParamAngleX" } | null,
                   "attr": "Value|Opacity|EyeOpening|MouthOpening|#<hash>" },
      "kind": "streamed",
      "segments": [ { "t": 0.0, "coeff": [a, b, c, d] }, ... ],        // value(t) = ((a·dt+b)·dt+c)·dt+d，dt = t − seg.t，一直有效到下一段
      "end": 2.35 },
    { "binding": {...}, "kind": "constant", "value": 0.0 },
    { "binding": {...}, "kind": "dense", "frames": [...] }              // 目前全为 0 条，保留以防以后出现
  ],
  "events": [ { "time": 0.183, "function": "OnLive2DInvokeUserData", "data": "eyeblink,0.2,0.79,0.24",
                "float": 0, "int": 0, "string": "..." } ],
  "source": { "bundle": "live2d/motion/01ichika_motion_base", "pathId": 123, "crc": 0 }
}
```

- 首帧 -FLT_MAX 哨兵会去掉，但原值记进 `source`。末尾的 +FLT_MAX 哨兵同样处理（待确认 Q9）。
- **binding 的哈希始终保留**。`resolved` 是 ripper 用「同一角色所有模型的 moc3 参数 ID 并集」反查出来的。sse 加载时也能用实际模型的参数表重新反查。
- 可选 `--keep-raw`：额外输出 AnimationClip 的原始 typetree JSON，方便排查和做 oracle 对比。

### 3.3 其他资产
- **贴图**：默认 PNG（RGBA8），并在 sidecar 里注明 alpha 语义是 straight 还是 premultiplied、颜色空间以及原始格式 50。解码器选符合 Khronos 规范的（和 astcenc 一致）。可选同时保留 `.astc` 原始数据（D8）。
- **moc3**：原样输出。另外 ripper 自己解析参数和部件 ID 表，写进 `params.json`，只读 ID，不链接 Cubism Core。
- **音频**：见 D9。
- **剧本**：原样 typetree JSON，不做语义加工。语义属于 sse 的 `sse-scenario`。

---

## 4. 缓存、增量、并发、配置

- **清单**：按 `(app, N)` 归档，存解密后的 msgpack+zstd，保留历史。`diff` 按 bundleName 比较 crc 和 fileSize，输出新增、变更、删除。
- **bundle 缓存**：键为 `(bundleName, crc)`，存反混淆后的 UnityFS，写一个 `.meta` 记录 downloadPath、size、下载时间。
  - 校验：长度必须等于 `fileSize+4`。crc 在解压后校验：按顺序拼接全部解压后的条目再算 CRC32（M0 已验证，Q13）。
  - 写入原子化：先写 tmp 再 rename。
  - 对大文件做流式下载，支持断点续传（Range 请求）。
- **产物增量**：每个 library 条目记录它的源 `(bundle, crc)` 和 format 版本。源没变且版本没变就跳过。
- **并发**：tokio + 信号量滑动窗口。下载并发默认 8；解包和编码走 rayon，默认 CPU 数。另设一个内存软上限，因为 SE 大包有 50 MB。
- **重试**：指数退避。4 个等价 CDN 主机轮询，某台 404 时不轮换，因为 404 说明 downloadPath 错了。
- **配置**：用 TOML，优先级为 CLI > 环境变量 > 文件 > 默认值。可配置项：
  - `cdn.hosts[]`、`cdn.app_version`（默认 6.4.0）、`cdn.version`（auto 或固定 N）
  - `cdn.user_agent`、`unity.fallback_version`（默认 2022.3.62f3）
  - `crypto.ab_key/iv`（见 D4）
  - `masterdata.source`：`github`（`base_url` 默认是 haruki raw 的 HEAD，可锁定 `ref`）或 `local_dir`
  - `paths.cache`、`paths.out`
  - 并发数、各输出格式开关
- **CLI 草案**：
  - `ripper manifest [--diff]`
  - `ripper plan <story-selector>`（dry-run，列出 bundle 和体积）
  - `ripper rip <selector>`
  - `ripper sync [--all-stories]`
  - `ripper verify`
  - `ripper inspect <bundle>`
  - selector 的例子：`unit:school-refusal-story-chapter/3`、`event:120`、`event:120/1-4`、`card:1234`、`scenario:<ScenarioId>`。

---

### 跨平台约束（第四批决策）

- **只用纯 Rust 或能在四个平台原生编译的依赖。** 目前唯一的 C 依赖是 `zstd-sys`（经 unity-rs-core 引入）。HTTP 用 reqwest + **rustls**，不引入 OpenSSL。
- **输出文件名要在 Windows 上合法**：替换 `<>:"/\|?*` 和控制字符；避开 `CON`、`PRN`、`AUX`、`NUL`、`COM1`–`COM9`、`LPT1`–`LPT9` 这些保留名；去掉结尾的点和空格。
- **大小写不敏感的文件系统**（Windows NTFS、macOS APFS 默认）：同一目录下只差大小写的文件名视为冲突，写出前检测，报错并在名字后加稳定后缀，不能静默覆盖。
- **长路径**：Rust std 在 Windows 上会自动加 `\\?\` 前缀，所以不需要额外处理。但索引 JSON 里的路径一律用 `/` 分隔的相对路径，不写平台路径。
- **外部程序**（ffmpeg）按平台查找可执行文件名，路径可配置；只用于 M5 的 ADX 转码。
- 不依赖符号链接、硬链接或文件权限位；缓存目录可以放在任何文件系统上。

## 5. 测试与验证

1. **Oracle 对比**（`tools/oracle/`，用 uv 跑 Python + UnityPy，复用你之前的 cdn.py 和 clip.py 的逻辑）：
   - 同一批 bundle，对比对象清单（类型、名字、path_id），要求完全一致。
   - AnimationClip 分段系数要求 **f32 逐位相等**，并且在 60 Hz 下采样比对，误差为 0。
   - 眨眼事件的数量要和 200 个全量对上。
   - 统计值复现：常量段 7185 / 零切线三次段 10895 / 其他三次段 1041 / 线性段 0。
2. **贴图**：和官方 astcenc 的解码结果逐字节比较，同时和 UnityPy 比较（允许 LDR 舍入差异，差异单独记录）。
3. **音频**：PCM 和 vgmstream 或 clHCA 移植版对比，重点测 HCA v3 + HFR。
4. **剧本**：把 2650 个剧本导出成 JSON，和 exmeaning 快照做语义比较（字段集合和取值一致）。
5. **resolver 全量回归**：对 2650 个剧本全部跑 `plan`。要求缺失项为 0，否则每一项都要有解释；未知 EffectType（比如 45）只告警。
6. **单元和属性测试**：反混淆、AES、StreamedClip 解码，用合成数据测，不需要游戏资产。
7. **不提交游戏资产**：真实数据测试通过 `RIPPER_TEST_CACHE` 指向本地缓存，CI 只跑合成测试。仓库里只放派生的哈希或统计作为期望值（需要你确认，见 D5）。

---

## 6. 里程碑

- **M0 spike（先做，决定 D1 是否成立）**：用 Rust + unity-rs-core 读 5 类黄金 bundle（model、motion、unitstory 剧本、bgm acb、se 大包），逐项对照 Q1、Q8、Q9、Q6。
- **M1**：CDN 客户端、清单与 diff、缓存、`manifest` / `fetch`。
- **M2**：解包 TextAsset、Texture2D→PNG、MonoBehaviour→JSON、AnimationClip→sse-motion，同时搭好 oracle 框架。
- **M3**：masterdata 源、resolver、`plan` / `rip`、episode 索引、`ripper.lock.json`。
- **M4**：ACB→WAV、cue 索引、SE 权威索引。
- **M5**：特效结构导出、USM 解复用、字体参考、`sync` 增量。
- **M6**：全量回归（2650 个剧本）、`ripper-format` crate 发布给 sse。

---

## 7. 需要你逐条拍板的决策

**已拍板（2026-09-23）**：D1 Rust · D2 CLI+ripper-format · D3 MIT OR Apache-2.0 · D4 用户自行提供 key（兼容 --manifest-file）· D6 **永远不做**（ipa 资产完全交给 sse）· D7 主线+活动+卡面+特别篇 · D8 PNG+可选 ASTC · D9 WAV+cue 元数据+原 ACB · D11 JSON · D13 只解复用 · D16 多 crate workspace。
**第二批已拍板**：D10 = A（分层解析 + 全量校验）**+ 先逆向真实规则**（工单见 `docs/reverse/RE-R01-motion-bundle-mapping.md`）· D12 = ripper 解析 + 保留哈希 · D14 = 默认告警，`--strict` 时失败 · D15 = masterdata 使用配置文件里可配置的 URL（默认 haruki raw HEAD，不强制锁 sha）。
**D5**：没有单独作答，按原始需求「本工具不分发任何游戏资产」执行，即仓库不放资产，只放哈希和统计。

**D10 逆向结论（RE-R01，2026-09-23）**：真实规则就是 R1，动作包 = `live2d/motion/<character2ds[Character2dId].assetName>_motion_base`，与 CostumeType 无关，客户端没有例外表；名字查找先查模型包 container、再查动作包，不区分大小写。详见 `docs/reverse/cn-6.4.0/live2d-bundle-resolution.md`。

**第三批已拍板（RE-R01 之后）**：
- **D10 修订**：resolver 只保留「逆向规则 + 配置覆盖表」两层，删除 R2 及之后的启发式（R2 在 42/592 个组合上会静默选错包）；解析不出来就报 warning。解析单位是 `(Character2dId, CostumeType)`。
- **模型包自带的 clip**：各包各存（模型包的放 `library/live2d/model/<costume>/clips/`），episode 索引按 `(id, costume)` 给出按游戏规则解析好的「动作名 → clip」表。
- **逆向结论进代码**：手写常量，并由单元测试对照 `live2d-bundle-resolution.yaml`（`ripper-resolve::live2d`）。

**M0 结论（2026-09-23）**：GO，保持 D1。详见 [`spike/M0-report.md`](spike/M0-report.md)。

**第四批已拍板（M0 之后，2026-09-23）**：
- **S6（HCA PCM 极性 / v3）**：暂不处理，放到 M4 再验证；在此之前 WAV 按 cridecoder 的原始输出写出。
- **影片音频（ADX）**：M5 **调用外部 ffmpeg** 转成 WAV，ffmpeg 路径可在配置里指定（默认从 PATH 找 `ffmpeg` / `ffmpeg.exe`）。找不到 ffmpeg 时，默认给 warning 并保留原始 `.adx`，`--strict` 时报错（与 D14 一致）。
- **分块 BGM**：M4 **按 waveform（AWB id）导出** WAV；`cues.json` 给出 cue → waveform，`tables.json` 给出 block → track → waveform。普通的单 cue BGM 结果不变。
- **跨平台（新增硬性要求）**：支持 **macOS arm64、Windows x64、Linux x64、Linux arm64**。
  - 构建：各平台用**原生 CI runner**（Gitea Actions，`.gitea/workflows/ci.yml`，runner 标签 `macos-arm64` / `windows-x64` / `linux-x64` / `linux-arm64`），每个平台都跑 fmt、clippy、test、release build。
  - Linux 用 **musl 静态链接**（`x86_64/aarch64-unknown-linux-musl`）。unity-rs-core 硬依赖 `zstd-sys`（C 代码），所以 Linux runner 要装 `musl-tools`；从 macOS 交叉编译 Windows 时会因为缺 Windows SDK 头文件而失败（M0 实测），这也是选原生 runner 的原因。
  - 代码约束见 §4「跨平台约束」。

**D10 新证据**（逆向前的途径 A 数据，保留备查）：BuildModelData 字段只有 Moc3FileName/TextureNames/PhysicsFileName/UserDataFileName/AdditionalMotionData/CategoryRules，**没有动作包引用**（Q2 = 否）；MonoBehaviour 的 typetree 可以直接读（Q1 倾向为「内嵌」）。规则 R1（character2ds.assetName+`_motion_base`）命中 357/371，另有 200 条没有 assetName；规则 R2（CostumeType 最长前缀匹配）命中 637/648，0 歧义；第 1 章 21 个组合两条规则全部一致。

> 每条格式：选项 / 代价 / **推荐**。

**D1 实现语言**

| 选项 | 代价 |
|---|---|
| A. Rust（unity-rs-core，reader 可替换） | unity-rs 是 Beta，需要 M0 验证；最坏情况要自研最小 reader |
| B. C#（引用 AssetStudio-haruki） | CRI 解码是短板；和 sse 只能靠 schema 对接；要按 RID 分别构建 NativeAOT |
| C. Rust 编排 + C# FFI（Moe 模式） | 两套工具链，复杂度最高 |

**推荐 A。**

**D2 与 sse 的集成形态**

| 选项 | 代价 |
|---|---|
| A. 独立 CLI + 独立仓库里的 `ripper-format` crate，sse 通过 git 依赖 | 要维护格式版本号 |
| B. 只有 CLI + JSON Schema | sse 得自己写一套类型 |
| C. ripper 作为库直接链进 sse | 耦合强，sse 会带上网络和解包依赖 |

**推荐 A**：以后要改成 C 也很容易。

**D3 ripper 的许可证**

| 选项 | 代价 |
|---|---|
| A. MIT OR Apache-2.0 | 别人可以闭源再利用 |
| B. AGPL-3.0-or-later，和 sse 一致 | format crate 也会变成 AGPL，限制第三方复用 |
| C. GPL-3.0 | 介于两者之间 |

**推荐 A**：依赖全是 MIT/Apache；sekai-viewer（GPL）**只借鉴路径规律这类事实，不复制代码**。ripper 不链接 Cubism Core，不涉及那条链接例外。

**D4 ABCrypt key 的处理**

| 选项 | 代价 |
|---|---|
| A. 硬编码进开源代码 | 最方便，但合规风险最高，也最容易被当成分发密钥 |
| B. 代码里不带 key，由用户在配置或环境变量里提供（Moe 的做法），文档只说明「从你合法持有的客户端提取」 | 首次使用多一步 |
| C. 只支持本地已解密的清单或 bundle | 失去自动更新能力 |

**推荐 B**，并且兼容 C（`--manifest-file` 可以导入已解密的清单）。反混淆规则不是密钥，可以写在代码里。以上不构成法律意见。

**D5 资产与测试数据的边界**

| 选项 | 代价 |
|---|---|
| A. 仓库和 release 里一律不放任何游戏资产或派生资产，只放哈希和统计 | 测试依赖本地缓存 |
| B. 允许放极小的裁剪片段作 fixture | 仍属于派生资产，有风险 |

**推荐 A**：README 声明工具不分发资产，输出目录默认加 `.gitignore`。

**D6 ipa 专有资产（对话框、Telop、转场、shader、TMP 材质）**

| 选项 | 代价 |
|---|---|
| A. v1 不做，sse 按逆向文档里的常量实现 | 像素对拍素材要另外找 |
| B. v1 就支持 `--local data.unity3d` 输入源 | 工作量 +1 个里程碑；需要用户自行持有 ipa |
| C. 不做 | — |

**推荐 A，并把 B 列为 v2**：reader 已经支持本地文件，增量成本不高。

**D7 v1 覆盖哪些剧情类型**

| 选项 | 代价 |
|---|---|
| A. 主线 + 活动 + 卡面 + 特别篇 | 卡面剧情路径待确认（Q3） |
| B. A 再加区域对话（actionset）和角色自我介绍（profile） | 多两种索引规则 |
| C. 只做主线 + 活动 | 最小 |

**推荐 A**：B 放到 v1.1。

**D8 贴图输出**

| 选项 | 代价 |
|---|---|
| A. PNG | 体积大约是 ASTC 的 4–6 倍（全部背景约 1.3 GB，换成 PNG 约 5 GB），需要选定解码器 |
| B. 保留 ASTC | Windows 端 GPU 不支持，sse 得自己解 |
| C. PNG + 可选保留原始 ASTC | — |

**推荐 C（默认只输出 PNG）**：解码器用符合 Khronos 规范的（和 astcenc 一致）。

**D9 音频输出**

| 选项 | 代价 |
|---|---|
| A. 按 cue 输出 WAV（PCM16）+ cue 元数据（循环点、block、AISAC，前提是能读出来） | 体积大（语音 5.8 GB 压缩数据解开后会大好几倍），但只解剧集用到的部分 |
| B. 只保留 ACB，由 sse 解码 | sse 要带 CRI 解码 |
| C. A + 保留原 ACB | 多一份磁盘占用 |
| D. 转成 FLAC 或 Opus | 再加一层编码依赖 |

**推荐 C**：BGM 的 block 和 AISAC 语义必须有原始数据兜底；可选 FLAC 作为 v2。

**D10 模型 → 动作包的映射**

| 选项 | 代价 |
|---|---|
| A. 优先读 BuildModelData 里的引用（前提是 Q2 证实存在），失败再走启发式 | 依赖 spike 结果 |
| B. 只用 sekai-viewer 那套启发式 | 有误配风险，而且那套规则来自 JP |
| C. 维护一份人工映射表，外加全量校验（所有剧本里出现的 MotionName 都必须能在映射到的包里找到） | 需要维护 |

**推荐 A，并用 C 的全量校验兜底**：映射规则写进配置，可以覆盖。

**D11 动作中间格式的编码**

| 选项 | 代价 |
|---|---|
| A. JSON（§3.2），f32 最短往返表示 | 体积稍大（全量约几十 MB），可以用 zstd |
| B. 二进制（bincode 或 FlatBuffers） | 不方便人读，不方便 diff |
| C. JSON 为主，可选 `.bin` 缓存 | 两套格式 |

**推荐 A。**

**D12 binding 名字在哪一侧解析**

| 选项 | 代价 |
|---|---|
| A. ripper 用同一角色所有模型的参数并集解析，同时保留哈希 | 并集里可能有同名冲突（CRC32 碰撞概率极低） |
| B. 只保留哈希，交给 sse | sse 要实现 CRC 反查 |
| C. 按「动作包 × 模型」分别输出 | 数据重复 |

**推荐 A**（sse 仍可以重新解析）。

**D13 剧情影片（USM）**

| 选项 | 代价 |
|---|---|
| A. 只解复用，输出原始视频流 + WAV | sse 要用 ffmpeg 读原始流 |
| B. 直接转成 mp4 | 要依赖 ffmpeg，且有损 |
| C. v1 不做 | 第 1 章就有影片（2/21 话） |

**推荐 A。**

**D14 缺失资产的策略**

| 选项 | 代价 |
|---|---|
| A. 默认只告警并写进 episode 报告，`--strict` 时失败 | — |
| B. 默认失败 | CDN 上的数据比客户端新（比如 EffectType 45），会经常失败 |

**推荐 A。**

**D15 masterdata 的可复现性**

| 选项 | 代价 |
|---|---|
| A. 每次先解析 HEAD 对应的 commit sha，按 sha 取 raw 文件，并把 sha 写进 lock | 多一次 GitHub API 调用，有速率限制 |
| B. 直接用 HEAD | 结果不可复现 |
| C. 默认 A，可配置固定 ref 或本地目录 | — |

**推荐 C。**

**D16 仓库初始化**

| 选项 | 代价 |
|---|---|
| A. 在当前目录 `git init`，建 cargo workspace：`ripper-cdn` / `ripper-unity` / `ripper-convert` / `ripper-resolve` / `ripper-format` / `ripper-cli` + `tools/oracle` | — |
| B. 单个 crate | 以后拆分成本高 |

**推荐 A**。rust-toolchain 和 sse 对齐：1.98.1，edition 2024。

---

## 8. 待确认清单（不阻塞规划，M0 或 M3 时核实）

- **Q1** CDN bundle 里的 ScenarioSceneData、Live2DBuildMotionMetaData、BuildModelData、BuildMotionData 是否**内嵌 typetree**？如果没有，要从 dump.cs 生成外部 schema（unity-rs 支持）。 → ✅ 已答（M0 S3）：全部内嵌，不需要外部 schema。
- **Q2** BuildModelData 或 model3.json 里有没有指向动作包的字段？ → ✅ 已答（RE-R01）：没有。
- **Q3** CN 卡面剧情的剧本 bundle 路径是什么：`character/member_scenario/<ab>` 还是 `character/member/<ab>`？清单里 `character/member` 有 1352 个。 → ✅ 已答（RE-R01）：`character/member/<cards[cardId].assetbundleName>`。
- **Q4** `vs<ScenarioId>` 语音包和 `part_voice*` 的规则是什么？sekai-viewer 里的 ScenarioId→bundle 修正（活动 167–176 加 1 等）在 CN 上是否成立？
- **Q5** `IncludeSoundDataBundleNames`（例如 `scenario/effect/hologram`）里是不是带 ACB？ → ✅ 已答（M0）：`hologram` 里没有 ACB；`IncludeSoundDataBundleNames` 的含义仍待确认。
- **Q6** cridecoder 能否读出 ACB 的 block、AISAC、循环点？它对 HCA v3 + HFR 的解码是否正确（对照 vgmstream 或 hca.py）？ → ⚠️ 部分已答（M0）：block/AISAC 表完整导出，HCA 循环点能读到；分块 BGM 必须按 waveform 导出；PCM 极性和 v3 正确性待 vgmstream 裁定（第四批决策：推迟到 M4）。
- **Q7** scenario/movie 里 USM 的视频编码是什么（VP9、H.264 还是 MPEG-1），有几条音轨？ → ✅ 已答（M0）：按 MovieBundleBuildData 拆片；视频 MPEG-1，音频 CRI ADX。
- **Q8** unity-rs-core 实测：显式版本覆盖能否处理抹成 `5.x.x` 的头；能否拿到 AnimationClip 完整的 `m_MuscleClip`、`m_ClipBindingConstant`、`m_Events` 原始字段；ASTC 解码是否和 astcenc 一致。 → ✅ 已答（M0 S1/S4/S5）：全部可以。
- **Q9** StreamedClip 末尾是否有 +FLT_MAX 哨兵帧；2022.3.62 下 curveCount 是 u16 还是 u32（AssetStudio 按 2022.3.19 以上 u16 处理）。 → ✅ 已答（M0）：末尾有 +∞ 哨兵帧；curveCount 和 discreteCurveCount 是两个独立字段。
- **Q10** m_Events 里除了 eyeblink，还有没有别的 functionName 或 data 前缀？
- **Q11** 名字里带 `_back`、`v2_`、`clb01_` 的模型在剧本里怎么出现（和 D10 相关）。 → ✅ 已答（RE-R01）：都是独立的 character2d 条目。
- **Q12** haruki masterdata 相对 CDN 的更新延迟；卡面剧情要用的 `cards.assetbundleName` 在不在里面。
- **Q13** 清单里的 `crc` 是哪种算法、针对什么内容（混淆前还是混淆后），能否用来做完整性校验。 → ✅ 已答（M0 S8）：按顺序拼接全部解压后的条目，再算 CRC32。
- **Q14** CN 客户端的服务条款对个人解包和复刻的约束（这是法律问题，由你判断）。
- **Q16** Gitea 上需要注册 4 个平台的 Gitea Actions runner（标签见第四批决策）。目前仓库和用户级 runner 都是 0 个。
- **Q15** 特效 prefab 在 sse 里打算怎么用（烘焙序列帧还是实时粒子），这决定 ripper 要导出多少对象图细节。

---

## 9. 验证（整个方案落地后如何端到端检验）

1. `ripper manifest`：能拿到 N，清单条数等于实时值（09-23 为 82,395）。
2. `ripper rip unit:school-refusal-story-chapter/1`：
   - episode 索引里的缺失项为 0；
   - 背景、模型、动作、语音、BGM、SE 都齐；
   - 影片已解复用。
3. `uv run tools/oracle/compare.py <out> <cache>`：
   - 对象清单与 UnityPy 一致；
   - 动作系数逐位相等；
   - 贴图和 astcenc 对齐；
   - PCM 和参考解码器对齐。
4. `ripper plan --all`：跑完 2650 个剧本，缺失为 0，否则每项都有解释。
5. sse 侧：依赖 `ripper-format`，写一个最小加载器，读取 sse-motion 并在 60 Hz 下求值，结果与 oracle 一致。
