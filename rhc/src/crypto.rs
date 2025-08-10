use blake3::Hasher;
use blst::{
    min_pk::{AggregateSignature, PublicKey, SecretKey, Signature as BlsSignature},
    BLST_ERROR,
};
use ed25519_dalek::{SigningKey, VerifyingKey, Signature as Ed25519Signature, Signer, Verifier};
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Signature {
    Ed25519(Vec<u8>),
    Bls(Vec<u8>),
}

impl Default for Signature {
    fn default() -> Self {
        Self::Ed25519(vec![0; 64])
    }
}

#[derive(Debug, Clone)]
pub struct Ed25519KeyPair {
    signing_key: SigningKey,
}

impl Ed25519KeyPair {
    pub fn generate() -> Self {
        let mut bytes = [0u8; 32];
        rand::Rng::fill(&mut rand::thread_rng(), &mut bytes);
        let signing_key = SigningKey::from_bytes(&bytes);
        Self { signing_key }
    }
    
    pub fn sign(&self, message: &[u8]) -> Signature {
        let signature = self.signing_key.sign(message);
        Signature::Ed25519(signature.to_bytes().to_vec())
    }
    
    pub fn verify(&self, message: &[u8], signature: &Signature) -> crate::Result<()> {
        match signature {
            Signature::Ed25519(bytes) => {
                if bytes.len() != 64 {
                    return Err(crate::Error::Crypto("Invalid signature length".to_string()));
                }
                let mut sig_bytes = [0u8; 64];
                sig_bytes.copy_from_slice(bytes);
                let sig = Ed25519Signature::from_bytes(&sig_bytes);
                let verifying_key = self.signing_key.verifying_key();
                verifying_key.verify(message, &sig)
                    .map_err(|e| crate::Error::Crypto(e.to_string()))
            }
            _ => Err(crate::Error::Crypto("Invalid signature type".to_string())),
        }
    }
    
    pub fn public_key(&self) -> VerifyingKey {
        self.signing_key.verifying_key()
    }
}

#[derive(Debug, Clone)]
pub struct BlsKeyPair {
    secret_key: SecretKey,
    public_key: PublicKey,
}

impl BlsKeyPair {
    pub fn generate() -> Self {
        let mut ikm = [0u8; 32];
        rand::Rng::fill(&mut rand::thread_rng(), &mut ikm);
        let secret_key = SecretKey::key_gen(&ikm, &[]).unwrap();
        let public_key = secret_key.sk_to_pk();
        Self { secret_key, public_key }
    }
    
    pub fn sign(&self, message: &[u8]) -> Signature {
        let sig = self.secret_key.sign(message, b"RHC", &[]);
        Signature::Bls(sig.to_bytes().to_vec())
    }
    
    pub fn public_key(&self) -> PublicKey {
        self.public_key.clone()
    }
}

#[derive(Debug)]
pub struct ThresholdSignatureAggregator {
    threshold: usize,
    shares: HashMap<crate::NodeId, BlsSignature>,
}

impl ThresholdSignatureAggregator {
    pub fn new(threshold: usize) -> Self {
        Self {
            threshold,
            shares: HashMap::new(),
        }
    }
    
    pub fn add_share(&mut self, node_id: crate::NodeId, share: &Signature) -> crate::Result<()> {
        match share {
            Signature::Bls(bytes) => {
                let sig = BlsSignature::from_bytes(bytes)
                    .map_err(|e| crate::Error::Crypto(format!("Invalid BLS signature: {:?}", e)))?;
                self.shares.insert(node_id, sig);
                Ok(())
            }
            _ => Err(crate::Error::Crypto("Expected BLS signature".to_string())),
        }
    }
    
    pub fn has_threshold(&self) -> bool {
        self.shares.len() >= self.threshold
    }
    
    pub fn aggregate(&self) -> crate::Result<Signature> {
        if !self.has_threshold() {
            return Err(crate::Error::InsufficientShares {
                got: self.shares.len(),
                need: self.threshold,
            });
        }
        
        let sigs: Vec<&BlsSignature> = self.shares.values().collect();
        let agg_sig = match AggregateSignature::aggregate(&sigs[..], false) {
            Ok(sig) => sig,
            Err(BLST_ERROR::BLST_AGGR_TYPE_MISMATCH) => {
                return Err(crate::Error::Crypto("Signature type mismatch".to_string()));
            }
            Err(e) => {
                return Err(crate::Error::Crypto(format!("Aggregation failed: {:?}", e)));
            }
        };
        
        Ok(Signature::Bls(agg_sig.to_signature().to_bytes().to_vec()))
    }
}

pub fn hash(data: &[u8]) -> [u8; 32] {
    let mut hasher = Hasher::new();
    hasher.update(data);
    *hasher.finalize().as_bytes()
}

pub fn verify_threshold_signature(
    message: &[u8],
    signature: &Signature,
    public_keys: &[PublicKey],
) -> crate::Result<()> {
    match signature {
        Signature::Bls(bytes) => {
            let sig = BlsSignature::from_bytes(bytes)
                .map_err(|e| crate::Error::Crypto(format!("Invalid signature: {:?}", e)))?;
            
            let pk_refs: Vec<&PublicKey> = public_keys.iter().collect();
            let result = sig.fast_aggregate_verify(true, message, b"RHC", &pk_refs);
            
            match result {
                BLST_ERROR::BLST_SUCCESS => Ok(()),
                e => Err(crate::Error::Crypto(format!("Verification failed: {:?}", e))),
            }
        }
        _ => Err(crate::Error::Crypto("Expected BLS signature".to_string())),
    }
}