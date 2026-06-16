use std::sync::Arc;
use mmapstruct::{MmapStruct, write_to_buf_raw};
use std::net::SocketAddr;

#[derive(MmapStruct)]
struct TestFile {
    id: u32,
    name: String,
    tags: Vec<u32>,
    #[mmap(byref)]
    comment: String,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let data = write_to_buf_raw(&TestFile {
        id: 42,
        name: "hello".into(),
        tags: vec![1, 2, 3],
        comment: "a comment".into(),
    });

    let addr: SocketAddr = "127.0.0.1:9876".parse()?;
    println!("Serving TestFile on http://{}", addr);
    println!("Try:");
    println!("  curl 'http://{}/api/query?q=/id'", addr);
    println!("  curl 'http://{}/api/query?q=/name'", addr);
    println!("  curl 'http://{}/api/query?q=/tags'", addr);
    println!("  curl 'http://{}/api/schema'", addr);
    println!("  curl 'http://{}/api/stats'", addr);
    println!("Or open http://{} in a browser", addr);
    println!("Press Ctrl+C to stop.");

    mmapview_http::serve::<TestFile>(Arc::from(data), addr).await
}
