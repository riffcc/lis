use std::path::{Path, PathBuf};

use criterion::{criterion_group, criterion_main, Criterion};
use rand::Rng;
use tempfile::TempDir;
use tokio::runtime::Runtime;
// use std::hint::black_box;

use lis::Lis;

async fn setup_lis(tmp_dir: &TempDir) -> Lis {
    let root = PathBuf::from(tmp_dir.path());
    let overwrite = true;
    Lis::new(&root, overwrite)
        .await
        .expect("Could not create new Lis node")
}

fn read_write_benchmark(c: &mut Criterion) {
    c.bench_function("1 KiB write", |b| {
        b.iter(|| {
            let rt = Runtime::new().unwrap();
            let mut rng = rand::rng();
            let tmp_dir = TempDir::new().unwrap();
            let mut lis = rt.block_on(setup_lis(&tmp_dir));
            let mut file = rt
                .block_on(lis.create_file(&Path::new(&format!("/file"))))
                .unwrap();
            let random_bytes: Vec<u8> = (0..1024).map(|_| rng.random()).collect();
            println!("{}", random_bytes.len());
            rt.block_on(file.write(&lis.iroh_node, 0, random_bytes.clone().into()))
                .unwrap();
        })
    });
    // c.bench_function("read 10 GiB file", |b| b.iter(|| lis.read_all()));
}

criterion_group!(benches, read_write_benchmark);
criterion_main!(benches);
