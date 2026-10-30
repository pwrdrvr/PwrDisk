//! The skills `pwrdisk mcp` carries: how to use its tools well, and what
//! each system hides from a scan or must never lose.
//!
//! They are compiled in from `skills/` and served three ways, so every
//! client finds them: as `skill://<name>/<file>` resources, through the
//! skills extension's `skills/list` and `skills/get`, and as prompts. The
//! list of files is written out here: a URI is looked up, never turned
//! into a path, so nothing outside these files can be read through it.

use serde_json::{Map, Value, json};
use sha2::{Digest as _, Sha256};

/// The skills extension, as servers that carry skills declare it.
pub const EXTENSION: &str = "io.modelcontextprotocol/skills";

/// One file of a skill.
#[derive(Debug)]
pub struct File {
    /// Relative to the skill's directory: `SKILL.md`, `references/…`.
    pub path: &'static str,
    pub text: &'static str,
}

/// A skill: its directory's name and its files, `SKILL.md` first.
#[derive(Debug)]
pub struct Skill {
    pub name: &'static str,
    pub files: &'static [File],
}

// Plain literals throughout: CI's change classifier finds every
// `include_str!` path and keeps it out of the docs-only allowlist.
pub const SKILLS: &[Skill] = &[
    Skill {
        name: "pwrdisk-explain-usage",
        files: &[File {
            path: "SKILL.md",
            text: include_str!(
                "../../../skills/pwrdisk-explain-usage/SKILL.md"
            ),
        }],
    },
    Skill {
        name: "pwrdisk-free-space",
        files: &[
            File {
                path: "SKILL.md",
                text: include_str!(
                    "../../../skills/pwrdisk-free-space/SKILL.md"
                ),
            },
            File {
                path: "references/macos.md",
                text: include_str!(
                    "../../../skills/pwrdisk-free-space/references/macos.md"
                ),
            },
            File {
                path: "references/linux.md",
                text: include_str!(
                    "../../../skills/pwrdisk-free-space/references/linux.md"
                ),
            },
            File {
                path: "references/windows.md",
                text: include_str!(
                    "../../../skills/pwrdisk-free-space/references/windows.md"
                ),
            },
        ],
    },
];

/// A file's address.
pub fn uri(skill: &str, path: &str) -> String {
    format!("skill://{skill}/{path}")
}

impl Skill {
    /// The skill's own file.
    pub const fn skill_md(&self) -> &File {
        &self.files[0]
    }

    /// The address of its `SKILL.md`, which names the skill.
    pub fn uri(&self) -> String {
        uri(self.name, self.skill_md().path)
    }

    /// The `SKILL.md` frontmatter, one `key: value` per line, as JSON.
    pub fn frontmatter(&self) -> Map<String, Value> {
        frontmatter(self.skill_md().text)
    }

    /// What the frontmatter says the skill is for.
    pub fn description(&self) -> String {
        self.frontmatter()
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    }

    /// The skills extension's entry: the address, the frontmatter, and
    /// every file with its digest and size, so a client can cache them.
    pub fn manifest(&self) -> Value {
        let resources: Vec<Value> = self
            .files
            .iter()
            .map(|file| {
                json!({
                    "uri": uri(self.name, file.path),
                    "digest": digest(file.text),
                    "size": file.text.len(),
                })
            })
            .collect();
        json!({
            "uri": self.uri(),
            "frontmatter": self.frontmatter(),
            "resources": resources,
        })
    }
}

/// The skill named `name`.
pub fn named(name: &str) -> Option<&'static Skill> {
    SKILLS.iter().find(|skill| skill.name == name)
}

/// The skill whose `SKILL.md` is at `uri`.
pub fn by_uri(uri: &str) -> Option<&'static Skill> {
    SKILLS.iter().find(|skill| skill.uri() == uri)
}

/// The text at `uri`: an exact match against the files above, or nothing.
pub fn read(address: &str) -> Option<&'static str> {
    SKILLS.iter().find_map(|skill| {
        skill
            .files
            .iter()
            .find(|file| uri(skill.name, file.path) == address)
            .map(|file| file.text)
    })
}

