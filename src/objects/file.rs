use bytes::{BufMut, BytesMut};

use crate::{
    doc::LisDoc,
    objects::{FromNamespaceId, Metadata, ObjectAttributes, ObjectType},
    prelude::*,
};

#[derive(Debug, Clone)]
pub struct LisFile {
    doc: LisDoc,
    chunks: Chunks,
    metadata: Metadata,
}

impl LisFile {
    pub async fn new(node: &Iroh) -> Result<(Self, NamespaceId)> {
        let (chunks, chunks_id) = Chunks::new(&node.clone()).await?;
        let (metadata, metadata_id) = Metadata::new(&node.clone(), ObjectType::File).await?;

        let doc = LisDoc::new(node).await?;
        doc.set(
            node,
            Key::from(".metadata".to_string()),
            namespace_id_to_bytes(metadata_id),
        )
        .await?;
        doc.set(
            node,
            Key::from(".chunks".to_string()),
            namespace_id_to_bytes(chunks_id),
        )
        .await?;
        doc.set(node, Key::from(".type".to_string()), "file".into())
            .await?;

        let id = doc.id();
        Ok((
            Self {
                doc,
                chunks,
                metadata,
            },
            id,
        ))
    }
    pub async fn size(&self, node: &Iroh) -> Result<usize> {
        let (chunks, chunk_size) = match self.metadata.attrs {
            ObjectAttributes::FileAttributes { chunks, chunk_size } => (chunks, chunk_size),
            _ => return Err(anyhow!("Incorrect file attributes in metadata")),
        };
        if chunks == 0 {
            return Ok(0);
        }
        let last_chunk_id = chunks.saturating_sub(1);
        let last_chunk_bytes: Bytes = self
            .chunks
            .doc
            .get(node, Key::from(last_chunk_id))
            .await?
            .ok_or_else(|| anyhow!("Could not read chunk {last_chunk_id}"))?;
        Ok((chunks - 1) * chunk_size + last_chunk_bytes.len())
    }
    pub async fn read(&self, node: &Iroh, offset: usize, size: usize) -> Result<Bytes> {
        if size == 0 {
            return Ok(Bytes::new());
        }

        let (chunks, chunk_size) = match self.metadata.attrs {
            ObjectAttributes::FileAttributes { chunks, chunk_size } => (chunks, chunk_size),
            _ => return Err(anyhow!("Incorrect file attributes in metadata")),
        };
        if offset > self.size(node).await? {
            return Err(anyhow!("Invalid offset, must be smaller than file size"));
        }
        let start_chunk: usize = offset / chunk_size;
        let mut end_chunk: usize = (offset + size - 1) / chunk_size;
        if end_chunk >= chunks {
            end_chunk = chunks.saturating_sub(1);
        }

        let mut bytes = BytesMut::new();
        for chunk_id in start_chunk..=end_chunk {
            let chunk_bytes = self.chunks.read(node, chunk_id).await?;
            bytes.extend_from_slice(&chunk_bytes);
        }
        // we might not want entire last and first chunks
        let first_byte = offset % chunk_size;
        let last_byte = first_byte + size;
        let bytes = Bytes::copy_from_slice(&bytes[first_byte..last_byte]);

        Ok(bytes)
    }
    pub async fn read_all(&self, node: &Iroh) -> Result<Bytes> {
        let offset = 0;
        let size = self.size(node).await?;
        self.read(node, offset, size).await
    }
    /// Returns the number of bytes written to file
    pub async fn write(&mut self, node: &Iroh, offset: usize, bytes: Bytes) -> Result<usize> {
        if bytes.len() == 0 {
            return Ok(0);
        }

        if offset > self.size(node).await? {
            return Err(anyhow!(
                "Invalid offset, must be smaller or equal to file size"
            ));
        }

        let (_chunks, chunk_size) = match self.metadata.attrs {
            ObjectAttributes::FileAttributes { chunks, chunk_size } => (chunks, chunk_size),
            _ => return Err(anyhow!("Incorrect file attributes in metadata")),
        };

        let start_chunk: usize = offset / chunk_size;
        if start_chunk > self.chunks.size {
            return Err(anyhow!("Offset is past end of file"));
        }
        // end_chunk can be past end of file, chunks are created
        let end_chunk: usize = (offset + bytes.len() - 1) / chunk_size;

        for i in start_chunk..=end_chunk {
            let last_byte_in_chunk = (chunk_size * (i + 1)).min(bytes.len());
            let bytes_in_chunk = bytes.slice((chunk_size * i)..last_byte_in_chunk);
            if i >= self.chunks.size {
                self.chunks.size += 1;
                if let ObjectAttributes::FileAttributes { ref mut chunks, .. } = self.metadata.attrs
                {
                    *chunks += 1;
                } else {
                    return Err(anyhow!("Incorrect file attributes in metadata"));
                }
            }
            let mut chunk_offset = 0;
            if i == start_chunk {
                chunk_offset = offset;
            }
            self.chunks
                .write(node, i, chunk_offset, bytes_in_chunk)
                .await?;
        }
        Ok(bytes.len())
    }
}

