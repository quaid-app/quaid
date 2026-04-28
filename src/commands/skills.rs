use anyhow::Result;
use clap::Subcommand;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Subcommand)]
pub enum SkillsAction {
    /// List all resolved skills with their source paths
    List,
    /// Verify skill resolution order, format, and content hashes
    Doctor,
}

struct EmbeddedSkill {
    name: &'static str,
    content: &'static str,
}

const EMBEDDED_SKILLS: &[EmbeddedSkill] = &[
    EmbeddedSkill {
        name: "ingest",
        content: include_str!("../../skills/ingest/SKILL.md"),
    },
    EmbeddedSkill {
        name: "query",
        content: include_str!("../../skills/query/SKILL.md"),
    },
    EmbeddedSkill {
        name: "maintain",
        content: include_str!("../../skills/maintain/SKILL.md"),
    },
    EmbeddedSkill {
        name: "briefing",
        content: include_str!("../../skills/briefing/SKILL.md"),
    },
    EmbeddedSkill {
        name: "alerts",
        content: include_str!("../../skills/alerts/SKILL.md"),
    },
    EmbeddedSkill {
        name: "research",
        content: include_str!("../../skills/research/SKILL.md"),
    },
    EmbeddedSkill {
        name: "upgrade",
        content: include_str!("../../skills/upgrade/SKILL.md"),
    },
    EmbeddedSkill {
        name: "enrich",
        content: include_str!("../../skills/enrich/SKILL.md"),
    },
];

#[derive(Debug, Serialize)]
struct SkillInfo {
    name: String,
    source: String,
    hash: String,
    shadowed: bool,
    #[serde(skip)]
    content: String,
}

pub fn run(action: SkillsAction, json: bool) -> Result<()> {
    match action {
        SkillsAction::List => run_list(json),
        SkillsAction::Doctor => run_doctor(json),
    }
}

fn resolve_skills() -> Vec<SkillInfo> {
    let global_dir = dirs::home_dir().map(|home| home.join(".quaid").join("skills"));
    let local_dir = std::env::current_dir().ok().map(|dir| dir.join("skills"));

    resolve_skills_with_dirs(global_dir, local_dir)
}

fn resolve_skills_with_dirs(
    global_dir: Option<PathBuf>,
    local_dir: Option<PathBuf>,
) -> Vec<SkillInfo> {
    let mut skills = Vec::new();
    let mut resolved: HashMap<String, SkillInfo> = HashMap::new();

    // Layer 1: embedded skills (bundled in binary)
    for skill in EMBEDDED_SKILLS {
        let hash = sha256_hex(skill.content);
        resolved.insert(
            skill.name.to_string(),
            SkillInfo {
                name: skill.name.to_string(),
                source: format!("embedded://skills/{}/SKILL.md", skill.name),
                hash,
                shadowed: false,
                content: skill.content.to_string(),
            },
        );
    }

    // Layer 2: user-global (~/.quaid/skills/)
    if let Some(dir) = global_dir.as_ref() {
        scan_skill_dir(dir, &mut resolved);
    }

    // Layer 3: working directory (./skills/)
    if let Some(dir) = local_dir.as_ref() {
        scan_skill_dir(dir, &mut resolved);
    }

    for skill in EMBEDDED_SKILLS {
        if let Some(info) = resolved.remove(skill.name) {
            skills.push(info);
        }
    }
    // Add any additional non-embedded skills
    let mut extra: Vec<_> = resolved.into_values().collect();
    extra.sort_by(|a, b| a.name.cmp(&b.name));
    skills.extend(extra);

    skills
}

fn scan_skill_dir(dir: &Path, resolved: &mut HashMap<String, SkillInfo>) {
    if !dir.is_dir() {
        return;
    }
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            if !entry.path().is_dir() {
                continue;
            }
            let skill_file = entry.path().join("SKILL.md");
            if skill_file.exists() {
                if let Ok(content) = std::fs::read_to_string(&skill_file) {
                    let name = entry.file_name().to_string_lossy().to_string();
                    let hash = sha256_hex(&content);
                    if let Some(existing) = resolved.get(&name) {
                        if existing.hash == hash {
                            continue;
                        }
                    }
                    let was_shadowed = resolved.contains_key(&name);
                    let display_path = skill_file
                        .canonicalize()
                        .unwrap_or_else(|_| skill_file.clone());
                    resolved.insert(
                        name.clone(),
                        SkillInfo {
                            name,
                            source: display_path.display().to_string(),
                            hash,
                            shadowed: was_shadowed,
                            content,
                        },
                    );
                }
            }
        }
    }
}

