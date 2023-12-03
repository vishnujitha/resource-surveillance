use std::error::Error;
use std::fs;
use std::fs::canonicalize;
use std::io::Read;
use std::path::Path;
use std::path::PathBuf;

use bitflags::bitflags;
use chrono::{DateTime, Utc};
use is_executable::IsExecutable;
use regex::{Regex, RegexSet};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use sha1::{Digest, Sha1};

use crate::shell::*;

use crate::frontmatter::frontmatter;

pub trait BinaryContent {
    fn content_digest_hash(&self) -> &str;
    fn content_binary(&self) -> &Vec<u8>;
}

pub type FrontmatterComponents = (
    crate::frontmatter::FrontmatterNature,
    Option<String>,
    Result<JsonValue, Box<dyn Error>>,
    String,
);

pub trait TextContent {
    fn content_digest_hash(&self) -> &str;
    fn content_text(&self) -> &str;
    fn frontmatter(&self) -> FrontmatterComponents;
}

pub type BinaryContentSupplier = Box<dyn Fn() -> Result<Box<dyn BinaryContent>, Box<dyn Error>>>;
pub type TextContentSupplier = Box<dyn Fn() -> Result<Box<dyn TextContent>, Box<dyn Error>>>;

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct EncounterableResourceFlags: u32 {
        const CONTENT_ACQUIRABLE    = 0b00000001;
        const IGNORE_RESOURCE       = EncounterableResourceFlags::CONTENT_ACQUIRABLE.bits() << 1;
        const CAPTURABLE_EXECUTABLE = EncounterableResourceFlags::IGNORE_RESOURCE.bits() << 1;
        const CAPTURABLE_SQL        = EncounterableResourceFlags::CAPTURABLE_EXECUTABLE.bits() << 1;

        // all the above are considered "common flags", this const is the "last" common
        const TERMINAL_COMMON       = EncounterableResourceFlags::CAPTURABLE_SQL.bits();

        // add any special ContentResource-only flags after this, starting with TERMINAL_COMMON
    }

    // EncounteredResourceFlags "inherits" flags from EncounterableResourceFlags
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct EncounteredResourceFlags: u32 {
        const CONTENT_ACQUIRABLE    = EncounterableResourceFlags::CONTENT_ACQUIRABLE.bits();
        const IGNORE_RESOURCE       = EncounterableResourceFlags::IGNORE_RESOURCE.bits();
        const CAPTURABLE_EXECUTABLE = EncounterableResourceFlags::CAPTURABLE_EXECUTABLE.bits();
        const CAPTURABLE_SQL        = EncounterableResourceFlags::CAPTURABLE_SQL.bits();
        const TERMINAL_INHERITED    = EncounterableResourceFlags::TERMINAL_COMMON.bits();

        // these flags are not "common" and are specific to EncounteredResourceFlags
        const IS_FILE                  = EncounteredResourceFlags::TERMINAL_INHERITED.bits() << 1;
        const IS_DIRECTORY             = EncounteredResourceFlags::IS_FILE.bits() << 1;
        const IS_SYMLINK               = EncounteredResourceFlags::IS_DIRECTORY.bits() << 1;
    }

    // ContentResourceFlags "inherits" flags from EncounteredResourceFlags
    #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
    pub struct ContentResourceFlags: u32 {
        const CONTENT_ACQUIRABLE    = EncounteredResourceFlags::CONTENT_ACQUIRABLE.bits();
        const IGNORE_RESOURCE       = EncounteredResourceFlags::IGNORE_RESOURCE.bits();
        const CAPTURABLE_EXECUTABLE = EncounteredResourceFlags::CAPTURABLE_EXECUTABLE.bits();
        const CAPTURABLE_SQL        = EncounteredResourceFlags::CAPTURABLE_SQL.bits();
        const TERMINAL_INHERITED    = EncounteredResourceFlags::TERMINAL_INHERITED.bits();

        // add any special ContentResource-only flags after this, starting with TERMINAL_INHERITED
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct NatureRewriteRule {
    #[serde(with = "serde_regex")]
    pub regex: Regex,
    pub nature: String,
}

impl NatureRewriteRule {
    pub fn is_match(&self, text: &str) -> Option<String> {
        if let Some(caps) = self.regex.captures(text) {
            if let Some(nature) = caps.name("nature") {
                return Some(nature.as_str().to_string());
            }
        }
        None
    }
}

const DEFAULT_IGNORE_PATHS_REGEX_PATTERNS: [&str; 1] = [r"/(\.git|node_modules)/"];
const DEFAULT_ACQUIRE_CONTENT_EXTNS_REGEX_PATTERNS: [&str; 1] =
    [r"\.(?P<nature>md|mdx|html|json|jsonc|txt|toml|yaml)$"];
const DEFAULT_CAPTURE_EXEC_REGEX_PATTERNS: [&str; 1] = [r"surveilr\[(?P<nature>[^\]]*)\]"];
const DEFAULT_CAPTURE_SQL_EXEC_REGEX_PATTERNS: [&str; 1] = [r"surveilr-SQL"];
const DEFAULT_REWRITE_NATURE_PATTERNS: [(&str, &str); 1] =
    [(r"\.(?P<nature>tap|text)$", "text/plain")];

#[derive(Clone, Serialize, Deserialize)]
pub struct EncounterableResourcePathRules {
    #[serde(with = "serde_regex")]
    pub ignore_paths_regexs: Vec<regex::Regex>,

    #[serde(with = "serde_regex")]
    // each regex must include a `nature` capture group
    pub acquire_content_for_paths_regexs: Vec<regex::Regex>,

    #[serde(with = "serde_regex")]
    // each regex must include a `nature` capture group
    pub capturable_executables_paths_regexs: Vec<regex::Regex>,

    #[serde(with = "serde_regex")]
    pub captured_exec_sql_paths_regexs: Vec<regex::Regex>,

    // each regex must include a `nature` capture group
    pub rewrite_nature_regexs: Vec<NatureRewriteRule>,
}

impl Default for EncounterableResourcePathRules {
    fn default() -> Self {
        EncounterableResourcePathRules {
            ignore_paths_regexs: DEFAULT_IGNORE_PATHS_REGEX_PATTERNS
                .map(|p| Regex::new(p).unwrap())
                .to_vec(),
            acquire_content_for_paths_regexs: DEFAULT_ACQUIRE_CONTENT_EXTNS_REGEX_PATTERNS
                .map(|p| Regex::new(p).unwrap())
                .to_vec(),
            capturable_executables_paths_regexs: DEFAULT_CAPTURE_EXEC_REGEX_PATTERNS
                .map(|p| Regex::new(p).unwrap())
                .to_vec(),
            captured_exec_sql_paths_regexs: DEFAULT_CAPTURE_SQL_EXEC_REGEX_PATTERNS
                .map(|p| Regex::new(p).unwrap())
                .to_vec(),
            rewrite_nature_regexs: DEFAULT_REWRITE_NATURE_PATTERNS
                .map(|p| NatureRewriteRule {
                    regex: Regex::new(p.0).unwrap(),
                    nature: p.1.to_string(),
                })
                .to_vec(),
        }
    }
}

impl EncounterableResourcePathRules {
    pub fn _from_json_text(json_text: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json_text)
    }

    pub fn _persistable_json_text(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    pub fn _add_ignore_exact(&mut self, pattern: &str) {
        self.ignore_paths_regexs
            .push(regex::Regex::new(format!("^{}$", regex::escape(pattern)).as_str()).unwrap());
    }
}

#[derive(Clone)]
pub struct EncounterableResourceClass {
    pub flags: EncounterableResourceFlags,
    pub nature: Option<String>,
}

pub trait EncounterableResourceUriClassifier {
    fn classify(
        &self,
        uri: &str,
        class: &mut EncounterableResourceClass,
        rewritten_natures: Option<&mut Vec<(String, String, String)>>,
    ) -> bool;
}

pub struct EncounterableResourcePathClassifier {
    pub ignore_paths_regex_set: RegexSet, // we do not care about which one matched so we use a set
    pub acquire_content_for_paths_regex_set: Vec<regex::Regex>, // we need to capture `nature` so we loop through each one
    pub capturable_executables_paths_regexs: Vec<regex::Regex>, // we need to capture `nature` so we loop through each one
    pub captured_exec_sql_paths_regex_set: RegexSet, // we do not care about which one matched so we use a set
    pub rewrite_nature_regexs: Vec<NatureRewriteRule>, // we need to capture `nature` so we loop through each one
}

impl Default for EncounterableResourcePathClassifier {
    fn default() -> Self {
        let default_rules = EncounterableResourcePathRules::default();
        EncounterableResourcePathClassifier::from_path_rules(default_rules).unwrap()
    }
}

impl EncounterableResourcePathClassifier {
    pub fn from_path_rules(erpr: EncounterableResourcePathRules) -> anyhow::Result<Self> {
        let ignore_paths_regex_set =
            RegexSet::new(erpr.ignore_paths_regexs.iter().map(|r| r.as_str())).unwrap();
        let acquire_content_for_paths_regex_set = erpr.acquire_content_for_paths_regexs.to_vec();
        let capturable_executables_paths_regexs = erpr.capturable_executables_paths_regexs.to_vec();
        let captured_exec_sql_paths_regex_set = RegexSet::new(
            erpr.captured_exec_sql_paths_regexs
                .iter()
                .map(|r| r.as_str()),
        )?;
        let rewrite_nature_regexs = erpr.rewrite_nature_regexs.to_vec();

        Ok(EncounterableResourcePathClassifier {
            ignore_paths_regex_set,
            acquire_content_for_paths_regex_set,
            capturable_executables_paths_regexs,
            captured_exec_sql_paths_regex_set,
            rewrite_nature_regexs,
        })
    }
}

impl EncounterableResourceUriClassifier for EncounterableResourcePathClassifier {
    fn classify(
        &self,
        text: &str,
        class: &mut EncounterableResourceClass,
        rewritten_natures: Option<&mut Vec<(String, String, String)>>,
    ) -> bool {
        if self.ignore_paths_regex_set.is_match(text) {
            class
                .flags
                .insert(EncounterableResourceFlags::IGNORE_RESOURCE);
            return true;
        }

        for regex in &self.acquire_content_for_paths_regex_set {
            if let Some(caps) = regex.captures(text) {
                if let Some(nature) = caps.name("nature") {
                    class
                        .flags
                        .insert(EncounterableResourceFlags::CONTENT_ACQUIRABLE);
                    let mut class_nature = nature.as_str().to_string();
                    for rnr in &self.rewrite_nature_regexs {
                        if let Some(rewritten) = rnr.is_match(text) {
                            if let Some(rewritten_natures) = rewritten_natures {
                                rewritten_natures.push((
                                    text.to_string(),
                                    class_nature,
                                    rewritten.to_owned(),
                                ));
                            }
                            class_nature = rewritten.to_owned();
                            break;
                        }
                    }
                    class.nature = Some(class_nature);
                    return true;
                }
            }
        }

        for regex in &self.capturable_executables_paths_regexs {
            if let Some(caps) = regex.captures(text) {
                if let Some(nature) = caps.name("nature") {
                    class
                        .flags
                        .insert(EncounterableResourceFlags::CAPTURABLE_EXECUTABLE);
                    let mut class_nature = nature.as_str().to_string();
                    for rnr in &self.rewrite_nature_regexs {
                        if let Some(rewritten) = rnr.is_match(text) {
                            if let Some(rewritten_natures) = rewritten_natures {
                                rewritten_natures.push((
                                    text.to_string(),
                                    class_nature,
                                    rewritten.to_owned(),
                                ));
                            }
                            class_nature = rewritten.to_owned();
                            break;
                        }
                    }
                    class.nature = Some(class_nature);
                    return true;
                }
            }
        }

        if self.captured_exec_sql_paths_regex_set.is_match(text) {
            class.flags.insert(
                EncounterableResourceFlags::CAPTURABLE_EXECUTABLE
                    | EncounterableResourceFlags::CAPTURABLE_SQL,
            );
            return true;
        }

        false
    }
}

pub struct ContentResource {
    pub flags: ContentResourceFlags,
    pub uri: String,
    pub nature: Option<String>,
    pub size: Option<u64>,
    pub created_at: Option<DateTime<Utc>>,
    pub last_modified_at: Option<DateTime<Utc>>,
    pub content_binary_supplier: Option<BinaryContentSupplier>,
    pub content_text_supplier: Option<TextContentSupplier>,
}

pub struct CapturableExecResource<Resource> {
    pub resource: Resource,
    pub executable: CapturableExecutable,
}

pub struct PlainTextResource<Resource> {
    pub resource: Resource,
}

pub struct HtmlResource<Resource> {
    pub resource: Resource,
}

pub struct ImageResource<Resource> {
    pub resource: Resource,
}

pub enum JsonFormat {
    Json,
    JsonWithComments,
    Unknown,
}

pub struct JsonResource<Resource> {
    pub resource: Resource,
    pub format: JsonFormat,
}

pub enum JsonableTextSchema {
    TestAnythingProtocol,
    Toml,
    Yaml,
    Unknown,
}

pub struct JsonableTextResource<Resource> {
    pub resource: Resource,
    pub schema: JsonableTextSchema,
}

pub struct MarkdownResource<Resource> {
    pub resource: Resource,
}

pub enum SourceCodeInterpreter {
    TypeScript,
    JavaScript,
    Rust,
    Unknown,
}

pub struct SourceCodeResource<Resource> {
    pub resource: Resource,
    pub interpreter: SourceCodeInterpreter,
}

pub enum XmlSchema {
    Svg,
    Unknown,
}

pub struct XmlResource<Resource> {
    pub resource: Resource,
    pub schema: XmlSchema,
}

pub enum UniformResource<Resource> {
    CapturableExec(CapturableExecResource<Resource>),
    Html(HtmlResource<Resource>),
    Image(ImageResource<Resource>),
    Json(JsonResource<Resource>),
    JsonableText(JsonableTextResource<Resource>),
    Markdown(MarkdownResource<Resource>),
    PlainText(PlainTextResource<Resource>),
    SourceCode(SourceCodeResource<Resource>),
    Xml(XmlResource<Resource>),
    Unknown(Resource, Option<String>),
}

pub trait UniformResourceSupplier<Resource> {
    fn uniform_resource(
        &self,
        rs: Resource,
    ) -> Result<Box<UniformResource<Resource>>, Box<dyn Error>>;
}

pub trait UriNatureSupplier<Resource> {
    fn uri(&self) -> &String;
    fn nature(&self) -> &Option<String>;
}

impl UriNatureSupplier<ContentResource> for UniformResource<ContentResource> {
    fn uri(&self) -> &String {
        match self {
            UniformResource::CapturableExec(cer) => &cer.resource.uri,
            UniformResource::Html(html) => &html.resource.uri,
            UniformResource::Image(img) => &img.resource.uri,
            UniformResource::Json(json) => &json.resource.uri,
            UniformResource::JsonableText(json) => &json.resource.uri,
            UniformResource::Markdown(md) => &md.resource.uri,
            UniformResource::PlainText(txt) => &txt.resource.uri,
            UniformResource::SourceCode(sc) => &sc.resource.uri,
            UniformResource::Xml(xml) => &xml.resource.uri,
            UniformResource::Unknown(cr, _alternate) => &cr.uri,
        }
    }

    fn nature(&self) -> &Option<String> {
        match self {
            UniformResource::CapturableExec(cer) => &cer.resource.nature,
            UniformResource::Html(html) => &html.resource.nature,
            UniformResource::Image(img) => &img.resource.nature,
            UniformResource::Json(json) => &json.resource.nature,
            UniformResource::JsonableText(jsonable) => &jsonable.resource.nature,
            UniformResource::Markdown(md) => &md.resource.nature,
            UniformResource::PlainText(txt) => &txt.resource.nature,
            UniformResource::SourceCode(sc) => &sc.resource.nature,
            UniformResource::Xml(xml) => &xml.resource.nature,
            UniformResource::Unknown(_cr, _alternate) => &None::<String>,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ResourceBinaryContent {
    pub hash: String,
    pub binary: Vec<u8>,
}

impl BinaryContent for ResourceBinaryContent {
    fn content_digest_hash(&self) -> &str {
        &self.hash
    }

    fn content_binary(&self) -> &Vec<u8> {
        &self.binary
    }
}

#[derive(Debug, Clone)]
pub struct ResourceTextContent {
    pub hash: String,
    pub text: String,
}

impl TextContent for ResourceTextContent {
    fn content_digest_hash(&self) -> &str {
        &self.hash
    }

    fn content_text(&self) -> &str {
        &self.text
    }

    fn frontmatter(&self) -> FrontmatterComponents {
        frontmatter(&self.text)
    }
}

#[derive(Debug)]
pub struct EncounteredResourceMetaData {
    pub flags: EncounteredResourceFlags,
    pub nature: Option<String>,
    pub file_size: u64,
    pub created_at: Option<chrono::prelude::DateTime<chrono::prelude::Utc>>,
    pub last_modified_at: Option<chrono::prelude::DateTime<chrono::prelude::Utc>>,
}

impl EncounteredResourceMetaData {
    pub fn from_fs_path(fs_path: &Path) -> anyhow::Result<EncounteredResourceMetaData> {
        let mut flags = EncounteredResourceFlags::empty();
        let file_size: u64;
        let created_at: Option<chrono::prelude::DateTime<chrono::prelude::Utc>>;
        let last_modified_at: Option<chrono::prelude::DateTime<chrono::prelude::Utc>>;

        match fs::metadata(fs_path) {
            Ok(metadata) => {
                flags.set(EncounteredResourceFlags::IS_FILE, metadata.is_file());
                flags.set(EncounteredResourceFlags::IS_DIRECTORY, metadata.is_dir());
                flags.set(EncounteredResourceFlags::IS_SYMLINK, metadata.is_symlink());
                file_size = metadata.len();
                created_at = metadata
                    .created()
                    .ok()
                    .map(chrono::DateTime::<chrono::Utc>::from);
                last_modified_at = metadata
                    .modified()
                    .ok()
                    .map(chrono::DateTime::<chrono::Utc>::from);
            }
            Err(err) => {
                let context = format!("ResourceContentMetaData::from_fs_path({:?})", fs_path,);
                return Err(anyhow::Error::new(err).context(context));
            }
        }

        let nature = fs_path
            .extension()
            .map(|ext| ext.to_string_lossy().to_string());

        Ok(EncounteredResourceMetaData {
            flags,
            nature,
            file_size,
            created_at,
            last_modified_at,
        })
    }

    pub fn from_vfs_path(vfs_path: &vfs::VfsPath) -> anyhow::Result<EncounteredResourceMetaData> {
        let mut flags = EncounteredResourceFlags::empty();

        let metadata = match vfs_path.metadata() {
            Ok(metadata) => {
                match metadata.file_type {
                    vfs::VfsFileType::File => {
                        flags.insert(EncounteredResourceFlags::IS_FILE);
                    }
                    vfs::VfsFileType::Directory => {
                        flags.insert(EncounteredResourceFlags::IS_DIRECTORY);
                    }
                };
                metadata
            }
            Err(err) => {
                let context = format!("ResourceContentMetaData::from_vfs_path({:?})", vfs_path);
                return Err(anyhow::Error::new(err).context(context));
            }
        };

        let nature = vfs_path
            .as_str()
            .rsplit_once('.')
            .map(|(_, ext)| ext.to_string());

        Ok(EncounteredResourceMetaData {
            flags,
            nature,
            file_size: metadata.len,
            created_at: None,
            last_modified_at: None,
        })
    }
}

pub struct EncounteredResourceContentSuppliers {
    pub text: Option<TextContentSupplier>,
    pub binary: Option<BinaryContentSupplier>,
}

impl EncounteredResourceContentSuppliers {
    pub fn from_fs_path(
        fs_path: &Path,
        options: &EncounterableResourceClass,
    ) -> EncounteredResourceContentSuppliers {
        let binary: Option<BinaryContentSupplier>;
        let text: Option<TextContentSupplier>;

        if options
            .flags
            .contains(EncounterableResourceFlags::CONTENT_ACQUIRABLE)
        {
            let path_cbs = fs_path.to_string_lossy().to_string(); // Clone for the first closure
            binary = Some(Box::new(
                move || -> Result<Box<dyn BinaryContent>, Box<dyn Error>> {
                    let mut binary = Vec::new();
                    let mut file = fs::File::open(&path_cbs)?;
                    file.read_to_end(&mut binary)?;

                    let hash = {
                        let mut hasher = Sha1::new();
                        hasher.update(&binary);
                        format!("{:x}", hasher.finalize())
                    };

                    Ok(Box::new(ResourceBinaryContent { hash, binary }) as Box<dyn BinaryContent>)
                },
            ));

            let path_cts = fs_path.to_string_lossy().to_string(); // Clone for the second closure
            text = Some(Box::new(
                move || -> Result<Box<dyn TextContent>, Box<dyn Error>> {
                    let mut text = String::new();
                    let mut file = fs::File::open(&path_cts)?;
                    file.read_to_string(&mut text)?;

                    let hash = {
                        let mut hasher = Sha1::new();
                        hasher.update(&text);
                        format!("{:x}", hasher.finalize())
                    };

                    Ok(Box::new(ResourceTextContent { hash, text }) as Box<dyn TextContent>)
                },
            ));
        } else {
            binary = None;
            text = None;
        }

        EncounteredResourceContentSuppliers { binary, text }
    }

    pub fn from_vfs_path(
        vfs_path: &vfs::VfsPath,
        options: &EncounterableResourceClass,
    ) -> EncounteredResourceContentSuppliers {
        let binary: Option<BinaryContentSupplier>;
        let text: Option<TextContentSupplier>;

        if options
            .flags
            .contains(EncounterableResourceFlags::CONTENT_ACQUIRABLE)
        {
            let path_clone_cbs = vfs_path.clone();
            binary = Some(Box::new(
                move || -> Result<Box<dyn BinaryContent>, Box<dyn Error>> {
                    let mut binary = Vec::new();
                    let mut file = path_clone_cbs.open_file()?;
                    file.read_to_end(&mut binary)?;

                    let hash = {
                        let mut hasher = Sha1::new();
                        hasher.update(&binary);
                        format!("{:x}", hasher.finalize())
                    };

                    Ok(Box::new(ResourceBinaryContent { hash, binary }) as Box<dyn BinaryContent>)
                },
            ));

            let path_clone_cts = vfs_path.clone();
            text = Some(Box::new(
                move || -> Result<Box<dyn TextContent>, Box<dyn Error>> {
                    let mut text = String::new();
                    let mut file = path_clone_cts.open_file()?;
                    file.read_to_string(&mut text)?;

                    let hash = {
                        let mut hasher = Sha1::new();
                        hasher.update(&text);
                        format!("{:x}", hasher.finalize())
                    };

                    Ok(Box::new(ResourceTextContent { hash, text }) as Box<dyn TextContent>)
                },
            ));
        } else {
            text = None;
            binary = None;
        }

        EncounteredResourceContentSuppliers { text, binary }
    }
}

pub enum EncounterableResource {
    WalkDir(walkdir::DirEntry),
    SmartIgnore(ignore::DirEntry),
    Vfs(vfs::VfsPath),
    DenoTaskShellLine(String, Option<String>, String),
}

impl EncounterableResource {
    /// Parses a given string input as a JSON value and returns a DenoTaskShellLine.
    ///
    /// # Arguments
    ///
    /// * `line` - A string slice that represents either a JSON object or a plain text.
    ///
    /// # Returns
    ///
    /// DenoTaskShellLine:
    /// - The first string value found in the JSON object, or the entire input string if not a JSON object.
    /// - An `Option<String>` containing the key corresponding to the first string value, or `None` if the input is not a JSON object or doesn't contain a string value.
    /// - A string that is either `"json"` or the value of the `"nature"` key in the JSON object, if present.
    ///
    /// # Examples
    ///
    /// ```
    /// let json_str = r#"{ "my_cmd_identity": "echo \"hello world\"", "nature": "text/plain" }"#;
    /// let result = dts_er(json_str);
    /// assert_eq!(result, ("echo \"hello world\"".to_string(), Some("my_cmd_identity".to_string()), "text/plain".to_string()));
    ///
    /// let non_json_str = "echo \"Hello, world!\"";
    /// let result = dts_er(non_json_str);
    /// assert_eq!(result, ("Hello, world!".to_string(), None, "json".to_string()));
    /// ```
    pub fn from_deno_task_shell_line(line: impl AsRef<str>) -> EncounterableResource {
        let default_nature = "json".to_string();
        let (commands, identity, nature) = match serde_json::from_str::<JsonValue>(line.as_ref()) {
            Ok(parsed) => {
                if let Some(obj) = parsed.as_object() {
                    let mut task: String = "no task found".to_string();
                    let mut identity: Option<String> = None;
                    let mut nature = default_nature.clone();
                    obj.iter()
                        .filter(|(_, v)| v.is_string())
                        .for_each(|(key, value)| {
                            if key == "nature" {
                                nature = JsonValue::as_str(value)
                                    .unwrap_or(default_nature.as_str())
                                    .to_string();
                            } else {
                                task = JsonValue::as_str(value)
                                    .unwrap_or(default_nature.as_str())
                                    .to_string();
                                identity = Some(key.to_owned());
                            }
                        });

                    (task, identity, nature)
                } else {
                    (line.as_ref().to_owned(), None, default_nature)
                }
            }
            Err(_) => (line.as_ref().to_owned(), None, default_nature),
        };
        EncounterableResource::DenoTaskShellLine(commands, identity, nature)
    }
}

pub enum EncounteredResource<T> {
    Ignored(String, EncounterableResourceClass),
    NotFound(String, EncounterableResourceClass),
    NotFile(String, EncounterableResourceClass),
    Resource(T, EncounterableResourceClass),
    CapturableExec(T, CapturableExecutable, EncounterableResourceClass),
}

impl ShellExecutive for EncounterableResource {
    fn execute(&self, std_in: ShellStdIn) -> anyhow::Result<ShellResult> {
        execute_subprocess(self.uri(), std_in)
    }
}

impl EncounterableResource {
    pub fn uri(&self) -> String {
        match self {
            EncounterableResource::WalkDir(de) => de.path().to_string_lossy().to_string(),
            EncounterableResource::SmartIgnore(de) => de.path().to_string_lossy().to_string(),
            EncounterableResource::Vfs(path) => path.as_str().to_string(),
            EncounterableResource::DenoTaskShellLine(line, identity, _) => {
                identity.to_owned().unwrap_or(line.as_str().to_string())
            }
        }
    }

    pub fn meta_data(&self) -> anyhow::Result<EncounteredResourceMetaData> {
        match self {
            EncounterableResource::WalkDir(de) => {
                EncounteredResourceMetaData::from_fs_path(de.path())
            }
            EncounterableResource::SmartIgnore(de) => {
                EncounteredResourceMetaData::from_fs_path(de.path())
            }
            EncounterableResource::Vfs(path) => EncounteredResourceMetaData::from_vfs_path(path),
            EncounterableResource::DenoTaskShellLine(_, _, nature) => {
                Ok(EncounteredResourceMetaData {
                    flags: EncounteredResourceFlags::empty(),
                    nature: Some(nature.clone()),
                    file_size: 0,
                    created_at: None,
                    last_modified_at: None,
                })
            }
        }
    }

    pub fn content_suppliers(
        &self,
        options: &EncounterableResourceClass,
    ) -> EncounteredResourceContentSuppliers {
        match self {
            EncounterableResource::WalkDir(de) => {
                EncounteredResourceContentSuppliers::from_fs_path(de.path(), options)
            }
            EncounterableResource::SmartIgnore(de) => {
                EncounteredResourceContentSuppliers::from_fs_path(de.path(), options)
            }
            EncounterableResource::Vfs(path) => {
                EncounteredResourceContentSuppliers::from_vfs_path(path, options)
            }
            EncounterableResource::DenoTaskShellLine(_, _, _) => {
                EncounteredResourceContentSuppliers {
                    text: None,
                    binary: None,
                }
            }
        }
    }

    pub fn encountered(
        &self,
        erc: &EncounterableResourceClass,
    ) -> EncounteredResource<ContentResource> {
        let uri = self.uri();

        if erc
            .flags
            .contains(EncounterableResourceFlags::IGNORE_RESOURCE)
        {
            return EncounteredResource::Ignored(uri, erc.to_owned());
        }

        let metadata = match self.meta_data() {
            Ok(metadata) => match self {
                EncounterableResource::WalkDir(_)
                | EncounterableResource::SmartIgnore(_)
                | EncounterableResource::Vfs(_) => {
                    if !metadata.flags.contains(EncounteredResourceFlags::IS_FILE) {
                        return EncounteredResource::NotFile(uri, erc.to_owned());
                    }
                    metadata
                }
                EncounterableResource::DenoTaskShellLine(_, _, _) => metadata,
            },
            Err(_) => return EncounteredResource::NotFound(uri, erc.to_owned()),
        };

        let content_suppliers = self.content_suppliers(erc);
        let nature: String;
        match &erc.nature {
            Some(classification_nature) => nature = classification_nature.to_owned(),
            None => match &metadata.nature {
                Some(md_nature) => nature = md_nature.to_owned(),
                None => nature = "json".to_string(),
            },
        }
        let cr: ContentResource = ContentResource {
            flags: ContentResourceFlags::from_bits_truncate(erc.flags.bits()),
            uri: uri.to_string(),
            nature: Some(nature.clone()),
            size: Some(metadata.file_size),
            created_at: metadata.created_at,
            last_modified_at: metadata.last_modified_at,
            content_binary_supplier: content_suppliers.binary,
            content_text_supplier: content_suppliers.text,
        };

        match self {
            EncounterableResource::WalkDir(_)
            | EncounterableResource::SmartIgnore(_)
            | EncounterableResource::Vfs(_) => {
                if erc
                    .flags
                    .contains(EncounterableResourceFlags::CAPTURABLE_EXECUTABLE)
                {
                    EncounteredResource::CapturableExec(
                        cr,
                        CapturableExecutable::from_encountered_content(self, erc),
                        erc.to_owned(),
                    )
                } else {
                    EncounteredResource::Resource(cr, erc.to_owned())
                }
            }
            EncounterableResource::DenoTaskShellLine(_, _, _) => {
                EncounteredResource::CapturableExec(
                    cr,
                    CapturableExecutable::from_encountered_content(self, erc),
                    erc.to_owned(),
                )
            }
        }
    }
}

pub enum CapturableExecutable {
    UriShellExecutive(Box<dyn ShellExecutive>, String, String, bool),
    RequestedButNotExecutable(String),
}

impl CapturableExecutable {
    pub fn from_encountered_content(
        er: &EncounterableResource,
        erc: &EncounterableResourceClass,
    ) -> CapturableExecutable {
        match er {
            EncounterableResource::WalkDir(de) => {
                CapturableExecutable::from_executable_file_path(de.path(), erc)
            }
            EncounterableResource::SmartIgnore(de) => {
                CapturableExecutable::from_executable_file_path(de.path(), erc)
            }
            EncounterableResource::Vfs(path) => {
                CapturableExecutable::from_executable_file_uri(path.as_str(), erc)
            }
            EncounterableResource::DenoTaskShellLine(line, identity, nature) => {
                CapturableExecutable::UriShellExecutive(
                    Box::new(DenoTaskShellExecutive::new(
                        line.clone(),
                        identity.to_owned(),
                    )),
                    line.clone(),
                    nature.to_string(),
                    erc.flags
                        .contains(EncounterableResourceFlags::CAPTURABLE_SQL),
                )
            }
        }
    }

    // check if URI is executable based only on the filename pattern
    pub fn from_executable_file_uri(
        uri: &str,
        erc: &EncounterableResourceClass,
    ) -> CapturableExecutable {
        let executable_file_uri = uri.to_string();
        CapturableExecutable::UriShellExecutive(
            Box::new(executable_file_uri.clone()), // String has the `ShellExecutive` trait
            executable_file_uri,
            erc.nature.clone().unwrap_or("?nature".to_string()),
            erc.flags
                .contains(EncounterableResourceFlags::CAPTURABLE_SQL),
        )
    }

    // check if URI is executable based the filename pattern first, then physical FS validation of execute permission
    pub fn from_executable_file_path(
        path: &std::path::Path,
        erc: &EncounterableResourceClass,
    ) -> CapturableExecutable {
        if path.is_executable() {
            CapturableExecutable::from_executable_file_uri(path.to_str().unwrap(), erc)
        } else {
            CapturableExecutable::RequestedButNotExecutable(path.to_string_lossy().to_string())
        }
    }

    pub fn uri(&self) -> &str {
        match self {
            CapturableExecutable::UriShellExecutive(_, uri, _, _)
            | CapturableExecutable::RequestedButNotExecutable(uri) => uri.as_str(),
        }
    }

    pub fn executed_result_as_text(
        &self,
        std_in: ShellStdIn,
    ) -> anyhow::Result<(String, String, bool), serde_json::Value> {
        match self {
            CapturableExecutable::UriShellExecutive(
                executive,
                interpretable_code,
                nature,
                is_batched_sql,
            ) => match executive.execute(std_in) {
                Ok(shell_result) => {
                    if shell_result.success() {
                        Ok((shell_result.stdout, nature.clone(), *is_batched_sql))
                    } else {
                        Err(serde_json::json!({
                            "src": self.uri(),
                            "interpretable-code": interpretable_code,
                            "issue": "[CapturableExecutable::TextFromExecutableUri.executed_text] invalid exit status",
                            "remediation": "ensure that executable is called with proper arguments and input formats",
                            "nature": nature,
                            "exit-status": format!("{:?}", shell_result.status),
                            "stdout": shell_result.stdout,
                            "stderr": shell_result.stderr
                        }))
                    }
                }
                Err(err) => Err(serde_json::json!({
                    "src": self.uri(),
                    "interpretable-code": interpretable_code,
                    "issue": "[CapturableExecutable::TextFromExecutableUri.executed_text] execution error",
                    "rust-err": format!("{:?}", err),
                    "nature": nature,
                })),
            },
            CapturableExecutable::RequestedButNotExecutable(src) => Err(serde_json::json!({
                "src": src,
                "issue": "[CapturableExecutable::RequestedButNotExecutable.executed_sql] executable permissions not set",
                "remediation": "make sure that script has executable permissions set",
            })),
        }
    }

    pub fn executed_result_as_json(
        &self,
        std_in: ShellStdIn,
    ) -> anyhow::Result<(serde_json::Value, String, bool), serde_json::Value> {
        match self {
            CapturableExecutable::UriShellExecutive(
                executive,
                interpretable_code,
                nature,
                is_batched_sql,
            ) => match executive.execute(std_in) {
                Ok(shell_result) => {
                    if shell_result.success() {
                        let captured_text = shell_result.stdout;
                        let value: serde_json::Result<serde_json::Value> =
                            serde_json::from_str(&captured_text);
                        match value {
                            Ok(value) => Ok((value, nature.clone(), *is_batched_sql)),
                            Err(_) => Err(serde_json::json!({
                                "src": self.uri(),
                                "interpretable-code": interpretable_code,
                                "issue": "[CapturableExecutable::TextFromExecutableUri.executed_result_as_json] unable to deserialize JSON",
                                "remediation": "ensure that executable is emitting JSON (e.g. `--json`)",
                                "nature": nature,
                                "is-batched-sql": is_batched_sql,
                                "stdout": captured_text,
                                "exit-status": format!("{:?}", shell_result.status),
                                "stderr": shell_result.stderr
                            })),
                        }
                    } else {
                        Err(serde_json::json!({
                            "src": self.uri(),
                            "interpretable-code": interpretable_code,
                            "issue": "[CapturableExecutable::TextFromExecutableUri.executed_result_as_json] invalid exit status",
                            "remediation": "ensure that executable is called with proper arguments and input formats",
                            "nature": nature,
                            "is-batched-sql": is_batched_sql,
                            "exit-status": format!("{:?}", shell_result.status),
                            "stderr": shell_result.stderr
                        }))
                    }
                }
                Err(err) => Err(serde_json::json!({
                    "src": self.uri(),
                    "issue": "[CapturableExecutable::TextFromExecutableUri.executed_result_as_json] execution error",
                    "rust-err": format!("{:?}", err),
                    "nature": nature,
                    "is-batched-sql": is_batched_sql,
                })),
            },
            CapturableExecutable::RequestedButNotExecutable(src) => Err(serde_json::json!({
                "src": src,
                "issue": "[CapturableExecutable::RequestedButNotExecutable.executed_result_as_json] executable permissions not set",
                "remediation": "make sure that script has executable permissions set",
            })),
        }
    }

    pub fn executed_result_as_sql(
        &self,
        std_in: ShellStdIn,
    ) -> anyhow::Result<(String, String), serde_json::Value> {
        match self {
            CapturableExecutable::UriShellExecutive(
                executive,
                interpretable_code,
                nature,
                is_batched_sql,
            ) => {
                if *is_batched_sql {
                    match executive.execute(std_in) {
                        Ok(shell_result) => {
                            if shell_result.status.success() {
                                Ok((shell_result.stdout, nature.clone()))
                            } else {
                                Err(serde_json::json!({
                                    "src": self.uri(),
                                    "interpretable-code": interpretable_code,
                                    "issue": "[CapturableExecutable::TextFromExecutableUri.executed_result_as_sql] invalid exit status",
                                    "remediation": "ensure that executable is called with proper arguments and input formats",
                                    "nature": nature,
                                    "exit-status": format!("{:?}", shell_result.status),
                                    "stdout": shell_result.stdout,
                                    "stderr": shell_result.stderr
                                }))
                            }
                        }
                        Err(err) => Err(serde_json::json!({
                            "src": self.uri(),
                            "interpretable-code": interpretable_code,
                            "issue": "[CapturableExecutable::TextFromExecutableUri.executed_result_as_sql] execution error",
                            "rust-err": format!("{:?}", err),
                            "nature": nature,
                        })),
                    }
                } else {
                    Err(serde_json::json!({
                        "src": self.uri(),
                        "interpretable-code": interpretable_code,
                        "issue": "[CapturableExecutable::TextFromExecutableUri.executed_result_as_sql] is not classified as batch SQL",
                        "nature": nature,
                    }))
                }
            }
            CapturableExecutable::RequestedButNotExecutable(src) => Err(serde_json::json!({
                "src": src,
                "issue": "[CapturableExecutable::RequestedButNotExecutable.executed_result_as_sql] executable permissions not set",
                "remediation": "make sure that script has executable permissions set",
            })),
        }
    }
}

pub struct ResourcesCollection {
    pub encounterable: Vec<EncounterableResource>,
    pub classifier: EncounterableResourcePathClassifier,
}

impl ResourcesCollection {
    pub fn new(
        encounterable: Vec<EncounterableResource>,
        classifier: EncounterableResourcePathClassifier,
    ) -> ResourcesCollection {
        ResourcesCollection {
            encounterable,
            classifier,
        }
    }

    // create a physical file system mapped via VFS, mainly for testing and experimental use
    pub fn from_vfs_physical_fs(
        fs_root_paths: &[String],
        classifier: EncounterableResourcePathClassifier,
    ) -> ResourcesCollection {
        let physical_fs = vfs::PhysicalFS::new("/");
        let vfs_fs_root = vfs::VfsPath::new(physical_fs);

        let vfs_iter = fs_root_paths
            .iter()
            .flat_map(move |physical_fs_root_path_orig| {
                let physical_fs_root_path: String;
                if let Ok(canonical) = canonicalize(physical_fs_root_path_orig.clone()) {
                    physical_fs_root_path = canonical.to_string_lossy().to_string();
                } else {
                    eprintln!(
                        "Error canonicalizing {}, trying original",
                        physical_fs_root_path_orig
                    );
                    physical_fs_root_path = physical_fs_root_path_orig.to_string();
                }

                let path = vfs_fs_root.join(physical_fs_root_path).unwrap();
                path.walk_dir().unwrap().flatten()
            });

        ResourcesCollection::new(
            vfs_iter.map(EncounterableResource::Vfs).collect(),
            classifier,
        )
    }

    // create a ignore::Walk instance which is a "smart" ignore because it honors .gitigore and .ignore
    // files in the walk path as well as the ignore and other directives passed in via options
    pub fn from_smart_ignore(
        fs_root_paths: &[String],
        classifier: EncounterableResourcePathClassifier,
        ignore_globs_conf_file: &str,
        ignore_hidden: bool,
    ) -> ResourcesCollection {
        let vfs_iter = fs_root_paths.iter().flat_map(move |root_path| {
            let ignorable_walk = ignore::WalkBuilder::new(root_path)
                .hidden(ignore_hidden)
                .add_custom_ignore_filename(ignore_globs_conf_file)
                .build();
            ignorable_walk.into_iter().flatten()
        });

        ResourcesCollection::new(
            vfs_iter.map(EncounterableResource::SmartIgnore).collect(),
            classifier,
        )
    }

    // create a traditional walkdir::WalkDir which only ignore files based on file names rules passed in
    pub fn from_walk_dir(
        fs_root_paths: &[String],
        classifier: EncounterableResourcePathClassifier,
    ) -> ResourcesCollection {
        let vfs_iter = fs_root_paths
            .iter()
            .flat_map(move |root_path| walkdir::WalkDir::new(root_path).into_iter().flatten());

        ResourcesCollection::new(
            vfs_iter.map(EncounterableResource::WalkDir).collect(),
            classifier,
        )
    }

    pub fn from_tasks_lines(
        tasks: &[String],
        classifier: EncounterableResourcePathClassifier,
    ) -> (Vec<String>, ResourcesCollection) {
        let encounterable: Vec<_> = tasks
            .iter()
            .filter(|line| !line.starts_with('#'))
            .filter(|line| !line.trim().is_empty())
            .map(|line| line.to_owned())
            .collect();

        (
            encounterable.clone(),
            ResourcesCollection::new(
                encounterable
                    .iter()
                    .map(EncounterableResource::from_deno_task_shell_line)
                    .collect(),
                classifier,
            ),
        )
    }

    pub fn ignored(&self) -> impl Iterator<Item = EncounteredResource<ContentResource>> + '_ {
        self.encountered()
            .filter(|er| matches!(er, EncounteredResource::Ignored(_, _)))
    }

    pub fn not_ignored(&self) -> impl Iterator<Item = EncounteredResource<ContentResource>> + '_ {
        self.encountered()
            .filter(|er| !matches!(er, EncounteredResource::Ignored(_, _)))
    }

    pub fn capturable_executables(&self) -> impl Iterator<Item = CapturableExecutable> + '_ {
        self.encountered().filter_map(|er| match er {
            EncounteredResource::CapturableExec(_, ce, _) => Some(ce),
            _ => None,
        })
    }

    pub fn encountered(&self) -> impl Iterator<Item = EncounteredResource<ContentResource>> + '_ {
        self.encounterable.iter().map(move |er| {
            let uri = er.uri();
            let mut ero = EncounterableResourceClass {
                nature: None,
                flags: EncounterableResourceFlags::empty(),
            };
            self.classifier.classify(&uri, &mut ero, None);
            er.encountered(&ero)
        })
    }

    pub fn uniform_resources(
        &self,
    ) -> impl Iterator<Item = anyhow::Result<UniformResource<ContentResource>, Box<dyn Error>>> + '_
    {
        self.encountered()
            .filter_map(move |er: EncounteredResource<ContentResource>| match er {
                EncounteredResource::Resource(resource, _) => {
                    match self.uniform_resource(resource) {
                        Ok(uniform_resource) => Some(Ok(*uniform_resource)),
                        Err(e) => Some(Err(e)), // error will be returned
                    }
                }
                EncounteredResource::CapturableExec(resource, executable, _) => Some(Ok(
                    UniformResource::CapturableExec(CapturableExecResource {
                        resource,
                        executable,
                    }),
                )),
                EncounteredResource::Ignored(_, _)
                | EncounteredResource::NotFile(_, _)
                | EncounteredResource::NotFound(_, _) => None, // these will be filtered via `filter_map`
            })
    }

    pub fn uniform_resource(
        &self,
        cr: ContentResource,
    ) -> Result<Box<UniformResource<ContentResource>>, Box<dyn Error>> {
        // Based on the nature of the resource, we determine the type of UniformResource
        if let Some(candidate_nature) = &cr.nature {
            let candidate_nature = candidate_nature.as_str();

            match candidate_nature {
                // Match different file extensions
                "html" | "text/html" => {
                    let html = HtmlResource {
                        resource: cr,
                        // TODO parse using
                        //      - https://github.com/y21/tl (performant but not spec compliant)
                        //      - https://github.com/cloudflare/lol-html (more performant, spec compliant)
                        //      - https://github.com/causal-agent/scraper or https://github.com/servo/html5ever directly
                        // create HTML parser presets which can go through all stored HTML, running selectors and putting them into tables?
                    };
                    Ok(Box::new(UniformResource::Html(html)))
                }
                "json" | "jsonc" | "application/json" => {
                    let format = match candidate_nature {
                        "json" | "application/json" => JsonFormat::Json,
                        "jsonc" => JsonFormat::JsonWithComments,
                        _ => JsonFormat::Unknown,
                    };
                    let json = JsonResource {
                        resource: cr,
                        format,
                    };
                    Ok(Box::new(UniformResource::Json(json)))
                }
                "tap" | "toml" | "application/toml" | "yml" | "application/yaml" => {
                    let format = match candidate_nature {
                        "tap" => JsonableTextSchema::TestAnythingProtocol,
                        "toml" | "application/toml" => JsonableTextSchema::Toml,
                        "yml" | "application/yaml" => JsonableTextSchema::Yaml,
                        _ => JsonableTextSchema::Unknown,
                    };
                    let yaml = JsonableTextResource {
                        resource: cr,
                        schema: format,
                    };
                    Ok(Box::new(UniformResource::JsonableText(yaml)))
                }
                "js" | "rs" | "ts" => {
                    let interpreter = match candidate_nature {
                        "js" => SourceCodeInterpreter::JavaScript,
                        "rs" => SourceCodeInterpreter::Rust,
                        "ts" => SourceCodeInterpreter::TypeScript,
                        _ => SourceCodeInterpreter::Unknown,
                    };
                    let source_code = SourceCodeResource {
                        resource: cr,
                        interpreter,
                    };
                    Ok(Box::new(UniformResource::SourceCode(source_code)))
                }
                "md" | "mdx" | "text/markdown" => {
                    let markdown = MarkdownResource { resource: cr };
                    Ok(Box::new(UniformResource::Markdown(markdown)))
                }
                "txt" | "text/plain" => {
                    let plain_text = PlainTextResource { resource: cr };
                    Ok(Box::new(UniformResource::PlainText(plain_text)))
                }
                "png" | "gif" | "tiff" | "jpg" | "jpeg" => {
                    // TODO: need to implement `infer` crate auto-detection
                    let image = ImageResource { resource: cr };
                    Ok(Box::new(UniformResource::Image(image)))
                }
                "svg" | "image/svg+xml" | "xml" | "text/xml" | "application/xml" => {
                    let schema = match candidate_nature {
                        "svg" | "image/svg+xml" => XmlSchema::Svg,
                        "xml" | "text/xml" | "application/xml" => XmlSchema::Unknown,
                        _ => XmlSchema::Unknown,
                    };
                    let xml = XmlResource {
                        resource: cr,
                        schema,
                    };
                    Ok(Box::new(UniformResource::Xml(xml)))
                }
                _ => Ok(Box::new(UniformResource::Unknown(cr, None))),
            }
        } else {
            Err(format!(
                "Unable to obtain nature for {} from supplied resource",
                cr.uri
            )
            .into())
        }
    }
}

