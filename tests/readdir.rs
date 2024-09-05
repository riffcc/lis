use lis::Lis;
use std::path::PathBuf;
use std::{fs, io::Write};
use tempfile::{NamedTempFile, TempDir};
use tokio::task;

async fn setup_lis(tmp_dir: &TempDir) -> Lis {
    let root = PathBuf::from(tmp_dir.path());
    let overwrite = true;
    Lis::new(&root, overwrite)
        .await
        .expect("Could not create new Lis node")
}

#[tokio::test]
async fn test_readdir_empty() {
    // Setup Lis
    let tmp_root = TempDir::new().expect("Could not create temp dir");
    let lis = setup_lis(&tmp_root).await;

    // Mount Lis
    let tmp_mountpoint = TempDir::new().expect("Could not create temp dir");
    let _handle = fuser::spawn_mount2(lis, &tmp_mountpoint, &[]).expect("could not mount Lis");

    // Offload blocking `read_dir` operation to a separate thread
    let mountpoint = tmp_mountpoint.path().to_path_buf();
    let entries = task::spawn_blocking(move || {
        let mut results = vec![];
        for entry in fs::read_dir(mountpoint).unwrap() {
            let entry = entry.unwrap();
            results.push(entry);
        }
        results
    })
    .await
    .expect("Failed to read directory");

    for entry in entries {
        println!("{}", entry.path().display());
        // Check if all entries are readonly
        assert!(entry.metadata().unwrap().permissions().readonly());
    }
}

#[tokio::test]
async fn test_readdir_files() {
    // Setup Lis
    let tmp_root = TempDir::new().expect("Could not create temp dir");
    let mut lis = setup_lis(&tmp_root).await;

    // Add files to lis
    // Create a file inside of `env::temp_dir()`.
    let mut file = NamedTempFile::new_in("/tmp/").expect("Could not create named temp file");
    let content = "Brian was here. Briefly.";
    write!(file, "{}", content).expect("Could not write to named temp file");
    lis.put(file.path()).await.expect("Could not put file");

    // Mount Lis
    let tmp_mountpoint = TempDir::new().expect("Could not create temp dir");
    let _handle = fuser::spawn_mount2(lis, &tmp_mountpoint, &[]).expect("could not mount Lis");

    // Offload blocking `read_dir` operation to a separate thread
    let mountpoint = tmp_mountpoint.path().to_path_buf();
    let entries = task::spawn_blocking(move || {
        let mut results = vec![];
        for entry in fs::read_dir(mountpoint).unwrap() {
            let entry = entry.unwrap();
            results.push(entry);
        }
        results
    })
    .await
    .expect("Failed to read directory");

    assert_eq!(entries.len(), 1);

    // Check if entries are readonly
    for entry in entries {
        assert!(entry.metadata().unwrap().permissions().readonly());
    }
}
