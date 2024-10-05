use crate::objects::Metadata;
use crate::prelude::*;

pub struct LisFile {
    chunks: FileChunks,
    metadata: Metadata,
}

pub struct FileChunks {
    doc_id: NamespaceId,
}
