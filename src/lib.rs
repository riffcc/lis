use anyhow::{anyhow, Result};
use futures_lite::StreamExt;
use iroh::docs::store::Query;
use iroh::{client::docs::Doc, node::Node, util::fs::path_to_key};
use std::fs;
use std::path::{Path, PathBuf};

mod cli;
pub use cli::{Cli, Commands};

pub struct Lis {
    pub iroh_node: Node<iroh::blobs::store::fs::Store>,
    author: iroh::docs::AuthorId,
    root: PathBuf,
}

impl Lis {
    /// Creates new Lis node
    /// If `root` path does not exist, it is created with `mkdir -p`
    /// If an Iroh node is found in `root`, a new one is created
    /// If an Iroh node is found in `root` but `overwrite` is `true`, the old one is truncated
    pub async fn new(root: &PathBuf, overwrite: bool) -> Result<Self> {
        if overwrite {
            // remove old root dir if one existed before
            fs::remove_dir_all(root)?;
        }
        // create root path if not exists
        fs::create_dir_all(root)?;

        let iroh_node = iroh::node::Node::persistent(root).await?.spawn().await?;
        let author = iroh_node.authors().create().await?;

        let lis = Lis {
            iroh_node,
            author,
            root: root.clone(),
        };
        Ok(lis)
    }

    /// List all files in node
    pub async fn list(&self) -> Result<()> {
        // let files = Vec::new();
        // get all the entries with default filtering and sorting

        let mut doc_ids = self.iroh_node.docs().list().await?;
        while let Some(doc_id) = doc_ids.next().await {
            let (doc_id, kind) = doc_id?;
            println!("doc:{doc_id} ({kind})");
            if let Some(doc) = self.iroh_node.docs().open(doc_id).await? {
                let query = Query::all().build();
                let entries = doc.get_many(query).await?.collect::<Vec<_>>().await;

                for entry in entries {
                    let entry = entry?;
                    let key = entry.key();
                    let hash = entry.content_hash();
                    let content = entry.content_bytes(self.iroh_node.client()).await?;
                    println!(
                        "{} : {} (hash: {})",
                        std::str::from_utf8(key)?,
                        std::str::from_utf8(&content)?,
                        hash
                    );
                }
            }
        }
        Ok(())
    }

    /// Creates a new Doc and adds a file to it
    /// Returns the key to the added file upon success
    pub async fn add_file(&mut self, src_path: &Path) -> Result<String> {
        if !src_path.exists() {
            return Err(anyhow!("File {} not found", src_path.display()));
        }
        if !src_path.is_file() {
            return Err(anyhow!("Path must be a file"));
        }

        let doc = self.iroh_node.docs().create().await?;

        let full_src_path = fs::canonicalize(&src_path)?;
        self.add_file_to_doc(full_src_path.as_path(), doc).await
    }

    /// Adds a file to a previously created document
    /// Returns the key to the added file upon success
    pub async fn add_file_to_doc(&mut self, path: &Path, doc: Doc) -> Result<String> {
        // Key is self.root + / + filename
        let mut prefix = self
            .root
            .as_os_str()
            .to_owned()
            .into_string()
            .expect("Could not make file path into string");
        prefix.push('/');

        let root: PathBuf = path
            .parent()
            .ok_or(anyhow!("Could not find parent for file"))?
            .into();

        // src_path = /os/path/filename.txt
        // prefix = /path/to/iroh/node
        // root = /os/path/
        // key = /path/to/iroh/node/filename.txt
        let key = path_to_key(path, Some(prefix), Some(root))?;

        doc.import_file(self.author, key.clone(), path, false)
            .await?
            .collect::<Vec<_>>()
            .await;

        let key_str = std::str::from_utf8(key.as_ref())?;

        Ok(key_str.to_string())
    }

    /// Removes a doc
    pub async fn rm_doc(&mut self, doc: &Doc) -> Result<()> {
        self.iroh_node.docs().drop_doc(doc.id()).await
    }
}
