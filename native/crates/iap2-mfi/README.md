# macOS Basic Accessory Authentication

LIVI's experimental BAA backend obtains a device-bound identity from macOS's
DeviceIdentity service and signs with Security.framework. It does not load or
redistribute CarPlay Simulator code, export its credentials, or emulate an MFi
coprocessor.

## Signing and running

Stock macOS does not authorize ordinary third-party processes to use this API.
The backend has been tested on a Mac with SIP and AMFI already disabled, with an
ad-hoc signature carrying these entitlements:

- `com.apple.mobileactivationd.deviceidentity-allowlist-exempt`
- `com.apple.mobileactivationd.spi`
- `com.apple.mobileactivationd.device-identifiers`
- `com.apple.security.attestation.access`
- `application-identifier` matching the executable's signing identifier

The allowlist exemption passes `mobileactivationd`'s client check. The application
identifier is also necessary for Keychain access: leaving it out, or using a
different signing identifier, produced `errSecMissingEntitlement` (-34018).
These observations are specific to the tested macOS build; the private ABI and
authorization policy can change.

To build a development package with BAA signing, from `native/`:

```sh
LIVI_MAC_BAA=1 cargo xtask package-mac --arch arm64
```

This signs only `livi-helperd` with
`assets/mac/entitlements.mac.baa.plist` and the identifier
`dev.f-io.livi.helperd`. It does not change SIP, AMFI, or any system settings.
Normal packages retain their existing entitlements and work on stock macOS.

At startup, the macOS UI also checks the active SIP status and `kern.bootargs`.
If SIP is fully disabled and `amfi_get_out_of_my_way` has a nonzero decimal or
hexadecimal value (including `0x1`), it automatically repairs a helper missing
the BAA entitlements. A detached worker signs and verifies a staged helper,
terminates the launching LIVI process before it starts core, installs the helper,
and relaunches LIVI with its original arguments. For a bundled helper it also
refreshes and verifies the outer app signature, preserving the app's signing
identifier and entitlements. It honors `LIVI_HELPER_BIN`, `LIVI_RESOURCES`, and
`LIVI_ROOT`. Signing errors go to `livi-baa-signing-<pid>/repair.log` in the user's
temporary directory; an error before shutdown leaves LIVI running normally.
This only reads the system configuration; it does not change SIP or boot args.
The worker restores the original helper if installation or app signing fails,
and its relaunch has a one-launch guard to prevent repeated repair attempts.
Fixture previews and a UI launched by core skip automatic signing.

To sign manually when automatic repair is skipped, sign the helper launched by
`out/bin/LIVI`, from the repository root after building it:

```sh
codesign --force --sign - --identifier dev.f-io.livi.helperd \
  --entitlements assets/mac/entitlements.mac.baa.plist \
  out/bin/livi-helperd
```

Signing `native/target/debug/livi-helperd` does not sign the separate copy in
`out/bin`. To use a debug helper instead, sign that executable and launch with
`LIVI_HELPER_BIN="$PWD/native/target/debug/livi-helperd" out/bin/LIVI`.

Rebuilding replaces the signature. Eligible machines repeat the automatic repair
on the next launch; otherwise repeat manual signing. The helper checks its
entitlements before selecting BAA or allowing wired CarPlay to start
without a LIVI Link. Imported MFi files take priority, then a connected LIVI Link,
then BAA. Disconnecting the Link restores BAA.

The initial certificate request may contact Apple and access the current user's
Keychain. Run the helper in that user's normal login session. The request has a
60-second network timeout and a bounded 65-second wait. The certificate and key
remain paired in memory, with certificate refresh requested after 11 hours of
the 12-hour requested lifetime. The private key stays with macOS.

## Protocol details and evidence

The implementation was derived from the installed CarPlay Simulator 4.0,
CarPlaySDK source version 51, and the local `mobileactivationd` executable:

- `BAADeviceIdentity._issueCertificateWithSCRT:` calls
  `DeviceIdentityIssueClientCertificateWithCompletion` with UCRT attestation,
  a device-only access control, existing-key reuse, 60-second timeout,
  43200-second validity, and the UCRT device-identifiers OID.
- iAP2 `AuthenticationCertificate` (0xAA01) carries the leaf DER certificate in
  parameter 0, authentication type 1 in parameter 1, and the intermediate DER
  certificate in parameter 2.
- Challenges are messages signed with
  `kSecKeyAlgorithmECDSASignatureMessageX962SHA256`, returning variable-length
  X9.62 DER ECDSA signatures. Hardware MFi instead signs supplied digests.
- AirPlay `/auth-setup` signs the raw 64 bytes `accessoryPublic || phonePublic`.
  Its normal certificate/signature response is followed by a big-endian u32
  length and an OPACK dictionary containing `baIC: intermediateDER`.
- BAA has no hardware MFi protocol major version. LIVI uses 0 internally and
  does not offer MFi4 mutual authentication for this backend.
- `mobileactivationd.isSupportedDeviceIdentityClient` accepts the explicit
  `deviceidentity-allowlist-exempt` entitlement in place of its process allowlist.

## Validation

The ordinary tests cover the iAP2 wire vector and challenge exchange, the raw
AirPlay signing input, AES decryption and OPACK trailer, clearing BAA metadata
when hardware takes over, and excluding BAA from MFi4 negotiation.

`livi-cp/tests/baa_macos.rs` is an ignored live test because it may issue a real
identity and contact Apple. Build it with:

```sh
cd native
cargo test -p livi-cp --test baa_macos --no-run
```

Copy the test executable printed by Cargo to a temporary path, sign the copy
using the BAA entitlement file and matching identifier above, then run the copy
with `--ignored --nocapture`, outside a sandbox that blocks Keychain access.

The live test has successfully obtained an Apple-issued certificate, verified
the leaf against its intermediate, verified a signed iAP2 challenge, and verified
the decrypted AirPlay exchange signature and intermediate-certificate trailer.
A complete CarPlay session with a physical iPhone has not yet been validated.