/// `sha256:` and the hex digest, as the skills extension writes it.
pub fn digest(text: &str) -> String {
    let hash = Sha256::digest(text.as_bytes());
    let mut out = String::with_capacity(7 + hash.len() * 2);
    out.push_str("sha256:");
    for byte in hash {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// The `key: value` lines between the opening `---` and the next. Our
/// skills keep every value on one line, so no YAML parser is needed; a
/// test holds them to that.
pub fn frontmatter(text: &str) -> Map<String, Value> {
    let mut fields = Map::new();
    let Some(rest) = text.strip_prefix("---\n") else {
        return fields;
    };
    for line in rest.lines().take_while(|line| *line != "---") {
        if let Some((key, value)) = line.split_once(": ") {
            fields.insert(key.trim().to_string(), json!(value.trim()));
        }
    }
    fields
}

/// The reference for the system this server runs on, if there is one.
pub const fn platform_reference() -> Option<&'static str> {
    if cfg!(target_os = "macos") {
        Some("references/macos.md")
    } else if cfg!(windows) {
        Some("references/windows.md")
    } else if cfg!(unix) {
        Some("references/linux.md")
    } else {
        None
    }
}

/// What a prompt hands the agent: the skill, then this system's reference,
/// so one message holds what it needs here.
pub fn prompt_text(skill: &Skill) -> String {
    let mut text = skill.skill_md().text.to_string();
    let reference = platform_reference().and_then(|path| {
        named("pwrdisk-free-space")?
            .files
            .iter()
            .find(|file| file.path == path)
    });
    if let Some(reference) = reference {
        text.push_str("\n---\n\n");
        text.push_str(reference.text);
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_skill_has_frontmatter_that_names_it() {
        for skill in SKILLS {
            let text = skill.skill_md().text;
            assert_eq!(skill.skill_md().path, "SKILL.md");
            assert!(text.starts_with("---\n"), "{}", skill.name);
            let end = text[4..].find("\n---\n").expect("closing ---");
            // One `key: value` per line: what [`frontmatter`] reads.
            for line in text[4..4 + end].lines() {
                assert!(line.contains(": "), "{}: {line}", skill.name);
            }
            let fields = skill.frontmatter();
            assert_eq!(fields["name"], skill.name);
            let description = skill.description();
            assert!(!description.is_empty(), "{}", skill.name);
            // Agent hosts cut descriptions at 1024 characters.
            assert!(description.len() <= 1024, "{}", skill.name);
        }
    }

    #[test]
    fn every_skill_address_in_a_skill_resolves() {
        for skill in SKILLS {
            for file in skill.files {
                let mut rest = file.text;
                while let Some(at) = rest.find("skill://") {
                    let tail = &rest[at..];
                    let end = tail
                        .find(|c: char| c == '`' || c.is_whitespace())
                        .unwrap_or(tail.len());
                    let address = tail[..end].trim_end_matches(['.', ',']);
                    assert!(
                        read(address).is_some(),
                        "{}/{} points at {address}",
                        skill.name,
                        file.path
                    );
                    rest = &tail[end..];
                }
            }
        }
    }

    #[test]
    fn only_listed_files_are_read() {
        assert!(read("skill://pwrdisk-free-space/SKILL.md").is_some());
        for address in [
            "",
            "skill://pwrdisk-free-space",
            "skill://pwrdisk-free-space/",
            "skill://pwrdisk-free-space/SKILL.md/",
            "skill://pwrdisk-free-space/SKILL.md?x=1",
            "skill://pwrdisk-free-space/../pwrdisk-explain-usage/SKILL.md",
            "skill://pwrdisk-free-space/%2e%2e/Cargo.toml",
            "skill://unknown/SKILL.md",
            "file:///etc/passwd",
        ] {
            assert!(read(address).is_none(), "{address}");
        }
    }

    #[test]
    fn manifests_list_every_file_with_its_digest() {
        let skill = named("pwrdisk-free-space").expect("free-space");
        let manifest = skill.manifest();
        assert_eq!(manifest["uri"], "skill://pwrdisk-free-space/SKILL.md");
        let resources = manifest["resources"].as_array().expect("files");
        assert_eq!(resources.len(), skill.files.len());
        for (entry, file) in resources.iter().zip(skill.files) {
            assert_eq!(entry["size"], file.text.len());
            assert_eq!(entry["digest"], digest(file.text));
        }
        // A known digest, so the format does not drift.
        assert_eq!(
            digest("abc"),
            "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn a_prompt_carries_this_systems_reference() {
        let skill = named("pwrdisk-explain-usage").expect("explain");
        let text = prompt_text(skill);
        assert!(text.starts_with(skill.skill_md().text));
        if cfg!(target_os = "macos") {
            assert!(text.contains("# PwrDisk on macOS"));
        } else if cfg!(windows) {
            assert!(text.contains("# PwrDisk on Windows"));
        } else {
            assert!(text.contains("# PwrDisk on Linux"));
        }
    }
}
