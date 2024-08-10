use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct Manifest {
    pub files_doc_id: String, // doc hash for Files document
}

impl Manifest {
    pub fn new(doc_id: String) -> Self {
        Manifest {
            files_doc_id: doc_id,
        }
    }
}
