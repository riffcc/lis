use iroh::client::docs::{Doc, Entry};

use crate::prelude::*;

/// A Lis directory
pub struct Directory {
    full_path: PathBuf,
    root_doc: Doc,
}

impl Directory {
    pub fn new(full_path: &Path, root_doc: Doc) -> Self {
        // check if directory doesn't already exist
        Directory {
            full_path: full_path.to_path_buf(),
            root_doc,
        }
    }
}

// impl Iterator for Directory {
//     // we will be counting with usize
//     type Item = Doc;

//     // next() is the only required method
//     fn next(&mut self) -> Option<Self::Item> {
//         // Increment our count. This is why we started at zero.
//         self.count += 1;

//         // Check to see if we've finished counting or not.
//         if self.count < 6 {
//             Some(self.count)
//         } else {
//             None
//         }
//     }
// }
