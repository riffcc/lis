use crate::{message::Operation, Result};
use async_trait::async_trait;

#[async_trait]
pub trait Storage: Send + Sync + std::fmt::Debug {
    async fn apply_operation(&self, operation: &Operation) -> Result<()>;
    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>>;
    async fn scan(&self, prefix: &str) -> Result<Vec<(String, Vec<u8>)>>;
    async fn checkpoint(&self) -> Result<[u8; 32]>;
}

#[derive(Debug)]
pub struct InMemoryStorage {
    data: dashmap::DashMap<String, Vec<u8>>,
}

impl InMemoryStorage {
    pub fn new() -> Self {
        Self {
            data: dashmap::DashMap::new(),
        }
    }
}

impl Default for InMemoryStorage {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Storage for InMemoryStorage {
    async fn apply_operation(&self, operation: &Operation) -> Result<()> {
        match operation.op_type {
            crate::message::OperationType::Write => {
                let (key, value): (String, Vec<u8>) = bincode::deserialize(&operation.data)?;
                self.data.insert(key, value);
            }
            crate::message::OperationType::Delete => {
                let key: String = bincode::deserialize(&operation.data)?;
                self.data.remove(&key);
            }
            _ => {
                // TODO: Implement other operations
            }
        }
        Ok(())
    }
    
    async fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        Ok(self.data.get(key).map(|v| v.clone()))
    }
    
    async fn scan(&self, prefix: &str) -> Result<Vec<(String, Vec<u8>)>> {
        Ok(self.data
            .iter()
            .filter(|entry| entry.key().starts_with(prefix))
            .map(|entry| (entry.key().clone(), entry.value().clone()))
            .collect())
    }
    
    async fn checkpoint(&self) -> Result<[u8; 32]> {
        // Simple hash of all data
        let mut data = Vec::new();
        for entry in self.data.iter() {
            data.extend_from_slice(entry.key().as_bytes());
            data.extend_from_slice(entry.value());
        }
        Ok(crate::crypto::hash(&data))
    }
}