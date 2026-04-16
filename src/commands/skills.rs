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

const EMBEDDED_SKILLS: &[&str] = &[
    "ingest", "query", "maintain", "briefing", "alerts", "research", "upgrade", "enrich",
];

#[derive(Debug, Serialize)]
struct SkillInfo {
    name: String,
    source: String,
    hash: String,
    shadowed: bool,
}

pub fn run(action: SkillsAction, json: bool) -> Result<()> {
    match action {
        SkillsAction::List => run_list(json),
        SkillsAction::Doctor => run_doctor(json),
    }
}

fn resolve_skills() -> Vec<SkillInfo> {
    let mut skills = Vec::new();
    let mut resolved: HashMap<String, SkillInfo> = HashMap::new();

    // Layer 1: embedded skills (bundled in binary)
    let embedded_dir = PathBuf::from("skills");
    for &name in EMBEDDED_SKILLS {
        let skill_path = embedded_dir.join(name).join("SKILL.md");
        if skill_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&skill_path) {
                let hash = sha256_hex(&content);
                resolved.insert(
                    name.to_string(),
                    SkillInfo {
                        name: name.to_string(),
                        source: skill_path.display().to_string(),
                        hash,
                        shadowed: false,
                    },
                );
            }
        }
    }

    // Layer 2: user-global (~/.gbrain/skills/)
    if let Some(home) = dirs::home_dir() {
        let global_dir = home.join(".gbrain").join("skills");
        scan_skill_dir(&global_dir, &mut resolved);
    }

    // Layer 3: working directory (./skills/)
    let local_dir = PathBuf::from("./skills");
    scan_skill_dir(&local_dir, &mut resolved);

    for name in EMBEDDED_SKILLS {
        if let Some(info) = resolved.remove(*name) {
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
                    let was_shadowed = resolved.contains_key(&name);
                    resolved.insert(
                        name.clone(),
                        SkillInfo {
                            name,
                            source: skill_file.display().to_string(),
                            hash,
                            shadowed: was_shadowed,
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
        let content = std::fs::read_to_string(&s.source).unwrap_or_default();
        let mut issues = Vec::new();
        let (valid_fm, has_name, has_desc) = check_frontmatter(&content, &mut issues);

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
