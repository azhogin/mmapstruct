use crate::parser::{AggregateFunc, Expr, FilterExpr, FilterOp, FilterValue};
use crate::query_context::QueryContext;
use crate::EvalResult;

/// Limits for query evaluation (pushdown of CLI `--skip` / `--limit`).
///
/// The budget counts **candidate items** emitted by multi-result operations
/// (`[?filter]`, slices, `[a:b]`). Exact index paths (`/items[5]`) are never
/// counted or blocked, and aggregations (`count()` etc.) always scan the full
/// container regardless of limits.
///
/// For pipelines the budget is approximate: it is consumed by intermediate
/// steps, so `/items[a:b] | select(...)` may yield fewer than `max` rows.
#[derive(Clone, Copy, Debug)]
pub struct QueryLimits {
    /// Number of initial candidate items to skip.
    pub skip: usize,
    /// Maximum number of candidate items to emit (`None` = unlimited).
    pub max: Option<usize>,
}

impl Default for QueryLimits {
    fn default() -> Self {
        Self {
            skip: 0,
            max: None,
        }
    }
}

impl QueryLimits {
    /// Recommended default cap for AI-facing tools (`--limit`).
    pub const DEFAULT_MAX: usize = 200;
}

#[derive(Default)]
struct Budget {
    to_skip: usize,
    remaining: Option<usize>,
}

impl Budget {
    fn new(limits: &QueryLimits) -> Self {
        Self {
            to_skip: limits.skip,
            remaining: limits.max,
        }
    }

    /// Returns `true` when the next candidate item should be emitted.
    /// Stops admitting once `skip + max` candidates have been seen.
    fn admit(&mut self) -> bool {
        if self.to_skip > 0 {
            self.to_skip -= 1;
            return false;
        }
        match self.remaining.as_mut() {
            Some(0) => false,
            Some(n) => {
                *n -= 1;
                true
            }
            None => true,
        }
    }

    fn exhausted(&self) -> bool {
        self.to_skip == 0 && self.remaining == Some(0)
    }
}

/// Parse and evaluate a query expression against a root value.
///
/// Generic over any [`QueryContext`] implementation.
pub fn interpret<'a, C: QueryContext<'a>>(
    expr: &Expr<'_>,
    ctx: &C,
    root: &C::Val,
) -> Result<Vec<EvalResult<C::Val>>, String> {
    let mut budget = Budget::default();
    eval_expr(expr, ctx, root, &mut budget)
}

/// Like [`interpret`] but stops emitting multi-result items once
/// [`QueryLimits`] are exhausted (early exit — the rest is never scanned).
pub fn interpret_with_limits<'a, C: QueryContext<'a>>(
    expr: &Expr<'_>,
    ctx: &C,
    root: &C::Val,
    limits: &QueryLimits,
) -> Result<Vec<EvalResult<C::Val>>, String> {
    let mut budget = Budget::new(limits);
    eval_expr(expr, ctx, root, &mut budget)
}

fn unwrap_buf<V>(v: &EvalResult<V>) -> Result<&V, String> {
    match v {
        EvalResult::Buf(v) => Ok(v),
        _ => Err("path navigation requires a buffer-backed value".into()),
    }
}

fn eval_expr<'a, C: QueryContext<'a>>(
    expr: &Expr<'_>,
    ctx: &C,
    current: &C::Val,
    budget: &mut Budget,
) -> Result<Vec<EvalResult<C::Val>>, String> {
    match expr {
        Expr::Path(segs) => {
            let mut vals: Vec<EvalResult<C::Val>> = vec![EvalResult::Buf(*current)];
            for seg in segs {
                match seg {
                    crate::parser::Segment::Field(name) => {
                        vals = vals
                            .into_iter()
                            .map(|v| {
                                let inner = unwrap_buf(&v)?;
                                ctx.field_get(inner, name)
                                    .ok_or_else(|| format!("no such field `{}`", name))
                                    .map(EvalResult::Buf)
                            })
                            .collect::<Result<Vec<_>, _>>()?;
                    }
                    crate::parser::Segment::Filter(filter) => {
                        let mut filtered = Vec::new();
                        for v in &vals {
                            let inner = unwrap_buf(v)?;
                            let batch = filter_array(ctx, &[*inner], filter, budget)?;
                            filtered.extend(batch.into_iter().map(EvalResult::Buf));
                        }
                        vals = filtered;
                    }
                    crate::parser::Segment::Index(idx) => {
                        vals = vals
                            .into_iter()
                            .map(|v| {
                                let inner = unwrap_buf(&v)?;
                                index_array(ctx, inner, *idx).map(EvalResult::Buf)
                            })
                            .collect::<Result<Vec<_>, _>>()?;
                    }
                    crate::parser::Segment::All => {}
                    crate::parser::Segment::Slice(s, e) => {
                        let mut new_vals = Vec::new();
                        for v in vals {
                            let inner = unwrap_buf(&v)?;
                            for elem in slice_array(ctx, inner, *s, *e, budget)? {
                                new_vals.push(EvalResult::Buf(elem));
                            }
                        }
                        vals = new_vals;
                    }
                }
            }
            Ok(vals)
        }
        Expr::Filter(filter) => {
            filter_array(ctx, &[*current], filter, budget)
                .map(|v| v.into_iter().map(EvalResult::Buf).collect())
        }
        Expr::Index(idx) => index_array(ctx, current, *idx).map(|v| vec![EvalResult::Buf(v)]),
        Expr::All => Ok(vec![EvalResult::Buf(*current)]),
        Expr::Slice(s, e) => slice_array(ctx, current, *s, *e, budget)
            .map(|v| v.into_iter().map(EvalResult::Buf).collect()),
        Expr::Projection(fields) => {
            let mut result = Vec::new();
            for f in fields {
                let v = ctx
                    .field_get(current, f)
                    .ok_or_else(|| format!("no such field `{}`", f))?;
                result.push(EvalResult::Buf(v));
            }
            Ok(result)
        }
        Expr::Pipe(left, right) => {
            let left_val = eval_expr(left, ctx, current, budget)?;
            let mut all = Vec::new();
            for v in left_val {
                let inner = unwrap_buf(&v)?;
                let r = eval_expr(right, ctx, inner, budget)?;
                all.extend(r);
            }
            Ok(all)
        }
        Expr::Select { filter } => {
            if ctx.is_array_type(current) {
                let filtered =
                    filter_array(ctx, &[*current], filter, budget).unwrap_or_default();
                Ok(filtered.into_iter().map(EvalResult::Buf).collect())
            } else {
                if eval_filter(ctx, current, filter) {
                    Ok(vec![EvalResult::Buf(*current)])
                } else {
                    Ok(vec![])
                }
            }
        }
        Expr::Aggregate { func } => aggregate(ctx, current, func),
    }
}

