use anyhow::Result;
use futures_lite::StreamExt;
use iroh::{client::docs::Doc, node::Node, util::fs::path_to_key};
use std::fs;
use std::path::{Path, PathBuf};

mod cli;
pub use cli::Cli;

pub struct Lis {
    pub iroh_node: Node<iroh::blobs::store::fs::Store>,
    author: iroh::docs::AuthorId,
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
        let lis = Lis { iroh_node, author };
        Ok(lis)
    }

    /// Adds a file to new doc
    /// Creates new doc
    pub async fn add_file(&mut self, path: &Path) -> Result<()> {
        // Create document
        let mut doc = self.iroh_node.docs().create().await?;

        self.add_file_to_doc(path, &mut doc).await?;

        Ok(())
    }

    /// Adds a file to a previously created document
    pub async fn add_file_to_doc(&mut self, path: &Path, doc: &mut Doc) -> Result<()> {
        let key = path_to_key(&path, None, None)?; // TODO: use prefix and root (see path_to_key
                                                   // docs)
        doc.import_file(self.author, key, path, false)
            .await?
            .collect::<Vec<_>>()
            .await;

        Ok(())
    }

    /// Removes a doc
    pub async fn rm_doc(&mut self, doc: &Doc) -> Result<()> {
        self.iroh_node.docs().drop_doc(doc.id()).await
    }
}