fn run_list(json: bool) -> Result<()> {
    let skills = resolve_skills();

    if json {
        println!("{}", serde_json::to_string_pretty(&skills)?);
    } else if skills.is_empty() {
        println!("No skills found.");
    } else {
        for s in &skills {
            let shadow = if s.shadowed { " (shadowed)" } else { "" };
            println!("{:<12} {}{}", s.name, s.source, shadow);
        }
        println!("{} skill(s) resolved.", skills.len());
    }
    Ok(())
}

#[derive(Debug, Serialize)]
struct DoctorResult {
    name: String,
    source: String,
    hash: String,
    shadowed: bool,
    valid_frontmatter: bool,
    has_name: bool,
    has_description: bool,
    issues: Vec<String>,
}

fn run_doctor(json: bool) -> Result<()> {
    let skills = resolve_skills();
    let mut results = Vec::new();
    let mut all_ok = true;

    for s in &skills {
        let mut issues = Vec::new();
        let (valid_fm, has_name, has_desc) = check_frontmatter(&s.content, &mut issues);

        if !issues.is_empty() {
            all_ok = false;
        }

        results.push(DoctorResult {
            name: s.name.clone(),
            source: s.source.clone(),
            hash: s.hash.clone(),
            shadowed: s.shadowed,
            valid_frontmatter: valid_fm,
            has_name,
            has_description: has_desc,
            issues,
        });
    }

    if json {
        println!("{}", serde_json::to_string_pretty(&results)?);
    } else {
        for r in &results {
            let status = if r.issues.is_empty() { "✓" } else { "✗" };
            let shadow = if r.shadowed { " [shadowed]" } else { "" };
            println!(
                "{status} {:<12} {} ({}){shadow}",
                r.name,
                r.source,
                &r.hash[..8]
            );
            for issue in &r.issues {
                println!("    ⚠ {issue}");
            }
        }
        if all_ok {
            println!("All {} skill(s) OK.", results.len());
        } else {
            let bad = results.iter().filter(|r| !r.issues.is_empty()).count();
            println!("{bad} skill(s) with issues out of {} total.", results.len());
        }
    }

    Ok(())
}

fn check_frontmatter(content: &str, issues: &mut Vec<String>) -> (bool, bool, bool) {
    let trimmed = content.trim();
    if !trimmed.starts_with("---") {
        issues.push("missing YAML frontmatter".into());
        return (false, false, false);
    }

    let after_start = &trimmed[3..];
    let end_pos = after_start.find("\n---");
    let fm_text = match end_pos {
        Some(pos) => &after_start[..pos],
        None => {
            issues.push("unclosed frontmatter block".into());
            return (false, false, false);
        }
    };

    let has_name = fm_text.contains("name:");
    let has_desc = fm_text.contains("description:");

    if !has_name {
        issues.push("frontmatter missing 'name' field".into());
    }
    if !has_desc {
        issues.push("frontmatter missing 'description' field".into());
    }

    (true, has_name, has_desc)
}

