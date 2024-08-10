use anyhow::{anyhow, Result};
use bytes::Bytes;
use futures_lite::StreamExt;
use iroh::docs::store::Query;
use iroh::{client::docs::Doc, docs::NamespaceId, node::Node};
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;

mod manifest;
use manifest::Manifest;

mod util;
use util::{key_from_file, key_to_string};

mod cli;
pub use cli::{Cli, Commands};

pub struct Lis {
    pub iroh_node: Node<iroh::blobs::store::fs::Store>,
    author: iroh::docs::AuthorId,
    manifest: Manifest,
    files_doc: Doc,
    root: PathBuf,
}

impl Lis {
    /// Creates new Lis node
    /// If `root` path does not exist, it is created with `mkdir -p`
    /// If an Iroh node is found in `root`, a new one is created
    /// If an Iroh node is found in `root` but `overwrite` is `true`, the old one is truncated
    pub async fn new(root: &PathBuf, overwrite: bool) -> Result<Self> {
        if overwrite {
            // TODO: add prompt for overwrite: are you sure? [Y/n]

            // remove old root dir if one existed before
            fs::remove_dir_all(root)?;
        }
        // create root path if not exists
        fs::create_dir_all(root)?;

        let iroh_node = iroh::node::Node::persistent(root).await?.spawn().await?;
        let author = iroh_node.authors().create().await?;

        // if manifest.json file found, load it
        // manifest.json holds data about the Files document (which points to all files)
        let manifest_path = root.join("manifest.json");
        let (manifest, files_doc): (Manifest, Doc) = if manifest_path.exists() {
            // load manifest
            let file_content = fs::read_to_string(manifest_path)?;
            let manifest: Manifest = serde_json::from_str(&file_content)?;
            let files_doc = iroh_node
                .docs()
                .open(NamespaceId::from_str(manifest.files_doc_id.as_str())?)
                .await?
                .ok_or_else(|| anyhow!("no files doc found"))?;
            (manifest, files_doc)
        } else {
            // create new Files doc and manifest file
            let files_doc = iroh_node.docs().create().await?;
            let manifest = Manifest::new(files_doc.id().to_string());
            // write to manifest.json file
            let json_string = serde_json::to_string(&manifest)?;
            fs::write(manifest_path, json_string)?;
            (manifest, files_doc)
        };

        let lis = Lis {
            iroh_node,
            author,
            manifest,
            files_doc,
            root: root.clone(),
        };
        Ok(lis)
    }

    /// List all files in node
    pub async fn list(&self) -> Result<()> {
        let query = Query::all().build();
        let entries = self
            .files_doc
            .get_many(query)
            .await?
            .collect::<Vec<_>>()
            .await;

        for entry in entries {
            let entry = entry?;
            let key = entry.key();
            let hash = entry.content_hash();
            // let content = entry.content_bytes(self.iroh_node.client()).await?;
            println!("{} (hash: {})", std::str::from_utf8(key)?, hash);
        }
        Ok(())
    }

    /// Creates a new Doc and adds a file to it
    /// Returns the key to the added file upon success
    pub async fn put_file(&mut self, src_path: &Path) -> Result<String> {
        if !src_path.exists() {
            return Err(anyhow!("File {} not found", src_path.display()));
        }
        if !src_path.is_file() {
            return Err(anyhow!("Path must be a file"));
        }

        let full_src_path = fs::canonicalize(&src_path)?;
        self.put_file_to_doc(full_src_path.as_path()).await
    }

    /// Puts a file to a previously created document
    /// Returns the key to the added file upon success
    pub async fn put_file_to_doc(&mut self, path: &Path) -> Result<String> {
        let key = key_from_file(&self.root, path)?;

        // if key already in filesystem, remove it first
        let query = Query::key_exact(key.clone());
        if self.files_doc.get_one(query).await?.is_some() {
            self.files_doc.del(self.author, key.clone()).await?; // delete old entry
        }

        self.files_doc
            .import_file(self.author, key.clone(), path, false)
            .await?
            .collect::<Vec<_>>()
            .await;

        key_to_string(key)
    }

    /// Remove a file
    pub async fn rm_file(&mut self, path: &Path) -> Result<String> {
        let key = key_from_file(&self.root, path)?;

        self.files_doc.del(self.author, key.clone()).await?;
        key_to_string(key)
    }

    /// Get contents of a file
    pub async fn get_file(&mut self, path: &Path) -> Result<Bytes> {
        let key = key_from_file(&self.root, path)?;

        // get content of the key from doc
        let query = Query::key_exact(key);
        let entry = self
            .files_doc
            .get_one(query)
            .await?
            .ok_or_else(|| anyhow!("entry not found"))?;

        entry.content_bytes(self.iroh_node.client()).await
    }
}