impl FromNamespaceId for LisFile {
    async fn from_namespace_id(node: &Iroh, id: NamespaceId) -> Result<Self> {
        let doc = LisDoc::from_namespace_id(node, id).await?;

        // check type
        if doc.doc_type(node).await? != DocType::FileDoc {
            return Err(anyhow!("NamespaceId does not correspond to a file doc"));
        }

        let chunks_key = Key::from(".chunks".to_string());
        let chunks_id = bytes_to_namespace_id(
            doc.get(node, chunks_key)
                .await?
                .ok_or(anyhow!("Could not find chunks key in file doc"))?,
        )?;

        let metadata_key = Key::from(".metadata".to_string());
        let metadata_id = bytes_to_namespace_id(
            doc.get(node, metadata_key)
                .await?
                .ok_or(anyhow!("Could not find metadata key in file doc"))?,
        )?;

        Ok(Self {
            doc,
            chunks: Chunks::from_namespace_id(&node.clone(), chunks_id).await?,
            metadata: Metadata::from_namespace_id(&node.clone(), metadata_id).await?,
        })
    }
}

#[derive(Debug, Clone)]
pub struct Chunks {
    doc: LisDoc,
    size: usize,
}

impl Chunks {
    pub async fn new(node: &Iroh) -> Result<(Self, NamespaceId)> {
        let doc = LisDoc::new(node).await?;
        doc.set(node, Key::from(".type".to_string()), "chunks".into())
            .await?;
        let id = doc.id();
        let size = 0;
        Ok((Self { doc, size }, id))
    }
    /// Returns the bytes inside a chunk
    pub async fn read(&self, node: &Iroh, chunk_id: usize) -> Result<Bytes> {
        let chunk_bytes: Bytes = self
            .doc
            .get(node, Key::from(chunk_id))
            .await?
            .ok_or_else(|| anyhow!("Could not read chunk {chunk_id}"))?;
        Ok(chunk_bytes)
    }

    /// Overwrites the first bytes in chunk
    /// Returns the number of bytes written
    pub async fn write(
        &mut self,
        node: &Iroh,
        chunk_id: usize,
        chunk_offset: usize,
        bytes: Bytes,
    ) -> Result<usize> {
        let chunk_bytes: BytesMut = self
            .doc
            .get(node, Key::from(chunk_id))
            .await?
            .unwrap_or_default()
            .into();

        if chunk_offset > chunk_bytes.len() {
            return Err(anyhow!("Chunk offset greater than chunk bytes length"));
        }

        let mut new_chunk_bytes = BytesMut::new();
        if chunk_offset > 0 {
            new_chunk_bytes.put(&chunk_bytes[..chunk_offset]); // copy existing data before chunk_offset
        }
        new_chunk_bytes.put(&bytes[..]);
        // preserve any remaining part of chunk_bytes after chunk_offset + bytes.len()
        let remaining_offset = chunk_offset + bytes.len();
        if chunk_bytes.len() > remaining_offset {
            new_chunk_bytes.put(&chunk_bytes[remaining_offset..]);
        }

        self.doc
            .set(node, Key::from(chunk_id), new_chunk_bytes.into())
            .await?;
        Ok(bytes.len())
    }
    // /// Deletes a chunk and renumbers following chunks
    // /// Returns id of deleted chunk.
    // pub async fn delete(&mut self, node: &Iroh, id: usize) -> Result<usize> {
    //     self.doc.set(node, Key::from(chunk_id), chunk_bytes).await?;
    //     Ok(id)
    // }
}
impl FromNamespaceId for Chunks {
    async fn from_namespace_id(node: &Iroh, id: NamespaceId) -> Result<Self> {
        let doc = LisDoc::from_namespace_id(node, id).await?;

        // check type
        if doc.doc_type(node).await? != DocType::ChunksDoc {
            return Err(anyhow!("NamespaceId does not correspond to a chunks doc"));
        }

        let size = 0;

        Ok(Self { doc, size })
    }
}
