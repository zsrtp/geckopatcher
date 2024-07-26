use std::collections::HashMap;
use std::path::PathBuf;

use serde_derive::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, Debug)]
pub struct Config {
    #[serde(default)]
    pub info: Info,
    pub src: Src,
    #[serde(default)]
    pub files: HashMap<String, PathBuf>,
    pub build: Build,
    pub link: Option<Link>,
}

#[derive(Deserialize, Serialize, Debug)]
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
}

#[derive(Deserialize, Serialize, Default, Debug)]
pub struct Build {
    pub map: Option<PathBuf>,
    pub iso: PathBuf,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct Link {
    pub entries: Vec<String>,
    pub base: String,
    pub libs: Vec<PathBuf>,
}
