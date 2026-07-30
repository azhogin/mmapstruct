use mmapstruct::{write_to_file, BufReader, MmapStruct};
use tempfile::NamedTempFile;

#[derive(MmapStruct, Debug, Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
struct DenseLayerLog {
    layer: u32,
    step: u32,
    t_all: f64,
    t_attn_proj: f64,
    t_attn_core: f64,
    t_attn_out: f64,
    t_mlp_gate_up: f64,
    t_mlp_hidden: f64,
    t_mlp_down: f64,
}

#[derive(MmapStruct, Debug, Clone)]
struct SparseLayerLog {
    layer: u32,
    step: u32,
    t_all: f64,
    t_attn_proj: f64,
    t_attn_core: f64,
    t_attn_out: f64,
    t_moe_route: f64,
    t_moe_shared: f64,
    t_moe_routed: f64,
    #[pod]
    experts: Vec<u32>,
}

#[derive(MmapStruct, Debug)]
enum LayerLog {
    Dense(DenseLayerLog),
    Sparse(SparseLayerLog),
}

#[derive(MmapStruct, Debug)]
struct DecodeLayers {
    entries: Vec<LayerLog>,
}

#[derive(MmapStruct, Debug)]
struct PerfLogFile {
    prefill: Vec<LayerLog>,
    decode: Vec<DecodeLayers>,
}

#[test]
fn test_perf_log_roundtrip() {
    let prefill = vec![
        LayerLog::Dense(DenseLayerLog {
            layer: 0, step: 0, t_all: 1.0,
            t_attn_proj: 0.1, t_attn_core: 0.2, t_attn_out: 0.3,
            t_mlp_gate_up: 0.4, t_mlp_hidden: 0.5, t_mlp_down: 0.6,
        }),
    ];

    let experts_1_9: Vec<u32> = (1..=9).collect();
    let decode0 = DecodeLayers {
        entries: vec![
            LayerLog::Sparse(SparseLayerLog {
                layer: 0, step: 1, t_all: 2.0,
                t_attn_proj: 0.1, t_attn_core: 0.2, t_attn_out: 0.3,
                t_moe_route: 0.4, t_moe_shared: 0.5, t_moe_routed: 0.6,
                experts: experts_1_9.clone(),
            }),
            LayerLog::Dense(DenseLayerLog {
                layer: 1, step: 2, t_all: 3.0,
                t_attn_proj: 0.7, t_attn_core: 0.8, t_attn_out: 0.9,
                t_mlp_gate_up: 1.0, t_mlp_hidden: 1.1, t_mlp_down: 1.2,
            }),
        ],
    };

    let decode1 = DecodeLayers {
        entries: vec![
            LayerLog::Sparse(SparseLayerLog {
                layer: 10, step: 3, t_all: 4.0,
                t_attn_proj: 0.0, t_attn_core: 0.0, t_attn_out: 0.0,
                t_moe_route: 1.0, t_moe_shared: 1.5, t_moe_routed: 2.0,
                experts: vec![5, 6, 7],
            }),
        ],
    };

    let w = PerfLogFile {
        prefill,
        decode: vec![decode0, decode1],
    };

    let tmp = NamedTempFile::new().unwrap();
    write_to_file(&w, tmp.path(), false).unwrap();

    let r = BufReader::from_file(tmp.path()).unwrap();
    let reader = r.read::<PerfLogFile>();

    // Iterate through all DecodeLayers → all SparseLayerLog entries
    let mut sparse_found = 0usize;
    for (di, dl) in reader.decode.iter().enumerate() {
        for entry in dl.entries.iter() {
            match entry {
                LayerLogReader::Sparse(sparse) => {
                    eprintln!(
                        "decode[{}]: Sparse layer={} step={} t_all={} experts={:?}",
                        di, *sparse.layer, *sparse.step, *sparse.t_all, sparse.experts,
                    );
                    match di {
                        0 => {
                            assert_eq!(*sparse.layer, 0);
                            assert_eq!(*sparse.step, 1);
                            assert_eq!(sparse.experts, &[1u32, 2, 3, 4, 5, 6, 7, 8, 9]);
                        }
                        1 => {
                            assert_eq!(*sparse.layer, 10);
                            assert_eq!(*sparse.step, 3);
                            assert_eq!(sparse.experts, &[5u32, 6, 7]);
                        }
                        _ => panic!("unexpected decode index {}", di),
                    }
                    sparse_found += 1;
                }
                LayerLogReader::Dense(_dense) => {
                    // skip
                }
            }
        }
    }
    assert_eq!(sparse_found, 2, "expected 2 Sparse entries across all DecodeLayers");
}