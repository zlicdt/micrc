use micrc::{
    config::{Account, Settings},
    minecraft::{
        download::fetch_bytes, install::fetch_manifest, launch::build_launch_command,
        metadata::VersionMetadata, paths::MinecraftPaths,
    },
};

#[tokio::test]
#[ignore = "requires access to Mojang services"]
async fn current_release_metadata_builds_a_launch_command() {
    let client = reqwest::Client::builder()
        .user_agent("micrc-live-test")
        .build()
        .unwrap();
    let manifest = fetch_manifest(&client).await.unwrap();
    let summary = manifest
        .versions
        .iter()
        .find(|version| version.id == manifest.latest.release)
        .unwrap();
    let bytes = fetch_bytes(&client, &summary.url, Some(&summary.sha1))
        .await
        .unwrap();
    let metadata: VersionMetadata = serde_json::from_slice(&bytes).unwrap();
    assert!(!metadata.libraries.is_empty());
    assert!(!metadata.main_class.is_empty());

    let temporary = temp_dir::TempDir::new().unwrap();
    let paths = MinecraftPaths::new(temporary.path());
    let command = build_launch_command(
        &metadata,
        &Account::offline("LiveTest"),
        &Settings::default(),
        &paths,
    )
    .unwrap();
    assert!(
        command
            .args
            .iter()
            .any(|value| value == &metadata.main_class)
    );
    assert!(command.args.iter().any(|value| value == "LiveTest"));
    assert!(command.args.iter().any(|value| value.starts_with("-Xmx")));
}
