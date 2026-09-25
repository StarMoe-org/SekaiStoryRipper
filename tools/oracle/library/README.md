# library oracle：用 UnityPy 交叉验证 `ripper unpack` 的输出

```bash
ripper --cache cache --out out unpack <bundle…>
uv run tools/oracle/library/compare.py cache out/library --report cache/library-report.json
```

对每个带 `_ripper.json` 的 bundle，用 UnityPy 加载缓存里的同一份 bundle（`cache/bundles/<name>.<crc>`），按 path id 逐个比较：

- typetree JSON：浮点按 f32 比较；unity-rs 的 `{key, value}` 与 UnityPy 的二元组视为等价；
- sse-motion：StreamedClip 分段系数与常量逐位比较，并比较事件；
- PNG：逐像素比较；Alpha8 格式只比 alpha，因为只有 alpha 通道有数据；
- TextAsset：逐字节比较。

退出码为 0 表示全部通过。只输出计数和差异，不写出任何资产内容（ADR-0005）。
