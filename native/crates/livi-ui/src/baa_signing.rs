// SPDX-FileCopyrightText: 2026 Lasse Heitgres
// SPDX-License-Identifier: GPL-3.0-or-later
// Additional terms: see NOTICE

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::DirBuilderExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const ENTITLEMENTS: &str = include_str!("../../../../assets/mac/entitlements.mac.baa.plist");
const WORKER: &str = include_str!("baa_signing.sh");
const ATTEMPTED: &str = "LIVI_BAA_REPAIR_ATTEMPTED";
const IDENTIFIER: &str = "dev.f-io.livi.helperd";
const REQUIRED: &[&str] = &[
    "com.apple.mobileactivationd.deviceidentity-allowlist-exempt",
    "com.apple.mobileactivationd.spi",
    "com.apple.mobileactivationd.device-identifiers",
    "com.apple.security.attestation.access",
];

fn sip_disabled(status: &str) -> bool {
    status
        .lines()
        .any(|line| line.trim() == "System Integrity Protection status: disabled.")
}

fn amfi_disabled(args: &str) -> bool {
    // Accept decimal and hexadecimal values, but skip ambiguous duplicates.
    let mut values = args
        .split_whitespace()
        .filter_map(|arg| arg.strip_prefix("amfi_get_out_of_my_way="));
    let Some(value) = values.next() else {
        return false;
    };
    if values.next().is_some() {
        return false;
    }
    if let Some(hex) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        u64::from_str_radix(hex, 16).is_ok_and(|v| v != 0)
    } else {
        value.parse::<u64>().is_ok_and(|v| v != 0)
    }
}

fn output(program: &str, args: &[&str]) -> io::Result<String> {
    let run = Command::new(program)
        .args(args)
        .env("LC_ALL", "C")
        .output()?;
    if !run.status.success() {
        return Err(io::Error::other(
            String::from_utf8_lossy(&run.stderr).trim(),
        ));
    }
    Ok(String::from_utf8_lossy(&run.stdout).into_owned())
}

fn entitled(values: &serde_json::Value) -> bool {
    values["application-identifier"].as_str() == Some(IDENTIFIER)
        && REQUIRED
            .iter()
            .all(|key| values[key].as_bool() == Some(true))
}

