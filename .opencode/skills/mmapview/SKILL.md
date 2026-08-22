---
name: mmapview
description: Headless inspection of .mms files — structure discovery (describe), JSONL queries (q) with filters/slices/aggregations, limit pushdown and byte guards; made for agents
---

## What I do

- Discover a `.mms` file's structure without reading data: `mmapview describe`
- Extract values / filtered rows / counts via DSL queries: `mmapview q`
- Output JSONL with automatic summarization of huge containers

## When to use me

Any time you need data **from** an `.mms` file: lookup, filter, count,
sample, drill-down. For on-screen visual checks see the `mmapview-tui` skill.

Schema is resolved automatically: embedded v2 header → `.mmshdr` sidecar →
registry (`~/.config/mmapstruct/schemas/`). Override with `--schema FILE`.

## Build

```bash
cargo build -p mmapview          # target/debug/mmapview
```

## Workflow for an unknown file

1. `mmapview describe F` → paths + types + array lengths (O(1), no bulk reads)
2. `mmapview q F '<path> | count()'` → container sizes
3. Pointed slices/filters: `q F '<path>[0:5]'`, `q F '<path>[?field=v]'`
4. Only then `--full`, if truly needed

`describe` output lines mirror DSL paths — copy them straight into queries:

```
/: ModelConfig
  /num_hidden_layers: u32
  /mlp_layer_types: [MlpLayerType] (pod) len=78
    [0]: MlpLayerType (enum)
```

## Queries (`q`) — output is JSONL by default

```bash
mmapview q FILE.mms '/items[0]/name'            # exact path — O(1), no scan
mmapview q FILE.mms '/items[0:10]'              # slice
mmapview q FILE.mms '/items[?price>100]'
mmapview q FILE.mms '/items[?name~sensor]'      # contains; also ^pre $suf
mmapview q FILE.mms '/items[?x>1&y<5|z=3]'      # AND (&) / OR (|)
mmapview q FILE.mms '/items | count()'          # count / avg / min / max(.field)
mmapview q FILE.mns --count-only '/items[?f=1]'
```

Large containers are summarized instead of dumped:

```json
{"points": {"$array": {"len": 1000000, "elem": "PodPoint", "layout": "pod",
            "preview": [{"id": 0}], "hint": "slice with [i:j] ..."}}}
```

Drill in with `[i]` / `[i:j]` — the middle is never materialized.

## Flags

```bash
--limit N        max candidate results for filters/slices (default 200;
                 early exit — the tail is never scanned)
--skip N         skip first N candidates
--count-only     print only the match count (scans without limit)
--max-string N   truncate strings to N chars (default 120)
--max-bytes N    stop serialized output after N bytes (default 1 MiB; 0 = off);
                 emits a final {"$meta":{"truncated":true,...}} line
--full           expand containers fully instead of summarizing
--text           legacy plain-text output instead of JSONL
--schema FILE    explicit schema path
```

## Guardrails worth knowing

- Exact index paths (`/items[5]`) are never limited.
- Aggregations (`| count()`) always scan the whole container regardless of limits.
- The byte budget applies even to `--full`.
- Pipelines are approximate under limits: `/items[a:b] | select(...)` may
  return fewer than `--limit` rows.

For visual tree inspection of the same files, use the `mmapview-tui` skill.
