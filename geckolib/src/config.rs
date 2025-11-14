use std::collections::HashMap;
use std::path::PathBuf;

use semver::Version;
use serde_derive::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, Default, Debug)]
pub struct Config {
    #[serde(default)]
    pub info: Info,
    pub src: Src,
    #[serde(default)]
    pub files: HashMap<String, PathBuf>,
    pub build: Build,
    pub link: Option<Link>,
    pub diffs: Option<Diffs>,
}

#[derive(Deserialize, Serialize, Default, Debug)]
#[serde(rename_all = "kebab-case")]
pub struct Src {
    pub src: Option<PathBuf>,
    pub iso: PathBuf,
    pub patch: Option<PathBuf>,
    pub map: Option<String>,
}

#[derive(Deserialize, Serialize, Default, Debug)]
#[serde(rename_all = "kebab-case")]
pub struct Info {
    pub game_name: Option<String>,
    pub developer_name: Option<String>,
    pub full_game_name: Option<String>,
    pub full_developer_name: Option<String>,
    pub description: Option<String>,
    pub image: Option<PathBuf>,
    pub version: Option<Version>,
}

#[derive(Deserialize, Serialize, Default, Debug)]
pub struct Build {
    pub map: Option<PathBuf>,
    pub iso: PathBuf,
}

#[derive(Deserialize, Serialize, Default, Debug, Clone)]
pub struct Link {
    pub entries: Vec<String>,
    pub base: String,
    pub libs: Vec<PathBuf>,
}

#[derive(Deserialize, Serialize, Default, Debug, Clone)]
pub struct Diffs {
    /// List of the paths in the game's filesystem to remove.
    pub deletions: Vec<PathBuf>,
    /// List of pairs of paths in the game's file system to modify, and the paths
    /// within the patch archive to the gzip2 compressed bsdiff file for the changes to apply to the file.
    pub changes: HashMap<PathBuf, PathBuf>,
    /// Path within the patch archive to the gzip2 compressed bsdiff file for the changes to apply to the main DOL of the game.
    pub dol: Option<PathBuf>,
    /// Path within the patch archive to the gzip2 compressed bsdiff file for the changes to apply to the AppLoader of the game.
    pub loader: Option<PathBuf>
}
