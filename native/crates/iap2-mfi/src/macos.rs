// SPDX-FileCopyrightText: 2025-2026 Lasse Heitgres
// SPDX-License-Identifier: GPL-3.0-or-later
// Additional terms: see NOTICE

use std::ffi::{CStr, c_char, c_void};
use std::ptr::NonNull;
use std::time::{Duration, Instant};

use crate::{AuthCertificate, AuthCoprocessor, CHALLENGE_MAX, CHALLENGE_MIN, MfiError};

unsafe extern "C" {
    fn livi_baa_available() -> bool;
    fn livi_baa_has_entitlements() -> bool;
    fn livi_baa_create(error: *mut c_char, capacity: usize) -> *mut c_void;
    fn livi_baa_free(handle: *mut c_void);
    fn livi_baa_certificate(
        handle: *mut c_void,
        intermediate: bool,
        bytes: *mut *mut u8,
        length: *mut usize,
    ) -> bool;
    fn livi_baa_sign(
        handle: *mut c_void,
        challenge: *const u8,
        length: usize,
        bytes: *mut *mut u8,
        out_length: *mut usize,
        error: *mut c_char,
        capacity: usize,
    ) -> bool;
    fn livi_baa_bytes_free(bytes: *mut u8);
}

struct Identity(NonNull<c_void>);

// SAFETY: the retained Objective-C identity has no thread affinity. All access
// is exclusive through &mut BasicAccessoryAuth (SharedCoprocessor adds a mutex).
unsafe impl Send for Identity {}

impl Drop for Identity {
    fn drop(&mut self) {
        // SAFETY: this handle owns exactly one retain from livi_baa_create.
        unsafe { livi_baa_free(self.0.as_ptr()) };
    }
}

fn error(buffer: &[c_char]) -> MfiError {
    // SAFETY: native errors are snprintf-written into a zero-initialized buffer.
    MfiError::Platform(
        unsafe { CStr::from_ptr(buffer.as_ptr()) }
            .to_string_lossy()
            .into_owned(),
    )
}

fn copy_bytes(bytes: *mut u8, length: usize) -> Result<Vec<u8>, MfiError> {
    if bytes.is_null() {
        return Err(MfiError::Platform(
            "could not allocate certificate or signature".into(),
        ));
    }
    // SAFETY: the shim returns length initialized bytes in a malloc allocation.
    let result = unsafe { std::slice::from_raw_parts(bytes, length) }.to_vec();
    unsafe { livi_baa_bytes_free(bytes) };
    Ok(result)
}

/// Apple's device-bound Basic Accessory Authentication. No Simulator files or
/// exported private keys are needed. Certificate issuance is lazy and blocking;
/// callers should use SharedCoprocessor's blocking worker.
#[derive(Default)]
pub struct BasicAccessoryAuth {
    identity: Option<(Identity, Instant)>,
}

impl BasicAccessoryAuth {
    pub fn available() -> bool {
        // SAFETY: this only resolves the runtime ABI; it does not issue a key.
        unsafe { livi_baa_available() }
    }

    /// Preflight for LIVI's separately signed helper. The OS still makes the
    /// final authorization decision when issuing the certificate.
    pub fn has_required_entitlements() -> bool {
        // SAFETY: this only reads this process's signing entitlements.
        unsafe { livi_baa_has_entitlements() }
    }

    fn identity(&mut self, refresh: bool) -> Result<&Identity, MfiError> {
        // The requested certificate lifetime is 12 hours. Refresh on certificate
        // requests, never between handing out a certificate and signing for it.
        if self.identity.is_none()
            || (refresh
                && self.identity.as_ref().unwrap().1.elapsed() >= Duration::from_secs(11 * 3600))
        {
            let mut message = [0; 1024];
            // SAFETY: message is a writable buffer of the declared size.
            let handle = unsafe { livi_baa_create(message.as_mut_ptr(), message.len()) };
            let handle = NonNull::new(handle).ok_or_else(|| error(&message))?;
            self.identity = Some((Identity(handle), Instant::now()));
        }
        Ok(&self.identity.as_ref().unwrap().0)
    }

    fn certificate_part(identity: &Identity, intermediate: bool) -> Result<Vec<u8>, MfiError> {
        let (mut bytes, mut length) = (std::ptr::null_mut(), 0);
        // SAFETY: identity is retained; both output pointers refer to live locals.
        if !unsafe {
            livi_baa_certificate(identity.0.as_ptr(), intermediate, &mut bytes, &mut length)
        } {
            return Err(MfiError::Platform(
                "could not copy the device certificate".into(),
            ));
        }
        copy_bytes(bytes, length)
    }
}

impl AuthCoprocessor for BasicAccessoryAuth {
    fn protocol_major(&mut self) -> Result<u8, MfiError> {
        // BAA has no hardware protocol version and cannot do MFi4 mutual auth.
        Ok(0)
    }

    fn read_certificate(&mut self) -> Result<Vec<u8>, MfiError> {
        Self::certificate_part(self.identity(true)?, false)
    }

    fn authentication_certificate(&mut self) -> Result<AuthCertificate, MfiError> {
        let identity = self.identity(true)?;
        Ok(AuthCertificate {
            data: Self::certificate_part(identity, false)?,
            intermediate: Some(Self::certificate_part(identity, true)?),
        })
    }

    fn generate_challenge_response(&mut self, challenge: &[u8]) -> Result<Vec<u8>, MfiError> {
        if !(CHALLENGE_MIN..=CHALLENGE_MAX).contains(&challenge.len()) {
            return Err(MfiError::ChallengeSize(challenge.len()));
        }
        let identity = self.identity(false)?;
        let (mut bytes, mut length) = (std::ptr::null_mut(), 0);
        let mut message = [0; 1024];
        // SAFETY: the identity and input slice are live for the call; all output
        // pointers refer to writable locals. The shim returns malloc-owned bytes.
        if !unsafe {
            livi_baa_sign(
                identity.0.as_ptr(),
                challenge.as_ptr(),
                challenge.len(),
                &mut bytes,
                &mut length,
                message.as_mut_ptr(),
                message.len(),
            )
        } {
            return Err(error(&message));
        }
        copy_bytes(bytes, length)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_challenges_never_issue_an_identity() {
        let mut auth = BasicAccessoryAuth::default();
        for challenge in [vec![], vec![0; CHALLENGE_MAX + 1]] {
            assert!(matches!(
                auth.generate_challenge_response(&challenge),
                Err(MfiError::ChallengeSize(_))
            ));
        }
        assert!(auth.identity.is_none());
        assert_eq!(auth.protocol_major().unwrap(), 0);
    }
}
