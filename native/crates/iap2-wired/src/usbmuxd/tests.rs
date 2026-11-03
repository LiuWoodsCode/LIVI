// SPDX-FileCopyrightText: 2025-2026 Lasse Heitgres
// SPDX-License-Identifier: GPL-3.0-or-later
// Additional terms: see NOTICE

use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};

use idevice::Idevice;
use idevice::pairing_file::PairingFile;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::*;

fn device(connection_type: Connection) -> UsbmuxdDevice {
    UsbmuxdDevice {
        connection_type,
        udid: "test-device".into(),
        device_id: 7,
    }
}

#[test]
fn discovery_excludes_wifi_and_unknown_transports() {
    let found = usb_only(vec![
        device(Connection::Network("127.0.0.1".parse().unwrap())),
        device(Connection::Usb),
        device(Connection::Unknown("other".into())),
    ]);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].connection_type, Connection::Usb);
}

#[tokio::test]
async fn probe_rejects_network_device_before_connecting() {
    let error = probe(&device(Connection::Network("127.0.0.1".parse().unwrap())))
        .await
        .unwrap_err();
    assert_eq!(error.stage, Stage::Discovery);
}

#[derive(Debug)]
struct UnpairedPhone {
    product: &'static str,
    pair_reads: AtomicUsize,
}

impl IdeviceProvider for UnpairedPhone {
    fn connect(
        &self,
        port: u16,
    ) -> Pin<Box<dyn Future<Output = Result<Idevice, IdeviceError>> + Send>> {
        assert_eq!(port, LockdownClient::LOCKDOWND_PORT);
        let product = self.product;
        Box::pin(async move {
            let (client, mut phone) = tokio::io::duplex(4096);
            tokio::spawn(async move {
                let length = phone.read_u32().await.unwrap();
                let mut bytes = vec![0; length as usize];
                phone.read_exact(&mut bytes).await.unwrap();
                let request = plist::Value::from_reader(std::io::Cursor::new(bytes)).unwrap();
                let request = request.as_dictionary().unwrap();
                assert_eq!(request["Request"].as_string(), Some("GetValue"));
                assert_eq!(request["Key"].as_string(), Some("ProductType"));
                let mut reply = plist::Dictionary::new();
                reply.insert("Value".into(), product.into());
                let mut bytes = Vec::new();
                plist::Value::Dictionary(reply)
                    .to_writer_xml(&mut bytes)
                    .unwrap();
                phone.write_u32(bytes.len() as u32).await.unwrap();
                phone.write_all(&bytes).await.unwrap();
            });
            Ok(Idevice::new(Box::new(client), "test-usbmuxd"))
        })
    }

    fn label(&self) -> &str {
        "test-usbmuxd"
    }

    fn get_pairing_file(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<PairingFile, IdeviceError>> + Send>> {
        self.pair_reads.fetch_add(1, Ordering::SeqCst);
        Box::pin(async {
            Err(IdeviceError::UnexpectedResponse(
                "missing PairRecordData in pair record response".into(),
            ))
        })
    }
}

#[tokio::test]
async fn untrusted_iphone_reports_pair_record_stage() {
    let phone = UnpairedPhone {
        product: "iPhone17,1",
        pair_reads: AtomicUsize::new(0),
    };
    let error = probe_provider(&phone).await.unwrap_err();
    assert_eq!(error.stage, Stage::PairRecord);
    assert!(error.waiting_for_pairing());
    assert!(error.detail.contains("Finder"));
    assert_eq!(phone.pair_reads.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn ipad_is_not_treated_as_carplay_phone() {
    let phone = UnpairedPhone {
        product: "iPad16,1",
        pair_reads: AtomicUsize::new(0),
    };
    let error = probe_provider(&phone).await.unwrap_err();
    assert_eq!(error.stage, Stage::Product);
    assert_eq!(phone.pair_reads.load(Ordering::SeqCst), 0);
}

#[tokio::test(start_paused = true)]
async fn stalled_io_is_bounded_and_keeps_its_stage() {
    let error = step::<()>(Stage::Carkit, std::future::pending())
        .await
        .unwrap_err();
    assert_eq!(error.stage, Stage::Carkit);
    assert!(error.detail.contains("timed out"));
}

#[tokio::test]
async fn socket_errors_preserve_the_actionable_cause() {
    let error = step::<()>(Stage::Discovery, async {
        Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "permission denied").into())
    })
    .await
    .unwrap_err();
    assert_eq!(error.stage, Stage::Discovery);
    assert!(error.detail.contains("permission denied"));
    assert!(!error.waiting_for_pairing());
}

fn dict<const N: usize>(entries: [(&str, plist::Value); N]) -> plist::Value {
    plist::Value::Dictionary(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_string(), value))
            .collect(),
    )
}

