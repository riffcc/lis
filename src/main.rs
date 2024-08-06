use anyhow::Result;
use futures_lite::StreamExt;
use iroh::{client::docs::Doc, util::fs::path_to_key};
use std::path::Path;

struct Lis {
    iroh_node: iroh::node::MemNode,
}

impl Lis {
    async fn new() -> Result<Self> {
        let lis = Lis {
            iroh_node: iroh::node::Node::memory().spawn().await?, // TODO: option to move to disk node
        };
        Ok(lis)
    }

    /// Adds a file to new doc
    /// Creates new doc
    async fn add_file(&mut self, path: &Path) -> Result<()> {
        // Create document
        let mut doc = self.iroh_node.docs().create().await?;

        self.add_file_to_doc(path, &mut doc).await?;

        Ok(())
    }

    /// Adds a file to a previously created document
    async fn add_file_to_doc(&mut self, path: &Path, doc: &mut Doc) -> Result<()> {
        // read file
        let bytes = std::fs::read(path)?;

        let client = self.iroh_node.client();
        let author = self.iroh_node.authors().create().await?; // TODO: add this to Lis

        let key = path_to_key(&path, None, None)?; // TODO: use prefix and root (see path_to_key
                                                   // docs)
        doc.import_file(author, key, path, false)
            .await?
            .collect::<Vec<_>>()
            .await;

        Ok(())
    }

    /// Removes a doc
    async fn rm_doc(&mut self, doc: &Doc) -> Result<()> {
        self.iroh_node.docs().drop_doc(doc.id()).await
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let mut lis = Lis::new().await?;

    lis.add_file(Path::new("/tmp/bigfile")).await;

    for entry in lis.iroh_node.docs().list().await?.collect::<Vec<_>>().await {
        let (ns, cap) = entry?;
        println!("\t{ns}\t{cap}");
    }

    Ok(())
}