fn filter_array<'a, C: QueryContext<'a>>(
    ctx: &C,
    vals: &[C::Val],
    filter: &crate::parser::FilterExpr<'_>,
    budget: &mut Budget,
) -> Result<Vec<C::Val>, String> {
    let mut filtered: Vec<C::Val> = Vec::new();
    'outer: for dv in vals {
        let len = ctx.array_len(dv);
        for i in 0..len {
            if let Some(elem) = ctx.array_get(dv, i) {
                if eval_filter(ctx, &elem, filter) && budget.admit() {
                    filtered.push(elem);
                    if budget.exhausted() {
                        break 'outer;
                    }
                }
            }
        }
    }
    Ok(filtered)
}

fn aggregate<'a, C: QueryContext<'a>>(
    ctx: &C,
    val: &C::Val,
    func: &AggregateFunc<'_>,
) -> Result<Vec<EvalResult<C::Val>>, String> {
    if !ctx.is_array_type(val) {
        return Err("aggregation requires an array value".into());
    }
    let len = ctx.array_len(val);
    match func {
        AggregateFunc::Count => Ok(vec![EvalResult::U64(len as u64)]),
        AggregateFunc::Avg { field } => {
            let (sum, count) = sum_field(ctx, val, field)?;
            if count == 0 {
                return Err("avg of empty set".into());
            }
            Ok(vec![EvalResult::F64(sum / count as f64)])
        }
        AggregateFunc::Min { field } => {
            let mut min_val: Option<f64> = None;
            for i in 0..len {
                if let Some(elem) = ctx.array_get(val, i) {
                    if let Some(fv) = ctx.field_get(&elem, field) {
                        if let Some(n) = ctx.read_as_f64(&fv) {
                            if min_val.map_or(true, |m| n < m) {
                                min_val = Some(n);
                            }
                        }
                    }
                }
            }
            Ok(vec![EvalResult::F64(min_val.unwrap_or(f64::NAN))])
        }
        AggregateFunc::Max { field } => {
            let mut max_val: Option<f64> = None;
            for i in 0..len {
                if let Some(elem) = ctx.array_get(val, i) {
                    if let Some(fv) = ctx.field_get(&elem, field) {
                        if let Some(n) = ctx.read_as_f64(&fv) {
                            if max_val.map_or(true, |m| n > m) {
                                max_val = Some(n);
                            }
                        }
                    }
                }
            }
            Ok(vec![EvalResult::F64(max_val.unwrap_or(f64::NAN))])
        }
    }
}

fn sum_field<'a, C: QueryContext<'a>>(
    ctx: &C,
    val: &C::Val,
    field: &str,
) -> Result<(f64, usize), String> {
    let len = ctx.array_len(val);
    let mut sum = 0.0f64;
    let mut count = 0usize;
    for i in 0..len {
        if let Some(elem) = ctx.array_get(val, i) {
            if let Some(fv) = ctx.field_get(&elem, field) {
                if let Some(n) = ctx.read_as_f64(&fv) {
                    sum += n;
                    count += 1;
                }
            }
        }
    }
    Ok((sum, count))
}

fn index_array<'a, C: QueryContext<'a>>(
    ctx: &C,
    val: &C::Val,
    idx: usize,
) -> Result<C::Val, String> {
    ctx.array_get(val, idx)
        .ok_or_else(|| format!("index {} out of bounds", idx))
}

fn slice_array<'a, C: QueryContext<'a>>(
    ctx: &C,
    val: &C::Val,
    start: usize,
    end: usize,
    budget: &mut Budget,
) -> Result<Vec<C::Val>, String> {
    let len = ctx.array_len(val);
    let s = start.min(len);
    let e = end.min(len);
    let mut sliced = Vec::new();
    for i in s..e {
        if budget.exhausted() {
            break;
        }
        if let Some(elem) = ctx.array_get(val, i) {
            if budget.admit() {
                sliced.push(elem);
            }
        }
    }
    Ok(sliced)
}

fn eval_filter<'a, C: QueryContext<'a>>(
    ctx: &C,
    element: &C::Val,
    filter: &FilterExpr<'_>,
) -> bool {
    match filter {
        FilterExpr::Simple { field, op, value } => ctx
            .field_get(element, field)
            .map(|fv| compare_values(ctx, &fv, op, value))
            .unwrap_or(false),
        FilterExpr::And(a, b) => eval_filter(ctx, element, a) && eval_filter(ctx, element, b),
        FilterExpr::Or(a, b) => eval_filter(ctx, element, a) || eval_filter(ctx, element, b),
    }
}

