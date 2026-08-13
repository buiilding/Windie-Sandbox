//! Generates the compile-time index for curated skill asset directories.
//!
//! Curated skills remain code-owned, but their source layout is intentionally
//! package-shaped. This build step embeds every text file under
//! `src/skills/curated/<skill-id>/` so adding a supporting reference file does
//! not require another hand-written Rust constant.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let curated_root = manifest_dir.join("src/skills/curated");
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").unwrap());

    println!("cargo:rerun-if-changed={}", curated_root.display());

    let mut skills = Vec::new();
    let entries = fs::read_dir(&curated_root).unwrap_or_else(|error| {
        panic!(
            "failed to read curated skills directory {}: {error}",
            curated_root.display()
        )
    });

    for entry in entries {
        let entry = entry.expect("failed to read curated skill entry");
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let skill_id = entry.file_name().to_string_lossy().into_owned();
        let mut files = Vec::new();
        collect_files(&path, &path, &mut files);
        files.sort_by(|left, right| left.0.cmp(&right.0));
        if files.is_empty() {
            panic!("curated skill {skill_id} has no files");
        }

        skills.push((skill_id, files));
    }

    skills.sort_by(|left, right| left.0.cmp(&right.0));

    let mut generated =
        String::from("pub(crate) const EMBEDDED_SKILLS: &[EmbeddedSkillDefinition] = &[\n");
    for (skill_id, files) in skills {
        generated.push_str("    EmbeddedSkillDefinition {\n");
        generated.push_str(&format!("        skill_id: {},\n", rust_string(&skill_id)));
        generated.push_str("        files: &[\n");
        for (relative_path, absolute_path) in files {
            println!("cargo:rerun-if-changed={}", absolute_path.display());
            generated.push_str("            EmbeddedSkillFile {\n");
            generated.push_str(&format!(
                "                path: {},\n",
                rust_string(&relative_path)
            ));
            generated.push_str(&format!(
                "                content: include_str!({}),\n",
                rust_string(&absolute_path.to_string_lossy())
            ));
            generated.push_str("            },\n");
        }
        generated.push_str("        ],\n");
        generated.push_str("    },\n");
    }
    generated.push_str("];\n");

    fs::write(out_dir.join("windie_embedded_skills.rs"), generated)
        .expect("failed to write embedded skill index");
}

fn collect_files(root: &Path, current: &Path, files: &mut Vec<(String, PathBuf)>) {
    let entries = fs::read_dir(current).unwrap_or_else(|error| {
        panic!(
            "failed to read curated skill directory {}: {error}",
            current.display()
        )
    });

    for entry in entries {
        let entry = entry.expect("failed to read curated skill file");
        let path = entry.path();
        if path.is_dir() {
            collect_files(root, &path, files);
            continue;
        }
        if !path.is_file() {
            continue;
        }

        let relative_path = path
            .strip_prefix(root)
            .expect("curated skill path escaped its root")
            .to_string_lossy()
            .replace(std::path::MAIN_SEPARATOR, "/");
        files.push((relative_path, path));
    }
}

fn rust_string(value: &str) -> String {
    format!("{value:?}")
}
