//! Time opening a vault and a few queries.
//!   cargo run --release -p cairn-core --example bench_open -- <vault>

use std::sync::Arc;
use std::time::Instant;

use cairn_core::{StdFs, TrashMode, Vault};

fn main() {
    let root = std::env::args().nth(1).expect("usage: bench_open <vault>");
    let t = Instant::now();
    let fs = StdFs::new(&root, TrashMode::Vault).unwrap();
    let v = Vault::open(Arc::new(fs)).unwrap();
    println!("open: {:?} ({} notes, {} entries)", t.elapsed(), v.index().note_count(), v.entries().len());
    for q in ["garden", "river stone", "\"coffee bread\"", "harv", "s", "1", "s 1", "設定", "ファイルを開く"] {
        let t = Instant::now();
        let r = v.search(q, 100);
        println!("search {q:?}: {} hits in {:?}", r.len(), t.elapsed());
    }
    let some = v.entries().into_iter().find(|e| e.path.ends_with(".md")).unwrap().path;
    let t = Instant::now();
    let b = v.backlinks(&some);
    println!("backlinks of {some}: {} sources in {:?}", b.len(), t.elapsed());
    let t = Instant::now();
    let g = v.graph(false);
    println!("graph: {} nodes, {} edges in {:?}", g.nodes.len(), g.edges.len(), t.elapsed());
    let t = Instant::now();
    let tags = v.tags();
    println!("tags: {} in {:?}", tags.len(), t.elapsed());
    let t = Instant::now();
    let c = v.rescan().unwrap();
    println!("full rescan (no changes): {} changes in {:?}", c.len(), t.elapsed());
}
