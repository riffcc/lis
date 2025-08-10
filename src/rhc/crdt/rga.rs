// RGA (Replicated Growable Array) CRDT implementation

use super::{CRDT, ActorId};
use crate::rhc::hlc::HLCTimestamp;
use std::collections::HashMap;

/// RGA - CRDT for ordered sequences (like text)
/// Supports concurrent insert/delete operations while maintaining order
#[derive(Debug, Clone)]
pub struct RGA<T: Clone> {
    /// Map from unique ID to element and its metadata
    elements: HashMap<ElementId, Element<T>>,
    /// The root element ID (virtual head)
    root: ElementId,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ElementId {
    actor: ActorId,
    timestamp: HLCTimestamp,
}

#[derive(Debug, Clone)]
struct Element<T> {
    value: Option<T>, // None for deleted elements
    prev: ElementId,
    next: Option<ElementId>,
    deleted: bool,
}

impl<T: Clone> RGA<T> {
    pub fn new(actor: ActorId) -> Self {
        let root = ElementId {
            actor,
            timestamp: HLCTimestamp::new(0, 0),
        };
        
        let mut elements = HashMap::new();
        elements.insert(root.clone(), Element {
            value: None,
            prev: root.clone(),
            next: None,
            deleted: false,
        });
        
        Self { elements, root }
    }
    
    /// Insert an element after the given position
    pub fn insert_after(&mut self, prev_id: &ElementId, value: T, actor: ActorId, timestamp: HLCTimestamp) -> ElementId {
        let new_id = ElementId { actor, timestamp };
        
        // Find the correct position considering concurrent inserts
        let next_id = self.find_insert_position(prev_id, &new_id);
        
        // Update previous element
        if let Some(prev_elem) = self.elements.get_mut(prev_id) {
            if next_id.is_none() {
                prev_elem.next = Some(new_id.clone());
            }
        }
        
        // Insert new element
        self.elements.insert(new_id.clone(), Element {
            value: Some(value),
            prev: prev_id.clone(),
            next: next_id.clone(),
            deleted: false,
        });
        
        // Update next element's prev pointer if exists
        if let Some(ref next_id_ref) = next_id {
            if let Some(next_elem) = self.elements.get_mut(next_id_ref) {
                next_elem.prev = new_id.clone();
            }
        }
        
        new_id
    }
    
    /// Delete an element
    pub fn delete(&mut self, id: &ElementId) {
        if let Some(elem) = self.elements.get_mut(id) {
            elem.deleted = true;
            elem.value = None;
        }
    }
    
    /// Get the sequence as a vector (excluding deleted elements)
    pub fn to_vec(&self) -> Vec<T> {
        let mut result = Vec::new();
        let mut current = self.root.clone();
        
        while let Some(elem) = self.elements.get(&current) {
            if let Some(value) = &elem.value {
                if !elem.deleted {
                    result.push(value.clone());
                }
            }
            
            if let Some(next) = &elem.next {
                current = next.clone();
            } else {
                break;
            }
        }
        
        result
    }
    
    /// Find the correct insertion position for concurrent inserts
    fn find_insert_position(&self, prev_id: &ElementId, new_id: &ElementId) -> Option<ElementId> {
        if let Some(prev_elem) = self.elements.get(prev_id) {
            let mut current = prev_elem.next.clone();
            
            while let Some(curr_id) = current {
                if let Some(curr_elem) = self.elements.get(&curr_id) {
                    // Insert before current if new_id < curr_id
                    if new_id < &curr_id {
                        return Some(curr_id);
                    }
                    current = curr_elem.next.clone();
                } else {
                    break;
                }
            }
        }
        
        None
    }
}

impl<T: Clone> CRDT for RGA<T> {
    fn merge(&mut self, other: &Self) {
        // Add all elements from other that we don't have
        for (id, elem) in &other.elements {
            if !self.elements.contains_key(id) {
                self.elements.insert(id.clone(), elem.clone());
            } else if elem.deleted {
                // If other has deleted this element, mark ours as deleted too
                if let Some(our_elem) = self.elements.get_mut(id) {
                    our_elem.deleted = true;
                    our_elem.value = None;
                }
            }
        }
        
        // Rebuild the linked structure
        self.rebuild_links();
    }
    
    fn happens_before(&self, _other: &Self) -> bool {
        false
    }
}

impl<T: Clone> RGA<T> {
    /// Rebuild the linked list structure after merge
    fn rebuild_links(&mut self) {
        // This is a simplified version - in production, you'd need
        // to handle concurrent inserts more carefully
        let mut ordered: Vec<_> = self.elements.keys()
            .filter(|id| *id != &self.root)
            .cloned()
            .collect();
        ordered.sort();
        
        // Update root's next
        if let Some(first) = ordered.first() {
            if let Some(root_elem) = self.elements.get_mut(&self.root) {
                root_elem.next = Some(first.clone());
            }
        }
        
        // Update links between elements
        for i in 0..ordered.len() {
            let id = &ordered[i];
            let prev = if i == 0 { &self.root } else { &ordered[i-1] };
            let next = ordered.get(i + 1).cloned();
            
            if let Some(elem) = self.elements.get_mut(id) {
                elem.prev = prev.clone();
                elem.next = next;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_rga_insert_and_iterate() {
        let actor = ActorId::new("node1");
        let mut rga = RGA::new(actor.clone());
        
        let id1 = rga.insert_after(&rga.root, 'H', actor.clone(), HLCTimestamp::new(100, 0));
        let id2 = rga.insert_after(&id1, 'e', actor.clone(), HLCTimestamp::new(200, 0));
        let id3 = rga.insert_after(&id2, 'l', actor.clone(), HLCTimestamp::new(300, 0));
        let id4 = rga.insert_after(&id3, 'l', actor.clone(), HLCTimestamp::new(400, 0));
        let _id5 = rga.insert_after(&id4, 'o', actor.clone(), HLCTimestamp::new(500, 0));
        
        let text: String = rga.to_vec().into_iter().collect();
        assert_eq!(text, "Hello");
    }
    
    #[test]
    fn test_rga_delete() {
        let actor = ActorId::new("node1");
        let mut rga = RGA::new(actor.clone());
        
        let id1 = rga.insert_after(&rga.root, 'A', actor.clone(), HLCTimestamp::new(100, 0));
        let id2 = rga.insert_after(&id1, 'B', actor.clone(), HLCTimestamp::new(200, 0));
        let _id3 = rga.insert_after(&id2, 'C', actor.clone(), HLCTimestamp::new(300, 0));
        
        rga.delete(&id2);
        
        let text: String = rga.to_vec().into_iter().collect();
        assert_eq!(text, "AC");
    }
}