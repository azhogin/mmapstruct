# Показывать размер контейнеров в mmapview-tui

## Мотивация

При работе с большими `.mms`-файлами (сотни МБ — ГБ) хочется видеть, сколько памяти занимает каждый контейнер (Vec, HashMap, FixedArray и т.д.) в дереве. Размер уже есть — это `range` (limit - offset) у SchemaValue. Осталось отформатировать и показать.

## Что показывать

Для любого значения-контейнера (контейнер = узел, у которого есть дети/элементы) добавить в заголовок строку вида:

```
rows: Vec<[u32; 4]>  [3 elem, 48 B]
mats: Vec<[[u32;4];4]>  [2 elem, 128 B]
matrix: [[u32; 16]; 16]  [256 B]
label: "hello"
```

Формат:
- `< 1 KB` → в байтах: `X B`
- 1–1023 KB → в килобайтах: `X.XX KiB`
- 1–1023 MB → в мегабайтах: `X.XX MiB`
- ≥ 1 GB → в гигабайтах: `X.XX GiB`
- ≥ 1 TB → в терабайтах: `X.XX TiB`

Показывать размер **всегда** для контейнеров (даже для маленьких — чтобы было единообразно). Для скаляров (u32, f32, bool) и строк размер не показывать (они и так очевидны из типа).

## Где менять

`mmapview-tui/src/print.rs` — при форматировании узла:

```rust
fn format_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KiB", "MiB", "GiB", "TiB"];
    let mut size = bytes as f64;
    let mut unit_idx = 0;
    while size >= 1024.0 && unit_idx < UNITS.len() - 1 {
        size /= 1024.0;
        unit_idx += 1;
    }
    if unit_idx == 0 {
        format!("{} {}", bytes, UNITS[unit_idx])
    } else {
        format!("{:.2} {}", size, UNITS[unit_idx])
    }
}
```

Вызов — при формировании `label` для узла, если `schema_value.is_container()` (или эквивалентный признак — есть `SchemaNode::children` или `range.len() > 0`).

## Пример вывода

Было:
```
MyFile
  points: PodArray<Point3d> [1000000 elem]
  labels: Vec<String> [1000000 elem]
```

Стало:
```
MyFile
  points: PodArray<Point3d> [1000000 elem, 76.29 MiB]
  labels: Vec<String> [1000000 elem, 48.62 MiB]
```
