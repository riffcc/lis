use futures_lite::StreamExt;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Create in memory iroh node
    let node = iroh::node::Node::memory().spawn().await?;

    // Create document
    let doc = node.docs().create().await?;
    println!("Created document {}", doc.id());

    println!("List of docs and their capabilities (0-Read, 1-Write):");

    // Returns an array of `iroh.NamespaceAndCapability`s
    // NamespaceId is the Doc's Id
    // and the Capability is whether you have read or write access to the doc
    let ns = node.docs().list().await?.collect::<Vec<_>>().await;
    for entry in ns {
        let (ns, cap) = entry?;
        println!("\t{ns}\t{cap}");
    }

    // Drop document
    node.docs().drop_doc(doc.id()).await?;
    println!("Dropped document {}", doc.id());

    println!("List of docs and their capabilities (0-Read, 1-Write):");
    let ns = node.docs().list().await?.collect::<Vec<_>>().await;
    // List no longer contains the dropped doc
    for entry in ns {
        let (ns, cap) = entry?;
        println!("\t{ns}\t{cap}");
    }
    Ok(())
}
// Output:
// Created document zdv4ciupnlhxzvydn3f227k7tkq3pdljie7de6gtsesghmuu6tyq
// List of docs and their capabilities:
// 	zdv4ciupnlhxzvydn3f227k7tkq3pdljie7de6gtsesghmuu6tyq	CapabilityKind.WRITE
// Dropped document zdv4ciupnlhxzvydn3f227k7tkq3pdljie7de6gtsesghmuu6tyq
// List of docs and their capabilities:
