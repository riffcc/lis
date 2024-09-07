use lis::Lis;
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;
use tokio::task;

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
    let _ = task::spawn_blocking(move || {
        let path = mountpoint.join("1").join("2");
        fs::create_dir_all(path)
    })
    .await
    .expect("Failed to create directories");

    let mountpoint = tmp_mountpoint.path().to_path_buf();

    // Offload blocking `read_dir` operation to a separate thread
    let (entries_mountpoint, entries_1, entries_2) = task::spawn_blocking(move || {
        let path_1 = mountpoint.join("1");
        let path_2 = mountpoint.join("1").join("2");

        (
            fs::read_dir(mountpoint)
                .unwrap()
                .filter_map(|entry| entry.ok())
                .collect::<Vec<_>>(),
            fs::read_dir(path_1)
                .unwrap()
                .filter_map(|entry| entry.ok())
                .collect::<Vec<_>>(),
            fs::read_dir(path_2)
                .unwrap()
                .filter_map(|entry| entry.ok())
                .collect::<Vec<_>>(),
        )
    })
    .await
    .expect("Failed to read directory");
    assert_eq!(1, entries_mountpoint.len());
    assert_eq!(1, entries_1.len());
    assert_eq!(0, entries_2.len());
}
