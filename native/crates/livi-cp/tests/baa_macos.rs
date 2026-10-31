// SPDX-License-Identifier: GPL-3.0-or-later
// Additional terms: see NOTICE
#![cfg(target_os = "macos")]

use std::sync::Mutex;

use iap2_mfi::{AuthCoprocessor, BasicAccessoryAuth};
use livi_cp::auth_setup::{self, MfiSigner};
use livi_cp::crypto::{X25519, aes_ctr128, sha1};
use livi_cp::opack;
use openssl::{hash::MessageDigest, sign::Verifier, x509::X509};

struct Signer {
    auth: Mutex<BasicAccessoryAuth>,
    intermediate: Mutex<Option<Vec<u8>>>,
}

impl MfiSigner for Signer {
    async fn certificate(&self) -> Result<Vec<u8>, String> {
        let cert = self
            .auth
            .lock()
            .unwrap()
            .authentication_certificate()
            .map_err(|e| e.to_string())?;
        *self.intermediate.lock().unwrap() = cert.intermediate;
        Ok(cert.data)
    }
    async fn intermediate_certificate(&self) -> Result<Option<Vec<u8>>, String> {
        Ok(self.intermediate.lock().unwrap().clone())
    }
    async fn protocol_major(&self) -> Result<u8, String> {
        Ok(0)
    }
    async fn sign(&self, message: &[u8]) -> Result<Vec<u8>, String> {
        self.auth
            .lock()
            .unwrap()
            .generate_challenge_response(message)
            .map_err(|e| e.to_string())
    }
}

fn verify(cert: &X509, message: &[u8], signature: &[u8]) {
    let key = cert.public_key().unwrap();
    let mut verifier = Verifier::new(MessageDigest::sha256(), &key).unwrap();
    verifier.update(message).unwrap();
    assert!(verifier.verify(signature).unwrap());
}

#[tokio::test]
#[ignore = "Issues a device-bound identity in the macOS Keychain and may contact Apple"]
async fn live_baa_certificate_and_both_handshakes() {
    assert!(
        BasicAccessoryAuth::available(),
        "macOS DeviceIdentity BAA ABI is unavailable"
    );
    assert!(
        BasicAccessoryAuth::has_required_entitlements(),
        "sign the test executable with the BAA entitlements and matching application identifier"
    );
    let signer = Signer {
        auth: Mutex::new(BasicAccessoryAuth::default()),
        intermediate: Mutex::new(None),
    };
    let cert = X509::from_der(&signer.certificate().await.unwrap()).unwrap();
    let intermediate =
        X509::from_der(&signer.intermediate_certificate().await.unwrap().unwrap()).unwrap();
    assert!(cert.verify(&intermediate.public_key().unwrap()).unwrap());
    let challenge = [0xa5; 32];
    verify(&cert, &challenge, &signer.sign(&challenge).await.unwrap());

    let phone = X25519::generate();
    let phone_public = phone.public;
    let request = [&[1][..], &phone_public].concat();
    let response = auth_setup::handle(&request, &signer)
        .await
        .unwrap()
        .unwrap();
    let ours: [u8; 32] = response[..32].try_into().unwrap();
    let cert_len = u32::from_be_bytes(response[32..36].try_into().unwrap()) as usize;
    let cert = X509::from_der(&response[36..36 + cert_len]).unwrap();
    let at = 36 + cert_len;
    let sig_len = u32::from_be_bytes(response[at..at + 4].try_into().unwrap()) as usize;
    let sig_at = at + 4;
    let shared = phone.shared(&ours).unwrap();
    let key = sha1(&[b"AES-KEY", &shared])[..16].try_into().unwrap();
    let iv = sha1(&[b"AES-IV", &shared])[..16].try_into().unwrap();
    let signature = aes_ctr128(&key, &iv, &response[sig_at..sig_at + sig_len]);
    verify(&cert, &[&ours[..], &phone_public[..]].concat(), &signature);
    let at = sig_at + sig_len;
    let extra_len = u32::from_be_bytes(response[at..at + 4].try_into().unwrap()) as usize;
    assert_eq!(response.len(), at + 4 + extra_len);
    let extra = opack::decode(&response[at + 4..]).unwrap();
    assert_eq!(
        extra.get("baIC").unwrap().as_data().unwrap(),
        intermediate.to_der().unwrap()
    );
}
