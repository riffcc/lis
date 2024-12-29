use std::{
    collections::BTreeMap,
    fs,
    sync::atomic::{AtomicU64, Ordering},
};

use crate::{fuse::FileKind, object::Object, prelude::*};

#[derive(Debug, Serialize, Deserialize)]
pub struct Manifest {
    manifest_path: PathBuf,
    pub root_doc_id: String, // doc hash for root document
    /// Maps inodes to objects
    pub objects: BTreeMap<Inode, Object>, // inode -> object
    /// Maps object keys to inodes
    pub inodes: BTreeMap<PathBuf, Inode>, // key -> inode
    pub cur_ino: AtomicU64,
    pub cur_fh: AtomicU64,
}

impl Manifest {
    pub fn new(manifest_path: PathBuf, doc_id: String) -> Result<Self> {
        let cur_ino = AtomicU64::new(1);
        let cur_fh = AtomicU64::new(1);
        let root_obj = Object::new(
            Path::new("/"),
            cur_ino.fetch_add(1, Ordering::SeqCst),
            FileKind::Directory,
            None,
            None,
            None,
            None,
        )?;

        let mut objects = BTreeMap::new();
        let mut inodes = BTreeMap::new();

        objects.insert(1, root_obj);
        inodes.insert(PathBuf::from("/"), 1);

        Ok(Manifest {
            manifest_path,
            root_doc_id: doc_id,
            objects,
            inodes,
            cur_ino,
            cur_fh,
        })
    }

    pub fn save(&self) -> Result<()> {
        // write to manifest.json file
        let json_string = serde_json::to_string(self)?;
        fs::write(self.manifest_path.clone(), json_string)?;
        Ok(())
    }

    pub fn load(manifest_path: &Path) -> Result<Option<Self>> {
        if manifest_path.exists() {
            // load manifest
            let file_content = fs::read_to_string(manifest_path)?;
            let manifest: Manifest = serde_json::from_str(&file_content)?;
            Ok(Some(manifest))
        } else {
            Ok(None)
        }
    }
}