fn compare_values<'a, C: QueryContext<'a>>(
    ctx: &C,
    fv: &C::Val,
    op: &FilterOp,
    expected: &FilterValue<'_>,
) -> bool {
    match (op, expected) {
        (FilterOp::Eq, FilterValue::Int(b)) => {
            ctx.read_as_i64(fv).map(|a| a == *b).unwrap_or(false)
        }
        (FilterOp::Eq, FilterValue::Float(b)) => ctx
            .read_as_f64(fv)
            .map(|a| (a - *b).abs() < f64::EPSILON)
            .unwrap_or(false),
        (FilterOp::Eq, FilterValue::Str(b)) => {
            if ctx.is_str_type(fv) {
                ctx.read_str(fv) == *b
            } else {
                false
            }
        }
        (FilterOp::Ne, _) => !compare_values(ctx, fv, &FilterOp::Eq, expected),

        (FilterOp::Gt, FilterValue::Int(b)) => ctx.read_as_i64(fv).map(|a| a > *b).unwrap_or(false),
        (FilterOp::Gt, FilterValue::Float(b)) => {
            ctx.read_as_f64(fv).map(|a| a > *b).unwrap_or(false)
        }

        (FilterOp::Ge, _) => {
            compare_values(ctx, fv, &FilterOp::Eq, expected)
                || compare_values(ctx, fv, &FilterOp::Gt, expected)
        }

        (FilterOp::Lt, _) => !compare_values(ctx, fv, &FilterOp::Ge, expected),

        (FilterOp::Le, _) => !compare_values(ctx, fv, &FilterOp::Gt, expected),

        (FilterOp::Contains, FilterValue::Str(b)) => ctx
            .read_str_opt(fv)
            .map(|s| s.contains(*b))
            .unwrap_or(false),
        (FilterOp::Contains, FilterValue::Int(b)) => ctx
            .read_str_opt(fv)
            .map(|s| s.contains(&b.to_string()))
            .unwrap_or(false),
        (FilterOp::Contains, FilterValue::Float(b)) => ctx
            .read_str_opt(fv)
            .map(|s| s.contains(&b.to_string()))
            .unwrap_or(false),

        (FilterOp::StartsWith, FilterValue::Str(b)) => ctx
            .read_str_opt(fv)
            .map(|s| s.starts_with(*b))
            .unwrap_or(false),
        (FilterOp::StartsWith, FilterValue::Int(b)) => ctx
            .read_str_opt(fv)
            .map(|s| s.starts_with(&b.to_string()))
            .unwrap_or(false),
        (FilterOp::StartsWith, FilterValue::Float(b)) => ctx
            .read_str_opt(fv)
            .map(|s| s.starts_with(&b.to_string()))
            .unwrap_or(false),

        (FilterOp::EndsWith, FilterValue::Str(b)) => ctx
            .read_str_opt(fv)
            .map(|s| s.ends_with(*b))
            .unwrap_or(false),
        (FilterOp::EndsWith, FilterValue::Int(b)) => ctx
            .read_str_opt(fv)
            .map(|s| s.ends_with(&b.to_string()))
            .unwrap_or(false),
        (FilterOp::EndsWith, FilterValue::Float(b)) => ctx
            .read_str_opt(fv)
            .map(|s| s.ends_with(&b.to_string()))
            .unwrap_or(false),

        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query_from;
    use mmapstruct::{SchemaCtx, GetSchema, MmapStruct};

    #[derive(MmapStruct, Debug)]
    #[mmap(scope)]
    struct Record {
        id: u32,
        name: String,
    }

    #[test]
    fn test_field_access() {
        let r = Record {
            id: 42,
            name: "hello".into(),
        };
        let buf = mmapstruct::write_to_buf_raw(&r);
        let ctx = SchemaCtx::new(&buf);
        let root = <Record as GetSchema>::root_value(buf.len() as u64);
        let result = query_from("/name", &ctx, &root).unwrap();
        assert_eq!(result.len(), 1);
        match &result[0] {
            EvalResult::Buf(v) => assert_eq!(ctx.read_str(v), "hello"),
            _ => panic!("expected Buf"),
        }
    }

    #[test]
    fn test_root_slash() {
        let r = Record {
            id: 42,
            name: "hello".into(),
        };
        let buf = mmapstruct::write_to_buf_raw(&r);
        let ctx = SchemaCtx::new(&buf);
        let root = <Record as GetSchema>::root_value(buf.len() as u64);
        let result = query_from("/", &ctx, &root).unwrap();
        assert_eq!(result.len(), 1);
        let EvalResult::Buf(ref v) = result[0] else {
            panic!("expected Buf")
        };
        let id = ctx.field_get(v, "id").unwrap();
        assert_eq!(ctx.read_u32(&id), 42);
    }

    #[test]
    fn test_negative_float_filter() {
        use mmapstruct::{write_to_buf_raw, SchemaCtx, GetSchema, MmapStruct};

        #[derive(
            MmapStruct, Debug, PartialEq, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable, Default,
        )]
        #[repr(C)]
        struct Point3d {
            x: f32,
            y: f32,
            z: f32,
        }

        #[derive(MmapStruct, Debug, PartialEq)]
        struct VecPod3d {
            #[pod]
            points: Vec<Point3d>,
        }

        let points = vec![
            Point3d {
                x: -1.0,
                y: 2.0,
                z: 3.0,
            },
            Point3d {
                x: 0.5,
                y: 5.0,
                z: 6.0,
            },
            Point3d {
                x: -2.5,
                y: 7.0,
                z: 8.0,
            },
        ];
        let s = VecPod3d { points };
        let buf = write_to_buf_raw(&s);
        let ctx = SchemaCtx::new(&buf);
        let root = <VecPod3d as GetSchema>::root_value(buf.len() as u64);

        // Filter: x > -1.5 (negative float comparison)
        let result = query_from("/points[?x>-1.5]", &ctx, &root).unwrap();
        assert_eq!(result.len(), 2); // -1.0 and 0.5, not -2.5

        // Filter: x < -0.5
        let result = query_from("/points[?x<-0.5]", &ctx, &root).unwrap();
        assert_eq!(result.len(), 2); // -1.0 and -2.5

        // Filter: x > -0.5
        let result = query_from("/points[?x>-.5]", &ctx, &root).unwrap();
        assert_eq!(result.len(), 1); // 0.5
    }

    #[test]
    fn test_leading_dot_float_filter() {
        use mmapstruct::{write_to_buf_raw, SchemaCtx, GetSchema, MmapStruct};

        #[derive(
            MmapStruct, Debug, PartialEq, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable, Default,
        )]
        #[repr(C)]
        struct Point3d {
            x: f32,
            y: f32,
            z: f32,
        }

        #[derive(MmapStruct, Debug, PartialEq)]
        struct VecPod3d {
            #[pod]
            points: Vec<Point3d>,
        }

        let points = vec![
            Point3d {
                x: 0.5,
                y: 2.0,
                z: 3.0,
            },
            Point3d {
                x: 1.5,
                y: 2.0,
                z: 3.0,
            },
        ];
        let s = VecPod3d { points };
        let buf = write_to_buf_raw(&s);
        let ctx = SchemaCtx::new(&buf);
        let root = <VecPod3d as GetSchema>::root_value(buf.len() as u64);

        // Filter: x > .75 (leading-dot float)
        let result = query_from("/points[?x>.75]", &ctx, &root).unwrap();
        assert_eq!(result.len(), 1);
        let EvalResult::Buf(ref v) = result[0] else {
            panic!("expected Buf")
        };
        let x = ctx.field_get(v, "x").unwrap();
        assert_eq!(ctx.read_f32(&x), 1.5);
    }

    #[test]
    fn test_filter_gt() {
        #[derive(MmapStruct, Debug)]
        struct RecordFile {
            items: Vec<Record>,
        }

        let records = vec![
            Record {
                id: 1,
                name: "a".into(),
            },
            Record {
                id: 5,
                name: "b".into(),
            },
            Record {
                id: 10,
                name: "c".into(),
            },
        ];
        let w = RecordFile { items: records };
        let buf = mmapstruct::write_to_buf_raw(&w);
        let ctx = SchemaCtx::new(&buf);
        let root = <RecordFile as GetSchema>::root_value(ctx.data().len() as u64);
        let result = query_from("/items[?id>3]", &ctx, &root).unwrap();
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_filter_contains() {
        #[derive(MmapStruct, Debug)]
        struct RecordFile {
            items: Vec<Record>,
        }

        let records = vec![
            Record {
                id: 1,
                name: "sensor_a".into(),
            },
            Record {
                id: 2,
                name: "actuator_b".into(),
            },
            Record {
                id: 3,
                name: "sensor_c".into(),
            },
        ];
        let w = RecordFile { items: records };
        let buf = mmapstruct::write_to_buf_raw(&w);
        let ctx = SchemaCtx::new(&buf);
        let root = <RecordFile as GetSchema>::root_value(ctx.data().len() as u64);
        let result = query_from("/items[?name~sensor]", &ctx, &root).unwrap();
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_filter_gt_via_outer_struct() {
        #[derive(MmapStruct, Debug)]
        struct RecordFile {
            items: Vec<Record>,
        }

        let records = vec![
            Record {
                id: 1,
                name: "a".into(),
            },
            Record {
                id: 5,
                name: "b".into(),
            },
            Record {
                id: 10,
                name: "c".into(),
            },
        ];
        let w = RecordFile { items: records };
        let buf = mmapstruct::write_to_buf_raw(&w);
        let ctx = SchemaCtx::new(&buf);
        let root = <RecordFile as GetSchema>::root_value(ctx.data().len() as u64);
        let result = query_from("/items[?id>3]", &ctx, &root).unwrap();
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_filter_contains_via_outer_struct() {
        #[derive(MmapStruct, Debug)]
        struct RecordFile {
            items: Vec<Record>,
        }

        let records = vec![
            Record {
                id: 1,
                name: "sensor_a".into(),
            },
            Record {
                id: 2,
                name: "actuator_b".into(),
            },
            Record {
                id: 3,
                name: "sensor_c".into(),
            },
        ];
        let w = RecordFile { items: records };
        let buf = mmapstruct::write_to_buf_raw(&w);
        let ctx = SchemaCtx::new(&buf);
        let root = <RecordFile as GetSchema>::root_value(ctx.data().len() as u64);
        let result = query_from("/items[?name~sensor]", &ctx, &root).unwrap();
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_filter_eq_via_outer_struct() {
        #[derive(MmapStruct, Debug)]
        struct RecordFile {
            items: Vec<Record>,
        }

        let records = vec![
            Record {
                id: 10,
                name: "target".into(),
            },
            Record {
                id: 20,
                name: "other".into(),
            },
            Record {
                id: 30,
                name: "target".into(),
            },
        ];
        let w = RecordFile { items: records };
        let buf = mmapstruct::write_to_buf_raw(&w);
        let ctx = SchemaCtx::new(&buf);
        let root = <RecordFile as GetSchema>::root_value(ctx.data().len() as u64);
        let result = query_from("/items[?name=target]", &ctx, &root).unwrap();
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_filter_pod_array() {
        use bytemuck;
        use mmapstruct::{write_to_buf_raw, SchemaCtx, GetSchema, MmapStruct};

        #[derive(
            MmapStruct, Debug, PartialEq, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable, Default,
        )]
        #[repr(C)]
        struct Point3d {
            x: f32,
            y: f32,
            z: f32,
        }

        #[derive(MmapStruct, Debug, PartialEq)]
        struct VecPod3d {
            #[pod]
            points: Vec<Point3d>,
        }

        let points = vec![
            Point3d {
                x: 1.0,
                y: 2.0,
                z: 3.0,
            },
            Point3d {
                x: 4.0,
                y: 5.0,
                z: 6.0,
            },
            Point3d {
                x: 1.5,
                y: 7.0,
                z: 8.0,
            },
        ];
        let s = VecPod3d { points };
        let buf = write_to_buf_raw(&s);
        let ctx = SchemaCtx::new(&buf);
        let root = <VecPod3d as GetSchema>::root_value(buf.len() as u64);

        // Filter: x > 3 (float literal comparison)
        let result = query_from("/points[?x>3.0]", &ctx, &root).unwrap();
        assert_eq!(result.len(), 1);
        let EvalResult::Buf(ref v) = result[0] else {
            panic!("expected Buf")
        };
        let x = ctx.field_get(v, "x").unwrap();
        assert_eq!(ctx.read_f32(&x), 4.0);

        // Filter: x < 2 (float literal)
        let result = query_from("/points[?x<2.0]", &ctx, &root).unwrap();
        assert_eq!(result.len(), 2);

        // Filter: x > 1.0 and x < 3.0 (should match 1.5)
        let result = query_from("/points[?x>1.0]", &ctx, &root).unwrap();
        assert_eq!(result.len(), 2);
        let result = query_from("/points[?x<3.0]", &ctx, &root).unwrap();
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_field_access_pod_array_elements() {
        use bytemuck;
        use mmapstruct::{write_to_buf_raw, SchemaCtx, GetSchema, MmapStruct};

        #[derive(
            MmapStruct, Debug, PartialEq, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable, Default,
        )]
        #[repr(C)]
        struct Point3d {
            x: f32,
            y: f32,
            z: f32,
        }

        #[derive(MmapStruct, Debug, PartialEq)]
        struct VecPod3d {
            #[pod]
            points: Vec<Point3d>,
        }

        let points = vec![
            Point3d {
                x: 1.0,
                y: 2.0,
                z: 3.0,
            },
            Point3d {
                x: 4.0,
                y: 5.0,
                z: 6.0,
            },
        ];
        let s = VecPod3d { points };
        let buf = write_to_buf_raw(&s);
        let ctx = SchemaCtx::new(&buf);

        let schema = <VecPod3d as GetSchema>::get_struct_schema();
        let root = <VecPod3d as GetSchema>::root_value(schema.header_size);

        let field = ctx.field_get(&root, "points").unwrap();
        assert_eq!(ctx.array_len(&field), 2);

        let p0 = ctx.array_get(&field, 0).unwrap();
        let x0 = ctx.field_get(&p0, "x").unwrap();
        assert_eq!(ctx.read_f32(&x0), 1.0);
        let y0 = ctx.field_get(&p0, "y").unwrap();
        assert_eq!(ctx.read_f32(&y0), 2.0);
        let z0 = ctx.field_get(&p0, "z").unwrap();
        assert_eq!(ctx.read_f32(&z0), 3.0);

        let p1 = ctx.array_get(&field, 1).unwrap();
        let x1 = ctx.field_get(&p1, "x").unwrap();
        assert_eq!(ctx.read_f32(&x1), 4.0);
        let y1 = ctx.field_get(&p1, "y").unwrap();
        assert_eq!(ctx.read_f32(&y1), 5.0);
        let z1 = ctx.field_get(&p1, "z").unwrap();
        assert_eq!(ctx.read_f32(&z1), 6.0);
    }

    #[test]
    fn test_select_filter_pipe() {
        use mmapstruct::{write_to_buf_raw, SchemaCtx, GetSchema, MmapStruct};

        #[derive(MmapStruct, Debug)]
        #[mmap(scope)]
        struct Record {
            id: u32,
            name: String,
        }

        #[derive(MmapStruct, Debug)]
        struct RecordFile {
            items: Vec<Record>,
        }

        let records = vec![
            Record {
                id: 1,
                name: "sensor_a".into(),
            },
            Record {
                id: 5,
                name: "actuator_b".into(),
            },
            Record {
                id: 10,
                name: "sensor_c".into(),
            },
        ];
        let w = RecordFile { items: records };
        let buf = write_to_buf_raw(&w);
        let ctx = SchemaCtx::new(&buf);
        let root = <RecordFile as GetSchema>::root_value(buf.len() as u64);

        // Pipe items into select with name~sensor filter
        let result = query_from("/items | select(.name~sensor)", &ctx, &root).unwrap();
        assert_eq!(result.len(), 2);

        // Filter with id>3
        let result = query_from("/items | select(.id>3)", &ctx, &root).unwrap();
        assert_eq!(result.len(), 2);

        // Filter with id=1
        let result = query_from("/items | select(.id=1)", &ctx, &root).unwrap();
        assert_eq!(result.len(), 1);
        let EvalResult::Buf(ref v) = result[0] else {
            panic!("expected Buf")
        };
        let id = ctx.field_get(v, "id").unwrap();
        assert_eq!(ctx.read_u32(&id), 1);
    }

    #[test]
    fn test_select_no_match() {
        use mmapstruct::{write_to_buf_raw, SchemaCtx, GetSchema, MmapStruct};

        #[derive(MmapStruct, Debug)]
        #[mmap(scope)]
        struct Record {
            id: u32,
            name: String,
        }

        #[derive(MmapStruct, Debug)]
        struct RecordFile {
            items: Vec<Record>,
        }

        let records = vec![Record {
            id: 1,
            name: "a".into(),
        }];
        let w = RecordFile { items: records };
        let buf = write_to_buf_raw(&w);
        let ctx = SchemaCtx::new(&buf);
        let root = <RecordFile as GetSchema>::root_value(buf.len() as u64);

        let result = query_from("/items | select(.id>999)", &ctx, &root).unwrap();
        assert_eq!(result.len(), 0);
    }

    // -----------------------------------------------------------------------
    // AND/OR filter tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_and_filter() {
        use mmapstruct::{write_to_buf_raw, SchemaCtx, GetSchema, MmapStruct};

        #[derive(MmapStruct, Debug)]
        #[mmap(scope)]
        struct Record {
            id: u32,
            name: String,
        }

        #[derive(MmapStruct, Debug)]
        struct RecordFile {
            items: Vec<Record>,
        }

        let records = vec![
            Record {
                id: 1,
                name: "sensor".into(),
            },
            Record {
                id: 5,
                name: "actuator".into(),
            },
            Record {
                id: 10,
                name: "sensor".into(),
            },
        ];
        let w = RecordFile { items: records };
        let buf = write_to_buf_raw(&w);
        let ctx = SchemaCtx::new(&buf);
        let root = <RecordFile as GetSchema>::root_value(buf.len() as u64);

        // AND: id>3 AND name~sensor
        let result = query_from("/items[?id>3&name~sensor]", &ctx, &root).unwrap();
        assert_eq!(result.len(), 1); // only id=10 has both
        let EvalResult::Buf(ref v) = result[0] else {
            panic!("expected Buf")
        };
        let id = ctx.field_get(v, "id").unwrap();
        assert_eq!(ctx.read_u32(&id), 10);
    }

    #[test]
    fn test_or_filter() {
        use mmapstruct::{write_to_buf_raw, SchemaCtx, GetSchema, MmapStruct};

        #[derive(MmapStruct, Debug)]
        #[mmap(scope)]
        struct Record {
            id: u32,
            name: String,
        }

        #[derive(MmapStruct, Debug)]
        struct RecordFile {
            items: Vec<Record>,
        }

        let records = vec![
            Record {
                id: 1,
                name: "sensor".into(),
            },
            Record {
                id: 5,
                name: "actuator".into(),
            },
            Record {
                id: 10,
                name: "target".into(),
            },
        ];
        let w = RecordFile { items: records };
        let buf = write_to_buf_raw(&w);
        let ctx = SchemaCtx::new(&buf);
        let root = <RecordFile as GetSchema>::root_value(buf.len() as u64);

        // OR: id>3 OR name~sensor
        let result = query_from("/items[?id>3|name~sensor]", &ctx, &root).unwrap();
        assert_eq!(result.len(), 3);
    }

    #[test]
    fn test_and_or_combined() {
        use mmapstruct::{write_to_buf_raw, SchemaCtx, GetSchema, MmapStruct};

        #[derive(MmapStruct, Debug)]
        #[mmap(scope)]
        struct Record {
            id: u32,
            name: String,
        }

        #[derive(MmapStruct, Debug)]
        struct RecordFile {
            items: Vec<Record>,
        }

        let records = vec![
            Record {
                id: 1,
                name: "sensor_a".into(),
            },
            Record {
                id: 5,
                name: "actuator".into(),
            },
            Record {
                id: 10,
                name: "sensor_b".into(),
            },
            Record {
                id: 20,
                name: "other".into(),
            },
        ];
        let w = RecordFile { items: records };
        let buf = write_to_buf_raw(&w);
        let ctx = SchemaCtx::new(&buf);
        let root = <RecordFile as GetSchema>::root_value(buf.len() as u64);

        // (id>3 AND name~sensor) OR id=20
        let result = query_from("/items[?id>3&name~sensor|id=20]", &ctx, &root).unwrap();
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_and_filter_pod_array() {
        use bytemuck;
        use mmapstruct::{write_to_buf_raw, SchemaCtx, GetSchema, MmapStruct};

        #[derive(
            MmapStruct, Debug, PartialEq, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable, Default,
        )]
        #[repr(C)]
        struct Point3d {
            x: f32,
            y: f32,
            z: f32,
        }

        #[derive(MmapStruct, Debug, PartialEq)]
        struct VecPod3d {
            #[pod]
            points: Vec<Point3d>,
        }

        let points = vec![
            Point3d {
                x: 1.0,
                y: 2.0,
                z: 3.0,
            },
            Point3d {
                x: 4.0,
                y: 5.0,
                z: 6.0,
            },
            Point3d {
                x: 2.0,
                y: 7.0,
                z: 8.0,
            },
        ];
        let s = VecPod3d { points };
        let buf = write_to_buf_raw(&s);
        let ctx = SchemaCtx::new(&buf);
        let root = <VecPod3d as GetSchema>::root_value(buf.len() as u64);

        // AND: x > 1 AND y < 6
        let result = query_from("/points[?x>1&y<6]", &ctx, &root).unwrap();
        assert_eq!(result.len(), 1);
    }

    #[test]
    fn test_or_filter_pod_array() {
        use bytemuck;
        use mmapstruct::{write_to_buf_raw, SchemaCtx, GetSchema, MmapStruct};

        #[derive(
            MmapStruct, Debug, PartialEq, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable, Default,
        )]
        #[repr(C)]
        struct Point3d {
            x: f32,
            y: f32,
            z: f32,
        }

        #[derive(MmapStruct, Debug, PartialEq)]
        struct VecPod3d {
            #[pod]
            points: Vec<Point3d>,
        }

        let points = vec![
            Point3d {
                x: 1.0,
                y: 2.0,
                z: 3.0,
            },
            Point3d {
                x: 4.0,
                y: 5.0,
                z: 6.0,
            },
            Point3d {
                x: 0.5,
                y: 7.0,
                z: 8.0,
            },
        ];
        let s = VecPod3d { points };
        let buf = write_to_buf_raw(&s);
        let ctx = SchemaCtx::new(&buf);
        let root = <VecPod3d as GetSchema>::root_value(buf.len() as u64);

        // OR: x > 3 OR x < 1
        let result = query_from("/points[?x>3|x<1]", &ctx, &root).unwrap();
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_select_and_filter() {
        use mmapstruct::{write_to_buf_raw, SchemaCtx, GetSchema, MmapStruct};

        #[derive(MmapStruct, Debug)]
        #[mmap(scope)]
        struct Record {
            id: u32,
            name: String,
        }

        #[derive(MmapStruct, Debug)]
        struct RecordFile {
            items: Vec<Record>,
        }

        let records = vec![
            Record {
                id: 1,
                name: "sensor".into(),
            },
            Record {
                id: 5,
                name: "actuator".into(),
            },
            Record {
                id: 10,
                name: "sensor".into(),
            },
        ];
        let w = RecordFile { items: records };
        let buf = write_to_buf_raw(&w);
        let ctx = SchemaCtx::new(&buf);
        let root = <RecordFile as GetSchema>::root_value(buf.len() as u64);

        // select with AND
        let result = query_from("/items | select(.id>3&.name~sensor)", &ctx, &root).unwrap();
        assert_eq!(result.len(), 1);
        let EvalResult::Buf(ref v) = result[0] else {
            panic!("expected Buf")
        };
        let id = ctx.field_get(v, "id").unwrap();
        assert_eq!(ctx.read_u32(&id), 10);
    }

    // -----------------------------------------------------------------------
    // Aggregate tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_count() {
        use mmapstruct::{write_to_buf_raw, SchemaCtx, GetSchema, MmapStruct};

        #[derive(MmapStruct, Debug)]
        #[mmap(scope)]
        struct Record {
            id: u32,
            _tag: String,
        }

        #[derive(MmapStruct, Debug)]
        struct RecordFile {
            items: Vec<Record>,
        }

        let records = vec![
            Record {
                id: 1,
                _tag: "a".into(),
            },
            Record {
                id: 5,
                _tag: "b".into(),
            },
            Record {
                id: 10,
                _tag: "c".into(),
            },
        ];
        let w = RecordFile { items: records };
        let buf = write_to_buf_raw(&w);
        let ctx = SchemaCtx::new(&buf);
        let root = <RecordFile as GetSchema>::root_value(buf.len() as u64);

        let result = query_from("/items | count()", &ctx, &root).unwrap();
        assert_eq!(result.len(), 1);
        assert!(matches!(result[0], EvalResult::U64(3)));
    }

    #[test]
    fn test_avg() {
        use mmapstruct::{write_to_buf_raw, SchemaCtx, GetSchema, MmapStruct};

        #[derive(MmapStruct, Debug)]
        #[mmap(scope)]
        struct Record {
            id: u32,
            price: f32,
            _tag: String,
        }

        #[derive(MmapStruct, Debug)]
        struct RecordFile {
            items: Vec<Record>,
        }

        let records = vec![
            Record {
                id: 1,
                price: 10.0,
                _tag: "a".into(),
            },
            Record {
                id: 2,
                price: 20.0,
                _tag: "b".into(),
            },
            Record {
                id: 3,
                price: 30.0,
                _tag: "c".into(),
            },
        ];
        let w = RecordFile { items: records };
        let buf = write_to_buf_raw(&w);
        let ctx = SchemaCtx::new(&buf);
        let root = <RecordFile as GetSchema>::root_value(buf.len() as u64);

        let result = query_from("/items | avg(.price)", &ctx, &root).unwrap();
        assert_eq!(result.len(), 1);
        let EvalResult::F64(avg) = result[0] else {
            panic!("expected F64")
        };
        assert!((avg - 20.0).abs() < 0.001);
    }

    #[test]
    fn test_min() {
        use mmapstruct::{write_to_buf_raw, SchemaCtx, GetSchema, MmapStruct};

        #[derive(MmapStruct, Debug)]
        #[mmap(scope)]
        struct Record {
            id: u32,
            price: f32,
            _tag: String,
        }

        #[derive(MmapStruct, Debug)]
        struct RecordFile {
            items: Vec<Record>,
        }

        let records = vec![
            Record {
                id: 1,
                price: 30.0,
                _tag: "a".into(),
            },
            Record {
                id: 2,
                price: 10.0,
                _tag: "b".into(),
            },
            Record {
                id: 3,
                price: 20.0,
                _tag: "c".into(),
            },
        ];
        let w = RecordFile { items: records };
        let buf = write_to_buf_raw(&w);
        let ctx = SchemaCtx::new(&buf);
        let root = <RecordFile as GetSchema>::root_value(buf.len() as u64);

        let result = query_from("/items | min(.price)", &ctx, &root).unwrap();
        assert_eq!(result.len(), 1);
        let EvalResult::F64(min) = result[0] else {
            panic!("expected F64")
        };
        assert!((min - 10.0).abs() < 0.001);
    }

    #[test]
    fn test_max() {
        use mmapstruct::{write_to_buf_raw, SchemaCtx, GetSchema, MmapStruct};

        #[derive(MmapStruct, Debug)]
        #[mmap(scope)]
        struct Record {
            id: u32,
            price: f32,
            _tag: String,
        }

        #[derive(MmapStruct, Debug)]
        struct RecordFile {
            items: Vec<Record>,
        }

        let records = vec![
            Record {
                id: 1,
                price: 30.0,
                _tag: "a".into(),
            },
            Record {
                id: 2,
                price: 10.0,
                _tag: "b".into(),
            },
            Record {
                id: 3,
                price: 20.0,
                _tag: "c".into(),
            },
        ];
        let w = RecordFile { items: records };
        let buf = write_to_buf_raw(&w);
        let ctx = SchemaCtx::new(&buf);
        let root = <RecordFile as GetSchema>::root_value(buf.len() as u64);

        let result = query_from("/items | max(.price)", &ctx, &root).unwrap();
        assert_eq!(result.len(), 1);
        let EvalResult::F64(max) = result[0] else {
            panic!("expected F64")
        };
        assert!((max - 30.0).abs() < 0.001);
    }

    #[test]
    fn test_count_pod_array() {
        use bytemuck;
        use mmapstruct::{write_to_buf_raw, SchemaCtx, GetSchema, MmapStruct};

        #[derive(
            MmapStruct, Debug, PartialEq, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable, Default,
        )]
        #[repr(C)]
        struct Point3d {
            x: f32,
            y: f32,
            z: f32,
        }

        #[derive(MmapStruct, Debug, PartialEq)]
        struct VecPod3d {
            #[pod]
            points: Vec<Point3d>,
        }

        let points = vec![
            Point3d {
                x: 1.0,
                y: 2.0,
                z: 3.0,
            },
            Point3d {
                x: 4.0,
                y: 5.0,
                z: 6.0,
            },
        ];
        let s = VecPod3d { points };
        let buf = write_to_buf_raw(&s);
        let ctx = SchemaCtx::new(&buf);
        let root = <VecPod3d as GetSchema>::root_value(buf.len() as u64);

        let result = query_from("/points | count()", &ctx, &root).unwrap();
        assert!(matches!(result[0], EvalResult::U64(2)));
    }

    #[test]
    fn test_avg_pod_array() {
        use bytemuck;
        use mmapstruct::{write_to_buf_raw, SchemaCtx, GetSchema, MmapStruct};

        #[derive(
            MmapStruct, Debug, PartialEq, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable, Default,
        )]
        #[repr(C)]
        struct Point3d {
            x: f32,
            y: f32,
            z: f32,
        }

        #[derive(MmapStruct, Debug, PartialEq)]
        struct VecPod3d {
            #[pod]
            points: Vec<Point3d>,
        }

        let points = vec![
            Point3d {
                x: 1.0,
                y: 2.0,
                z: 3.0,
            },
            Point3d {
                x: 4.0,
                y: 5.0,
                z: 6.0,
            },
            Point3d {
                x: 7.0,
                y: 8.0,
                z: 9.0,
            },
        ];
        let s = VecPod3d { points };
        let buf = write_to_buf_raw(&s);
        let ctx = SchemaCtx::new(&buf);
        let root = <VecPod3d as GetSchema>::root_value(buf.len() as u64);

        let result = query_from("/points | avg(.x)", &ctx, &root).unwrap();
        let EvalResult::F64(avg_x) = result[0] else {
            panic!("expected F64")
        };
        assert!((avg_x - 4.0).abs() < 0.001);
    }

    #[test]
    fn test_min_pod_array() {
        use bytemuck;
        use mmapstruct::{write_to_buf_raw, SchemaCtx, GetSchema, MmapStruct};

        #[derive(
            MmapStruct, Debug, PartialEq, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable, Default,
        )]
        #[repr(C)]
        struct Point3d {
            x: f32,
            y: f32,
            z: f32,
        }

        #[derive(MmapStruct, Debug, PartialEq)]
        struct VecPod3d {
            #[pod]
            points: Vec<Point3d>,
        }

        let points = vec![
            Point3d {
                x: 10.0,
                y: 2.0,
                z: 3.0,
            },
            Point3d {
                x: 4.0,
                y: 5.0,
                z: 6.0,
            },
            Point3d {
                x: 7.0,
                y: 8.0,
                z: 9.0,
            },
        ];
        let s = VecPod3d { points };
        let buf = write_to_buf_raw(&s);
        let ctx = SchemaCtx::new(&buf);
        let root = <VecPod3d as GetSchema>::root_value(buf.len() as u64);

        let result = query_from("/points | min(.x)", &ctx, &root).unwrap();
        let EvalResult::F64(min_x) = result[0] else {
            panic!("expected F64")
        };
        assert!((min_x - 4.0).abs() < 0.001);
    }

    #[test]
    fn test_max_pod_array() {
        use bytemuck;
        use mmapstruct::{write_to_buf_raw, SchemaCtx, GetSchema, MmapStruct};

        #[derive(
            MmapStruct, Debug, PartialEq, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable, Default,
        )]
        #[repr(C)]
        struct Point3d {
            x: f32,
            y: f32,
            z: f32,
        }

        #[derive(MmapStruct, Debug, PartialEq)]
        struct VecPod3d {
            #[pod]
            points: Vec<Point3d>,
        }

        let points = vec![
            Point3d {
                x: 10.0,
                y: 2.0,
                z: 3.0,
            },
            Point3d {
                x: 4.0,
                y: 5.0,
                z: 6.0,
            },
            Point3d {
                x: 7.0,
                y: 8.0,
                z: 9.0,
            },
        ];
        let s = VecPod3d { points };
        let buf = write_to_buf_raw(&s);
        let ctx = SchemaCtx::new(&buf);
        let root = <VecPod3d as GetSchema>::root_value(buf.len() as u64);

        let result = query_from("/points | max(.x)", &ctx, &root).unwrap();
        let EvalResult::F64(max_x) = result[0] else {
            panic!("expected F64")
        };
        assert!((max_x - 10.0).abs() < 0.001);
    }

    #[test]
    fn test_aggregate_non_array_fails() {
        use mmapstruct::{write_to_buf_raw, SchemaCtx, GetSchema, MmapStruct};

        #[derive(MmapStruct, Debug)]
        #[mmap(scope)]
        struct Record {
            id: u32,
            _tag: String,
        }

        let r = Record {
            id: 42,
            _tag: "x".into(),
        };
        let buf = write_to_buf_raw(&r);
        let ctx = SchemaCtx::new(&buf);
        let root = <Record as GetSchema>::root_value(buf.len() as u64);

        let result = query_from("/ | count()", &ctx, &root);
        match result {
            Err(msg) => assert!(msg.contains("requires an array")),
            Ok(_) => panic!("expected error, got ok"),
        }
    }

    #[test]
    fn test_format_aggregate_count() {
        use mmapstruct::{write_to_buf_raw, SchemaCtx, GetSchema, MmapStruct};

        #[derive(MmapStruct, Debug)]
        #[mmap(scope)]
        struct Record {
            id: u32,
            _tag: String,
        }

        #[derive(MmapStruct, Debug)]
        struct RecordFile {
            items: Vec<Record>,
        }

        let records = vec![
            Record {
                id: 1,
                _tag: "a".into(),
            },
            Record {
                id: 5,
                _tag: "b".into(),
            },
        ];
        let w = RecordFile { items: records };
        let buf = write_to_buf_raw(&w);
        let ctx = SchemaCtx::new(&buf);
        let root = <RecordFile as GetSchema>::root_value(buf.len() as u64);

        let result = query_from("/items | count()", &ctx, &root).unwrap();
        let s = crate::format_result(&ctx, &result[0]);
        assert_eq!(s, "2");
    }

    #[test]
    fn test_format_aggregate_avg() {
        use mmapstruct::{write_to_buf_raw, SchemaCtx, GetSchema, MmapStruct};

        #[derive(MmapStruct, Debug)]
        #[mmap(scope)]
        struct Record {
            id: u32,
            price: f32,
            _tag: String,
        }

        #[derive(MmapStruct, Debug)]
        struct RecordFile {
            items: Vec<Record>,
        }

        let records = vec![
            Record {
                id: 1,
                price: 10.0,
                _tag: "a".into(),
            },
            Record {
                id: 2,
                price: 20.0,
                _tag: "b".into(),
            },
        ];
        let w = RecordFile { items: records };
        let buf = write_to_buf_raw(&w);
        let ctx = SchemaCtx::new(&buf);
        let root = <RecordFile as GetSchema>::root_value(buf.len() as u64);

        let result = query_from("/items | avg(.price)", &ctx, &root).unwrap();
        let s = crate::format_result(&ctx, &result[0]);
        assert_eq!(s, "15");
    }

    // -----------------------------------------------------------------------
    // Option<byref> DSL tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_option_byref_some_via_dsl() {
        use mmapstruct::{write_to_buf_raw, SchemaCtx, GetSchema, MmapStruct};

        #[derive(MmapStruct, Debug)]
        struct Doc {
            id: u32,
            #[mmap(byref)]
            label: Option<String>,
        }

        let d = Doc {
            id: 42,
            label: Some("hello".into()),
        };
        let buf = write_to_buf_raw(&d);
        let ctx = SchemaCtx::new(&buf);
        let root = <Doc as GetSchema>::root_value(buf.len() as u64);

        let result = query_from("/label", &ctx, &root).unwrap();
        assert_eq!(result.len(), 1);
        let s = crate::format_result(&ctx, &result[0]);
        assert_eq!(s, r#""hello""#);
    }

    #[test]
    fn test_option_byref_none_via_dsl() {
        use mmapstruct::{write_to_buf_raw, SchemaCtx, GetSchema, MmapStruct};

        #[derive(MmapStruct, Debug)]
        struct Doc {
            id: u32,
            #[mmap(byref)]
            label: Option<String>,
        }

        let d = Doc {
            id: 99,
            label: None,
        };
        let buf = write_to_buf_raw(&d);
        let ctx = SchemaCtx::new(&buf);
        let root = <Doc as GetSchema>::root_value(buf.len() as u64);

        let result = query_from("/label", &ctx, &root).unwrap();
        assert_eq!(result.len(), 1);
        let s = crate::format_result(&ctx, &result[0]);
        assert_eq!(s, "null");
    }

    #[test]
    fn test_option_byref_none_in_struct_context() {
        use mmapstruct::{write_to_buf_raw, SchemaCtx, GetSchema, MmapStruct};

        #[derive(MmapStruct, Debug)]
        struct Doc {
            id: u32,
            #[mmap(byref)]
            label: Option<String>,
        }

        let d = Doc {
            id: 99,
            label: None,
        };
        let buf = write_to_buf_raw(&d);
        let ctx = SchemaCtx::new(&buf);
        let root = <Doc as GetSchema>::root_value(buf.len() as u64);
        let s = crate::format_value(&ctx, &root);
        assert_eq!(s, r#"{"id": 99, "label": null}"#);

        let d = Doc {
            id: 42,
            label: Some("hello".into()),
        };
        let buf = write_to_buf_raw(&d);
        let ctx = SchemaCtx::new(&buf);
        let root = <Doc as GetSchema>::root_value(buf.len() as u64);
        let s = crate::format_value(&ctx, &root);
        assert_eq!(s, r#"{"id": 42, "label": "hello"}"#);
    }

    #[test]
    fn test_enum_schema_variant_a() {
        use mmapstruct::{write_to_buf_raw, SchemaCtx, GetSchema, MmapStruct};

        #[derive(MmapStruct, Debug, PartialEq)]
        enum MyEnum {
            Alpha { id: u32 },
            Beta { name: String, val: u64 },
        }

        let w = MyEnum::Alpha { id: 42 };
        let buf = write_to_buf_raw(&w);
        let ctx = SchemaCtx::new(&buf);
        let root = <MyEnum as GetSchema>::root_value(buf.len() as u64);
        let s = crate::format_value(&ctx, &root);
        assert_eq!(s, r#"MyEnum::Alpha { "id": 42 }"#);

        let w = MyEnum::Beta {
            name: "hello".into(),
            val: 99,
        };
        let buf = write_to_buf_raw(&w);
        let ctx = SchemaCtx::new(&buf);
        let root = <MyEnum as GetSchema>::root_value(buf.len() as u64);
        let s = crate::format_value(&ctx, &root);
        assert_eq!(s, r#"MyEnum::Beta { "name": "hello", "val": 99 }"#);
    }

    #[test]
    fn test_enum_schema_variant_navigate() {
        use mmapstruct::{write_to_buf_raw, SchemaCtx, GetSchema, MmapStruct};

        #[derive(MmapStruct, Debug, PartialEq)]
        enum MyEnum {
            Alpha { id: u32 },
            Beta { name: String, val: u64 },
        }

        let w = MyEnum::Alpha { id: 7 };
        let buf = write_to_buf_raw(&w);
        let ctx = SchemaCtx::new(&buf);
        let root = <MyEnum as GetSchema>::root_value(buf.len() as u64);

        let (var_schema, _data_off) = ctx.enum_variant(&root).unwrap();
        assert_eq!(var_schema.name, "MyEnum::Alpha");
        assert_eq!(var_schema.fields.len(), 1);

        let id_val = ctx.field_get(&root, "id").unwrap();
        assert_eq!(ctx.read_u32(&id_val), 7);
    }
}