fn helper_entitled(helper: &Path) -> io::Result<bool> {
    let signature = Command::new("/usr/bin/codesign")
        .args(["--display", "--verbose=2", "--entitlements", "-", "--xml"])
        .arg(helper)
        .output()?;
    if !signature.status.success()
        || signature.stdout.is_empty()
        || !String::from_utf8_lossy(&signature.stderr)
            .lines()
            .any(|line| line == format!("Identifier={IDENTIFIER}"))
    {
        return Ok(false);
    }
    let mut plist = Command::new("/usr/bin/plutil")
        .args(["-convert", "json", "-o", "-", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    plist.stdin.take().unwrap().write_all(&signature.stdout)?;
    let converted = plist.wait_with_output()?;
    if !converted.status.success() {
        return Err(io::Error::other(
            "cannot read the helper's signing entitlements",
        ));
    }
    let values = serde_json::from_slice(&converted.stdout)?;
    if !entitled(&values) {
        return Ok(false);
    }
    Ok(Command::new("/usr/bin/codesign")
        .arg("--verify")
        .arg(helper)
        .output()?
        .status
        .success())
}

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

fn helper_path(exe: &Path, bundle: Option<&crate::bundle::Bundle>) -> io::Result<PathBuf> {
    let helper = if let Some(helper) = env_path("LIVI_HELPER_BIN") {
        helper
    } else if let Some(resources) = bundle
        .map(|b| b.resources.clone())
        .or_else(|| env_path("LIVI_RESOURCES"))
    {
        resources.join("driver/livi-helperd")
    } else {
        let root = env_path("LIVI_ROOT")
            .or_else(|| {
                exe.ancestors()
                    .find(|dir| dir.join("native/Cargo.toml").is_file())
                    .map(Path::to_path_buf)
            })
            .map(Ok)
            .unwrap_or_else(std::env::current_dir)?;
        root.join("out/bin/livi-helperd")
    };
    helper.canonicalize()
}

pub fn prepare() {
    // This runs before starting core, device watchers, or the UI. A core-owned
    // UI cannot shut down its owner, and fixture previews need no helper.
    if std::env::var_os(ATTEMPTED).is_some()
        || std::env::var(livi_core_proto::LIFELINE_ENV).is_ok_and(|v| v == "1")
        || std::env::var_os("LIVI_UI_STATE").is_some()
    {
        return;
    }
    if let Err(e) = prepare_inner() {
        eprintln!("[ui] automatic BAA signing skipped: {e}");
    }
}

fn prepare_inner() -> io::Result<()> {
    if !sip_disabled(&output("/usr/bin/csrutil", &["status"])?)
        || !amfi_disabled(&output("/usr/sbin/sysctl", &["-n", "kern.bootargs"])?)
    {
        return Ok(());
    }
    let exe = std::env::current_exe()?;
    let bundle = crate::bundle::find();
    let helper = helper_path(&exe, bundle.as_ref())?;
    if helper_entitled(&helper)? {
        return Ok(());
    }
    let work = std::env::temp_dir().join(format!("livi-baa-signing-{}", std::process::id()));
    fs::DirBuilder::new().mode(0o700).create(&work)?;
    fs::write(work.join("entitlements.plist"), ENTITLEMENTS)?;
    let log_path = work.join("repair.log");
    let log = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&log_path)?;
    let app = bundle
        .as_ref()
        .and_then(|b| b.resources.parent()?.parent())
        .filter(|app| helper.starts_with(app))
        .unwrap_or(Path::new(""));
    println!(
        "[ui] SIP and AMFI are disabled; a detached worker will sign {} for BAA and restart LIVI (log: {})",
        helper.display(),
        log_path.display()
    );
    let mut worker = Command::new("/bin/sh")
        .args(["-c", WORKER, "livi-baa-signing"])
        .arg(std::process::id().to_string())
        .arg(&helper)
        .arg(&exe)
        .arg(&work)
        .arg(app)
        .args(std::env::args_os().skip(1))
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log)
        .spawn()?;
    // The worker terminates this process only after signing and validating a
    // staged helper. An early failure returns here and normal startup resumes.
    let status = worker.wait()?;
    Err(io::Error::other(format!(
        "signing worker exited with {status}; see {}",
        log_path.display()
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::process::ExitStatusExt;

    struct RepairTest {
        dir: PathBuf,
        helper: PathBuf,
    }

    impl RepairTest {
        fn new() -> Self {
            let unique = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            // Exercise argument passing with spaces and shell metacharacters.
            let dir = std::env::temp_dir().join(format!("livi-baa-test-{unique} ' $(literal)"));
            fs::create_dir(&dir).unwrap();
            let helper = dir.join("livi-helperd");
            fs::copy("/usr/bin/true", &helper).unwrap();
            fs::write(dir.join("entitlements.plist"), ENTITLEMENTS).unwrap();
            fs::write(dir.join("relaunch.sh"),
                "printf '%s\\n' \"$LIVI_BAA_REPAIR_ATTEMPTED\" \"$@\" > \"$LIVI_BAA_TEST_OUTPUT\"\n").unwrap();
            Self { dir, helper }
        }

        fn worker(&self, parent: u32, app: &Path) -> Command {
            let mut cmd = Command::new("/bin/sh");
            cmd.args(["-c", WORKER, "livi-baa-test"])
                .arg(parent.to_string())
                .arg(&self.helper)
                .arg("/bin/sh")
                .arg(&self.dir)
                .arg(app)
                .arg(self.dir.join("relaunch.sh"))
                .args(["argument with spaces", "' $(literal)"])
                .env("LIVI_BAA_TEST_OUTPUT", self.dir.join("relaunched"))
                .stdin(Stdio::null());
            cmd
        }

        fn assert_relaunched(&self) {
            assert_eq!(
                fs::read_to_string(self.dir.join("relaunched")).unwrap(),
                "1\nargument with spaces\n' $(literal)\n"
            );
        }
    }

    impl Drop for RepairTest {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    #[test]
    fn requires_fully_disabled_sip() {
        assert!(sip_disabled(
            "System Integrity Protection status: disabled.\n"
        ));
        for status in [
            "System Integrity Protection status: enabled.",
            "System Integrity Protection status: unknown (Custom Configuration).\nFilesystem Protections: disabled",
            "",
            "csrutil: failed",
        ] {
            assert!(!sip_disabled(status), "{status}");
        }
    }

    #[test]
    fn requires_active_amfi_boot_argument() {
        for args in [
            "amfi_get_out_of_my_way=1",
            "-v amfi_get_out_of_my_way=0x1 ipc_control_port_options=0",
        ] {
            assert!(amfi_disabled(args), "{args}");
        }
        for args in [
            "",
            "amfi_get_out_of_my_way=0",
            "amfi_get_out_of_my_way=0x0",
            "amfi_get_out_of_my_way",
            "amfi_get_out_of_my_way=false",
            "other_amfi_get_out_of_my_way=1",
            "amfi_get_out_of_my_way=1 amfi_get_out_of_my_way=0",
        ] {
            assert!(!amfi_disabled(args), "{args}");
        }
    }

    #[test]
    fn requires_every_entitlement_and_matching_identifier() {
        let mut values = serde_json::json!({"application-identifier": IDENTIFIER});
        for key in REQUIRED {
            values[key] = true.into();
        }
        assert!(entitled(&values));
        for key in REQUIRED {
            let mut missing = values.clone();
            missing[key] = false.into();
            assert!(!entitled(&missing), "{key}");
        }
        values["application-identifier"] = "another.helper".into();
        assert!(!entitled(&values));
    }

    #[test]
    fn embedded_entitlements_match_the_checks() {
        let mut plist = Command::new("/usr/bin/plutil")
            .args(["-convert", "json", "-o", "-", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        plist
            .stdin
            .take()
            .unwrap()
            .write_all(ENTITLEMENTS.as_bytes())
            .unwrap();
        let result = plist.wait_with_output().unwrap();
        assert!(result.status.success());
        assert!(entitled(&serde_json::from_slice(&result.stdout).unwrap()));
    }

    #[test]
    fn worker_signs_helper_stops_only_its_parent_and_relaunches() {
        let test = RepairTest::new();
        assert!(!helper_entitled(&test.helper).unwrap());
        let mut parent = Command::new("/bin/sleep").arg("20").spawn().unwrap();
        let mut cmd = test.worker(parent.id(), Path::new(""));
        let reaper = std::thread::spawn(move || parent.wait().unwrap());
        let result = cmd.output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(reaper.join().unwrap().signal(), Some(libc::SIGTERM));
        assert!(helper_entitled(&test.helper).unwrap());
        test.assert_relaunched();
        assert!(!test.dir.join("original-helper").exists());
    }

    #[test]
    fn signing_failure_keeps_parent_alive_and_helper_unchanged() {
        let test = RepairTest::new();
        let original = fs::read(&test.helper).unwrap();
        fs::write(test.dir.join("entitlements.plist"), "invalid plist").unwrap();
        let mut parent = Command::new("/bin/sleep").arg("20").spawn().unwrap();
        let result = test.worker(parent.id(), Path::new("")).output().unwrap();
        let alive = parent.try_wait().unwrap().is_none();
        let _ = parent.kill();
        let _ = parent.wait();
        assert!(!result.status.success());
        assert!(alive);
        assert_eq!(fs::read(&test.helper).unwrap(), original);
        assert!(!test.dir.join("relaunched").exists());
    }

    #[test]
    fn bundle_failure_restores_original_helper_and_relaunches_once() {
        let test = RepairTest::new();
        let original = fs::read(&test.helper).unwrap();
        let invalid_app = test.dir.join("invalid.app");
        fs::create_dir(&invalid_app).unwrap();
        let mut parent = Command::new("/bin/sleep").arg("20").spawn().unwrap();
        let mut cmd = test.worker(parent.id(), &invalid_app);
        let reaper = std::thread::spawn(move || parent.wait().unwrap());
        let result = cmd.output().unwrap();
        assert!(!result.status.success());
        assert_eq!(reaper.join().unwrap().signal(), Some(libc::SIGTERM));
        assert_eq!(fs::read(&test.helper).unwrap(), original);
        assert!(test.dir.join("original-helper").exists());
        test.assert_relaunched();
    }

    #[test]
    fn bundle_repair_preserves_main_entitlements_and_refreshes_seal() {
        let mut test = RepairTest::new();
        let app = test.dir.join("LIVI.app");
        let contents = app.join("Contents");
        fs::create_dir_all(contents.join("MacOS")).unwrap();
        fs::create_dir_all(contents.join("Resources/driver")).unwrap();
        fs::copy("/usr/bin/true", contents.join("MacOS/LIVI")).unwrap();
        fs::write(
            contents.join("Info.plist"),
            r#"<?xml version="1.0"?>
            <plist version="1.0"><dict>
            <key>CFBundleExecutable</key><string>LIVI</string>
            <key>CFBundleIdentifier</key><string>dev.f-io.livi</string>
            <key>CFBundlePackageType</key><string>APPL</string>
            </dict></plist>"#,
        )
        .unwrap();
        let helper = contents.join("Resources/driver/livi-helperd");
        fs::rename(&test.helper, &helper).unwrap();
        test.helper = helper;
        let main_entitlements = test.dir.join("main.plist");
        fs::write(
            &main_entitlements,
            include_str!("../../../../assets/mac/entitlements.mac.plist"),
        )
        .unwrap();
        let signed = Command::new("/usr/bin/codesign")
            .args(["--force", "--sign", "-", "--entitlements"])
            .arg(main_entitlements)
            .arg(&app)
            .output()
            .unwrap();
        assert!(
            signed.status.success(),
            "{}",
            String::from_utf8_lossy(&signed.stderr)
        );
        let main_signature = || {
            Command::new("/usr/bin/codesign")
                .args(["--display", "--entitlements", "-", "--xml"])
                .arg(&app)
                .output()
                .unwrap()
                .stdout
        };
        let before = main_signature();
        let mut parent = Command::new("/bin/sleep").arg("20").spawn().unwrap();
        let mut cmd = test.worker(parent.id(), &app);
        let reaper = std::thread::spawn(move || parent.wait().unwrap());
        let result = cmd.output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(reaper.join().unwrap().signal(), Some(libc::SIGTERM));
        assert!(helper_entitled(&test.helper).unwrap());
        assert_eq!(main_signature(), before);
        let verified = Command::new("/usr/bin/codesign")
            .args(["--verify", "--deep", "--strict"])
            .arg(&app)
            .status()
            .unwrap();
        assert!(verified.success());
        test.assert_relaunched();
    }
}
