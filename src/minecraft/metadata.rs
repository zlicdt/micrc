use std::collections::HashMap;

use regex::Regex;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
pub struct VersionManifest {
    pub latest: LatestVersions,
    pub versions: Vec<VersionSummary>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LatestVersions {
    pub release: String,
    pub snapshot: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct VersionSummary {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub url: String,
    pub sha1: String,
    #[serde(rename = "releaseTime")]
    pub release_time: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct VersionMetadata {
    pub id: String,
    #[serde(rename = "type", default)]
    pub kind: String,
    #[serde(rename = "mainClass")]
    pub main_class: String,
    #[serde(default)]
    pub assets: String,
    #[serde(rename = "assetIndex")]
    pub asset_index: Option<Download>,
    pub downloads: VersionDownloads,
    #[serde(default)]
    pub libraries: Vec<Library>,
    pub arguments: Option<Arguments>,
    #[serde(rename = "minecraftArguments")]
    pub minecraft_arguments: Option<String>,
    #[serde(rename = "javaVersion")]
    pub java_version: Option<JavaVersion>,
    pub logging: Option<Logging>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct VersionDownloads {
    pub client: Download,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Download {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub sha1: String,
    #[serde(default)]
    pub size: u64,
    pub url: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct JavaVersion {
    pub component: String,
    #[serde(rename = "majorVersion")]
    pub major_version: u32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Arguments {
    #[serde(default)]
    pub game: Vec<Argument>,
    #[serde(default)]
    pub jvm: Vec<Argument>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum Argument {
    Plain(String),
    Conditional(ConditionalArgument),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ConditionalArgument {
    #[serde(default)]
    pub rules: Vec<Rule>,
    pub value: ArgumentValue,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub enum ArgumentValue {
    One(String),
    Many(Vec<String>),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Library {
    pub name: String,
    #[serde(default)]
    pub downloads: LibraryDownloads,
    #[serde(default)]
    pub rules: Vec<Rule>,
    #[serde(default)]
    pub natives: HashMap<String, String>,
    pub extract: Option<Extract>,
    pub url: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct LibraryDownloads {
    pub artifact: Option<Download>,
    #[serde(default)]
    pub classifiers: HashMap<String, Download>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Extract {
    #[serde(default)]
    pub exclude: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Rule {
    pub action: RuleAction,
    pub os: Option<OsRule>,
    #[serde(default)]
    pub features: HashMap<String, bool>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RuleAction {
    Allow,
    Disallow,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OsRule {
    pub name: Option<String>,
    pub arch: Option<String>,
    pub version: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Logging {
    pub client: LoggingClient,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LoggingClient {
    pub argument: String,
    pub file: Download,
    #[serde(rename = "type")]
    pub kind: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AssetIndex {
    #[serde(rename = "virtual", default)]
    pub is_virtual: bool,
    #[serde(rename = "map_to_resources", default)]
    pub map_to_resources: bool,
    pub objects: HashMap<String, AssetObject>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AssetObject {
    pub hash: String,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Platform {
    pub os: String,
    pub arch: String,
    pub bits: &'static str,
    pub version: String,
}

impl Platform {
    pub fn current() -> Self {
        let os = if cfg!(target_os = "windows") {
            "windows"
        } else if cfg!(target_os = "macos") {
            "osx"
        } else {
            "linux"
        };
        let arch = std::env::consts::ARCH.to_owned();
        let bits = if cfg!(target_pointer_width = "64") {
            "64"
        } else {
            "32"
        };
        Self {
            os: os.to_owned(),
            arch,
            bits,
            version: os_version(os),
        }
    }

    pub fn test(os: &str, arch: &str, bits: &'static str) -> Self {
        Self {
            os: os.to_owned(),
            arch: arch.to_owned(),
            bits,
            version: String::new(),
        }
    }
}

pub fn rules_allow(rules: &[Rule], platform: &Platform) -> bool {
    if rules.is_empty() {
        return true;
    }

    let mut allowed = false;
    for rule in rules {
        if rule_matches(rule, platform) {
            allowed = rule.action == RuleAction::Allow;
        }
    }
    allowed
}

fn rule_matches(rule: &Rule, platform: &Platform) -> bool {
    for (feature, expected) in &rule.features {
        if feature_enabled(feature) != *expected {
            return false;
        }
    }
    if let Some(os) = &rule.os {
        if os.name.as_ref().is_some_and(|name| name != &platform.os) {
            return false;
        }
        if let Some(arch) = &os.arch {
            let normalized = if platform.bits == "32" && platform.arch == "x86_64" {
                "x86"
            } else {
                &platform.arch
            };
            if !Regex::new(arch).is_ok_and(|pattern| pattern.is_match(normalized)) {
                return false;
            }
        }
        if let Some(version) = &os.version
            && !Regex::new(version).is_ok_and(|pattern| pattern.is_match(&platform.version))
        {
            return false;
        }
    }
    true
}

fn os_version(os: &str) -> String {
    if let Ok(version) = std::env::var("OS_VERSION") {
        return version;
    }
    let output = match os {
        "windows" => std::process::Command::new("cmd")
            .args(["/C", "ver"])
            .output(),
        "osx" => std::process::Command::new("sw_vers")
            .arg("-productVersion")
            .output(),
        _ => std::process::Command::new("uname").arg("-r").output(),
    };
    output
        .ok()
        .map(|value| String::from_utf8_lossy(&value.stdout).trim().to_owned())
        .unwrap_or_default()
}

pub fn expand_arguments(arguments: &[Argument], platform: &Platform) -> Vec<String> {
    let mut expanded = Vec::new();
    for argument in arguments {
        match argument {
            Argument::Plain(value) => expanded.push(value.clone()),
            Argument::Conditional(value) if rules_allow(&value.rules, platform) => {
                match &value.value {
                    ArgumentValue::One(value) => expanded.push(value.clone()),
                    ArgumentValue::Many(values) => expanded.extend(values.iter().cloned()),
                }
            }
            Argument::Conditional(_) => {}
        }
    }
    expanded
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeLibrary {
    Mapped,
    Coordinate,
}

pub fn native_library(library: &Library, platform: &Platform) -> Option<(NativeLibrary, String)> {
    if let Some(classifier) = library.natives.get(&platform.os) {
        return Some((
            NativeLibrary::Mapped,
            classifier.replace("${arch}", platform.bits),
        ));
    }
    library
        .name
        .split(':')
        .nth(3)
        .filter(|classifier| classifier.starts_with("natives-"))
        .map(|classifier| (NativeLibrary::Coordinate, classifier.to_owned()))
}

pub fn native_classifier(library: &Library, platform: &Platform) -> Option<String> {
    native_library(library, platform).map(|(_, classifier)| classifier)
}

fn feature_enabled(feature: &str) -> bool {
    match feature {
        "has_custom_resolution" => true,
        "is_demo_user" => false,
        "has_quick_plays_support" => false,
        "is_quick_play_singleplayer" => false,
        "is_quick_play_multiplayer" => false,
        "is_quick_play_realms" => false,
        _ => false,
    }
}

pub fn maven_download(library: &Library, classifier: Option<&str>) -> Option<Download> {
    let (coordinates, extension) = library
        .name
        .split_once('@')
        .map_or((library.name.as_str(), "jar"), |(left, right)| {
            (left, right)
        });
    let parts: Vec<_> = coordinates.split(':').collect();
    if parts.len() < 3 {
        return None;
    }
    let group = parts[0].replace('.', "/");
    let artifact = parts[1];
    let version = parts[2];
    let classifier = classifier.or_else(|| parts.get(3).copied());
    let suffix = classifier.map_or(String::new(), |value| format!("-{value}"));
    let path = format!("{group}/{artifact}/{version}/{artifact}-{version}{suffix}.{extension}");
    let base = library
        .url
        .as_deref()
        .unwrap_or("https://libraries.minecraft.net/");
    Some(Download {
        id: String::new(),
        path: path.clone(),
        sha1: String::new(),
        size: 0,
        url: format!("{}{path}", base.trim_end_matches('/').to_owned() + "/"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(action: RuleAction, os: Option<&str>) -> Rule {
        Rule {
            action,
            os: os.map(|name| OsRule {
                name: Some(name.to_owned()),
                arch: None,
                version: None,
            }),
            features: HashMap::new(),
        }
    }

    #[test]
    fn applies_rules_in_order() {
        let linux = Platform::test("linux", "x86_64", "64");
        let rules = vec![
            rule(RuleAction::Allow, None),
            rule(RuleAction::Disallow, Some("linux")),
        ];
        assert!(!rules_allow(&rules, &linux));
    }

    #[test]
    fn expands_native_architecture() {
        let mut natives = HashMap::new();
        natives.insert("windows".to_owned(), "natives-windows-${arch}".to_owned());
        let library = Library {
            name: "example:demo:1".to_owned(),
            downloads: LibraryDownloads::default(),
            rules: Vec::new(),
            natives,
            extract: None,
            url: None,
        };
        let windows = Platform::test("windows", "x86_64", "64");
        assert_eq!(
            native_classifier(&library, &windows).as_deref(),
            Some("natives-windows-64")
        );
    }

    #[test]
    fn applies_known_feature_rules() {
        let linux = Platform::test("linux", "x86_64", "64");
        let rule = |features: HashMap<String, bool>| Rule {
            action: RuleAction::Allow,
            os: None,
            features,
        };

        assert!(rules_allow(
            &[rule(HashMap::from([(
                "has_custom_resolution".to_owned(),
                true,
            )]))],
            &linux
        ));
        assert!(!rules_allow(
            &[rule(HashMap::from([("is_demo_user".to_owned(), true)]))],
            &linux
        ));
        assert!(rules_allow(
            &[rule(HashMap::from([("is_demo_user".to_owned(), false)]))],
            &linux
        ));
    }

    #[test]
    fn recognizes_coordinate_native_libraries() {
        let library = Library {
            name: "org.lwjgl:lwjgl:3.3.3:natives-linux".to_owned(),
            downloads: LibraryDownloads::default(),
            rules: Vec::new(),
            natives: HashMap::new(),
            extract: None,
            url: None,
        };
        let linux = Platform::test("linux", "x86_64", "64");
        assert_eq!(
            native_library(&library, &linux),
            Some((NativeLibrary::Coordinate, "natives-linux".to_owned()))
        );
    }

    #[test]
    fn creates_legacy_maven_path() {
        let library = Library {
            name: "org.lwjgl:lwjgl:3.3.3".to_owned(),
            downloads: LibraryDownloads::default(),
            rules: Vec::new(),
            natives: HashMap::new(),
            extract: None,
            url: None,
        };
        let download = maven_download(&library, None).unwrap();
        assert_eq!(download.path, "org/lwjgl/lwjgl/3.3.3/lwjgl-3.3.3.jar");
    }
}