fn sha256_hex(data: &str) -> String {
    let digest = Sha256::digest(data.as_bytes());
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn test_dir(name: &str) -> PathBuf {
        let base = PathBuf::from("target").join("skill_tests").join(name);
        if base.exists() {
            let _ = std::fs::remove_dir_all(&base);
        }
        std::fs::create_dir_all(&base).unwrap();
        base
    }

    #[test]
    fn embedded_skills_resolve_without_overrides() {
        let skills = resolve_skills_with_dirs(None, None);
        let ingest = skills
            .iter()
            .find(|skill| skill.name == "ingest")
            .expect("embedded ingest skill");
        assert!(ingest.source.starts_with("embedded://"));
        assert!(!ingest.shadowed);
        assert!(!ingest.content.is_empty());
    }

    #[test]
    fn local_overrides_shadow_embedded() {
        let local_dir = test_dir("local_override");
        let ingest_dir = local_dir.join("ingest");
        std::fs::create_dir_all(&ingest_dir).unwrap();
        std::fs::write(
            ingest_dir.join("SKILL.md"),
            "---\nname: ingest\ndescription: override\n---\n",
        )
        .unwrap();

        let skills = resolve_skills_with_dirs(None, Some(local_dir));
        let ingest = skills
            .iter()
            .find(|skill| skill.name == "ingest")
            .expect("ingest skill");
        assert!(ingest.shadowed);
        assert!(ingest.source.contains("skill_tests"));
    }

    #[test]
    fn identical_local_skills_do_not_shadow_embedded() {
        let local_dir = test_dir("local_identical");
        let ingest_dir = local_dir.join("ingest");
        std::fs::create_dir_all(&ingest_dir).unwrap();
        let embedded = EMBEDDED_SKILLS
            .iter()
            .find(|skill| skill.name == "ingest")
            .expect("embedded ingest skill");
        std::fs::write(ingest_dir.join("SKILL.md"), embedded.content).unwrap();

        let skills_with_local = resolve_skills_with_dirs(None, Some(local_dir));
        let skills_without_local = resolve_skills_with_dirs(None, None);
        let ingest_with_local = skills_with_local
            .iter()
            .find(|skill| skill.name == "ingest")
            .expect("ingest skill with local");
        let ingest_without_local = skills_without_local
            .iter()
            .find(|skill| skill.name == "ingest")
            .expect("ingest skill without local");

        assert_eq!(ingest_with_local.source, ingest_without_local.source);
        assert_eq!(ingest_with_local.hash, ingest_without_local.hash);
        assert!(!ingest_with_local.shadowed);
    }

    #[test]
    fn additional_non_embedded_skills_are_appended_in_name_order() {
        let local_dir = test_dir("extra_skills");
        for name in ["zeta", "alpha"] {
            let skill_dir = local_dir.join(name);
            std::fs::create_dir_all(&skill_dir).unwrap();
            std::fs::write(
                skill_dir.join("SKILL.md"),
                format!("---\nname: {name}\ndescription: extra\n---\n"),
            )
            .unwrap();
        }

        let skills = resolve_skills_with_dirs(None, Some(local_dir));
        let extra_names: Vec<_> = skills
            .iter()
            .filter(|skill| !EMBEDDED_SKILLS.iter().any(|embedded| embedded.name == skill.name))
            .map(|skill| skill.name.as_str())
            .collect();

        assert_eq!(extra_names, vec!["alpha", "zeta"]);
    }

    #[test]
    fn check_frontmatter_rejects_missing_yaml_block() {
        let mut issues = Vec::new();

        let (valid, has_name, has_desc) = check_frontmatter("name: ingest", &mut issues);

        assert!(!valid);
        assert!(!has_name);
        assert!(!has_desc);
        assert_eq!(issues, vec!["missing YAML frontmatter"]);
    }

    #[test]
    fn check_frontmatter_rejects_unclosed_block() {
        let mut issues = Vec::new();

        let (valid, has_name, has_desc) =
            check_frontmatter("---\nname: ingest\ndescription: test", &mut issues);

        assert!(!valid);
        assert!(!has_name);
        assert!(!has_desc);
        assert_eq!(issues, vec!["unclosed frontmatter block"]);
    }

    #[test]
    fn check_frontmatter_reports_missing_required_fields() {
        let mut issues = Vec::new();

        let (valid, has_name, has_desc) = check_frontmatter("---\nname: ingest\n---", &mut issues);

        assert!(valid);
        assert!(has_name);
        assert!(!has_desc);
        assert_eq!(issues, vec!["frontmatter missing 'description' field"]);
    }

    #[test]
    fn check_frontmatter_reports_missing_name_field() {
        let mut issues = Vec::new();

        let (valid, has_name, has_desc) =
            check_frontmatter("---\ndescription: test\n---", &mut issues);

        assert!(valid);
        assert!(!has_name);
        assert!(has_desc);
        assert_eq!(issues, vec!["frontmatter missing 'name' field"]);
    }

    #[test]
    fn run_list_outputs_json_and_text() {
        run_list(true).unwrap();
        run_list(false).unwrap();
    }

    #[test]
    fn run_doctor_reports_all_ok_when_no_overrides_are_present() {
        run_doctor(true).unwrap();
        run_doctor(false).unwrap();
    }
}
