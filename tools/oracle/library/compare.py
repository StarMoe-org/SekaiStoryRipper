# /// script
# requires-python = ">=3.11"
# dependencies = ["UnityPy==1.25.*", "numpy"]
# ///
"""Cross-check `ripper unpack` output (the library) against UnityPy.

    uv run tools/oracle/library/compare.py <ripper cache dir> <library dir> [--report r.json] [--limit N]

For every bundle in the library that has a `_ripper.json`, the matching cached bundle
(`<cache>/bundles/<bundleName>.<crc:08x>`) is loaded with UnityPy and each recorded file is
compared with the object it came from (by path id):
  typetree  MonoBehaviour/other typetree JSON, floats at f32 precision
  motion    AnimationClip -> sse-motion: StreamedClip segments and constants bit-exact, events
  png       Texture2D pixels
  text      TextAsset bytes
Only counts and diffs are printed; no asset content is written.
"""
import argparse
import glob
import io
import json
import os
import struct
import sys
import warnings

import numpy as np
import UnityPy

warnings.filterwarnings("ignore")
UnityPy.config.FALLBACK_UNITY_VERSION = "2022.3.62f3"


def f32(x):
    return struct.unpack("<I", struct.pack("<f", float(x)))[0]


def same(a, b):
    # unity-rs renders map entries as {"key", "value"}; UnityPy as (key, value) tuples.
    if isinstance(a, dict) and a.keys() == {"key", "value"} and isinstance(b, (list, tuple)) and len(b) == 2:
        return same(a["key"], b[0]) and same(a["value"], b[1])
    # ... and pairs as {"first", "second"}.
    if isinstance(a, dict) and a.keys() == {"first", "second"} and isinstance(b, (list, tuple)) and len(b) == 2:
        return same(a["first"], b[0]) and same(a["second"], b[1])
    if isinstance(a, bool) or isinstance(b, bool):
        return bool(a) == bool(b)
    if isinstance(a, (int, float)) and isinstance(b, (int, float)):
        return a == b if isinstance(a, int) and isinstance(b, int) else f32(a) == f32(b)
    if isinstance(a, dict) and isinstance(b, dict):
        return a.keys() == b.keys() and all(same(a[k], b[k]) for k in a)
    if isinstance(a, (list, tuple)) and isinstance(b, (list, tuple)):
        return len(a) == len(b) and all(same(x, y) for x, y in zip(a, b))
    if isinstance(a, (bytes, bytearray)) or isinstance(b, (bytes, bytearray)):
        return list(a) == list(b)
    return a == b


def decode_streamed(words):
    raw = struct.pack(f"<{len(words)}I", *words)
    p, frames = 0, []
    while p < len(raw):
        t, n = struct.unpack_from("<fi", raw, p)
        p += 8
        keys = []
        for _ in range(n):
            i, a, b, c, d = struct.unpack_from("<i4f", raw, p)
            p += 20
            keys.append((i, (a, b, c, d)))
        frames.append((t, keys))
    return frames


def check_motion(tree, motion):
    data = tree["m_MuscleClip"]["m_Clip"]["data"]
    streamed = data["m_StreamedClip"]
    expected = [[] for _ in range(streamed["curveCount"])]
    for t, keys in decode_streamed(streamed["data"]):
        if t != float("inf"):
            for i, c in keys:
                expected[i].append((t, c))
    curves = motion["curves"]
    constants = list(data["m_ConstantClip"]["data"])
    offset = len(expected) + data["m_DenseClip"]["m_CurveCount"]
    if len(curves) != offset + len(constants):
        return False
    for i, segs in enumerate(expected):
        got = curves[i].get("segments", [])
        if len(got) != len(segs) or any(
            f32(g["t"]) != f32(t) or [f32(x) for x in g["coeff"]] != [f32(x) for x in c] for g, (t, c) in zip(got, segs)
        ):
            return False
    if any(f32(curves[offset + j].get("value", float("nan"))) != f32(v) for j, v in enumerate(constants)):
        return False
    events = tree["m_Events"]
    return len(events) == len(motion["events"]) and all(
        f32(e["time"]) == f32(m["time"]) and e["data"] == m["data"] for e, m in zip(events, motion["events"])
    )


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("cache")
    ap.add_argument("library")
    ap.add_argument("--report")
    ap.add_argument("--limit", type=int, default=0)
    args = ap.parse_args()
    results = {}

    def add(kind, ok, detail):
        entry = results.setdefault(kind, {"pass": 0, "fail": 0, "failures": []})
        entry["pass" if ok else "fail"] += 1
        if not ok and len(entry["failures"]) < 20:
            entry["failures"].append(detail)

    records = sorted(glob.glob(os.path.join(args.library, "**", "_ripper.json"), recursive=True))
    if args.limit:
        records = records[: args.limit]
    for record_path in records:
        record = json.load(open(record_path))
        bundle_dir = os.path.dirname(record_path)
        cached = os.path.join(args.cache, "bundles", *record["bundle"].split("/"))
        cached = f"{cached}.{record['crc']:08x}"
        if not os.path.exists(cached):
            add("bundle", False, f"{record['bundle']}: not in cache")
            continue
        env = UnityPy.load(open(cached, "rb").read())
        by_id = {o.path_id: o for o in env.objects}
        for entry in record["files"]:
            obj = by_id.get(entry["pathId"])
            path = os.path.join(bundle_dir, *entry["path"].split("/"))
            kind = entry["kind"]
            label = f"{record['bundle']}:{entry['path']}"
            if obj is None:
                if kind not in ("acb-index", "acb-tables", "audio", "movie-stream", "movie-audio", "object-graph"):
                    add(kind, False, f"{label}: path id {entry['pathId']} not in bundle")
                continue
            try:
                if kind == "typetree":
                    add(kind, same(json.load(open(path)), obj.read_typetree()), label)
                elif kind == "motion":
                    add(kind, check_motion(obj.read_typetree(), json.load(open(path))), label)
                elif kind == "png":
                    from PIL import Image

                    ours = np.asarray(Image.open(path).convert("RGBA"))
                    texture = obj.read()
                    theirs = np.asarray(texture.image.convert("RGBA"))
                    if int(texture.m_TextureFormat) == 1:
                        # Alpha8: only alpha carries data (ripper writes white, UnityPy black RGB).
                        ours, theirs = ours[..., 3], theirs[..., 3]
                    add(kind, ours.shape == theirs.shape and np.array_equal(ours, theirs), label)
                elif kind == "text":
                    script = obj.read().m_Script
                    raw = script.encode("utf-8", "surrogateescape") if isinstance(script, str) else bytes(script)
                    add(kind, open(path, "rb").read() == raw, label)
            except Exception as error:  # noqa: BLE001 - report and continue
                add(kind, False, f"{label}: {error!r}")
    width = max((len(k) for k in results), default=4)
    for kind, entry in sorted(results.items()):
        print(f"{kind:<{width}}  {'PASS' if entry['fail'] == 0 else 'FAIL'}  {entry['pass']} ok / {entry['fail']} failed")
        for failure in entry["failures"][:5]:
            print(f"    - {failure}")
    if args.report:
        json.dump(results, open(args.report, "w"), indent=1)
    sys.exit(0 if all(e["fail"] == 0 for e in results.values()) else 1)


if __name__ == "__main__":
    main()