fn scripted_lockdown(
    exchanges: Vec<(plist::Value, plist::Value)>,
) -> (LockdownClient, tokio::task::JoinHandle<()>) {
    let (client, mut phone) = tokio::io::duplex(4096);
    let server = tokio::spawn(async move {
        for (expected, reply) in exchanges {
            let length = phone.read_u32().await.unwrap();
            let mut bytes = vec![0; length as usize];
            phone.read_exact(&mut bytes).await.unwrap();
            let request = plist::Value::from_reader(std::io::Cursor::new(bytes)).unwrap();
            for (key, value) in expected.as_dictionary().unwrap() {
                assert_eq!(request.as_dictionary().unwrap().get(key), Some(value));
            }
            let mut bytes = Vec::new();
            reply.to_writer_xml(&mut bytes).unwrap();
            phone.write_u32(bytes.len() as u32).await.unwrap();
            phone.write_all(&bytes).await.unwrap();
        }
    });
    (
        LockdownClient::new(Idevice::new(Box::new(client), "test")),
        server,
    )
}

#[tokio::test]
async fn unavailable_service_reports_ios_version_without_retrying_pairing() {
    let (mut lockdown, server) = scripted_lockdown(vec![
        (
            dict([
                ("Request", "StartService".into()),
                ("Service", LOCKDOWN_SERVICE.into()),
            ]),
            dict([("Error", "InvalidService".into())]),
        ),
        (
            dict([
                ("Request", "GetValue".into()),
                ("Key", "ProductVersion".into()),
            ]),
            dict([("Value", "12.5.5".into())]),
        ),
    ]);
    let error = start_carkit(&mut lockdown).await.unwrap_err();
    server.await.unwrap();
    assert_eq!(error.stage, Stage::Carkit);
    assert!(error.detail.contains(LOCKDOWN_SERVICE));
    assert!(error.detail.contains("iOS 12.5.5"));
    assert!(error.detail.contains("InvalidService"));
    assert!(!error.waiting_for_pairing());
}

#[tokio::test]
async fn service_keeps_the_advertised_port_and_tls_mode() {
    let (mut lockdown, server) = scripted_lockdown(vec![(
        dict([
            ("Request", "StartService".into()),
            ("Service", LOCKDOWN_SERVICE.into()),
        ]),
        dict([("Port", 12345u16.into()), ("EnableServiceSSL", true.into())]),
    )]);
    assert_eq!(start_carkit(&mut lockdown).await.unwrap(), (12345, true));
    server.await.unwrap();
}

#[tokio::test]
async fn version_lookup_failure_preserves_the_service_error() {
    let (mut lockdown, server) = scripted_lockdown(vec![
        (
            dict([
                ("Request", "StartService".into()),
                ("Service", LOCKDOWN_SERVICE.into()),
            ]),
            dict([("Error", "InvalidService".into())]),
        ),
        (
            dict([
                ("Request", "GetValue".into()),
                ("Key", "ProductVersion".into()),
            ]),
            dict([("Error", "DeviceLocked".into())]),
        ),
    ]);
    let error = start_carkit(&mut lockdown).await.unwrap_err();
    server.await.unwrap();
    assert_eq!(error.stage, Stage::Carkit);
    assert!(error.detail.contains("InvalidService"));
    assert!(error.detail.contains("iOS unknown"));
}

#[tokio::test]
async fn locked_device_is_not_reported_as_an_unavailable_service() {
    let (mut lockdown, server) = scripted_lockdown(vec![(
        dict([
            ("Request", "StartService".into()),
            ("Service", LOCKDOWN_SERVICE.into()),
        ]),
        dict([("Error", "DeviceLocked".into())]),
    )]);
    let error = start_carkit(&mut lockdown).await.unwrap_err();
    server.await.unwrap();
    assert_eq!(error.stage, Stage::Carkit);
    assert!(!error.detail.contains("unavailable"));
    assert!(!error.waiting_for_pairing());
}
