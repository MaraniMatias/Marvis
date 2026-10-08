use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileTree {
    pub entries: Vec<FileEntry>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub kind: FileEntryKind,
    /// A Git checkout ignores this file. It is still listed, so the tree can show it a step
    /// quieter; only `.git` itself is ever withheld.
    pub ignored: bool,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FileEntryKind {
    File,
    Directory,
    Symlink,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileContent {
    pub path: String,
    pub content: String,
}

/// The Prettier options that govern a file, read as JSON so the caller hands them straight
/// to Prettier. Absent rather than an empty object: a checkout that configures nothing is not a
/// checkout with nothing to say, it is one that says Prettier's defaults apply.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrettierConfig {
    /// File path relative to the config directory, for worker-side override matching.
    pub path: String,
    pub options: serde_json::Value,
}

/// A path a terminal printed that this checkout holds and the preview can open. Absent rather than
/// a flag on a bare path, so the caller cannot open something the probe never confirmed.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileProbe {
    pub path: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckoutImage {
    pub mime_type: String,
    pub data_base64: String,
    pub size_bytes: usize,
}
