use anyhow::Result;
use futures_lite::StreamExt;
use iroh::blobs::store::Store; // trait
use iroh::{client::docs::Doc, util::fs::path_to_key};
use std::path::{Path, PathBuf};

mod cli;
pub use cli::Cli;

pub struct Lis<D: Store> {
    pub iroh_node: iroh::node::Node<D>,
    author: iroh::docs::AuthorId,
}

pub enum NodeType {
    Mem,
    Disk(PathBuf),
}

impl<D: Store> Lis<D> {
    pub async fn new(node_type: NodeType) -> Result<Self> {
        let iroh_node;
        match node_type {
            NodeType::Mem => {
                iroh_node = iroh::node::Node::memory().spawn().await?;
            }
            NodeType::Disk(root) => {
                iroh_node = iroh::node::Node::persistent(root).await?.spawn().await?;
            }
        };
        let author = iroh_node.authors().create().await?; // TODO: add this to Lis
        let lis = Lis {
            iroh_node, // TODO: option to move to disk node
            author,    // TODO: add this to Lis
        };
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
        // read file
        let bytes = std::fs::read(path)?;

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