/// Extracts various path-related information from the given root path and entry.
///
/// # Parameters
///
/// * `root_path` - The root directory path as a reference to a `Path`.
/// * `root_path_entry` - The file or directory entry path as a reference to a `Path`.
///
/// # Returns
///
/// A tuple containing:
/// - `file_path_abs`: Absolute path of `root_path_entry`.
/// - `file_path_rel_parent`: The parent directory of `root_path_entry`.
/// - `file_path_rel`: Path of `root_path_entry` relative to `root_path`.
/// - `file_basename`: The basename of `root_path_entry` (with extension).
/// - `file_extn`: The file extension of `root_path_entry` (without `.`).
///
/// # Errors
///
/// Returns `None` if any of the path conversions fail.
pub fn extract_path_info(
    root_path: &Path,
    root_path_entry: &Path,
) -> Option<(PathBuf, PathBuf, PathBuf, String, Option<String>)> {
    let file_path_abs = root_path_entry.canonicalize().ok()?;
    let file_path_rel_parent = root_path_entry.parent()?.to_path_buf();
    let file_path_rel = root_path_entry.strip_prefix(root_path).ok()?.to_path_buf();
    let file_basename = root_path_entry.file_name()?.to_str()?.to_string();
    let file_extn = root_path_entry
        .extension()
        .and_then(|s| s.to_str())
        .map(String::from);

    Some((
        file_path_abs,
        file_path_rel_parent,
        file_path_rel,
        file_basename,
        file_extn,
    ))
}
