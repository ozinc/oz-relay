// Copyright (c) 2026 OZ Global Inc.
// Licensed under the Apache License, Version 2.0.

//! Artifact signer — Ed25519 signing of compiled binaries.

use ring::rand::SystemRandom;
use ring::signature::{Ed25519KeyPair, KeyPair, UnparsedPublicKey, ED25519};
use sha2::{Digest, Sha256};

use oz_relay_common::intent::ArtifactManifest;

pub struct ArtifactSigner {
    keypair: Ed25519KeyPair,
}

impl ArtifactSigner {
    pub fn generate() -> Result<Self, String> {
        let rng = SystemRandom::new();
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&rng)
            .map_err(|e| format!("key generation failed: {}", e))?;
        let keypair = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref())
            .map_err(|e| format!("key parsing failed: {}", e))?;
        Ok(Self { keypair })
    }

    pub fn from_pkcs8(der: &[u8]) -> Result<Self, String> {
        let keypair = Ed25519KeyPair::from_pkcs8(der)
            .map_err(|e| format!("key parsing failed: {}", e))?;
        Ok(Self { keypair })
    }

    pub fn public_key_bytes(&self) -> &[u8] {
        self.keypair.public_key().as_ref()
    }

    pub fn public_key_hex(&self) -> String {
        hex::encode(self.public_key_bytes())
    }

    pub fn sign_artifact(
        &self,
        artifact_bytes: &[u8],
        abi_version: &str,
        target_triple: &str,
        arcflow_version: &str,
    ) -> ArtifactManifest {
        let mut hasher = Sha256::new();
        hasher.update(artifact_bytes);
        let hash = hasher.finalize();
        let sha256_hex = hex::encode(hash);

        let signature = self.keypair.sign(hash.as_slice());
        let signature_hex = hex::encode(signature.as_ref());

        ArtifactManifest {
            sha256: sha256_hex,
            signature: signature_hex,
            abi_version: abi_version.to_string(),
            target_triple: target_triple.to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            arcflow_version: arcflow_version.to_string(),
        }
    }
}

pub fn verify_signature(
    artifact_bytes: &[u8],
    manifest: &ArtifactManifest,
    public_key_bytes: &[u8],
) -> Result<bool, String> {
    let mut hasher = Sha256::new();
    hasher.update(artifact_bytes);
    let hash = hasher.finalize();
    let computed_hex = hex::encode(hash);

    if computed_hex != manifest.sha256 {
        return Ok(false);
    }

    let signature_bytes =
        hex::decode(&manifest.signature).map_err(|e| format!("invalid signature hex: {}", e))?;

    let public_key = UnparsedPublicKey::new(&ED25519, public_key_bytes);
    match public_key.verify(hash.as_slice(), &signature_bytes) {
        Ok(()) => Ok(true),
        Err(_) => Ok(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_and_verify() {
        let signer = ArtifactSigner::generate().unwrap();
        let artifact = b"fake compiled binary content for testing";
        let manifest = signer.sign_artifact(artifact, "1.0", "aarch64-apple-darwin", "1.7.0");
        assert!(!manifest.sha256.is_empty());
        assert!(!manifest.signature.is_empty());
        let verified = verify_signature(artifact, &manifest, signer.public_key_bytes()).unwrap();
        assert!(verified);
    }

    #[test]
    fn tampered_artifact_rejected() {
        let signer = ArtifactSigner::generate().unwrap();
        let artifact = b"original binary";
        let manifest = signer.sign_artifact(artifact, "1.0", "aarch64-apple-darwin", "1.7.0");
        let tampered = b"tampered binary";
        let verified = verify_signature(tampered, &manifest, signer.public_key_bytes()).unwrap();
        assert!(!verified);
    }

    #[test]
    fn wrong_key_rejected() {
        let signer1 = ArtifactSigner::generate().unwrap();
        let signer2 = ArtifactSigner::generate().unwrap();
        let artifact = b"some binary";
        let manifest = signer1.sign_artifact(artifact, "1.0", "x86_64-unknown-linux-gnu", "1.7.0");
        let verified = verify_signature(artifact, &manifest, signer2.public_key_bytes()).unwrap();
        assert!(!verified);
    }

    #[test]
    fn public_key_hex_stable() {
        let signer = ArtifactSigner::generate().unwrap();
        let hex1 = signer.public_key_hex();
        let hex2 = signer.public_key_hex();
        assert_eq!(hex1, hex2);
        assert_eq!(hex1.len(), 64);
    }
}
