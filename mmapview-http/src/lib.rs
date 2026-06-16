mod server;

use std::sync::Arc;
use std::net::SocketAddr;
use std::path::Path;
use mmapstruct::{GetSchema, MmapReader};
pub use server::{AppState, build_router};

/// Serve the mmapview web UI for a type-erased binary file.
///
/// The caller provides raw mmap'd bytes (header already stripped, or
/// constructed from any source) and the concrete type `T` for schema
/// introspection via `GetSchema`.
///
/// # Example
///
/// ```no_run
/// use std::sync::Arc;
/// use mmapstruct::MmapStruct;
///
/// #[derive(MmapStruct)]
/// struct MyFile { records: Vec<MyRecord> }
/// #[derive(MmapStruct)]
/// struct MyRecord { id: u32, name: String }
///
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let data: Arc<[u8]> = Arc::from(&[0u8; 4096][..]);
/// let addr: std::net::SocketAddr = "127.0.0.1:9876".parse().unwrap();
/// tokio::runtime::Runtime::new()?.block_on(
///     mmapview_http::serve::<MyFile>(data, addr)
/// )?;
/// # Ok(())
/// # }
/// ```
pub async fn serve<T: GetSchema>(
    data: Arc<[u8]>,
    addr: impl Into<SocketAddr>,
) -> Result<(), Box<dyn std::error::Error>> {
    let state = Arc::new(AppState {
        data,
        root_schema: T::root_value_type(),
    });

    let app = build_router(state);
    let listener = tokio::net::TcpListener::bind(addr.into()).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

/// Convenience wrapper: read a file via `MmapReader` (auto-strips header)
/// and serve it.
pub async fn serve_from_reader<T: GetSchema>(
    reader: MmapReader,
    addr: impl Into<SocketAddr>,
) -> Result<(), Box<dyn std::error::Error>> {
    let data: Arc<[u8]> = reader.payload().into();
    serve::<T>(data, addr).await
}

/// Read a file from disk, memory-map it, and serve.
pub async fn serve_file<T: GetSchema>(
    path: impl AsRef<Path>,
    addr: impl Into<SocketAddr>,
) -> Result<(), Box<dyn std::error::Error>> {
    let reader = MmapReader::from_file(path)?;
    serve_from_reader::<T>(reader, addr).await
}
