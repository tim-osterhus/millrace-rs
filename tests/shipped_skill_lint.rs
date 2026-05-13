use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn collect_skill_packages(root: &Path) -> Vec<PathBuf> {
    let mut packages = Vec::new();
    collect_skill_packages_inner(root, &mut packages);
    packages.sort();
    packages
}

fn collect_skill_packages_inner(path: &Path, packages: &mut Vec<PathBuf>) {
    if path.join("SKILL.md").is_file() {
        packages.push(path.to_owned());
    }

    let entries = fs::read_dir(path)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
    for entry in entries {
        let entry = entry.unwrap_or_else(|error| {
            panic!(
                "failed to read directory entry under {}: {error}",
                path.display()
            )
        });
        let child = entry.path();
        if child.is_dir() {
            collect_skill_packages_inner(&child, packages);
        }
    }
}

fn python_binary() -> String {
    std::env::var("MILLRACE_PYTHON").unwrap_or_else(|_| "python".to_owned())
}

#[test]
fn every_shipped_skill_package_passes_packaged_skill_lint() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let skills_root = repo_root.join("src/assets/baseline/skills");
    let lint_script = skills_root.join("millrace-skill-creator/scripts/lint_skill.py");
    assert!(lint_script.is_file(), "missing packaged linter");

    let skill_packages = collect_skill_packages(&skills_root);
    assert!(
        !skill_packages.is_empty(),
        "no shipped skill packages found"
    );

    let mut failures = Vec::new();
    for package_path in skill_packages {
        let output = Command::new(python_binary())
            .arg(&lint_script)
            .arg(&package_path)
            .current_dir(repo_root)
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .output()
            .unwrap_or_else(|error| {
                panic!(
                    "failed to run {} for {}: {error}",
                    lint_script.display(),
                    package_path.display()
                )
            });

        if !output.status.success() {
            let relative_path = package_path
                .strip_prefix(repo_root)
                .unwrap_or(&package_path)
                .display();
            failures.push(format!(
                "{relative_path}\nstdout:\n{}\nstderr:\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "shipped skill lint failures:\n\n{}",
        failures.join("\n\n")
    );
}
