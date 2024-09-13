use lis::Lis;
use std::path::{Path, PathBuf};
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

    let mountpoint = tmp_mountpoint.path().to_path_buf();
    let mut entries = tokio::fs::read_dir(mountpoint).await.unwrap();
    // Check if all entries are read-write
    while let Some(entry) = entries.next_entry().await.unwrap() {
        assert_eq!(
            false,
            entry.metadata().await.unwrap().permissions().readonly()
        );
    }
}

#[tokio::test]
async fn test_readdir_non_empty() {
    // Setup Lis
    let tmp_root = TempDir::new().expect("Could not create temp dir");
    let mut lis = setup_lis(&tmp_root).await;

    // Add files to lis
    // Create a file inside of `env::temp_dir()`.
    let mut file = NamedTempFile::new_in("/tmp/").expect("Could not create named temp file");
    let content = "Brian was here. Briefly.";
    write!(file, "{}", content).expect("Could not write to named temp file");
    lis.put(file.path(), Path::new(file.path().file_name().unwrap()))
        .await
        .expect("Could not put file"); // should succeed

    // Mount Lis
    let tmp_mountpoint = TempDir::new().expect("Could not create temp dir");
    let _handle = fuser::spawn_mount2(lis, &tmp_mountpoint, &[]).expect("could not mount Lis");

    // Offload blocking `read_dir` operation to a separate thread
    let mountpoint = tmp_mountpoint.path().to_path_buf();
    let entries = task::spawn_blocking(move || {
        let mut results = vec![];
        for entry in fs::read_dir(mountpoint).unwrap() {
            if let Ok(entry) = entry {
                results.push(entry);
            }
        }
        results
    })
    .await
    .expect("Failed to read directory");

    assert_eq!(entries.len(), 1);

    // Check if all entries are read-write
    for entry in entries {
        assert_eq!(false, entry.metadata().unwrap().permissions().readonly());
    }
}

#[tokio::test]
async fn test_read() {
    // Setup Lis
    let tmp_root = TempDir::new().expect("Could not create temp dir");
    let mut lis = setup_lis(&tmp_root).await;

    // Add file to lis
    // Create a file inside of `env::temp_dir()`.
    let mut file = NamedTempFile::new_in("/tmp/").expect("Could not create named temp file");
    let content = "Brian was here. Briefly.";
    write!(file, "{}", content).expect("Could not write to named temp file");
    lis.put(file.path(), Path::new(file.path().file_name().unwrap()))
        .await
        .expect("Could not put file"); // should succeed

    // Mount Lis
    let tmp_mountpoint = TempDir::new().expect("Could not create temp dir");
    let _handle = fuser::spawn_mount2(lis, &tmp_mountpoint, &[]).expect("could not mount Lis");

    // Read added file
    let contents = fs::read_to_string(file.path()).expect("Could not read file");

    assert_eq!(contents, "Brian was here. Briefly.");
}
