use std::{
    collections::HashMap,
    io::{Cursor, Write},
    sync::Arc,
};

use micrc::minecraft::{
    TaskEvent, install::install_version, metadata::VersionSummary, paths::MinecraftPaths,
};
use sha1::{Digest, Sha1};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::mpsc::unbounded_channel,
};
use zip::{ZipWriter, write::SimpleFileOptions};

#[tokio::test]
async fn installs_client_library_and_native_archive() {
    let client_bytes = b"mock client".to_vec();
    let library_bytes = b"mock library".to_vec();
    let native_bytes = native_archive();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let metadata = serde_json::json!({
        "id": "mock-version",
        "type": "release",
        "mainClass": "com.example.Main",
        "assets": "",
        "downloads": {
            "client": download_json(&base_url, "/client.jar", &client_bytes, "")
        },
        "libraries": [{
            "name": "com.example:mock:1.0",
            "downloads": {
                "artifact": download_json(
                    &base_url,
                    "/library.jar",
                    &library_bytes,
                    "com/example/mock/1.0/mock-1.0.jar"
                ),
                "classifiers": {
                    "natives-linux": download_json(
                        &base_url,
                        "/native.jar",
                        &native_bytes,
                        "com/example/mock/1.0/mock-1.0-natives-linux.jar"
                    ),
                    "natives-windows": download_json(
                        &base_url,
                        "/native.jar",
                        &native_bytes,
                        "com/example/mock/1.0/mock-1.0-natives-windows.jar"
                    ),
                    "natives-osx": download_json(
                        &base_url,
                        "/native.jar",
                        &native_bytes,
                        "com/example/mock/1.0/mock-1.0-natives-osx.jar"
                    )
                }
            },
            "natives": {
                "linux": "natives-linux",
                "windows": "natives-windows",
                "osx": "natives-osx"
            }
        }],
        "arguments": { "jvm": [], "game": [] }
    });
    let metadata_bytes = serde_json::to_vec(&metadata).unwrap();
    let mut responses = HashMap::new();
    responses.insert("/version.json".to_owned(), metadata_bytes.clone());
    responses.insert("/client.jar".to_owned(), client_bytes);
    responses.insert("/library.jar".to_owned(), library_bytes);
    responses.insert("/native.jar".to_owned(), native_bytes);
    let server = tokio::spawn(serve(listener, Arc::new(responses)));

    let temporary = temp_dir::TempDir::new().unwrap();
    let paths = MinecraftPaths::new(temporary.path());
    let summary = VersionSummary {
        id: "mock-version".to_owned(),
        kind: "release".to_owned(),
        url: format!("{base_url}/version.json"),
        sha1: sha1(&metadata_bytes),
        release_time: "2026-01-01T00:00:00Z".to_owned(),
    };
    let (events, mut receiver) = unbounded_channel();
    install_version(reqwest::Client::new(), paths.clone(), summary, events)
        .await
        .unwrap();
    server.abort();

    assert_eq!(
        tokio::fs::read(paths.version_jar("mock-version"))
            .await
            .unwrap(),
        b"mock client"
    );
    assert!(
        paths
            .libraries()
            .join("com/example/mock/1.0/mock-1.0.jar")
            .is_file()
    );
    assert!(paths.natives("mock-version").join("mock-native").is_file());
    let mut installed = false;
    while let Ok(event) = receiver.try_recv() {
        installed |= matches!(event, TaskEvent::Installed(ref id) if id == "mock-version");
    }
    assert!(installed);
}

#[tokio::test]
async fn installs_coordinate_native_library_into_natives_dir() {
    let client_bytes = b"mock client".to_vec();
    let library_bytes = b"mock library".to_vec();
    let native_bytes = native_archive();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let metadata = serde_json::json!({
        "id": "mock-version-coordinate",
        "type": "release",
        "mainClass": "com.example.Main",
        "assets": "",
        "downloads": {
            "client": download_json(&base_url, "/client.jar", &client_bytes, "")
        },
        "libraries": [
            {
                "name": "com.example:mock:1.0",
                "downloads": {
                    "artifact": download_json(
                        &base_url,
                        "/library.jar",
                        &library_bytes,
                        "com/example/mock/1.0/mock-1.0.jar"
                    )
                }
            },
            {
                "name": "com.example:mock:1.0:natives-linux",
                "rules": [{ "action": "allow", "os": { "name": "linux" } }],
                "downloads": {
                    "artifact": download_json(
                        &base_url,
                        "/native.jar",
                        &native_bytes,
                        "com/example/mock/1.0/mock-1.0-natives-linux.jar"
                    )
                }
            }
        ],
        "arguments": { "jvm": [], "game": [] }
    });
    let metadata_bytes = serde_json::to_vec(&metadata).unwrap();
    let mut responses = HashMap::new();
    responses.insert("/version.json".to_owned(), metadata_bytes.clone());
    responses.insert("/client.jar".to_owned(), client_bytes);
    responses.insert("/library.jar".to_owned(), library_bytes);
    responses.insert("/native.jar".to_owned(), native_bytes);
    let server = tokio::spawn(serve(listener, Arc::new(responses)));

    let temporary = temp_dir::TempDir::new().unwrap();
    let paths = MinecraftPaths::new(temporary.path());
    let summary = VersionSummary {
        id: "mock-version-coordinate".to_owned(),
        kind: "release".to_owned(),
        url: format!("{base_url}/version.json"),
        sha1: sha1(&metadata_bytes),
        release_time: "2026-01-01T00:00:00Z".to_owned(),
    };
    let (events, mut receiver) = unbounded_channel();
    install_version(reqwest::Client::new(), paths.clone(), summary, events)
        .await
        .unwrap();
    server.abort();

    assert!(
        paths
            .natives("mock-version-coordinate")
            .join("mock-native")
            .is_file()
    );
    assert!(
        paths
            .libraries()
            .join("com/example/mock/1.0/mock-1.0.jar")
            .is_file()
    );
    assert_eq!(
        tokio::fs::read(paths.version_jar("mock-version-coordinate"))
            .await
            .unwrap(),
        b"mock client"
    );
    let mut installed = false;
    while let Ok(event) = receiver.try_recv() {
        installed |=
            matches!(event, TaskEvent::Installed(ref id) if id == "mock-version-coordinate");
    }
    assert!(installed);
}

async fn serve(listener: TcpListener, responses: Arc<HashMap<String, Vec<u8>>>) {
    loop {
        let Ok((mut stream, _)) = listener.accept().await else {
            return;
        };
        let responses = Arc::clone(&responses);
        tokio::spawn(async move {
            let mut request = [0_u8; 4096];
            let count = stream.read(&mut request).await.unwrap();
            let request = String::from_utf8_lossy(&request[..count]);
            let path = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .unwrap();
            let body = responses.get(path).unwrap();
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(header.as_bytes()).await.unwrap();
            stream.write_all(body).await.unwrap();
        });
    }
}

fn download_json(base_url: &str, route: &str, bytes: &[u8], path: &str) -> serde_json::Value {
    serde_json::json!({
        "path": path,
        "sha1": sha1(bytes),
        "size": bytes.len(),
        "url": format!("{base_url}{route}")
    })
}

fn sha1(bytes: &[u8]) -> String {
    format!("{:x}", Sha1::digest(bytes))
}

fn native_archive() -> Vec<u8> {
    let cursor = Cursor::new(Vec::new());
    let mut archive = ZipWriter::new(cursor);
    archive
        .start_file("mock-native", SimpleFileOptions::default())
        .unwrap();
    archive.write_all(b"mock native").unwrap();
    archive.finish().unwrap().into_inner()
}
