# ADR-0012: S3 作为输出存储

- **状态**：accepted
- **日期**：2026-09-25

## 背景

导出的 library 体积大（一话数百 MB，全量数十 GB），往往要给多台机器或线上服务（例如 SekaiStoryExporter 渲染节点）共用。对象存储（AWS S3 及 MinIO、Cloudflare R2 等兼容服务）是最通用的共享方式。

解包流程本身依赖本地文件：增量判断要读 `_ripper.json`，影片的 ADX 要交给 ffmpeg 转 WAV，生成索引时要回读剧本和 cue 索引。

## 决策

- `--out`（或 `paths.out`）可以写成 `s3://bucket/prefix`。命令照常写到本地**暂存目录** `<cache>/s3-out/<bucket>/<prefix>`，写输出的命令（`plan`、`rip`、`unpack`）结束后再**发布**到 S3。暂存目录是缓存，可以随时删除。
- **增量上传**：暂存根目录下的台账（`.ripper-s3.json`）记录每个已上传对象的 SHA-256、大小和修改时间；未变化的文件不再上传。
- **发布顺序**保持"有 record 即完整"的约定：先传普通文件，再传 `_ripper.json`，然后是 `_index`、episode 索引，最后是 `ripper.lock.json`。命令中途失败时也会发布，已写入暂存的 bundle 都是完整的。
- **回填**：暂存里没有某个 bundle 的 record 时，先从 S3 取回 record 和该 bundle 的 JSON 文件（后续步骤要回读的只有 JSON），其余文件在台账中登记为"远端已有"。这样换一台机器也不会重复解包、重复上传，本地也只需要保留 JSON。
- **不删除远端对象**。bundle 重新解包后，旧版本多出来的文件会留在 S3 上；以 record 列出的文件为准。
- **签名**用 `rusty-s3`（SigV4 预签名，sans-IO），HTTP 沿用 reqwest + rustls/ring（ADR-0010），不引入 aws-lc 或 AWS SDK。
- **凭据只从环境变量读取**：`AWS_ACCESS_KEY_ID`、`AWS_SECRET_ACCESS_KEY`、`AWS_SESSION_TOKEN`，不写进配置文件。端点、区域、寻址方式可以写在 `[s3]`，也可以用 `AWS_ENDPOINT_URL(_S3)`、`AWS_REGION`、`S3_ADDRESSING_STYLE`。自定义端点默认用 path-style。

## 后果

- 正面：library 可以直接放在对象存储上共享；本地只需要暂存，换机器时代价很小。
- 负面：首次发布时暂存里仍有完整的一份本地副本；远端不会自动清理过期文件。

## 复审条件

需要对象存储上的原子发布（例如按版本前缀切换），或要清理过期对象时。
