/// Bytes of the self-schema file (`self-schema.mms`) — a serialized
/// `Schema` that describes the layout of `Schema` itself.
///
/// This allows reading any `schema.mms` file using only the
/// self-schema (since self-schema describes how `Schema` is laid out
/// in memory), which in turn describes how the data file is laid out.
pub const SELF_SCHEMA_MMS: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/self-schema.mms"));
