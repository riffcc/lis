use std::path::PathBuf;

use tempfile::TempDir;
use tokio::fs::{self, create_dir_all, DirEntry, File};

use lis::Lis;

async fn setup_lis(tmp_dir: &TempDir) -> Lis {
    let root = PathBuf::from(tmp_dir.path());
    let overwrite = true;
    Lis::new(&root, overwrite)
        .await
        .expect("Could not create new Lis node")
}

#[tokio::test]
async fn test_mkdir() {
    // Setup Lis
    let tmp_root = TempDir::new().expect("Could not create temp dir");
    let lis = setup_lis(&tmp_root).await;

    // Mount Lis
    let tmp_mountpoint = TempDir::new().expect("Could not create temp dir");
    let _handle = fuser::spawn_mount2(lis, &tmp_mountpoint, &[]).expect("could not mount Lis");

    let mountpoint = tmp_mountpoint.path().to_path_buf();

    // Offload blocking `create_dir` operation to a separate thread
    let path = mountpoint.join("1").join("2");
    create_dir_all(path)
        .await
        .expect("Failed to create directories");

    let mountpoint = tmp_mountpoint.path().to_path_buf();

    // Offload blocking `read_dir` operation to a separate thread
    let path_1 = mountpoint.join("1");
    let path_2 = mountpoint.join("1").join("2");

    let mut entries_mountpoint: Vec<DirEntry> = Vec::new();
    let mut entries = fs::read_dir(mountpoint).await.unwrap();
    while let Some(entry) = entries.next_entry().await.unwrap() {
        entries_mountpoint.push(entry);
    }
    assert_eq!(1, entries_mountpoint.len());

    let mut entries_1: Vec<DirEntry> = Vec::new();
    entries = fs::read_dir(path_1).await.unwrap();
    while let Some(entry) = entries.next_entry().await.unwrap() {
        entries_1.push(entry);
    }
    assert_eq!(1, entries_1.len());

    let mut entries_2: Vec<DirEntry> = Vec::new();
    entries = fs::read_dir(path_2).await.unwrap();
    while let Some(entry) = entries.next_entry().await.unwrap() {
        entries_2.push(entry);
    }
    assert_eq!(0, entries_2.len());
}

#[tokio::test]
async fn test_touch() {
    // Setup Lis
    let tmp_root = TempDir::new().expect("Could not create temp dir");
    let lis = setup_lis(&tmp_root).await;

    // Mount Lis
    let tmp_mountpoint = TempDir::new().expect("Could not create temp dir");
    let _handle = fuser::spawn_mount2(lis, &tmp_mountpoint, &[]).expect("could not mount Lis");

    let mountpoint = tmp_mountpoint.path().to_path_buf();
    let clone_mountpoint = mountpoint.clone();

    // Offload blocking `create_dir` operation to a separate thread
    let path = clone_mountpoint.join("foo.txt");
    let _f = File::create_new(path).await.unwrap();

    let clone_mountpoint = mountpoint.clone();
    let mut entries = tokio::fs::read_dir(clone_mountpoint).await.unwrap();
    let mut all_entries: Vec<DirEntry> = Vec::new();
    while let Some(entry) = entries.next_entry().await.unwrap() {
        all_entries.push(entry);
    }
    assert_eq!(1, all_entries.len());

    // check file was created
    let path = mountpoint.join("foo.txt");
    assert_eq!(all_entries[0].path(), path);

    // newly created files have "null" inside them
    // we need "null" because empty files are interpreted as deleted by iroh
    let contents = fs::read_to_string(path).await.expect("Could not read file");
    assert_eq!(contents, "null");
}
